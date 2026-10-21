use crate::db::Db;
use crate::error::{AppError, ErrorCode, Result};
use crate::types::{
    Actor, Checkin, CheckinQuery, CheckinReply, CheckinStatus, ChecklistItem, EventPayload, Id,
    NotificationStatus, Occurrence, OccurrenceState, Page, Planner, Priority, ProgressSource, Task,
    TaskDraft, TaskEvent, TaskEventKind, TaskPatch, TaskQuery, TaskStatus,
};
use chrono::{NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;
use rusqlite::params;
use std::collections::HashSet;
use uuid::Uuid;

pub struct TaskService {
    db: Db,
}

impl TaskService {
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    pub fn db(&self) -> &Db {
        &self.db
    }

    pub async fn create(&self, draft: TaskDraft, actor: Actor) -> Result<Task> {
        let title = draft.title.trim().to_string();
        if title.is_empty() || title.len() > 200 {
            return Err(AppError::new(
                ErrorCode::InvalidRequest,
                "Task title must be between 1 and 200 characters",
            ));
        }

        // Validate parent_task_id
        if let Some(parent_id) = draft.parent_task_id {
            let parent = self.get(parent_id).await?;
            if parent.project_id != draft.project_id {
                return Err(AppError::new(
                    ErrorCode::InvalidRequest,
                    "Parent task must belong to the same project",
                ));
            }
            // Check depth limit <= 8
            let depth = self.get_task_depth(parent_id).await?;
            if depth >= 8 {
                return Err(AppError::new(
                    ErrorCode::InvalidRequest,
                    "Task hierarchy depth cannot exceed 8 levels",
                ));
            }
        }

        let id = Uuid::new_v4();
        let id_str = id.to_string();
        let pid_str = draft.project_id.map(|p| p.to_string());
        let parent_str = draft.parent_task_id.map(|p| p.to_string());
        let prio_str = priority_to_str(draft.priority);
        let now = Utc::now().timestamp_millis();
        let desc = draft.description.clone();
        let due_at = draft.due_at;
        let tz = draft.timezone.clone();
        let est_mins = draft.estimated_minutes.map(|m| m as i64);

        self.db
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO tasks (
                        id, parent_task_id, project_id, title, description,
                        priority, status, progress_pct, progress_source, due_at,
                        timezone, estimated_minutes, completed_at, revision,
                        created_at, updated_at
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'todo', NULL, NULL, ?7, ?8, ?9, NULL, 1, ?10, ?10)",
                    params![
                        id_str,
                        parent_str,
                        pid_str,
                        title,
                        desc,
                        prio_str,
                        due_at,
                        tz,
                        est_mins,
                        now
                    ],
                )?;

                // Append initial event
                let ev_id = Uuid::new_v4().to_string();
                let actor_json = serde_json::to_string(&actor).unwrap_or_else(|_| "{}".to_string());
                let payload_json = serde_json::to_string(&EventPayload {
                    text: Some("Task created".to_string()),
                    from_status: None,
                    to_status: Some(TaskStatus::Todo),
                    ..Default::default()
                })
                .unwrap_or_else(|_| "{}".to_string());

                conn.execute(
                    "INSERT INTO task_events (id, task_id, kind, payload_json, evidence_artifact_id, actor_json, at)
                     VALUES (?1, ?2, 'status', ?3, NULL, ?4, ?5)",
                    params![ev_id, id_str, payload_json, actor_json, now],
                )?;

                Ok(())
            })
            .await?;

        self.get(id).await
    }

    pub async fn get(&self, id: Id) -> Result<Task> {
        let id_str = id.to_string();
        self.db
            .read(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, parent_task_id, project_id, title, description, priority,
                            status, progress_pct, progress_source, due_at, timezone,
                            estimated_minutes, completed_at, revision, created_at, updated_at
                     FROM tasks WHERE id = ?1",
                )?;
                let mut rows = stmt.query(params![id_str])?;
                if let Some(r) = rows.next()? {
                    let mut task = parse_task_row(r)?;
                    // Read checklist
                    let mut cl_stmt = conn.prepare(
                        "SELECT id, text, done, position FROM task_checklist
                         WHERE task_id = ?1 ORDER BY position ASC",
                    )?;
                    let cl_rows = cl_stmt.query_map(params![id_str], |crow| {
                        let cid_str: String = crow.get(0)?;
                        let text: String = crow.get(1)?;
                        let done: i64 = crow.get(2)?;
                        let pos: i64 = crow.get(3)?;
                        let cid = Uuid::parse_str(&cid_str).map_err(|e| {
                            rusqlite::Error::FromSqlConversionFailure(
                                0,
                                rusqlite::types::Type::Text,
                                Box::new(e),
                            )
                        })?;
                        Ok(ChecklistItem {
                            id: cid,
                            text,
                            done: done != 0,
                            position: pos as u16,
                        })
                    })?;

                    for cl in cl_rows {
                        task.checklist.push(cl?);
                    }
                    Ok(task)
                } else {
                    Err(AppError::new(ErrorCode::NotFound, "Task not found"))
                }
            })
            .await
    }

    pub async fn update(&self, id: Id, patch: TaskPatch, actor: Actor) -> Result<Task> {
        let existing = self.get(id).await?;
        if existing.revision != patch.revision {
            return Err(AppError::new(
                ErrorCode::StaleRevision,
                "Task revision mismatch",
            ));
        }

        // Parent cycle check
        if let Some(new_parent_opt) = patch.parent_task_id
            && let Some(new_parent_id) = new_parent_opt
        {
            if new_parent_id == id {
                return Err(AppError::new(
                    ErrorCode::InvalidRequest,
                    "Task cannot be its own parent",
                ));
            }
            // Check cycle
            if self.is_ancestor(id, new_parent_id).await? {
                return Err(AppError::new(
                    ErrorCode::InvalidRequest,
                    "Task parent cycle detected",
                ));
            }
            // Check same project
            let parent = self.get(new_parent_id).await?;
            let effective_proj = patch.project_id.unwrap_or(existing.project_id);
            if parent.project_id != effective_proj {
                return Err(AppError::new(
                    ErrorCode::InvalidRequest,
                    "Parent task must belong to the same project",
                ));
            }
        }

        let new_title = patch.title.unwrap_or(existing.title);
        let new_desc = patch.description.unwrap_or(existing.description);
        let new_project = patch.project_id.unwrap_or(existing.project_id);
        let new_parent = patch.parent_task_id.unwrap_or(existing.parent_task_id);
        let new_priority = patch.priority.unwrap_or(existing.priority);
        let new_due = patch.due_at.unwrap_or(existing.due_at);
        let new_tz = patch.timezone.unwrap_or(existing.timezone);
        let new_est = patch
            .estimated_minutes
            .unwrap_or(existing.estimated_minutes);
        let new_status = patch.status.unwrap_or(existing.status);

        // Progress pct & source invariant
        let (new_pct, new_src) = if let Some(pct_opt) = patch.progress_pct {
            let src = patch.progress_source.flatten().or(existing.progress_source);
            if pct_opt.is_some() && src.is_none() {
                (pct_opt, Some(ProgressSource::Manual))
            } else {
                (pct_opt, src)
            }
        } else {
            (existing.progress_pct, existing.progress_source)
        };

        let now = Utc::now().timestamp_millis();
        let completed_at = match new_status {
            TaskStatus::Done => existing.completed_at.or(Some(now)),
            _ => None,
        };

        let id_str = id.to_string();
        let pid_str = new_project.map(|p| p.to_string());
        let parent_str = new_parent.map(|p| p.to_string());
        let prio_str = priority_to_str(new_priority);
        let status_str = status_to_str(new_status);
        let src_str = new_src.map(source_to_str);
        let new_rev = existing.revision + 1;
        let est_mins = new_est.map(|m| m as i64);

        let checklist_items = patch.checklist;

        self.db
            .write(move |conn| {
                conn.execute(
                    "UPDATE tasks SET
                        parent_task_id = ?1, project_id = ?2, title = ?3, description = ?4,
                        priority = ?5, status = ?6, progress_pct = ?7, progress_source = ?8,
                        due_at = ?9, timezone = ?10, estimated_minutes = ?11, completed_at = ?12,
                        revision = ?13, updated_at = ?14
                     WHERE id = ?15",
                    params![
                        parent_str,
                        pid_str,
                        new_title,
                        new_desc,
                        prio_str,
                        status_str,
                        new_pct.map(|p| p as i64),
                        src_str,
                        new_due,
                        new_tz,
                        est_mins,
                        completed_at,
                        new_rev,
                        now,
                        id_str
                    ],
                )?;

                // Update checklist if supplied
                if let Some(items) = checklist_items {
                    conn.execute("DELETE FROM task_checklist WHERE task_id = ?1", params![id_str])?;
                    for (i, item) in items.iter().enumerate() {
                        let cid = item.id.to_string();
                        let done_val = if item.done { 1 } else { 0 };
                        conn.execute(
                            "INSERT INTO task_checklist (id, task_id, text, done, position)
                             VALUES (?1, ?2, ?3, ?4, ?5)",
                            params![cid, id_str, item.text, done_val, (i + 1) as i64],
                        )?;
                    }
                }

                // If status changed, record event
                if new_status != existing.status {
                    let ev_id = Uuid::new_v4().to_string();
                    let actor_json = serde_json::to_string(&actor).unwrap_or_else(|_| "{}".to_string());
                    let payload_json = serde_json::to_string(&EventPayload {
                        from_status: Some(existing.status),
                        to_status: Some(new_status),
                        ..Default::default()
                    })
                    .unwrap_or_else(|_| "{}".to_string());

                    conn.execute(
                        "INSERT INTO task_events (id, task_id, kind, payload_json, evidence_artifact_id, actor_json, at)
                         VALUES (?1, ?2, 'status', ?3, NULL, ?4, ?5)",
                        params![ev_id, id_str, payload_json, actor_json, now],
                    )?;
                }

                Ok(())
            })
            .await?;

        self.get(id).await
    }

    pub async fn event(
        &self,
        id: Id,
        payload: EventPayload,
        kind: TaskEventKind,
        actor: Actor,
    ) -> Result<TaskEvent> {
        let _task = self.get(id).await?;
        let ev_id = Uuid::new_v4();
        let ev_id_str = ev_id.to_string();
        let id_str = id.to_string();
        let kind_str = match kind {
            TaskEventKind::Note => "note",
            TaskEventKind::Decision => "decision",
            TaskEventKind::Blocker => "blocker",
            TaskEventKind::Status => "status",
            TaskEventKind::Progress => "progress",
            TaskEventKind::Reschedule => "reschedule",
            TaskEventKind::Evidence => "evidence",
            TaskEventKind::Checkin => "checkin",
        };
        let now = Utc::now().timestamp_millis();
        let payload_json = serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string());
        let actor_json = serde_json::to_string(&actor).unwrap_or_else(|_| "{}".to_string());

        let payload_clone = payload.clone();

        self.db
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO task_events (id, task_id, kind, payload_json, evidence_artifact_id, actor_json, at)
                     VALUES (?1, ?2, ?3, ?4, NULL, ?5, ?6)",
                    params![ev_id_str, id_str, kind_str, payload_json, actor_json, now],
                )?;

                // If progress or status, update task
                if let Some(pct) = payload_clone.percent {
                    let src = payload_clone.source.unwrap_or(ProgressSource::Manual);
                    let src_str = source_to_str(src);
                    conn.execute(
                        "UPDATE tasks SET progress_pct = ?1, progress_source = ?2, updated_at = ?3 WHERE id = ?4",
                        params![pct as i64, src_str, now, id_str],
                    )?;
                }

                if let Some(to_s) = payload_clone.to_status {
                    let s_str = status_to_str(to_s);
                    let completed = if to_s == TaskStatus::Done { Some(now) } else { None };
                    conn.execute(
                        "UPDATE tasks SET status = ?1, completed_at = ?2, updated_at = ?3 WHERE id = ?4",
                        params![s_str, completed, now, id_str],
                    )?;
                }

                Ok(())
            })
            .await?;

        Ok(TaskEvent {
            id: ev_id,
            task_id: id,
            kind,
            payload,
            evidence_artifact_id: None,
            actor,
            at: now,
        })
    }

    pub async fn checkin(&self, id: Id, input: CheckinReply) -> Result<Checkin> {
        let now = Utc::now().timestamp_millis();
        let id_str = id.to_string();

        self.db
            .write(move |conn| {
                conn.execute(
                    "UPDATE checkins SET
                        responded_at = ?1, percent = ?2, blocker = ?3, note = ?4,
                        revision = revision + 1
                     WHERE id = ?5",
                    params![
                        now,
                        input.percent.map(|p| p as i64),
                        input.blocker,
                        input.note,
                        id_str
                    ],
                )?;
                Ok(())
            })
            .await?;

        let id_str = id.to_string();
        self.db
            .read(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, task_id, occurrence_id, requested_at, revision, responded_at, percent, blocker, note
                     FROM checkins WHERE id = ?1",
                )?;
                let mut rows = stmt.query(params![id_str])?;
                if let Some(r) = rows.next()? {
                    Ok(parse_checkin_row(r)?)
                } else {
                    Err(AppError::new(ErrorCode::NotFound, "Checkin not found"))
                }
            })
            .await
    }

    pub async fn list(&self, query: TaskQuery) -> Result<Page<Task>> {
        let status_str = query.status.map(status_to_str);
        let pid_str = query.project_id.map(|p| p.to_string());
        let parent_str = query.parent_task_id.map(|p| p.to_string());
        let limit = if query.limit == 0 || query.limit > 100 {
            50
        } else {
            query.limit
        };

        self.db
            .read(move |conn| {
                let mut sql = "SELECT id, parent_task_id, project_id, title, description, priority,
                                      status, progress_pct, progress_source, due_at, timezone,
                                      estimated_minutes, completed_at, revision, created_at, updated_at
                               FROM tasks WHERE 1=1".to_string();

                if status_str.is_some() {
                    sql.push_str(" AND status = :status");
                }
                if pid_str.is_some() {
                    sql.push_str(" AND project_id = :project_id");
                }
                if parent_str.is_some() {
                    sql.push_str(" AND parent_task_id = :parent_task_id");
                }

                sql.push_str(" ORDER BY created_at DESC LIMIT :limit");

                let mut stmt = conn.prepare(&sql)?;
                let mut params: Vec<(&str, &dyn rusqlite::ToSql)> = Vec::new();
                if let Some(s) = &status_str {
                    params.push((":status", s));
                }
                if let Some(p) = &pid_str {
                    params.push((":project_id", p));
                }
                if let Some(pt) = &parent_str {
                    params.push((":parent_task_id", pt));
                }
                let lim_i64 = limit as i64;
                params.push((":limit", &lim_i64));

                let rows = stmt.query_map(&params[..], parse_task_row)?;
                let mut items = Vec::new();
                for r in rows {
                    items.push(r?);
                }

                Ok(Page {
                    items,
                    next_cursor: None,
                })
            })
            .await
    }

    pub async fn checkins(&self, task_id: Id, query: CheckinQuery) -> Result<Page<CheckinStatus>> {
        let tid_str = task_id.to_string();
        let limit = if query.limit == 0 || query.limit > 100 {
            50
        } else {
            query.limit
        };
        let pending_only = query.pending_only;

        self.db
            .read(move |conn| {
                let mut sql = "SELECT c.id, c.task_id, c.occurrence_id, c.requested_at, c.revision,
                                      c.responded_at, c.percent, c.blocker, c.note,
                                      o.id, o.schedule_id, o.scheduled_at, o.state, o.notification_status
                               FROM checkins c
                               LEFT JOIN occurrences o ON c.occurrence_id = o.id
                               WHERE c.task_id = ?1".to_string();

                if pending_only {
                    sql.push_str(" AND c.responded_at IS NULL");
                }
                sql.push_str(" ORDER BY c.requested_at DESC LIMIT ?2");

                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map(params![tid_str, limit as i64], |row| {
                    let checkin = parse_checkin_row(row)?;
                    let occ_id_str: Option<String> = row.get(9)?;
                    let occurrence = if let Some(oid_str) = occ_id_str {
                        let oid = Uuid::parse_str(&oid_str).unwrap_or_default();
                        let sid_str: String = row.get(10)?;
                        let sched_at: i64 = row.get(11)?;
                        let state_str: String = row.get(12)?;
                        let notif_str: String = row.get(13)?;

                        let state = match state_str.as_str() {
                            "delivered" => OccurrenceState::Delivered,
                            "missed" => OccurrenceState::Missed,
                            "failed" => OccurrenceState::Failed,
                            _ => OccurrenceState::Pending,
                        };

                        let notification_status = match notif_str.as_str() {
                            "accepted" => NotificationStatus::Accepted,
                            "unavailable" => NotificationStatus::Unavailable,
                            "failed" => NotificationStatus::Failed,
                            "unknown" => NotificationStatus::Unknown,
                            _ => NotificationStatus::NotAttempted,
                        };

                        Some(Occurrence {
                            id: oid,
                            schedule_id: Uuid::parse_str(&sid_str).unwrap_or_default(),
                            scheduled_at: sched_at,
                            state,
                            notification_status,
                            checkin_id: Some(checkin.id),
                        })
                    } else {
                        None
                    };

                    Ok(CheckinStatus { checkin, occurrence })
                })?;

                let mut items = Vec::new();
                for r in rows {
                    items.push(r?);
                }

                Ok(Page {
                    items,
                    next_cursor: None,
                })
            })
            .await
    }

    pub async fn planner(&self, date: NaiveDate, tz: Tz) -> Result<Planner> {
        // Calculate day boundaries in timezone
        let naive_start = NaiveDateTime::new(date, NaiveTime::from_hms_opt(0, 0, 0).unwrap());
        let naive_end = NaiveDateTime::new(
            date.succ_opt().unwrap_or(date),
            NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
        );

        let start_ms = tz
            .from_local_datetime(&naive_start)
            .single()
            .map(|dt| dt.timestamp_millis())
            .unwrap_or(0);

        let end_ms = tz
            .from_local_datetime(&naive_end)
            .single()
            .map(|dt| dt.timestamp_millis())
            .unwrap_or(start_ms + 86_400_000);

        let all_tasks = self
            .list(TaskQuery {
                limit: 100,
                ..Default::default()
            })
            .await?
            .items;

        let mut overdue = Vec::new();
        let mut today = Vec::new();
        let mut upcoming = Vec::new();

        for t in all_tasks {
            if t.status == TaskStatus::Done || t.status == TaskStatus::Cancelled {
                continue;
            }
            if let Some(due) = t.due_at {
                if due < start_ms {
                    overdue.push(t);
                } else if due < end_ms {
                    today.push(t);
                } else {
                    upcoming.push(t);
                }
            } else {
                today.push(t);
            }
        }

        // Sort deterministic priority: Urgent -> High -> Normal -> Low, then due_at
        sort_planner_tasks(&mut overdue);
        sort_planner_tasks(&mut today);
        sort_planner_tasks(&mut upcoming);

        let mut recommended = Vec::new();
        for t in &overdue {
            recommended.push(t.id);
        }
        for t in &today {
            recommended.push(t.id);
        }

        Ok(Planner {
            date: date.to_string(),
            timezone: tz.name().to_string(),
            overdue,
            today,
            upcoming,
            recommended,
            truncated: false,
        })
    }

    async fn get_task_depth(&self, task_id: Id) -> Result<u32> {
        let mut current_id = task_id;
        let mut depth = 1;
        let mut visited = HashSet::new();

        while depth <= 8 {
            visited.insert(current_id);
            let task = self.get(current_id).await?;
            if let Some(parent) = task.parent_task_id {
                if visited.contains(&parent) {
                    return Err(AppError::new(ErrorCode::InvalidRequest, "Cycle detected"));
                }
                current_id = parent;
                depth += 1;
            } else {
                break;
            }
        }
        Ok(depth)
    }

    async fn is_ancestor(&self, possible_ancestor: Id, start: Id) -> Result<bool> {
        let mut current_id = start;
        let mut visited = HashSet::new();

        while visited.insert(current_id) {
            if current_id == possible_ancestor {
                return Ok(true);
            }
            let task = self.get(current_id).await?;
            if let Some(parent) = task.parent_task_id {
                current_id = parent;
            } else {
                break;
            }
        }
        Ok(false)
    }
}

