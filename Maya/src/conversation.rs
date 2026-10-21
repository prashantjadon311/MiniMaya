use crate::db::Db;
use crate::error::{AppError, ErrorCode, Result};
use crate::policy::Policy;
use crate::types::{
    Actor, Conversation, ConversationInput, ConversationPatch, Id, ListQuery, Message, MessageRole,
    ModelMessage, ModelMessageRole, Page, Revision, UserMessage, UtcMs,
};
use chrono::Utc;
use rusqlite::params;
use uuid::Uuid;

const MAX_ACTIVE_MESSAGES: usize = 20;
const MAX_HISTORY_BYTES: usize = 64 * 1024;
const MAX_SUMMARY_BYTES: usize = 8192;
const MAX_MESSAGE_TEXT_BYTES: usize = 65536;

#[derive(Clone)]
pub struct ConversationService {
    db: Db,
}

impl ConversationService {
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    pub fn db(&self) -> &Db {
        &self.db
    }

    pub async fn create(&self, input: ConversationInput, _actor: Actor) -> Result<Conversation> {
        let now = Utc::now().timestamp_millis();
        let id = Uuid::new_v4();

        self.db
            .write(move |tx| {
                if let Some(pid) = input.project_id {
                    let mut stmt = tx.prepare(
                        "SELECT archived FROM projects WHERE id = ?1",
                    )?;
                    let archived: i64 = stmt
                        .query_row(params![pid.to_string()], |row| row.get(0))
                        .map_err(|e| match e {
                            rusqlite::Error::QueryReturnedNoRows => AppError::new(
                                ErrorCode::NotFound,
                                format!("Project {} not found", pid),
                            ),
                            other => AppError::from(other),
                        })?;
                    if archived != 0 {
                        return Err(AppError::new(
                            ErrorCode::InvalidRequest,
                            "Cannot attach conversation to archived project",
                        ));
                    }
                }

                if let Some(mid) = input.model_override_id {
                    let mut stmt = tx.prepare(
                        "SELECT enabled FROM models WHERE id = ?1",
                    )?;
                    let enabled: i64 = stmt
                        .query_row(params![mid.to_string()], |row| row.get(0))
                        .map_err(|e| match e {
                            rusqlite::Error::QueryReturnedNoRows => AppError::new(
                                ErrorCode::ModelUnavailable,
                                format!("Model {} not found", mid),
                            ),
                            other => AppError::from(other),
                        })?;
                    if enabled == 0 {
                        return Err(AppError::new(
                            ErrorCode::ModelUnavailable,
                            format!("Model {} is disabled", mid),
                        ));
                    }
                }

                if let Some(tid) = input.current_task_id {
                    let mut stmt = tx.prepare(
                        "SELECT project_id FROM tasks WHERE id = ?1",
                    )?;
                    let task_project_id: Option<String> = stmt
                        .query_row(params![tid.to_string()], |row| row.get(0))
                        .map_err(|e| match e {
                            rusqlite::Error::QueryReturnedNoRows => AppError::new(
                                ErrorCode::NotFound,
                                format!("Task {} not found", tid),
                            ),
                            other => AppError::from(other),
                        })?;

                    if let Some(pid) = input.project_id {
                        let task_pid = task_project_id.and_then(|s| Uuid::parse_str(&s).ok());
                        if task_pid != Some(pid) {
                            return Err(AppError::new(
                                ErrorCode::Conflict,
                                "Task project does not match conversation project",
                            ));
                        }
                    }
                }

                tx.execute(
                    "INSERT INTO conversations (id, project_id, model_override_id, current_task_id, summary, revision, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, '', 1, ?5, ?5)",
                    params![
                        id.to_string(),
                        input.project_id.map(|u| u.to_string()),
                        input.model_override_id.map(|u| u.to_string()),
                        input.current_task_id.map(|u| u.to_string()),
                        now
                    ],
                )?;

                Ok(Conversation {
                    id,
                    project_id: input.project_id,
                    model_override_id: input.model_override_id,
                    current_task_id: input.current_task_id,
                    summary: String::new(),
                    created_at: now,
                    updated_at: now,
                    revision: 1,
                })
            })
            .await
    }

