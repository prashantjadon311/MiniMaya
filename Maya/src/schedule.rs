use crate::db::Db;
use crate::error::{AppError, ErrorCode, Result};
use crate::types::{
    Id, Recurrence, Revision, Schedule, ScheduleDraft, ScheduleKind, SchedulePatch, UtcMs,
};
use chrono::{DateTime, Datelike, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;
use futures_util::future::BoxFuture;
use rusqlite::params;
use std::sync::Arc;
use tokio::sync::mpsc::Receiver;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

pub trait Clock: Send + Sync {
    fn now_ms(&self) -> UtcMs;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> UtcMs {
        Utc::now().timestamp_millis()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub occurrence_id: Id,
    pub title: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationStatusKind {
    Accepted,
    Unavailable,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationReceipt {
    pub status: NotificationStatusKind,
    pub desktop_id: Option<u32>,
}

pub trait Notifier: Send + Sync {
    fn notify<'a>(&'a self, notice: Notice) -> BoxFuture<'a, Result<NotificationReceipt>>;
}

pub struct DummyNotifier;

impl Notifier for DummyNotifier {
    fn notify<'a>(&'a self, _notice: Notice) -> BoxFuture<'a, Result<NotificationReceipt>> {
        Box::pin(async move {
            Ok(NotificationReceipt {
                status: NotificationStatusKind::Accepted,
                desktop_id: Some(1),
            })
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScheduleWake {
    Changed(Id),
    ClockChanged,
    Resumed,
    Shutdown,
}

#[derive(Clone)]
pub struct Scheduler {
    db: Db,
    clock: Arc<dyn Clock>,
    notifier: Arc<dyn Notifier>,
}

impl Scheduler {
    pub fn new(db: Db, clock: Arc<dyn Clock>, notifier: Arc<dyn Notifier>) -> Self {
        Self {
            db,
            clock,
            notifier,
        }
    }

    pub fn db(&self) -> &Db {
        &self.db
    }

    // --- Schedule CRUD ---

    pub async fn list_schedules(&self) -> Result<Vec<Schedule>> {
        self.db
            .read(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, task_id, kind, label, first_at, timezone, recurrence,
                            weekdays_json, local_time, enabled, next_at, revision
                     FROM schedules ORDER BY first_at ASC",
                )?;
                let rows = stmt.query_map([], parse_schedule_row)?;
                let mut out = Vec::new();
                for r in rows {
                    out.push(r?);
                }
                Ok(out)
            })
            .await
    }

    pub async fn get_schedule(&self, id: Id) -> Result<Schedule> {
        let id_str = id.to_string();
        self.db
            .read(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, task_id, kind, label, first_at, timezone, recurrence,
                            weekdays_json, local_time, enabled, next_at, revision
                     FROM schedules WHERE id = ?1",
                )?;
                let mut rows = stmt.query(params![id_str])?;
                if let Some(row) = rows.next()? {
                    Ok(parse_schedule_row(row)?)
                } else {
                    Err(AppError::new(ErrorCode::NotFound, "Schedule not found"))
                }
            })
            .await
    }

    pub async fn create_schedule(&self, draft: ScheduleDraft) -> Result<Schedule> {
        // Validate timezone
        let _tz: Tz = draft.timezone.parse().map_err(|_| {
            AppError::new(
                ErrorCode::InvalidRequest,
                format!("Invalid timezone: {}", draft.timezone),
            )
        })?;

        // Validate local_time format HH:MM
        if NaiveTime::parse_from_str(&draft.local_time, "%H:%M").is_err() {
            return Err(AppError::new(
                ErrorCode::InvalidRequest,
                "local_time must be in HH:MM format",
            ));
        }

        let id = Uuid::new_v4();
        let id_str = id.to_string();
        let tid_str = draft.task_id.map(|t| t.to_string());
        let kind_str = match draft.kind {
            ScheduleKind::Reminder => "reminder",
            ScheduleKind::Checkin => "checkin",
            ScheduleKind::DailyReview => "daily_review",
        };
        let rec_str = match draft.recurrence {
            Recurrence::Once => "once",
            Recurrence::Daily => "daily",
            Recurrence::Weekdays => "weekdays",
            Recurrence::Weekly => "weekly",
        };
        let weekdays_json =
            serde_json::to_string(&draft.weekdays).unwrap_or_else(|_| "[]".to_string());
        let enabled_val = if draft.enabled { 1 } else { 0 };

        // Compute next_at
        let temp_schedule = Schedule {
            id,
            task_id: draft.task_id,
            kind: draft.kind,
            label: draft.label.clone(),
            first_at: draft.first_at,
            timezone: draft.timezone.clone(),
            recurrence: draft.recurrence,
            weekdays: draft.weekdays.clone(),
            local_time: draft.local_time.clone(),
            enabled: draft.enabled,
            next_at: None,
            revision: 1,
        };

        let now = self.clock.now_ms();
        let next_at = if draft.enabled {
            if draft.first_at >= now {
                Some(draft.first_at)
            } else {
                next_occurrence(&temp_schedule, now)?
            }
        } else {
            None
        };

        let label = draft.label.clone();
        let first_at = draft.first_at;
        let timezone = draft.timezone.clone();
        let local_time = draft.local_time.clone();

        self.db
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO schedules (
                        id, task_id, kind, label, first_at, timezone, recurrence,
                        weekdays_json, local_time, enabled, next_at, revision
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 1)",
                    params![
                        id_str,
                        tid_str,
                        kind_str,
                        label,
                        first_at,
                        timezone,
                        rec_str,
                        weekdays_json,
                        local_time,
                        enabled_val,
                        next_at
                    ],
                )?;
                Ok(())
            })
            .await?;

        self.get_schedule(id).await
    }

    pub async fn patch_schedule(&self, id: Id, patch: SchedulePatch) -> Result<Schedule> {
        let existing = self.get_schedule(id).await?;
        if existing.revision != patch.revision {
            return Err(AppError::new(
                ErrorCode::StaleRevision,
                "Schedule revision mismatch",
            ));
        }

        let new_label = patch.label.unwrap_or(existing.label);
        let new_first_at = patch.first_at.unwrap_or(existing.first_at);
        let new_tz = patch.timezone.unwrap_or(existing.timezone);
        let new_rec = patch.recurrence.unwrap_or(existing.recurrence);
        let new_weekdays = patch.weekdays.unwrap_or(existing.weekdays);
        let new_local_time = patch.local_time.unwrap_or(existing.local_time);
        let new_enabled = patch.enabled.unwrap_or(existing.enabled);
        let new_task_id = patch.task_id.unwrap_or(existing.task_id);

        let temp_schedule = Schedule {
            id,
            task_id: new_task_id,
            kind: existing.kind,
            label: new_label.clone(),
            first_at: new_first_at,
            timezone: new_tz.clone(),
            recurrence: new_rec,
            weekdays: new_weekdays.clone(),
            local_time: new_local_time.clone(),
            enabled: new_enabled,
            next_at: None,
            revision: existing.revision + 1,
        };

        let now = self.clock.now_ms();
        let next_at = if new_enabled {
            if new_first_at >= now {
                Some(new_first_at)
            } else {
                next_occurrence(&temp_schedule, now)?
            }
        } else {
            None
        };

        let id_str = id.to_string();
        let tid_str = new_task_id.map(|t| t.to_string());
        let rec_str = match new_rec {
            Recurrence::Once => "once",
            Recurrence::Daily => "daily",
            Recurrence::Weekdays => "weekdays",
            Recurrence::Weekly => "weekly",
        };
        let weekdays_json =
            serde_json::to_string(&new_weekdays).unwrap_or_else(|_| "[]".to_string());
        let new_rev = existing.revision + 1;

        self.db
            .write(move |conn| {
                conn.execute(
                    "UPDATE schedules SET
                        task_id = ?1, label = ?2, first_at = ?3, timezone = ?4,
                        recurrence = ?5, weekdays_json = ?6, local_time = ?7,
                        enabled = ?8, next_at = ?9, revision = ?10
                     WHERE id = ?11",
                    params![
                        tid_str,
                        new_label,
                        new_first_at,
                        new_tz,
                        rec_str,
                        weekdays_json,
                        new_local_time,
                        if new_enabled { 1 } else { 0 },
                        next_at,
                        new_rev,
                        id_str
                    ],
                )?;
                Ok(())
            })
            .await?;

        self.get_schedule(id).await
    }

    pub async fn postpone(&self, id: Id, duration_ms: i64) -> Result<Schedule> {
        let existing = self.get_schedule(id).await?;
        let now = self.clock.now_ms();
        let base_time = existing.next_at.unwrap_or(now);
        let new_next_at = base_time + duration_ms;
        let id_str = id.to_string();
        let new_rev = existing.revision + 1;

        self.db
            .write(move |conn| {
                conn.execute(
                    "UPDATE schedules SET next_at = ?1, revision = ?2 WHERE id = ?3",
                    params![new_next_at, new_rev, id_str],
                )?;
                Ok(())
            })
            .await?;

        self.get_schedule(id).await
    }

    pub async fn disable_or_delete_schedule(
        &self,
        id: Id,
        expected_revision: Revision,
    ) -> Result<()> {
        let existing = self.get_schedule(id).await?;
        if existing.revision != expected_revision {
            return Err(AppError::new(
                ErrorCode::StaleRevision,
                "Schedule revision mismatch",
            ));
        }

        let id_str = id.to_string();
        self.db
            .write(move |conn| {
                conn.execute("DELETE FROM schedules WHERE id = ?1", params![id_str])?;
                Ok(())
            })
            .await
    }

    // --- Background Runner ---

    pub async fn run(
        self,
        cancel: CancellationToken,
        mut wake: Receiver<ScheduleWake>,
    ) -> Result<()> {
        loop {
            if cancel.is_cancelled() {
                break;
            }

            // Find earliest next_at among enabled schedules
            let now = self.clock.now_ms();
            let earliest: Option<(Id, UtcMs)> = self
                .db
                .read(move |conn| {
                    let mut stmt = conn.prepare(
                        "SELECT id, next_at FROM schedules
                         WHERE enabled = 1 AND next_at IS NOT NULL
                         ORDER BY next_at ASC LIMIT 1",
                    )?;
                    let mut rows = stmt.query([])?;
                    if let Some(r) = rows.next()? {
                        let id_str: String = r.get(0)?;
                        let next: i64 = r.get(1)?;
                        let id = Uuid::parse_str(&id_str).map_err(|e| {
                            rusqlite::Error::FromSqlConversionFailure(
                                0,
                                rusqlite::types::Type::Text,
                                Box::new(e),
                            )
                        })?;
                        Ok(Some((id, next)))
                    } else {
                        Ok(None)
                    }
                })
                .await?;

            let sleep_duration = if let Some((_, next_time)) = earliest {
                if next_time <= now {
                    std::time::Duration::from_millis(0)
                } else {
                    std::time::Duration::from_millis((next_time - now) as u64)
                }
            } else {
                std::time::Duration::from_secs(3600)
            };

            tokio::select! {
                _ = cancel.cancelled() => {
                    break;
                }
                Some(w) = wake.recv() => {
                    match w {
                        ScheduleWake::Shutdown => break,
                        ScheduleWake::ClockChanged | ScheduleWake::Resumed | ScheduleWake::Changed(_) => {
                            continue;
                        }
                    }
                }
                _ = tokio::time::sleep(sleep_duration) => {
                    if let Some((sched_id, next_time)) = earliest
                        && next_time <= self.clock.now_ms() {
                            let _ = self.trigger_occurrence(sched_id, next_time).await;
                        }
                }
            }
        }
        Ok(())
    }

    pub async fn trigger_occurrence(&self, schedule_id: Id, scheduled_at: UtcMs) -> Result<Id> {
        let schedule = self.get_schedule(schedule_id).await?;
        let occ_id = Uuid::new_v4();
        let occ_id_str = occ_id.to_string();
        let sid_str = schedule_id.to_string();
        let now = self.clock.now_ms();

        // Check completed-task suppression
        if let Some(tid) = schedule.task_id {
            let task_status: Option<String> = self
                .db
                .read(move |conn| {
                    let mut stmt = conn.prepare("SELECT status FROM tasks WHERE id = ?1")?;
                    let mut rows = stmt.query(params![tid.to_string()])?;
                    if let Some(r) = rows.next()? {
                        Ok(Some(r.get(0)?))
                    } else {
                        Ok(None)
                    }
                })
                .await?;

            if let Some(status) = task_status
                && (status == "done" || status == "cancelled")
            {
                let next = next_occurrence(&schedule, scheduled_at)?;
                let is_once = schedule.recurrence == Recurrence::Once;
                let new_enabled = if is_once { 0 } else { 1 };
                let new_rev = schedule.revision + 1;

                self.db
                        .write(move |conn| {
                            conn.execute(
                                "INSERT INTO occurrences (id, schedule_id, scheduled_at, state, notification_status, attempted_at, delivered_at)
                                 VALUES (?1, ?2, ?3, 'missed', 'not_attempted', ?4, NULL)",
                                params![occ_id_str, sid_str, scheduled_at, now],
                            )?;
                            conn.execute(
                                "UPDATE schedules SET enabled = ?1, next_at = ?2, revision = ?3 WHERE id = ?4",
                                params![new_enabled, next, new_rev, sid_str],
                            )?;
                            Ok(())
                        })
                        .await?;
                return Ok(occ_id);
            }
        }

        // Compute next occurrence
        let next = next_occurrence(&schedule, scheduled_at)?;
        let is_once = schedule.recurrence == Recurrence::Once;
        let new_enabled = if is_once { 0 } else { 1 };
        let new_rev = schedule.revision + 1;

        // Atomic transaction: claim occurrence + advance schedule
        self.db
            .write(move |conn| {
                // Claim occurrence
                conn.execute(
                    "INSERT INTO occurrences (id, schedule_id, scheduled_at, state, notification_status, attempted_at, delivered_at)
                     VALUES (?1, ?2, ?3, 'pending', 'not_attempted', ?4, NULL)",
                    params![occ_id_str, sid_str, scheduled_at, now],
                )?;

                // Advance schedule or disable if once
                conn.execute(
                    "UPDATE schedules SET enabled = ?1, next_at = ?2, revision = ?3 WHERE id = ?4",
                    params![new_enabled, next, new_rev, sid_str],
                )?;

                // If checkin schedule and task_id exists, create checkin record
                if schedule.kind == ScheduleKind::Checkin
                    && let Some(tid) = schedule.task_id {
                        let cid = Uuid::new_v4().to_string();
                        conn.execute(
                            "INSERT INTO checkins (id, task_id, occurrence_id, requested_at, revision)
                             VALUES (?1, ?2, ?3, ?4, 1)",
                            params![cid, tid.to_string(), occ_id_str, now],
                        )?;
                    }

                Ok(())
            })
            .await?;

        // Dispatch notification
        let notice = Notice {
            occurrence_id: occ_id,
            title: schedule.label.clone(),
            body: format!("Reminder for {}", schedule.label),
        };

        let notify_res = self.notifier.notify(notice).await;
        let delivered_now = self.clock.now_ms();
        let occ_id_str = occ_id.to_string();

        let (state, notif_status) = match notify_res {
            Ok(receipt) => match receipt.status {
                NotificationStatusKind::Accepted => ("delivered", "accepted"),
                NotificationStatusKind::Unavailable => ("delivered", "unavailable"),
                NotificationStatusKind::Failed => ("failed", "failed"),
            },
            Err(_) => ("failed", "unknown"),
        };

        self.db
            .write(move |conn| {
                conn.execute(
                    "UPDATE occurrences SET state = ?1, notification_status = ?2, delivered_at = ?3 WHERE id = ?4",
                    params![state, notif_status, delivered_now, occ_id_str],
                )?;
                Ok(())
            })
            .await?;

        Ok(occ_id)
    }
}