fn sort_planner_tasks(tasks: &mut [Task]) {
    tasks.sort_by(|a, b| {
        let prio_a = priority_rank(a.priority);
        let prio_b = priority_rank(b.priority);
        prio_a
            .cmp(&prio_b)
            .then_with(|| match (a.due_at, b.due_at) {
                (Some(da), Some(db)) => da.cmp(&db),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => a.created_at.cmp(&b.created_at),
            })
    });
}

fn priority_rank(p: Priority) -> u8 {
    match p {
        Priority::Urgent => 0,
        Priority::High => 1,
        Priority::Normal => 2,
        Priority::Low => 3,
    }
}

pub fn parse_task_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Task> {
    let id_str: String = row.get(0)?;
    let parent_str: Option<String> = row.get(1)?;
    let pid_str: Option<String> = row.get(2)?;
    let title: String = row.get(3)?;
    let description: String = row.get(4)?;
    let prio_str: String = row.get(5)?;
    let status_str: String = row.get(6)?;
    let progress_pct: Option<i64> = row.get(7)?;
    let progress_src_str: Option<String> = row.get(8)?;
    let due_at: Option<i64> = row.get(9)?;
    let timezone: String = row.get(10)?;
    let est_mins: Option<i64> = row.get(11)?;
    let completed_at: Option<i64> = row.get(12)?;
    let revision: i64 = row.get(13)?;
    let created_at: i64 = row.get(14)?;
    let updated_at: i64 = row.get(15)?;

    let id = Uuid::parse_str(&id_str).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })?;
    let parent_task_id = parent_str.and_then(|s| Uuid::parse_str(&s).ok());
    let project_id = pid_str.and_then(|s| Uuid::parse_str(&s).ok());

    let priority = match prio_str.as_str() {
        "urgent" => Priority::Urgent,
        "high" => Priority::High,
        "low" => Priority::Low,
        _ => Priority::Normal,
    };

    let status = match status_str.as_str() {
        "in_progress" => TaskStatus::InProgress,
        "blocked" => TaskStatus::Blocked,
        "done" => TaskStatus::Done,
        "cancelled" => TaskStatus::Cancelled,
        _ => TaskStatus::Todo,
    };

    let progress_source = progress_src_str.as_deref().map(parse_source);

    Ok(Task {
        id,
        title,
        description,
        project_id,
        parent_task_id,
        priority,
        status,
        progress_pct: progress_pct.map(|p| p as u8),
        progress_source,
        checklist: Vec::new(),
        due_at,
        timezone,
        estimated_minutes: est_mins.map(|m| m as u32),
        completed_at,
        revision,
        created_at,
        updated_at,
    })
}