    pub async fn get(&self, id: Id) -> Result<Conversation> {
        let id_str = id.to_string();
        self.db
            .read(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, project_id, model_override_id, current_task_id, summary, revision, created_at, updated_at
                     FROM conversations WHERE id = ?1",
                )?;
                stmt.query_row(params![id_str], parse_conversation_row)
                    .map_err(|e| match e {
                        rusqlite::Error::QueryReturnedNoRows => {
                            AppError::new(ErrorCode::NotFound, format!("Conversation {} not found", id))
                        }
                        other => AppError::from(other),
                    })
            })
            .await
    }

    pub async fn list(&self, q: ListQuery) -> Result<Page<Conversation>> {
        let limit = q.limit.unwrap_or(50).clamp(1, 100) as usize;
        let cursor_ms = q.cursor.as_deref().and_then(|s| s.parse::<UtcMs>().ok());

        self.db
            .read(move |conn| {
                let mut sql = "SELECT id, project_id, model_override_id, current_task_id, summary, revision, created_at, updated_at FROM conversations".to_string();
                let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

                if let Some(ms) = cursor_ms {
                    sql.push_str(" WHERE created_at < ?1 ORDER BY created_at DESC LIMIT ?2");
                    params_vec.push(Box::new(ms));
                    params_vec.push(Box::new((limit + 1) as i64));
                } else {
                    sql.push_str(" ORDER BY created_at DESC LIMIT ?1");
                    params_vec.push(Box::new((limit + 1) as i64));
                }

                let mut stmt = conn.prepare(&sql)?;
                let rusqlite_params: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|b| b.as_ref()).collect();
                let mut rows = stmt.query(rusqlite_params.as_slice())?;
                let mut items = Vec::new();

                while let Some(row) = rows.next()? {
                    items.push(parse_conversation_row(row)?);
                }

                let next_cursor = if items.len() > limit {
                    items.pop();
                    items.last().map(|c| c.created_at.to_string())
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

    pub async fn patch(
        &self,
        id: Id,
        patch: ConversationPatch,
        _actor: Actor,
    ) -> Result<Conversation> {
        let now = Utc::now().timestamp_millis();
        let id_str = id.to_string();

        self.db
            .write(move |tx| {
                let current = {
                    let mut stmt = tx.prepare(
                        "SELECT id, project_id, model_override_id, current_task_id, summary, revision, created_at, updated_at
                         FROM conversations WHERE id = ?1",
                    )?;
                    stmt.query_row(params![id_str], parse_conversation_row)
                        .map_err(|e| match e {
                            rusqlite::Error::QueryReturnedNoRows => {
                                AppError::new(ErrorCode::NotFound, format!("Conversation {} not found", id))
                            }
                            other => AppError::from(other),
                        })?
                };

                if current.revision != patch.revision {
                    return Err(AppError::new(
                        ErrorCode::Conflict,
                        format!(
                            "Stale revision: current is {}, patch expected {}",
                            current.revision, patch.revision
                        ),
                    ));
                }

                let new_project_id = match patch.project_id {
                    Some(val) => val,
                    None => current.project_id,
                };

                if let Some(pid) = new_project_id {
                    let mut stmt = tx.prepare("SELECT archived FROM projects WHERE id = ?1")?;
                    let archived: i64 = stmt
                        .query_row(params![pid.to_string()], |row| row.get(0))
                        .map_err(|e| match e {
                            rusqlite::Error::QueryReturnedNoRows => AppError::new(
                                ErrorCode::NotFound,
                                format!("Project {} not found", pid),
                            ),
                            other => AppError::from(other),
                        })?;
                    if archived != 0 {
                        return Err(AppError::new(
                            ErrorCode::InvalidRequest,
                            "Cannot set conversation to archived project",
                        ));
                    }
                }

                let new_model_id = match patch.model_override_id {
                    Some(val) => val,
                    None => current.model_override_id,
                };

                if let Some(mid) = new_model_id {
                    let mut stmt = tx.prepare("SELECT enabled FROM models WHERE id = ?1")?;
                    let enabled: i64 = stmt
                        .query_row(params![mid.to_string()], |row| row.get(0))
                        .map_err(|e| match e {
                            rusqlite::Error::QueryReturnedNoRows => AppError::new(
                                ErrorCode::ModelUnavailable,
                                format!("Model {} not found", mid),
                            ),
                            other => AppError::from(other),
                        })?;
                    if enabled == 0 {
                        return Err(AppError::new(
                            ErrorCode::ModelUnavailable,
                            format!("Model {} is disabled", mid),
                        ));
                    }
                }

                let new_task_id = match patch.current_task_id {
                    Some(val) => val,
                    None => current.current_task_id,
                };

                if let Some(tid) = new_task_id {
                    let mut stmt = tx.prepare("SELECT project_id FROM tasks WHERE id = ?1")?;
                    let task_pid_str: Option<String> = stmt
                        .query_row(params![tid.to_string()], |row| row.get(0))
                        .map_err(|e| match e {
                            rusqlite::Error::QueryReturnedNoRows => AppError::new(
                                ErrorCode::NotFound,
                                format!("Task {} not found", tid),
                            ),
                            other => AppError::from(other),
                        })?;

                    if let Some(pid) = new_project_id {
                        let task_pid = task_pid_str.and_then(|s| Uuid::parse_str(&s).ok());
                        if task_pid != Some(pid) {
                            return Err(AppError::new(
                                ErrorCode::Conflict,
                                "Task project does not match conversation project",
                            ));
                        }
                    }
                }

                let new_revision = current.revision + 1;
                tx.execute(
                    "UPDATE conversations
                     SET project_id = ?1, model_override_id = ?2, current_task_id = ?3, revision = ?4, updated_at = ?5
                     WHERE id = ?6 AND revision = ?7",
                    params![
                        new_project_id.map(|u| u.to_string()),
                        new_model_id.map(|u| u.to_string()),
                        new_task_id.map(|u| u.to_string()),
                        new_revision,
                        now,
                        id.to_string(),
                        current.revision
                    ],
                )?;

                Ok(Conversation {
                    id,
                    project_id: new_project_id,
                    model_override_id: new_model_id,
                    current_task_id: new_task_id,
                    summary: current.summary,
                    created_at: current.created_at,
                    updated_at: now,
                    revision: new_revision,
                })
            })
            .await
    }

    pub async fn append(&self, id: Id, input: UserMessage) -> Result<Message> {
        if input.text.is_empty() {
            return Err(AppError::new(
                ErrorCode::InvalidRequest,
                "Message text cannot be empty",
            ));
        }
        if input.text.len() > MAX_MESSAGE_TEXT_BYTES {
            return Err(AppError::new(
                ErrorCode::InvalidRequest,
                format!("Message text exceeds {} byte limit", MAX_MESSAGE_TEXT_BYTES),
            ));
        }

        let id_str = id.to_string();
        let message_id = Uuid::new_v4();
        let now = Utc::now().timestamp_millis();
        let safe_text = Policy::redact_static(&input.text);

        let created_message = self
            .db
            .write(move |tx| {
                let mut check_stmt = tx.prepare("SELECT id FROM conversations WHERE id = ?1")?;
                let _exists: String = check_stmt
                    .query_row(params![id_str], |row| row.get(0))
                    .map_err(|e| match e {
                        rusqlite::Error::QueryReturnedNoRows => {
                            AppError::new(ErrorCode::NotFound, format!("Conversation {} not found", id))
                        }
                        other => AppError::from(other),
                    })?;

                tx.execute(
                    "INSERT INTO messages (id, conversation_id, role, text, model_id, job_id, intent, created_at)
                     VALUES (?1, ?2, 'owner', ?3, ?4, NULL, NULL, ?5)",
                    params![
                        message_id.to_string(),
                        id.to_string(),
                        safe_text,
                        input.model_override_id.map(|m| m.to_string()),
                        now
                    ],
                )?;

                tx.execute(
                    "UPDATE conversations SET updated_at = ?1 WHERE id = ?2",
                    params![now, id.to_string()],
                )?;

                Ok(Message {
                    id: message_id,
                    conversation_id: id,
                    role: MessageRole::Owner,
                    text: safe_text,
                    model_id: input.model_override_id,
                    job_id: None,
                    intent: None,
                    created_at: now,
                })
            })
            .await?;

        // Check if compaction is needed
        let (count, total_bytes) = self.conversation_stats(id).await?;
        if count > MAX_ACTIVE_MESSAGES || total_bytes > MAX_HISTORY_BYTES {
            let _ = self.compact(id).await;
        }

        Ok(created_message)
    }

    pub async fn append_assistant_message(
        &self,
        conversation_id: Id,
        text: String,
        model_id: Option<Id>,
        job_id: Option<Id>,
    ) -> Result<Message> {
        let message_id = Uuid::new_v4();
        let now = Utc::now().timestamp_millis();
        let safe_text = Policy::redact_static(&text);
        let id_str = conversation_id.to_string();

        let created = self
            .db
            .write(move |tx| {
                tx.execute(
                    "INSERT INTO messages (id, conversation_id, role, text, model_id, job_id, intent, created_at)
                     VALUES (?1, ?2, 'assistant', ?3, ?4, ?5, NULL, ?6)",
                    params![
                        message_id.to_string(),
                        id_str,
                        safe_text,
                        model_id.map(|m| m.to_string()),
                        job_id.map(|j| j.to_string()),
                        now
                    ],
                )?;

                tx.execute(
                    "UPDATE conversations SET updated_at = ?1 WHERE id = ?2",
                    params![now, id_str],
                )?;

                Ok(Message {
                    id: message_id,
                    conversation_id,
                    role: MessageRole::Assistant,
                    text: safe_text,
                    model_id,
                    job_id,
                    intent: None,
                    created_at: now,
                })
            })
            .await?;

        let (count, total_bytes) = self.conversation_stats(conversation_id).await?;
        if count > MAX_ACTIVE_MESSAGES || total_bytes > MAX_HISTORY_BYTES {
            let _ = self.compact(conversation_id).await;
        }

        Ok(created)
    }

    async fn conversation_stats(&self, id: Id) -> Result<(usize, usize)> {
        let id_str = id.to_string();
        self.db
            .read(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT COUNT(*), COALESCE(SUM(LENGTH(text)), 0) FROM messages WHERE conversation_id = ?1",
                )?;
                let (count, total_len): (i64, i64) = stmt.query_row(params![id_str], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })?;
                Ok((count as usize, total_len as usize))
            })
            .await
    }

    pub async fn context(&self, id: Id) -> Result<Vec<ModelMessage>> {
        let conv = self.get(id).await?;
        let id_str = id.to_string();

        let messages = self
            .db
            .read(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, conversation_id, role, text, model_id, job_id, intent, created_at
                     FROM messages WHERE conversation_id = ?1
                     ORDER BY created_at ASC LIMIT ?2",
                )?;
                let rows = stmt.query_map(
                    params![id_str, MAX_ACTIVE_MESSAGES as i64],
                    parse_message_row,
                )?;
                let mut res = Vec::new();
                for r in rows {
                    res.push(r?);
                }
                Ok(res)
            })
            .await?;

        let mut out = Vec::new();

        if !conv.summary.trim().is_empty() {
            out.push(ModelMessage {
                role: ModelMessageRole::System,
                text: format!("Previous context summary:\n{}", conv.summary.trim()),
            });
        }

        for m in messages {
            let role = match m.role {
                MessageRole::Owner => ModelMessageRole::User,
                MessageRole::Assistant => ModelMessageRole::Assistant,
                MessageRole::System => ModelMessageRole::System,
            };
            out.push(ModelMessage { role, text: m.text });
        }

        Ok(out)
    }

    pub async fn compact(&self, id: Id) -> Result<()> {
        let conv = self.get(id).await?;
        let id_str = id.to_string();

        let all_messages = self
            .db
            .read({
                let id_str = id_str.clone();
                move |conn| {
                    let mut stmt = conn.prepare(
                        "SELECT id, conversation_id, role, text, model_id, job_id, intent, created_at
                         FROM messages WHERE conversation_id = ?1
                         ORDER BY created_at ASC",
                    )?;
                    let rows = stmt.query_map(params![id_str], parse_message_row)?;
                    let mut res = Vec::new();
                    for r in rows {
                        res.push(r?);
                    }
                    Ok(res)
                }
            })
            .await?;

        if all_messages.len() <= 10 {
            return Ok(());
        }

        let split_idx = all_messages.len().saturating_sub(10);
        let to_summarize = &all_messages[..split_idx];
        let to_delete_ids: Vec<String> = to_summarize.iter().map(|m| m.id.to_string()).collect();

        let mut extracted_points = Vec::new();
        for msg in to_summarize {
            let prefix = match msg.role {
                MessageRole::Owner => "User",
                MessageRole::Assistant => "Assistant",
                MessageRole::System => "System",
            };
            for line in msg.text.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let lower = trimmed.to_lowercase();
                if lower.contains("decide")
                    || lower.contains("prefer")
                    || lower.contains("remember")
                    || lower.contains("task")
                    || lower.contains("goal")
                    || lower.contains("project")
                    || lower.contains("fix")
                    || lower.contains("error")
                    || lower.contains("complete")
                    || lower.starts_with("- ")
                    || lower.starts_with("* ")
                    || lower.starts_with("1. ")
                    || extracted_points.is_empty()
                {
                    let point = format!("{}: {}", prefix, trimmed);
                    if !extracted_points.contains(&point) {
                        extracted_points.push(point);
                    }
                }
            }
        }

        let mut new_summary = if !conv.summary.is_empty() {
            format!("{}\n{}", conv.summary, extracted_points.join("\n"))
        } else {
            extracted_points.join("\n")
        };

        if new_summary.len() > MAX_SUMMARY_BYTES {
            let keep_len = MAX_SUMMARY_BYTES - 64;
            new_summary = format!("{}... [summary truncated]", &new_summary[..keep_len]);
        }

        let now = Utc::now().timestamp_millis();
        let safe_summary = Policy::redact_static(&new_summary);

        self.db
            .write(move |tx| {
                for mid in &to_delete_ids {
                    tx.execute("DELETE FROM messages WHERE id = ?1", params![mid])?;
                }

                tx.execute(
                    "UPDATE conversations
                     SET summary = ?1, revision = revision + 1, updated_at = ?2
                     WHERE id = ?3",
                    params![safe_summary, now, id_str],
                )?;

                Ok(())
            })
            .await?;

        Ok(())
    }

    pub async fn list_messages(&self, conversation_id: Id, q: ListQuery) -> Result<Page<Message>> {
        let _ = self.get(conversation_id).await?;
        let limit = q.limit.unwrap_or(50).clamp(1, 100) as usize;
        let conv_str = conversation_id.to_string();
        let cursor_ms = q.cursor.as_deref().and_then(|s| s.parse::<UtcMs>().ok());

        self.db
            .read(move |conn| {
                let mut sql = "SELECT id, conversation_id, role, text, model_id, job_id, intent, created_at FROM messages WHERE conversation_id = ?1".to_string();
                let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(conv_str)];

                if let Some(ms) = cursor_ms {
                    sql.push_str(" AND created_at > ?2 ORDER BY created_at ASC LIMIT ?3");
                    params_vec.push(Box::new(ms));
                    params_vec.push(Box::new((limit + 1) as i64));
                } else {
                    sql.push_str(" ORDER BY created_at ASC LIMIT ?2");
                    params_vec.push(Box::new((limit + 1) as i64));
                }

                let mut stmt = conn.prepare(&sql)?;
                let rusqlite_params: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|b| b.as_ref()).collect();
                let mut rows = stmt.query(rusqlite_params.as_slice())?;
                let mut items = Vec::new();

                while let Some(row) = rows.next()? {
                    items.push(parse_message_row(row)?);
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

    pub async fn clear_messages(&self, conversation_id: Id) -> Result<u64> {
        let _ = self.get(conversation_id).await?;
        let id_str = conversation_id.to_string();
        let now = Utc::now().timestamp_millis();

        self.db
            .write(move |tx| {
                let deleted = tx.execute(
                    "DELETE FROM messages WHERE conversation_id = ?1",
                    params![id_str],
                )?;
                tx.execute(
                    "UPDATE conversations SET summary = '', updated_at = ?1 WHERE id = ?2",
                    params![now, id_str],
                )?;
                Ok(deleted as u64)
            })
            .await
    }

    pub async fn delete(&self, id: Id) -> Result<()> {
        let _ = self.get(id).await?;
        let id_str = id.to_string();

        self.db
            .write(move |tx| {
                let mut job_stmt = tx.prepare(
                    "SELECT COUNT(*) FROM jobs
                     WHERE conversation_id = ?1
                       AND state NOT IN ('completed', 'completed_with_issues', 'failed', 'cancelled')",
                )?;
                let active_jobs: i64 = job_stmt.query_row(params![id_str], |r| r.get(0))?;
                if active_jobs > 0 {
                    return Err(AppError::new(
                        ErrorCode::ResourceInUse,
                        format!("Conversation {} is in use by active jobs", id),
                    ));
                }

                tx.execute("DELETE FROM messages WHERE conversation_id = ?1", params![id_str])?;
                tx.execute("DELETE FROM conversations WHERE id = ?1", params![id_str])?;
                Ok(())
            })
            .await
    }
}

fn parse_conversation_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Conversation> {
    let id_str: String = row.get(0)?;
    let project_id_str: Option<String> = row.get(1)?;
    let model_override_id_str: Option<String> = row.get(2)?;
    let current_task_id_str: Option<String> = row.get(3)?;
    let summary: String = row.get(4)?;
    let revision: Revision = row.get(5)?;
    let created_at: UtcMs = row.get(6)?;
    let updated_at: UtcMs = row.get(7)?;

    Ok(Conversation {
        id: Uuid::parse_str(&id_str).unwrap_or_default(),
        project_id: project_id_str.and_then(|s| Uuid::parse_str(&s).ok()),
        model_override_id: model_override_id_str.and_then(|s| Uuid::parse_str(&s).ok()),
        current_task_id: current_task_id_str.and_then(|s| Uuid::parse_str(&s).ok()),
        summary,
        revision,
        created_at,
        updated_at,
    })
}

fn parse_message_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Message> {
    let id_str: String = row.get(0)?;
    let conversation_id_str: String = row.get(1)?;
    let role_str: String = row.get(2)?;
    let text: String = row.get(3)?;
    let model_id_str: Option<String> = row.get(4)?;
    let job_id_str: Option<String> = row.get(5)?;
    let intent_str: Option<String> = row.get(6)?;
    let created_at: UtcMs = row.get(7)?;

    let role = match role_str.as_str() {
        "owner" => MessageRole::Owner,
        "assistant" => MessageRole::Assistant,
        "system" => MessageRole::System,
        _ => MessageRole::Owner,
    };

    let intent = intent_str.and_then(|s| match s.as_str() {
        "question" => Some(crate::types::Intent::Question),
        "task" => Some(crate::types::Intent::Task),
        "reminder" => Some(crate::types::Intent::Reminder),
        "project_work" => Some(crate::types::Intent::ProjectWork),
        "system_action" => Some(crate::types::Intent::SystemAction),
        "clarify" => Some(crate::types::Intent::Clarify),
        _ => None,
    });

    Ok(Message {
        id: Uuid::parse_str(&id_str).unwrap_or_default(),
        conversation_id: Uuid::parse_str(&conversation_id_str).unwrap_or_default(),
        role,
        text,
        model_id: model_id_str.and_then(|s| Uuid::parse_str(&s).ok()),
        job_id: job_id_str.and_then(|s| Uuid::parse_str(&s).ok()),
        intent,
        created_at,
    })
}
