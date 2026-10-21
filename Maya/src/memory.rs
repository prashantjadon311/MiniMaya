use crate::artifacts::{ArtifactInput, ArtifactStore};
use crate::db::Db;
use crate::error::{AppError, ErrorCode, Result};
use crate::policy::{ExecutionPermit, Policy};
use crate::types::{
    ActionClass, Actor, Artifact, ArtifactKind, DeleteSelection, DeleteSelectionKind,
    DeletionReceipt, Digest, ExportFormat, ExportSelection, Id, MemoryInput, MemoryItem,
    MemoryKind, MemoryPatch, MemoryQuery, MemorySource, OperationReceipt, Page, Provenance,
    ProvenanceSource, Revision, Task, UtcMs,
};
use chrono::Utc;
use rusqlite::params;
use sha2::{Digest as Sha2Digest, Sha256};
use std::sync::Arc;
use uuid::Uuid;

const MAX_MEMORY_TEXT_BYTES: usize = 8192;
const MAX_EXPORT_BYTES: u64 = 10 * 1024 * 1024; // 10 MB

#[derive(Clone)]
pub struct MemoryService {
    db: Db,
    artifacts: Arc<ArtifactStore>,
}

impl MemoryService {
    pub fn new(db: Db, artifacts: Arc<ArtifactStore>) -> Self {
        Self { db, artifacts }
    }

    pub fn db(&self) -> &Db {
        &self.db
    }

    pub fn artifacts(&self) -> &Arc<ArtifactStore> {
        &self.artifacts
    }

