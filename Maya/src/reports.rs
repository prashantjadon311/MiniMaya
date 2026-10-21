use crate::db::Db;
use crate::error::{AppError, ErrorCode, Result};
use crate::types::{
    ProgressSource, Report, ReportEntry, ReportPageQuery, ReportProgress, ReportSection,
    ReportSections, SectionPage, UtcMs,
};
use chrono::{IsoWeek, NaiveDate, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;
use rusqlite::params;
use uuid::Uuid;

const DEFAULT_SECTION_LIMIT: usize = 100;

#[derive(Clone)]
pub struct ReportService {
    db: Db,
}

impl ReportService {
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    pub fn db(&self) -> &Db {
        &self.db
    }

    pub async fn daily(&self, date: NaiveDate, tz: Tz) -> Result<Report> {
        let (period_start, period_end) = self.daily_bounds(date, tz)?;
        self.generate_report(
            period_start,
            period_end,
            tz.to_string(),
            None,
            None,
            DEFAULT_SECTION_LIMIT,
        )
        .await
    }

    pub async fn weekly(&self, iso_week: IsoWeek, tz: Tz) -> Result<Report> {
        let (period_start, period_end) = self.weekly_bounds(iso_week, tz)?;
        self.generate_report(
            period_start,
            period_end,
            tz.to_string(),
            None,
            None,
            DEFAULT_SECTION_LIMIT,
        )
        .await
    }

    pub async fn daily_page(
        &self,
        date: NaiveDate,
        tz: Tz,
        page: ReportPageQuery,
    ) -> Result<Report> {
        let (period_start, period_end) = self.daily_bounds(date, tz)?;
        let limit = if page.limit == 0 {
            DEFAULT_SECTION_LIMIT
        } else {
            (page.limit as usize).clamp(1, 100)
        };
        self.generate_report(
            period_start,
            period_end,
            tz.to_string(),
            page.section,
            page.cursor,
            limit,
        )
        .await
    }

    pub async fn weekly_page(
        &self,
        iso_week: IsoWeek,
        tz: Tz,
        page: ReportPageQuery,
    ) -> Result<Report> {
        let (period_start, period_end) = self.weekly_bounds(iso_week, tz)?;
        let limit = if page.limit == 0 {
            DEFAULT_SECTION_LIMIT
        } else {
            (page.limit as usize).clamp(1, 100)
        };
        self.generate_report(
            period_start,
            period_end,
            tz.to_string(),
            page.section,
            page.cursor,
            limit,
        )
        .await
    }

    fn daily_bounds(&self, date: NaiveDate, tz: Tz) -> Result<(UtcMs, UtcMs)> {
        let next_date = date
            .succ_opt()
            .ok_or_else(|| AppError::new(ErrorCode::InvalidRequest, "Date overflow"))?;

        let start_ms = local_day_start_to_utc(date, tz)?;
        let end_ms = local_day_start_to_utc(next_date, tz)?;

        Ok((start_ms, end_ms))
    }

    fn weekly_bounds(&self, iso_week: IsoWeek, tz: Tz) -> Result<(UtcMs, UtcMs)> {
        let monday =
            NaiveDate::from_isoywd_opt(iso_week.year(), iso_week.week(), chrono::Weekday::Mon)
                .ok_or_else(|| AppError::new(ErrorCode::InvalidRequest, "Invalid ISO week"))?;

        let next_monday = monday + chrono::Duration::days(7);

        let start_ms = local_day_start_to_utc(monday, tz)?;
        let end_ms = local_day_start_to_utc(next_monday, tz)?;

        Ok((start_ms, end_ms))
    }

    async fn generate_report(
        &self,
        period_start: UtcMs,
        period_end: UtcMs,
        timezone: String,
        paged_section: Option<ReportSection>,
        _cursor: Option<String>,
        limit: usize,
    ) -> Result<Report> {
        let now = Utc::now().timestamp_millis();

        self.db
            .read(move |conn| {
                // 1. Completed tasks
                let (completed, completed_page) = {
                    let mut count_stmt = conn.prepare(
                        "SELECT COUNT(*) FROM tasks
                         WHERE status = 'done'
                           AND completed_at >= ?1
                           AND completed_at < ?2",
                    )?;
                    let total: i64 = count_stmt.query_row(params![period_start, period_end], |r| r.get(0))?;

                    let mut query_stmt = conn.prepare(
                        "SELECT id FROM tasks
                         WHERE status = 'done'
                           AND completed_at >= ?1
                           AND completed_at < ?2
                         ORDER BY completed_at ASC, id ASC",
                    )?;
                    let rows = query_stmt.query_map(params![period_start, period_end], |r| {
                        let id_str: String = r.get(0)?;
                        Ok(Uuid::parse_str(&id_str).unwrap_or_default())
                    })?;
                    let mut items = Vec::new();
                    for r in rows {
                        items.push(r?);
                    }

                    let cur_limit = if paged_section == Some(ReportSection::Completed) {
                        limit
                    } else {
                        DEFAULT_SECTION_LIMIT
                    };

                    let truncated = items.len() > cur_limit;
                    if truncated {
                        items.truncate(cur_limit);
                    }
                    let next_cursor = if truncated {
                        items.last().map(|id| id.to_string())
                    } else {
                        None
                    };

                    (
                        items,
                        SectionPage {
                            total: total as u64,
                            next_cursor,
                            truncated,
                        },
                    )
                };

                // 2. Pending tasks
                let (pending, pending_page) = {
                    let mut count_stmt = conn.prepare(
                        "SELECT COUNT(*) FROM tasks
                         WHERE status IN ('todo', 'in_progress', 'blocked')
                           AND created_at < ?1",
                    )?;
                    let total: i64 = count_stmt.query_row(params![period_end], |r| r.get(0))?;

                    let mut query_stmt = conn.prepare(
                        "SELECT id FROM tasks
                         WHERE status IN ('todo', 'in_progress', 'blocked')
                           AND created_at < ?1
                         ORDER BY created_at ASC, id ASC",
                    )?;
                    let rows = query_stmt.query_map(params![period_end], |r| {
                        let id_str: String = r.get(0)?;
                        Ok(Uuid::parse_str(&id_str).unwrap_or_default())
                    })?;
                    let mut items = Vec::new();
                    for r in rows {
                        items.push(r?);
                    }

                    let cur_limit = if paged_section == Some(ReportSection::Pending) {
                        limit
                    } else {
                        DEFAULT_SECTION_LIMIT
                    };

                    let truncated = items.len() > cur_limit;
                    if truncated {
                        items.truncate(cur_limit);
                    }
                    let next_cursor = if truncated {
                        items.last().map(|id| id.to_string())
                    } else {
                        None
                    };

                    (
                        items,
                        SectionPage {
                            total: total as u64,
                            next_cursor,
                            truncated,
                        },
                    )
                };

                // 3. Delayed tasks
                let (delayed, delayed_page) = {
                    let mut count_stmt = conn.prepare(
                        "SELECT COUNT(*) FROM tasks
                         WHERE due_at IS NOT NULL
                           AND due_at < ?1
                           AND (status != 'done' OR (completed_at IS NOT NULL AND completed_at > due_at))",
                    )?;
                    let total: i64 = count_stmt.query_row(params![period_end], |r| r.get(0))?;

                    let mut query_stmt = conn.prepare(
                        "SELECT id FROM tasks
                         WHERE due_at IS NOT NULL
                           AND due_at < ?1
                           AND (status != 'done' OR (completed_at IS NOT NULL AND completed_at > due_at))
                         ORDER BY due_at ASC, id ASC",
                    )?;
                    let rows = query_stmt.query_map(params![period_end], |r| {
                        let id_str: String = r.get(0)?;
                        Ok(Uuid::parse_str(&id_str).unwrap_or_default())
                    })?;
                    let mut items = Vec::new();
                    for r in rows {
                        items.push(r?);
                    }

                    let cur_limit = if paged_section == Some(ReportSection::Delayed) {
                        limit
                    } else {
                        DEFAULT_SECTION_LIMIT
                    };

                    let truncated = items.len() > cur_limit;
                    if truncated {
                        items.truncate(cur_limit);
                    }
                    let next_cursor = if truncated {
                        items.last().map(|id| id.to_string())
                    } else {
                        None
                    };

                    (
                        items,
                        SectionPage {
                            total: total as u64,
                            next_cursor,
                            truncated,
                        },
                    )
                };

                // 4. Blockers
                let (blockers, blockers_page) = {
                    let mut count_stmt = conn.prepare(
                        "SELECT COUNT(*) FROM task_events
                         WHERE kind = 'blocker'
                           AND at >= ?1
                           AND at < ?2",
                    )?;
                    let total: i64 = count_stmt.query_row(params![period_start, period_end], |r| r.get(0))?;

                    let mut query_stmt = conn.prepare(
                        "SELECT id, task_id, payload_json, evidence_artifact_id, at
                         FROM task_events
                         WHERE kind = 'blocker'
                           AND at >= ?1
                           AND at < ?2
                         ORDER BY at ASC, id ASC",
                    )?;
                    let rows = query_stmt.query_map(params![period_start, period_end], parse_report_entry)?;
                    let mut items = Vec::new();
                    for r in rows {
                        items.push(r?);
                    }

                    let cur_limit = if paged_section == Some(ReportSection::Blockers) {
                        limit
                    } else {
                        DEFAULT_SECTION_LIMIT
                    };

                    let truncated = items.len() > cur_limit;
                    if truncated {
                        items.truncate(cur_limit);
                    }
                    let next_cursor = if truncated {
                        items.last().map(|e| e.at.to_string())
                    } else {
                        None
                    };

                    (
                        items,
                        SectionPage {
                            total: total as u64,
                            next_cursor,
                            truncated,
                        },
                    )
                };

                // 5. Work log
                let (work_log, work_log_page) = {
                    let mut count_stmt = conn.prepare(
                        "SELECT COUNT(*) FROM task_events
                         WHERE kind IN ('note', 'decision', 'progress', 'status', 'checkin', 'evidence')
                           AND at >= ?1
                           AND at < ?2",
                    )?;
                    let total: i64 = count_stmt.query_row(params![period_start, period_end], |r| r.get(0))?;

                    let mut query_stmt = conn.prepare(
                        "SELECT id, task_id, payload_json, evidence_artifact_id, at
                         FROM task_events
                         WHERE kind IN ('note', 'decision', 'progress', 'status', 'checkin', 'evidence')
                           AND at >= ?1
                           AND at < ?2
                         ORDER BY at ASC, id ASC",
                    )?;
                    let rows = query_stmt.query_map(params![period_start, period_end], parse_report_entry)?;
                    let mut items = Vec::new();
                    for r in rows {
                        items.push(r?);
                    }

                    let cur_limit = if paged_section == Some(ReportSection::WorkLog) {
                        limit
                    } else {
                        DEFAULT_SECTION_LIMIT
                    };

                    let truncated = items.len() > cur_limit;
                    if truncated {
                        items.truncate(cur_limit);
                    }
                    let next_cursor = if truncated {
                        items.last().map(|e| e.at.to_string())
                    } else {
                        None
                    };

                    (
                        items,
                        SectionPage {
                            total: total as u64,
                            next_cursor,
                            truncated,
                        },
                    )
                };

                // 6. Progress
                let (progress, progress_page) = {
                    let mut count_stmt = conn.prepare(
                        "SELECT COUNT(*) FROM task_events
                         WHERE kind = 'progress'
                           AND at >= ?1
                           AND at < ?2",
                    )?;
                    let total: i64 = count_stmt.query_row(params![period_start, period_end], |r| r.get(0))?;

                    let mut query_stmt = conn.prepare(
                        "SELECT id, task_id, payload_json, at
                         FROM task_events
                         WHERE kind = 'progress'
                           AND at >= ?1
                           AND at < ?2
                         ORDER BY at ASC, id ASC",
                    )?;
                    let rows = query_stmt.query_map(params![period_start, period_end], parse_report_progress)?;
                    let mut items = Vec::new();
                    for r in rows {
                        items.push(r?);
                    }

                    let cur_limit = if paged_section == Some(ReportSection::Progress) {
                        limit
                    } else {
                        DEFAULT_SECTION_LIMIT
                    };

                    let truncated = items.len() > cur_limit;
                    if truncated {
                        items.truncate(cur_limit);
                    }
                    let next_cursor = if truncated {
                        items.last().map(|p| p.evidence_event_id.to_string())
                    } else {
                        None
                    };

                    (
                        items,
                        SectionPage {
                            total: total as u64,
                            next_cursor,
                            truncated,
                        },
                    )
                };

                Ok(Report {
                    period_start,
                    period_end,
                    timezone,
                    completed,
                    pending,
                    delayed,
                    blockers,
                    work_log,
                    progress,
                    sections: ReportSections {
                        completed: completed_page,
                        pending: pending_page,
                        delayed: delayed_page,
                        blockers: blockers_page,
                        work_log: work_log_page,
                        progress: progress_page,
                    },
                    generated_at: now,
                })
            })
            .await
    }

    pub fn markdown(&self, report: &Report) -> String {
        let mut md = String::new();
        md.push_str(&format!(
            "# Factual Activity Report\n\n- Period: {} - {} (UTC ms)\n- Timezone: {}\n- Generated at: {}\n\n",
            report.period_start, report.period_end, report.timezone, report.generated_at
        ));

        // Completed
        md.push_str(&format!(
            "## Completed Tasks ({})\n\n",
            report.sections.completed.total
        ));
        if report.completed.is_empty() {
            md.push_str("None\n\n");
        } else {
            for id in &report.completed {
                md.push_str(&format!("- Task ID: {}\n", id));
            }
            md.push('\n');
        }

        // Pending
        md.push_str(&format!(
            "## Pending Tasks ({})\n\n",
            report.sections.pending.total
        ));
        if report.pending.is_empty() {
            md.push_str("None\n\n");
        } else {
            for id in &report.pending {
                md.push_str(&format!("- Task ID: {}\n", id));
            }
            md.push('\n');
        }

        // Delayed
        md.push_str(&format!(
            "## Delayed Tasks ({})\n\n",
            report.sections.delayed.total
        ));
        if report.delayed.is_empty() {
            md.push_str("None\n\n");
        } else {
            for id in &report.delayed {
                md.push_str(&format!("- Task ID: {}\n", id));
            }
            md.push('\n');
        }

        // Blockers
        md.push_str(&format!(
            "## Blockers Encountered ({})\n\n",
            report.sections.blockers.total
        ));
        if report.blockers.is_empty() {
            md.push_str("None\n\n");
        } else {
            for entry in &report.blockers {
                md.push_str(&format!(
                    "- [{:?}] Task {}: {}\n",
                    entry.source, entry.task_id, entry.text
                ));
            }
            md.push('\n');
        }

        // Work Log
        md.push_str(&format!(
            "## Work Log Entries ({})\n\n",
            report.sections.work_log.total
        ));
        if report.work_log.is_empty() {
            md.push_str("None\n\n");
        } else {
            for entry in &report.work_log {
                md.push_str(&format!(
                    "- [{:?}] Task {}: {}\n",
                    entry.source, entry.task_id, entry.text
                ));
            }
            md.push('\n');
        }

        // Progress
        md.push_str(&format!(
            "## Progress Milestones ({})\n\n",
            report.sections.progress.total
        ));
        if report.progress.is_empty() {
            md.push_str("None\n\n");
        } else {
            for p in &report.progress {
                let pct_str = p
                    .percent
                    .map(|v| format!("{}%", v))
                    .unwrap_or_else(|| "N/A".to_string());
                md.push_str(&format!(
                    "- Task {}: Progress {} (Source: {:?})\n",
                    p.task_id, pct_str, p.source
                ));
            }
            md.push('\n');
        }

        md
    }
}

fn local_day_start_to_utc(date: NaiveDate, tz: Tz) -> Result<UtcMs> {
    let naive_dt = date.and_time(NaiveTime::from_hms_opt(0, 0, 0).unwrap());
    match tz.from_local_datetime(&naive_dt) {
        chrono::LocalResult::Single(dt) => Ok(dt.timestamp_millis()),
        chrono::LocalResult::Ambiguous(earliest, _) => Ok(earliest.timestamp_millis()),
        chrono::LocalResult::None => {
            let shifted = naive_dt + chrono::Duration::hours(1);
            match tz.from_local_datetime(&shifted) {
                chrono::LocalResult::Single(dt) => Ok(dt.timestamp_millis()),
                chrono::LocalResult::Ambiguous(earliest, _) => Ok(earliest.timestamp_millis()),
                chrono::LocalResult::None => Err(AppError::new(
                    ErrorCode::InvalidRequest,
                    format!(
                        "Cannot resolve local time for date {} in timezone {}",
                        date, tz
                    ),
                )),
            }
        }
    }
}

fn parse_report_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<ReportEntry> {
    let event_id_str: String = row.get(0)?;
    let task_id_str: String = row.get(1)?;
    let payload_json: String = row.get(2)?;
    let artifact_id_str: Option<String> = row.get(3)?;
    let at: UtcMs = row.get(4)?;

    let payload: serde_json::Value =
        serde_json::from_str(&payload_json).unwrap_or(serde_json::Value::Null);

    let text = payload
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let source_str = payload
        .get("source")
        .and_then(|v| v.as_str())
        .unwrap_or("manual");

    let source = match source_str {
        "manual" => ProgressSource::Manual,
        "observed" => ProgressSource::Observed,
        "verified" => ProgressSource::Verified,
        _ => ProgressSource::Manual,
    };

    Ok(ReportEntry {
        task_id: Uuid::parse_str(&task_id_str).unwrap_or_default(),
        event_id: Uuid::parse_str(&event_id_str).unwrap_or_default(),
        artifact_id: artifact_id_str.and_then(|s| Uuid::parse_str(&s).ok()),
        text,
        source,
        at,
    })
}

fn parse_report_progress(row: &rusqlite::Row<'_>) -> rusqlite::Result<ReportProgress> {
    let event_id_str: String = row.get(0)?;
    let task_id_str: String = row.get(1)?;
    let payload_json: String = row.get(2)?;

    let payload: serde_json::Value =
        serde_json::from_str(&payload_json).unwrap_or(serde_json::Value::Null);

    let percent = payload
        .get("percent")
        .and_then(|v| v.as_u64())
        .map(|v| v as u8);

    let source_str = payload
        .get("source")
        .and_then(|v| v.as_str())
        .unwrap_or("manual");

    let source = match source_str {
        "manual" => ProgressSource::Manual,
        "observed" => ProgressSource::Observed,
        "verified" => ProgressSource::Verified,
        _ => ProgressSource::Manual,
    };

    Ok(ReportProgress {
        task_id: Uuid::parse_str(&task_id_str).unwrap_or_default(),
        percent,
        source,
        evidence_event_id: Uuid::parse_str(&event_id_str).unwrap_or_default(),
    })
}