fn parse_checkin_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Checkin> {
    let id_str: String = row.get(0)?;
    let tid_str: String = row.get(1)?;
    let occ_str: Option<String> = row.get(2)?;
    let req_at: i64 = row.get(3)?;
    let rev: i64 = row.get(4)?;
    let resp_at: Option<i64> = row.get(5)?;
    let pct: Option<i64> = row.get(6)?;
    let blocker: Option<String> = row.get(7)?;
    let note: Option<String> = row.get(8)?;

    Ok(Checkin {
        id: Uuid::parse_str(&id_str).unwrap_or_default(),
        task_id: Uuid::parse_str(&tid_str).unwrap_or_default(),
        occurrence_id: occ_str.and_then(|s| Uuid::parse_str(&s).ok()),
        requested_at: req_at,
        revision: rev,
        responded_at: resp_at,
        percent: pct.map(|p| p as u8),
        blocker,
        note,
    })
}

fn priority_to_str(p: Priority) -> &'static str {
    match p {
        Priority::Low => "low",
        Priority::Normal => "normal",
        Priority::High => "high",
        Priority::Urgent => "urgent",
    }
}

fn status_to_str(s: TaskStatus) -> &'static str {
    match s {
        TaskStatus::Todo => "todo",
        TaskStatus::InProgress => "in_progress",
        TaskStatus::Blocked => "blocked",
        TaskStatus::Done => "done",
        TaskStatus::Cancelled => "cancelled",
    }
}

fn source_to_str(s: ProgressSource) -> &'static str {
    match s {
        ProgressSource::Manual => "manual",
        ProgressSource::Observed => "observed",
        ProgressSource::Verified => "verified",
    }
}

fn parse_source(s: &str) -> ProgressSource {
    match s {
        "observed" => ProgressSource::Observed,
        "verified" => ProgressSource::Verified,
        _ => ProgressSource::Manual,
    }
}
