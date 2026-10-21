use crate::db::Db;
use crate::error::{AppError, ErrorCode, Result};
use crate::types::{Actor, AuditEvent, AuditInput, AuditMetadata, Id, Page, StreamEvent, UtcMs};
use rusqlite::{Transaction, params};
use std::sync::Arc;
use tokio::sync::broadcast;

#[derive(Debug, Clone, Default)]
pub struct AuditQuery {
    pub limit: Option<u16>,
    pub cursor: Option<String>,
    pub job_id: Option<Id>,
    pub action: Option<String>,
    pub since: Option<UtcMs>,
}

#[derive(Clone, Debug)]
pub struct EventHub {
    tx: broadcast::Sender<StreamEvent>,
}

impl EventHub {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(256);
        Self { tx }
    }

    pub fn publish(&self, event: StreamEvent) {
        let _ = self.tx.send(event);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<StreamEvent> {
        self.tx.subscribe()
    }
}

impl Default for EventHub {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug)]
pub struct AuditLog {
    db: Db,
    hub: Arc<EventHub>,
}

impl AuditLog {
    pub fn new(db: Db, hub: Arc<EventHub>) -> Self {
        Self { db, hub }
    }

    pub fn hub(&self) -> Arc<EventHub> {
        self.hub.clone()
    }

    pub fn append(tx: &Transaction<'_>, event: AuditInput) -> Result<i64> {
        let actor_json = serde_json::to_string(&event.actor).map_err(|e| {
            AppError::new(
                ErrorCode::Internal,
                format!("Failed to serialize actor: {e}"),
            )
        })?;
        let metadata_json = serde_json::to_string(&event.metadata).map_err(|e| {
            AppError::new(
                ErrorCode::Internal,
                format!("Failed to serialize audit metadata: {e}"),
            )
        })?;

        let job_id_str = event.job_id.map(|id| id.to_string());
        let res_id_str = event.resource_id.map(|id| id.to_string());

        tx.execute(
            r#"INSERT INTO audit_events (job_id, actor_json, action, resource_kind, resource_id, decision, metadata_json, at)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"#,
            params![
                job_id_str,
                actor_json,
                event.action,
                event.resource_kind,
                res_id_str,
                event.decision,
                metadata_json,
                event.at,
            ],
        )
        .map_err(|e| AppError::new(ErrorCode::DbUnavailable, e.to_string()))?;

        let seq = tx.last_insert_rowid();
        Ok(seq)
    }

    pub async fn list(&self, query: AuditQuery) -> Result<Page<AuditEvent>> {
        let limit = query.limit.unwrap_or(50).clamp(1, 100) as usize;
        let cursor_seq: Option<i64> = query.cursor.as_deref().and_then(|c| c.parse::<i64>().ok());

        self.db
            .read(move |conn| {
                let mut sql = String::from(
                    r#"SELECT seq, job_id, actor_json, action, resource_kind, resource_id, decision, metadata_json, at
                       FROM audit_events WHERE 1=1"#,
                );
                let mut params_vec: Vec<rusqlite::types::Value> = Vec::new();

                if let Some(c) = cursor_seq {
                    sql.push_str(" AND seq > ?");
                    params_vec.push(c.into());
                }
                if let Some(job_id) = query.job_id {
                    sql.push_str(" AND job_id = ?");
                    params_vec.push(job_id.to_string().into());
                }
                if let Some(action) = query.action {
                    sql.push_str(" AND action = ?");
                    params_vec.push(action.into());
                }
                if let Some(since) = query.since {
                    sql.push_str(" AND at >= ?");
                    params_vec.push(since.into());
                }

                sql.push_str(&format!(" ORDER BY seq ASC LIMIT {}", limit + 1));

                let mut stmt = conn
                    .prepare(&sql)
                    .map_err(|e| AppError::new(ErrorCode::DbUnavailable, e.to_string()))?;

                let param_refs: Vec<&dyn rusqlite::ToSql> = params_vec
                    .iter()
                    .map(|v| v as &dyn rusqlite::ToSql)
                    .collect();

                let rows = stmt
                    .query_map(&param_refs[..], |r| {
                        let seq: i64 = r.get(0)?;
                        let job_id_str: Option<String> = r.get(1)?;
                        let actor_json: String = r.get(2)?;
                        let action: String = r.get(3)?;
                        let resource_kind: String = r.get(4)?;
                        let res_id_str: Option<String> = r.get(5)?;
                        let decision: String = r.get(6)?;
                        let metadata_json: String = r.get(7)?;
                        let at: UtcMs = r.get(8)?;

                        let job_id = job_id_str.and_then(|s| s.parse::<Id>().ok());
                        let resource_id = res_id_str.and_then(|s| s.parse::<Id>().ok());
                        let actor: Actor = serde_json::from_str(&actor_json).unwrap_or(Actor {
                            kind: crate::types::ActorKind::System,
                            id: None,
                        });
                        let metadata: AuditMetadata =
                            serde_json::from_str(&metadata_json).unwrap_or_default();

                        Ok(AuditEvent {
                            seq,
                            job_id,
                            actor,
                            action,
                            resource_kind,
                            resource_id,
                            decision,
                            metadata,
                            at,
                        })
                    })
                    .map_err(|e| AppError::new(ErrorCode::DbUnavailable, e.to_string()))?;

                let mut items = Vec::new();
                for r in rows {
                    items.push(r.map_err(|e| AppError::new(ErrorCode::DbUnavailable, e.to_string()))?);
                }

                let next_cursor = if items.len() > limit {
                    items.pop();
                    items.last().map(|last| last.seq.to_string())
                } else {
                    None
                };

                Ok(Page { items, next_cursor })
            })
            .await
    }

    pub async fn replay(
        &self,
        after: i64,
        job: Option<Id>,
        limit: u16,
    ) -> Result<Page<AuditEvent>> {
        self.list(AuditQuery {
            limit: Some(limit),
            cursor: Some(after.to_string()),
            job_id: job,
            action: None,
            since: None,
        })
        .await
    }
}