pub fn next_occurrence(schedule: &Schedule, after: UtcMs) -> Result<Option<UtcMs>> {
    let tz: Tz = schedule.timezone.parse().map_err(|_| {
        AppError::new(
            ErrorCode::InvalidRequest,
            format!("Invalid timezone: {}", schedule.timezone),
        )
    })?;

    let time_parts: Vec<&str> = schedule.local_time.split(':').collect();
    if time_parts.len() != 2 {
        return Err(AppError::new(
            ErrorCode::InvalidRequest,
            "Invalid local_time format",
        ));
    }
    let target_hour: u32 = time_parts[0].parse().unwrap_or(9);
    let target_min: u32 = time_parts[1].parse().unwrap_or(0);
    let target_time = NaiveTime::from_hms_opt(target_hour, target_min, 0).unwrap_or_default();

    match schedule.recurrence {
        Recurrence::Once => {
            if schedule.first_at > after {
                Ok(Some(schedule.first_at))
            } else {
                Ok(None)
            }
        }
        Recurrence::Daily => {
            let after_dt = DateTime::from_timestamp_millis(after).unwrap_or_default();
            let local_after = after_dt.with_timezone(&tz);
            let mut candidate_date = local_after.date_naive();

            for _ in 0..365 {
                if let Some(candidate_utc) = local_to_utc(candidate_date, target_time, tz)
                    && candidate_utc > after
                {
                    return Ok(Some(candidate_utc));
                }
                candidate_date = candidate_date.succ_opt().unwrap_or(candidate_date);
            }
            Ok(None)
        }
        Recurrence::Weekdays => {
            let after_dt = DateTime::from_timestamp_millis(after).unwrap_or_default();
            let local_after = after_dt.with_timezone(&tz);
            let mut candidate_date = local_after.date_naive();

            for _ in 0..365 {
                let weekday = candidate_date.weekday().number_from_monday(); // 1..=7
                if weekday <= 5
                    && let Some(candidate_utc) = local_to_utc(candidate_date, target_time, tz)
                    && candidate_utc > after
                {
                    return Ok(Some(candidate_utc));
                }
                candidate_date = candidate_date.succ_opt().unwrap_or(candidate_date);
            }
            Ok(None)
        }
        Recurrence::Weekly => {
            let after_dt = DateTime::from_timestamp_millis(after).unwrap_or_default();
            let local_after = after_dt.with_timezone(&tz);
            let mut candidate_date = local_after.date_naive();

            for _ in 0..365 {
                let weekday = candidate_date.weekday().number_from_monday() as u8;
                if schedule.weekdays.contains(&weekday)
                    && let Some(candidate_utc) = local_to_utc(candidate_date, target_time, tz)
                    && candidate_utc > after
                {
                    return Ok(Some(candidate_utc));
                }
                candidate_date = candidate_date.succ_opt().unwrap_or(candidate_date);
            }
            Ok(None)
        }
    }
}