    pub async fn get(&self, id: Id) -> Result<MemoryItem> {
        let id_str = id.to_string();
        self.db
            .read(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, kind, project_id, conversation_id, text, source, revision, created_at, updated_at
                     FROM memory_items WHERE id = ?1",
                )?;
                stmt.query_row(params![id_str], parse_memory_row)
                    .map_err(|e| match e {
                        rusqlite::Error::QueryReturnedNoRows => {
                            AppError::new(ErrorCode::NotFound, format!("Memory item {} not found", id))
                        }
                        other => AppError::from(other),
                    })
            })
            .await
    }

    pub async fn search(&self, query: MemoryQuery) -> Result<Page<MemoryItem>> {
        let limit = if query.limit == 0 {
            50
        } else {
            query.limit.clamp(1, 100)
        } as usize;

        let cursor_ms = query
            .cursor
            .as_deref()
            .and_then(|s| s.parse::<UtcMs>().ok());

        self.db
            .read(move |conn| {
                let mut sql = "SELECT id, kind, project_id, conversation_id, text, source, revision, created_at, updated_at FROM memory_items WHERE 1=1".to_string();
                let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

                if let Some(ref q) = query.q
                    && !q.trim().is_empty() {
                        sql.push_str(" AND text LIKE ?");
                        params_vec.push(Box::new(format!("%{}%", q.trim())));
                    }

                if let Some(k) = query.kind {
                    sql.push_str(" AND kind = ?");
                    params_vec.push(Box::new(memory_kind_to_str(k)));
                }

                if let Some(pid) = query.project_id {
                    sql.push_str(" AND project_id = ?");
                    params_vec.push(Box::new(pid.to_string()));
                }

                if let Some(cid) = query.conversation_id {
                    sql.push_str(" AND conversation_id = ?");
                    params_vec.push(Box::new(cid.to_string()));
                }

                if let Some(ms) = cursor_ms {
                    sql.push_str(" AND created_at < ?");
                    params_vec.push(Box::new(ms));
                }

                sql.push_str(" ORDER BY created_at DESC LIMIT ?");
                params_vec.push(Box::new((limit + 1) as i64));

                let mut stmt = conn.prepare(&sql)?;
                let rusqlite_params: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|b| b.as_ref()).collect();
                let mut rows = stmt.query(rusqlite_params.as_slice())?;
                let mut items = Vec::new();

                while let Some(row) = rows.next()? {
                    items.push(parse_memory_row(row)?);
                }

                let next_cursor = if items.len() > limit {
                    items.pop();
                    items.last().map(|m| m.created_at.to_string())
                } else {
                    None
                };

                Ok(Page {
                    items,
                    next_cursor,
                })
            })
            .await
    }

    pub async fn upsert(&self, input: MemoryInput) -> Result<MemoryItem> {
        let trimmed_text = input.text.trim();
        if trimmed_text.is_empty() {
            return Err(AppError::new(
                ErrorCode::InvalidRequest,
                "Memory text cannot be empty",
            ));
        }
        if trimmed_text.len() > MAX_MEMORY_TEXT_BYTES {
            return Err(AppError::new(
                ErrorCode::InvalidRequest,
                format!("Memory text exceeds {} byte limit", MAX_MEMORY_TEXT_BYTES),
            ));
        }

        let safe_text = Policy::redact_static(trimmed_text);
        let id = Uuid::new_v4();
        let now = Utc::now().timestamp_millis();
        let kind_str = memory_kind_to_str(input.kind);

        self.db
            .write(move |tx| {
                if let Some(pid) = input.project_id {
                    let mut stmt = tx.prepare("SELECT id FROM projects WHERE id = ?1")?;
                    let _exists: String = stmt
                        .query_row(params![pid.to_string()], |r| r.get(0))
                        .map_err(|e| match e {
                            rusqlite::Error::QueryReturnedNoRows => {
                                AppError::new(ErrorCode::NotFound, format!("Project {} not found", pid))
                            }
                            other => AppError::from(other),
                        })?;
                }

                if let Some(cid) = input.conversation_id {
                    let mut stmt = tx.prepare("SELECT id FROM conversations WHERE id = ?1")?;
                    let _exists: String = stmt
                        .query_row(params![cid.to_string()], |r| r.get(0))
                        .map_err(|e| match e {
                            rusqlite::Error::QueryReturnedNoRows => AppError::new(
                                ErrorCode::NotFound,
                                format!("Conversation {} not found", cid),
                            ),
                            other => AppError::from(other),
                        })?;
                }

                tx.execute(
                    "INSERT INTO memory_items (id, kind, project_id, conversation_id, text, source, revision, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, 'owner', 1, ?6, ?6)",
                    params![
                        id.to_string(),
                        kind_str,
                        input.project_id.map(|p| p.to_string()),
                        input.conversation_id.map(|c| c.to_string()),
                        safe_text,
                        now
                    ],
                )?;

                Ok(MemoryItem {
                    id,
                    kind: input.kind,
                    project_id: input.project_id,
                    conversation_id: input.conversation_id,
                    text: safe_text,
                    source: MemorySource::Owner,
                    revision: 1,
                    created_at: now,
                    updated_at: now,
                })
            })
            .await
    }

    pub async fn patch(&self, id: Id, input: MemoryPatch, _actor: Actor) -> Result<MemoryItem> {
        let trimmed_text = input.text.trim();
        if trimmed_text.is_empty() {
            return Err(AppError::new(
                ErrorCode::InvalidRequest,
                "Memory text cannot be empty",
            ));
        }
        if trimmed_text.len() > MAX_MEMORY_TEXT_BYTES {
            return Err(AppError::new(
                ErrorCode::InvalidRequest,
                format!("Memory text exceeds {} byte limit", MAX_MEMORY_TEXT_BYTES),
            ));
        }

        let safe_text = Policy::redact_static(trimmed_text);
        let id_str = id.to_string();
        let now = Utc::now().timestamp_millis();

        self.db
            .write(move |tx| {
                let current = {
                    let mut stmt = tx.prepare(
                        "SELECT id, kind, project_id, conversation_id, text, source, revision, created_at, updated_at
                         FROM memory_items WHERE id = ?1",
                    )?;
                    stmt.query_row(params![id_str], parse_memory_row)
                        .map_err(|e| match e {
                            rusqlite::Error::QueryReturnedNoRows => {
                                AppError::new(ErrorCode::NotFound, format!("Memory item {} not found", id))
                            }
                            other => AppError::from(other),
                        })?
                };

                if current.revision != input.revision {
                    return Err(AppError::new(
                        ErrorCode::Conflict,
                        format!(
                            "Stale revision: current is {}, patch expected {}",
                            current.revision, input.revision
                        ),
                    ));
                }

                let new_revision = current.revision + 1;
                tx.execute(
                    "UPDATE memory_items
                     SET text = ?1, revision = ?2, updated_at = ?3
                     WHERE id = ?4 AND revision = ?5",
                    params![safe_text, new_revision, now, id.to_string(), current.revision],
                )?;

                Ok(MemoryItem {
                    id,
                    kind: current.kind,
                    project_id: current.project_id,
                    conversation_id: current.conversation_id,
                    text: safe_text,
                    source: current.source,
                    revision: new_revision,
                    created_at: current.created_at,
                    updated_at: now,
                })
            })
            .await
    }

    pub async fn delete_preview(&self, selection: DeleteSelection) -> Result<OperationReceipt> {
        if selection.ids.is_empty() {
            return Err(AppError::new(
                ErrorCode::InvalidRequest,
                "Selection ids cannot be empty",
            ));
        }
        if selection.ids.len() > 100 {
            return Err(AppError::new(
                ErrorCode::InvalidRequest,
                "Cannot delete more than 100 items at once",
            ));
        }

        let count = selection.ids.len() as u64;
        let mut hasher = Sha256::new();
        hasher.update(format!("{:?}", selection.kind).as_bytes());
        for id in &selection.ids {
            hasher.update(id.as_bytes());
        }
        let _digest_bytes: [u8; 32] = hasher.finalize().into();

        Ok(OperationReceipt {
            job_id: Uuid::new_v4(),
            state: crate::types::JobState::AwaitingApproval,
            result: None,
            approval: None,
            preview: format!("{:?}: {} items", selection.kind, count),
        })
    }

    pub async fn export(
        &self,
        selection: ExportSelection,
        permit: &ExecutionPermit,
    ) -> Result<Artifact> {
        permit.permits(&ActionClass::ExportData, None)?;

        let mut export_doc = serde_json::Map::new();

        if selection.settings {
            let settings_json = self
                .db
                .read(|conn| {
                    let mut stmt = conn.prepare("SELECT key, value_json FROM settings")?;
                    let mut map = serde_json::Map::new();
                    let rows = stmt
                        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
                    for r in rows {
                        let (k, v_str) = r?;
                        if !k.contains("secret")
                            && !k.contains("credential")
                            && !k.contains("token")
                        {
                            let val: serde_json::Value = serde_json::from_str(&v_str)
                                .unwrap_or(serde_json::Value::String(v_str));
                            map.insert(k, val);
                        }
                    }
                    Ok(serde_json::Value::Object(map))
                })
                .await?;
            export_doc.insert("settings".to_string(), settings_json);
        }

        if !selection.memory.is_empty() {
            let mem_ids = selection.memory.clone();
            let items = self
                .db
                .read(move |conn| {
                    let mut res = Vec::new();
                    for id in mem_ids {
                        let mut stmt = conn.prepare(
                            "SELECT id, kind, project_id, conversation_id, text, source, revision, created_at, updated_at
                             FROM memory_items WHERE id = ?1",
                        )?;
                        if let Ok(item) = stmt.query_row(params![id.to_string()], parse_memory_row) {
                            res.push(item);
                        }
                    }
                    Ok(res)
                })
                .await?;
            export_doc.insert(
                "memory".to_string(),
                serde_json::to_value(&items).unwrap_or_default(),
            );
        }

        if !selection.tasks.is_empty() || selection.all_tasks == Some(true) {
            let task_ids = selection.tasks.clone();
            let all = selection.all_tasks == Some(true);
            let tasks = self
                .db
                .read(move |conn| {
                    let mut res = Vec::new();
                    if all {
                        let mut stmt = conn.prepare(
                            "SELECT id, parent_task_id, project_id, title, description, priority, status, progress_pct, progress_source, due_at, timezone, estimated_minutes, revision, created_at, updated_at, completed_at
                             FROM tasks LIMIT 500",
                        )?;
                        let rows = stmt.query_map([], crate::tasks::parse_task_row)?;
                        for r in rows {
                            res.push(r?);
                        }
                    } else {
                        for id in task_ids {
                            let mut stmt = conn.prepare(
                                "SELECT id, parent_task_id, project_id, title, description, priority, status, progress_pct, progress_source, due_at, timezone, estimated_minutes, revision, created_at, updated_at, completed_at
                                 FROM tasks WHERE id = ?1",
                            )?;
                            if let Ok(task) = stmt.query_row(params![id.to_string()], crate::tasks::parse_task_row) {
                                res.push(task);
                            }
                        }
                    }
                    Ok(res)
                })
                .await?;

            let sanitized_tasks: Vec<Task> = tasks
                .into_iter()
                .map(|mut t| {
                    if !selection.include_project_paths {
                        t.project_id = None;
                    }
                    t
                })
                .collect();

            export_doc.insert(
                "tasks".to_string(),
                serde_json::to_value(&sanitized_tasks).unwrap_or_default(),
            );
        }

        let (data_bytes, mime) = match selection.format {
            ExportFormat::Json => {
                let bytes = serde_json::to_vec_pretty(&export_doc).map_err(|e| {
                    AppError::new(
                        ErrorCode::Internal,
                        format!("Failed to serialize export: {e}"),
                    )
                })?;
                (bytes, "application/json".to_string())
            }
            ExportFormat::Markdown => {
                let mut md = String::new();
                md.push_str("# Maya Export\n\n");
                if let Some(mem_val) = export_doc.get("memory") {
                    md.push_str("## Memory Items\n\n");
                    if let Some(arr) = mem_val.as_array() {
                        for item in arr {
                            if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                                md.push_str(&format!("- {}\n", text));
                            }
                        }
                    }
                    md.push('\n');
                }
                if let Some(task_val) = export_doc.get("tasks") {
                    md.push_str("## Tasks\n\n");
                    if let Some(arr) = task_val.as_array() {
                        for item in arr {
                            if let Some(title) = item.get("title").and_then(|t| t.as_str()) {
                                let status = item
                                    .get("status")
                                    .and_then(|s| s.as_str())
                                    .unwrap_or("unknown");
                                md.push_str(&format!("- [{}] {}\n", status, title));
                            }
                        }
                    }
                    md.push('\n');
                }
                (md.into_bytes(), "text/markdown".to_string())
            }
        };

        let now = Utc::now().timestamp_millis();
        let artifact_input = ArtifactInput {
            job_id: None,
            task_id: None,
            kind: ArtifactKind::Export,
            mime,
            provenance: Provenance {
                source: ProvenanceSource::Owner,
                provider_id: None,
                model_id: None,
                url: None,
                observed_at: now,
            },
            max_bytes: MAX_EXPORT_BYTES,
            expires_at: None,
        };

        self.artifacts.put_bytes(artifact_input, &data_bytes).await
    }

    pub async fn purge(
        &self,
        selection: DeleteSelection,
        permit: &ExecutionPermit,
    ) -> Result<DeletionReceipt> {
        permit.permits(&ActionClass::DeleteHistory, None)?;

        if selection.ids.is_empty() {
            return Err(AppError::new(
                ErrorCode::InvalidRequest,
                "Selection ids cannot be empty",
            ));
        }
        if selection.ids.len() > 100 {
            return Err(AppError::new(
                ErrorCode::InvalidRequest,
                "Cannot delete more than 100 items at once",
            ));
        }

        let mut hasher = Sha256::new();
        hasher.update(format!("{:?}", selection.kind).as_bytes());
        let mut sorted_ids = selection.ids.clone();
        sorted_ids.sort();
        for id in &sorted_ids {
            hasher.update(id.as_bytes());
        }
        let selection_digest: [u8; 32] = hasher.finalize().into();

        let (deleted_count, retained_audit_count, backup_copies_remain) = self
            .db
            .write(move |tx| {
                let mut count = 0u64;

                match selection.kind {
                    DeleteSelectionKind::MemoryItems => {
                        for id in &sorted_ids {
                            let deleted = tx.execute(
                                "DELETE FROM memory_items WHERE id = ?1",
                                params![id.to_string()],
                            )?;
                            count += deleted as u64;
                        }
                    }
                    DeleteSelectionKind::ChatHistory => {
                        for cid in &sorted_ids {
                            // Check active jobs
                            let mut job_stmt = tx.prepare(
                                "SELECT COUNT(*) FROM jobs
                                 WHERE conversation_id = ?1
                                   AND state NOT IN ('completed', 'completed_with_issues', 'failed', 'cancelled')",
                            )?;
                            let active_jobs: i64 = job_stmt.query_row(params![cid.to_string()], |r| r.get(0))?;
                            if active_jobs > 0 {
                                return Err(AppError::new(
                                    ErrorCode::ResourceInUse,
                                    format!("Conversation {} is in use by active jobs", cid),
                                ));
                            }

                            // Delete messages
                            let deleted = tx.execute(
                                "DELETE FROM messages WHERE conversation_id = ?1",
                                params![cid.to_string()],
                            )?;
                            count += deleted as u64;

                            // Clear summary
                            tx.execute(
                                "UPDATE conversations SET summary = '' WHERE id = ?1",
                                params![cid.to_string()],
                            )?;
                        }
                    }
                    DeleteSelectionKind::TaskHistory => {
                        for tid in &sorted_ids {
                            let mut task_stmt = tx.prepare("SELECT status FROM tasks WHERE id = ?1")?;
                            let status_str: String = task_stmt
                                .query_row(params![tid.to_string()], |r| r.get(0))
                                .map_err(|e| match e {
                                    rusqlite::Error::QueryReturnedNoRows => AppError::new(
                                        ErrorCode::NotFound,
                                        format!("Task {} not found", tid),
                                    ),
                                    other => AppError::from(other),
                                })?;

                            if status_str != "done" && status_str != "cancelled" {
                                return Err(AppError::new(
                                    ErrorCode::InvalidRequest,
                                    format!("Cannot delete active task {} (status: {})", tid, status_str),
                                ));
                            }

                            if selection.include_subtasks {
                                let mut sub_stmt = tx.prepare("SELECT id, status FROM tasks WHERE parent_task_id = ?1")?;
                                let sub_rows = sub_stmt.query_map(params![tid.to_string()], |r| {
                                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                                })?;
                                for r in sub_rows {
                                    let (sub_id, sub_status) = r?;
                                    if sub_status != "done" && sub_status != "cancelled" {
                                        return Err(AppError::new(
                                            ErrorCode::InvalidRequest,
                                            format!("Cannot delete active subtask {} of parent {}", sub_id, tid),
                                        ));
                                    }
                                    tx.execute("DELETE FROM task_events WHERE task_id = ?1", params![sub_id])?;
                                    tx.execute("DELETE FROM checkins WHERE task_id = ?1", params![sub_id])?;
                                    tx.execute("DELETE FROM checklist_items WHERE task_id = ?1", params![sub_id])?;
                                    tx.execute("DELETE FROM schedules WHERE task_id = ?1", params![sub_id])?;
                                    count += tx.execute("DELETE FROM tasks WHERE id = ?1", params![sub_id])? as u64;
                                }
                            }

                            tx.execute("DELETE FROM task_events WHERE task_id = ?1", params![tid.to_string()])?;
                            tx.execute("DELETE FROM checkins WHERE task_id = ?1", params![tid.to_string()])?;
                            tx.execute("DELETE FROM checklist_items WHERE task_id = ?1", params![tid.to_string()])?;
                            tx.execute("DELETE FROM schedules WHERE task_id = ?1", params![tid.to_string()])?;
                            count += tx.execute("DELETE FROM tasks WHERE id = ?1", params![tid.to_string()])? as u64;
                        }
                    }
                }

                let mut audit_stmt = tx.prepare("SELECT COUNT(*) FROM audit_events")?;
                let audit_count: i64 = audit_stmt.query_row([], |r| r.get(0))?;

                let mut backup_stmt = tx.prepare("SELECT COUNT(*) FROM backups")?;
                let backup_count: i64 = backup_stmt.query_row([], |r| r.get(0))?;

                Ok((count, audit_count as u64, backup_count > 0))
            })
            .await?;

        Ok(DeletionReceipt {
            selection_digest: Digest::new(selection_digest),
            deleted_count,
            retained_audit_count,
            backup_copies_remain,
        })
    }
}

fn parse_memory_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MemoryItem> {
    let id_str: String = row.get(0)?;
    let kind_str: String = row.get(1)?;
    let project_id_str: Option<String> = row.get(2)?;
    let conversation_id_str: Option<String> = row.get(3)?;
    let text: String = row.get(4)?;
    let source_str: String = row.get(5)?;
    let revision: Revision = row.get(6)?;
    let created_at: UtcMs = row.get(7)?;
    let updated_at: UtcMs = row.get(8)?;

    let kind = match kind_str.as_str() {
        "preference" => MemoryKind::Preference,
        "decision" => MemoryKind::Decision,
        "note" => MemoryKind::Note,
        "summary" => MemoryKind::Summary,
        _ => MemoryKind::Note,
    };

    let source = match source_str.as_str() {
        "owner" => MemorySource::Owner,
        "conversation_summary" => MemorySource::ConversationSummary,
        "task_event" => MemorySource::TaskEvent,
        _ => MemorySource::Owner,
    };

    Ok(MemoryItem {
        id: Uuid::parse_str(&id_str).unwrap_or_default(),
        kind,
        project_id: project_id_str.and_then(|s| Uuid::parse_str(&s).ok()),
        conversation_id: conversation_id_str.and_then(|s| Uuid::parse_str(&s).ok()),
        text,
        source,
        revision,
        created_at,
        updated_at,
    })
}

fn memory_kind_to_str(k: MemoryKind) -> &'static str {
    match k {
        MemoryKind::Preference => "preference",
        MemoryKind::Decision => "decision",
        MemoryKind::Note => "note",
        MemoryKind::Summary => "summary",
    }
}