fn local_to_utc(date: NaiveDate, time: NaiveTime, tz: Tz) -> Option<UtcMs> {
    let naive = NaiveDateTime::new(date, time);
    match tz.from_local_datetime(&naive) {
        chrono::LocalResult::Single(dt) => Some(dt.timestamp_millis()),
        chrono::LocalResult::Ambiguous(dt1, _) => {
            // First occurrence for fall-back per BP §11.1
            Some(dt1.timestamp_millis())
        }
        chrono::LocalResult::None => {
            // Nonexistent time during spring-forward: advance to first valid minute per BP §11.1
            let mut test_time = time;
            for _ in 0..120 {
                test_time += chrono::Duration::minutes(1);
                let alt_naive = NaiveDateTime::new(date, test_time);
                if let chrono::LocalResult::Single(dt) = tz.from_local_datetime(&alt_naive) {
                    return Some(dt.timestamp_millis());
                }
            }
            None
        }
    }
}

fn parse_schedule_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Schedule> {
    let id_str: String = row.get(0)?;
    let tid_str: Option<String> = row.get(1)?;
    let kind_str: String = row.get(2)?;
    let label: String = row.get(3)?;
    let first_at: i64 = row.get(4)?;
    let timezone: String = row.get(5)?;
    let rec_str: String = row.get(6)?;
    let weekdays_json: String = row.get(7)?;
    let local_time: String = row.get(8)?;
    let enabled: i64 = row.get(9)?;
    let next_at: Option<i64> = row.get(10)?;
    let rev: i64 = row.get(11)?;

    let id = Uuid::parse_str(&id_str).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })?;
    let task_id = tid_str.and_then(|s| Uuid::parse_str(&s).ok());

    let kind = match kind_str.as_str() {
        "reminder" => ScheduleKind::Reminder,
        "checkin" => ScheduleKind::Checkin,
        "daily_review" => ScheduleKind::DailyReview,
        _ => ScheduleKind::Reminder,
    };

    let recurrence = match rec_str.as_str() {
        "once" => Recurrence::Once,
        "daily" => Recurrence::Daily,
        "weekdays" => Recurrence::Weekdays,
        "weekly" => Recurrence::Weekly,
        _ => Recurrence::Daily,
    };

    let weekdays: Vec<u8> = serde_json::from_str(&weekdays_json).unwrap_or_default();

    Ok(Schedule {
        id,
        task_id,
        kind,
        label,
        first_at,
        timezone,
        recurrence,
        weekdays,
        local_time,
        enabled: enabled != 0,
        next_at,
        revision: rev,
    })
}
