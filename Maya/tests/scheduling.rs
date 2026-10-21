mod common;

use chrono::{Datelike, NaiveDate, NaiveTime, TimeZone};
use chrono_tz::Tz;
use common::{FakeNotifier, fake_clock, test_db};
use maya_mini::schedule::{Scheduler, next_occurrence};
use maya_mini::tasks::TaskService;
use maya_mini::types::{
    Actor, ActorKind, Priority, Recurrence, Schedule, ScheduleDraft, ScheduleKind, TaskDraft,
};
use std::sync::Arc;
use uuid::Uuid;

#[tokio::test]
async fn test_x05_clock_dst_recurrence_and_atomic_advancement() {
    let (_dir, db) = test_db().await;
    let clock = Arc::new(fake_clock(1700000000000));
    let notifier = Arc::new(FakeNotifier::new());
    let scheduler = Scheduler::new(db.clone(), clock.clone(), notifier.clone());

    let tz_ny: Tz = "America/New_York".parse().unwrap();

    // 1. DST Spring-Forward test (2026-03-08): 02:30 doesn't exist, shifts to 03:00
    let sched_spring = Schedule {
        id: Uuid::new_v4(),
        task_id: None,
        kind: ScheduleKind::Reminder,
        label: "Spring forward test".to_string(),
        first_at: 0,
        timezone: "America/New_York".to_string(),
        recurrence: Recurrence::Daily,
        weekdays: vec![],
        local_time: "02:30".to_string(),
        enabled: true,
        next_at: None,
        revision: 1,
    };

    // Calculate reference time just before the spring-forward: 2026-03-07 12:00:00 UTC
    let march7_dt = tz_ny
        .with_ymd_and_hms(2026, 3, 7, 12, 0, 0)
        .single()
        .unwrap();
    let next_spring = next_occurrence(&sched_spring, march7_dt.timestamp_millis())
        .expect("Failed to compute next occurrence for spring-forward");
    assert!(next_spring.is_some());
    let next_spring_ms = next_spring.unwrap();
    let spring_dt = tz_ny.timestamp_millis_opt(next_spring_ms).single().unwrap();
    // In spring-forward, 02:30 gap is resolved by shifting to 03:00
    assert_eq!(
        spring_dt.date_naive(),
        NaiveDate::from_ymd_opt(2026, 3, 8).unwrap()
    );
    assert_eq!(spring_dt.time(), NaiveTime::from_hms_opt(3, 0, 0).unwrap());

    // 2. DST Fall-Back test (2026-11-01): 01:30 is repeated; uses first occurrence only
    let sched_fall = Schedule {
        id: Uuid::new_v4(),
        task_id: None,
        kind: ScheduleKind::Reminder,
        label: "Fall back test".to_string(),
        first_at: 0,
        timezone: "America/New_York".to_string(),
        recurrence: Recurrence::Daily,
        weekdays: vec![],
        local_time: "01:30".to_string(),
        enabled: true,
        next_at: None,
        revision: 1,
    };

    let oct31_dt = tz_ny
        .with_ymd_and_hms(2026, 10, 31, 12, 0, 0)
        .single()
        .unwrap();
    let next_fall = next_occurrence(&sched_fall, oct31_dt.timestamp_millis())
        .expect("Failed to compute next occurrence for fall-back");
    assert!(next_fall.is_some());
    let next_fall_ms = next_fall.unwrap();
    let fall_dt = tz_ny.timestamp_millis_opt(next_fall_ms).single().unwrap();
    assert_eq!(
        fall_dt.date_naive(),
        NaiveDate::from_ymd_opt(2026, 11, 1).unwrap()
    );
    assert_eq!(fall_dt.time(), NaiveTime::from_hms_opt(1, 30, 0).unwrap());

    // 3. Weekly recurrence on Mon, Wed, Fri (1, 3, 5)
    let sched_weekly = Schedule {
        id: Uuid::new_v4(),
        task_id: None,
        kind: ScheduleKind::Reminder,
        label: "MWF reminder".to_string(),
        first_at: 0,
        timezone: "UTC".to_string(),
        recurrence: Recurrence::Weekly,
        weekdays: vec![1, 3, 5],
        local_time: "10:00".to_string(),
        enabled: true,
        next_at: None,
        revision: 1,
    };

    // Starting on a Monday at 11:00 UTC (past 10:00), next should be Wednesday (day 3)
    let monday_utc = chrono::Utc
        .with_ymd_and_hms(2026, 3, 9, 11, 0, 0)
        .single()
        .unwrap();
    let next_weekly = next_occurrence(&sched_weekly, monday_utc.timestamp_millis())
        .expect("Failed to compute weekly recurrence");
    assert!(next_weekly.is_some());
    let wed_dt = chrono::Utc
        .timestamp_millis_opt(next_weekly.unwrap())
        .single()
        .unwrap();
    assert_eq!(wed_dt.weekday(), chrono::Weekday::Wed);
    assert_eq!(wed_dt.time(), NaiveTime::from_hms_opt(10, 0, 0).unwrap());

    // 4. Atomic occurrence claiming and unique (schedule_id, scheduled_at) duplicate prevention
    let draft = ScheduleDraft {
        task_id: None,
        kind: ScheduleKind::Reminder,
        label: "Atomic advance test".to_string(),
        first_at: 1700000000000,
        timezone: "UTC".to_string(),
        recurrence: Recurrence::Daily,
        weekdays: vec![],
        local_time: "12:00".to_string(),
        enabled: true,
    };

    let created = scheduler
        .create_schedule(draft)
        .await
        .expect("Failed to create schedule");

    let sched_at = created.next_at.expect("Schedule must have next_at");
    clock.set_ms(sched_at + 1000);

    // First trigger succeeds
    let occ1 = scheduler
        .trigger_occurrence(created.id, sched_at)
        .await
        .expect("First trigger should succeed");

    // Second trigger with the exact same (schedule_id, scheduled_at) fails with unique constraint
    let occ2_res = scheduler.trigger_occurrence(created.id, sched_at).await;
    assert!(
        occ2_res.is_err(),
        "Duplicate occurrence claim should fail constraint"
    );

    // Verify schedule was advanced to a future timestamp
    let updated_sched = scheduler
        .get_schedule(created.id)
        .await
        .expect("Failed to fetch updated schedule");
    assert!(
        updated_sched.next_at.unwrap() > sched_at,
        "Schedule next_at should be advanced atomically"
    );
    assert_eq!(updated_sched.revision, created.revision + 1);

    // Verify notifier received the notice
    let notices = notifier.notices.lock().unwrap();
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].occurrence_id, occ1);
}

#[tokio::test]
async fn test_x06_crash_dbus_unknown_status_and_suppression() {
    let (_dir, db) = test_db().await;
    let clock = Arc::new(fake_clock(1700000000000));
    let notifier = Arc::new(FakeNotifier::new());
    let scheduler = Scheduler::new(db.clone(), clock.clone(), notifier.clone());
    let task_service = TaskService::new(db.clone());

    let actor = Actor {
        kind: ActorKind::Owner,
        id: None,
    };

    // 1. Completed-task suppression
    let task = task_service
        .create(
            TaskDraft {
                title: "Task with reminder".to_string(),
                description: "Test suppression".to_string(),
                project_id: None,
                parent_task_id: None,
                priority: Priority::Normal,
                due_at: None,
                timezone: "UTC".to_string(),
                estimated_minutes: None,
                reminder: None,
            },
            actor.clone(),
        )
        .await
        .expect("Failed to create task");

    let sched = scheduler
        .create_schedule(ScheduleDraft {
            task_id: Some(task.id),
            kind: ScheduleKind::Reminder,
            label: "Reminder for task".to_string(),
            first_at: 1700000000000,
            timezone: "UTC".to_string(),
            recurrence: Recurrence::Daily,
            weekdays: vec![],
            local_time: "09:00".to_string(),
            enabled: true,
        })
        .await
        .expect("Failed to create schedule");

    // Mark task as done
    task_service
        .update(
            task.id,
            maya_mini::types::TaskPatch {
                revision: task.revision,
                title: None,
                description: None,
                project_id: None,
                parent_task_id: None,
                priority: None,
                due_at: None,
                timezone: None,
                estimated_minutes: None,
                status: Some(maya_mini::types::TaskStatus::Done),
                progress_pct: None,
                progress_source: None,
                checklist: None,
            },
            actor.clone(),
        )
        .await
        .expect("Failed to mark task done");

    let sched_at = sched.next_at.unwrap();
    let occ_id = scheduler
        .trigger_occurrence(sched.id, sched_at)
        .await
        .expect("Trigger should succeed and record suppressed occurrence");

    // Verify occurrence was recorded as 'missed' and notification was suppressed
    let (occ_state, notif_status): (String, String) = db
        .read(move |conn| {
            let mut stmt =
                conn.prepare("SELECT state, notification_status FROM occurrences WHERE id = ?1")?;
            let res = stmt.query_row(rusqlite::params![occ_id.to_string()], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?;
            Ok(res)
        })
        .await
        .unwrap();

    assert_eq!(occ_state, "missed");
    assert_eq!(notif_status, "not_attempted");
    assert_eq!(
        notifier.notices.lock().unwrap().len(),
        0,
        "No notification should be dispatched for completed task"
    );

    // 2. Simulated crash after attempt / notification error -> status unknown/failed
    *notifier.crash_after_attempt.lock().unwrap() = true;

    let sched2 = scheduler
        .create_schedule(ScheduleDraft {
            task_id: None,
            kind: ScheduleKind::Reminder,
            label: "Crash test reminder".to_string(),
            first_at: 1700000000000,
            timezone: "UTC".to_string(),
            recurrence: Recurrence::Daily,
            weekdays: vec![],
            local_time: "15:00".to_string(),
            enabled: true,
        })
        .await
        .expect("Failed to create schedule 2");

    let sched2_at = sched2.next_at.unwrap();
    let occ2_id = scheduler
        .trigger_occurrence(sched2.id, sched2_at)
        .await
        .expect("Trigger should handle error cleanly without crashing");

    let (occ2_state, occ2_notif): (String, String) = db
        .read(move |conn| {
            let mut stmt =
                conn.prepare("SELECT state, notification_status FROM occurrences WHERE id = ?1")?;
            let res = stmt.query_row(rusqlite::params![occ2_id.to_string()], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?;
            Ok(res)
        })
        .await
        .unwrap();

    assert_eq!(occ2_state, "failed");
    assert_eq!(occ2_notif, "unknown");

    // 3. Postpone 1h advances next_at without duplicating delivery
    *notifier.crash_after_attempt.lock().unwrap() = false;
    let sched2_before_postpone = scheduler
        .get_schedule(sched2.id)
        .await
        .expect("Failed to fetch schedule before postpone");

    let postponed = scheduler
        .postpone(sched2.id, 3_600_000)
        .await
        .expect("Postpone should succeed");

    assert_eq!(
        postponed.next_at.unwrap(),
        sched2_before_postpone.next_at.unwrap() + 3_600_000,
        "Postpone should advance next_at by exactly 1 hour"
    );
    assert_eq!(postponed.revision, sched2.revision + 2); // 1 from trigger + 1 from postpone
}

#[tokio::test]
async fn test_conversation_service_bounds_redaction_and_compact() {
    let (_dir, db) = test_db().await;
    let conv_service = maya_mini::conversation::ConversationService::new(db.clone());

    let actor = Actor {
        kind: ActorKind::Owner,
        id: None,
    };

    let conv = conv_service
        .create(
            maya_mini::types::ConversationInput {
                project_id: None,
                model_override_id: None,
                current_task_id: None,
            },
            actor.clone(),
        )
        .await
        .expect("Failed to create conversation");

    // Test secret redaction on append
    let secret_canary = "sk-abcdefghijklmnopqrstuvwxyz123456";
    let msg = conv_service
        .append(
            conv.id,
            maya_mini::types::UserMessage {
                text: format!("Please do not reveal {}", secret_canary),
                model_override_id: None,
                confirmed_task: None,
            },
        )
        .await
        .expect("Failed to append message");

    assert!(
        !msg.text.contains(secret_canary),
        "Secret canary should be redacted"
    );

    // Append 25 messages to trigger automatic compaction (max 20 active)
    for i in 1..=25 {
        conv_service
            .append(
                conv.id,
                maya_mini::types::UserMessage {
                    text: format!("Message {} with decision and task to remember", i),
                    model_override_id: None,
                    confirmed_task: None,
                },
            )
            .await
            .expect("Failed to append message");
    }

    // Check bounded context
    let ctx = conv_service
        .context(conv.id)
        .await
        .expect("Failed to fetch context");
    // Context includes at most 1 system summary message + 20 active messages = <= 21
    assert!(
        ctx.len() <= 21,
        "Context should be bounded, got {}",
        ctx.len()
    );

    // Check compact summary was generated
    let updated_conv = conv_service.get(conv.id).await.expect("Failed to get conv");
    assert!(
        !updated_conv.summary.is_empty(),
        "Compacted summary should be generated"
    );

    // Clear messages
    let deleted = conv_service
        .clear_messages(conv.id)
        .await
        .expect("Failed to clear messages");
    assert!(deleted > 0);

    let cleared_conv = conv_service.get(conv.id).await.expect("Failed to get conv");
    assert!(
        cleared_conv.summary.is_empty(),
        "Summary should be reset after clearing messages"
    );
}

#[tokio::test]
async fn test_memory_service_crud_export_and_purge() {
    let (dir, db) = test_db().await;
    let art_dir = dir.path().join("artifacts");
    std::fs::create_dir_all(&art_dir).unwrap();
    let artifacts = Arc::new(maya_mini::artifacts::ArtifactStore::new(
        art_dir,
        db.clone(),
    ));
    let mem_service = maya_mini::memory::MemoryService::new(db.clone(), artifacts.clone());

    let _actor = Actor {
        kind: ActorKind::Owner,
        id: None,
    };

    // 1. Upsert memory items with redaction
    let secret_canary = "sk-abcdefghijklmnopqrstuvwxyz123456";
    let item1 = mem_service
        .upsert(maya_mini::types::MemoryInput {
            kind: maya_mini::types::MemoryKind::Preference,
            text: format!("Always use dark mode and secret {}", secret_canary),
            project_id: None,
            conversation_id: None,
        })
        .await
        .expect("Failed to upsert memory item");

    assert!(
        !item1.text.contains(secret_canary),
        "Secret canary in memory must be redacted"
    );

    let item2 = mem_service
        .upsert(maya_mini::types::MemoryInput {
            kind: maya_mini::types::MemoryKind::Decision,
            text: "Decided to implement SQLite storage architecture".to_string(),
            project_id: None,
            conversation_id: None,
        })
        .await
        .expect("Failed to upsert memory item 2");

    // 2. Search
    let search_res = mem_service
        .search(maya_mini::types::MemoryQuery {
            q: Some("dark mode".to_string()),
            kind: Some(maya_mini::types::MemoryKind::Preference),
            project_id: None,
            conversation_id: None,
            limit: 10,
            cursor: None,
        })
        .await
        .expect("Failed to search memory");

    assert_eq!(search_res.items.len(), 1);
    assert_eq!(search_res.items[0].id, item1.id);

    // 3. Export
    let permit = maya_mini::policy::ExecutionPermit::new_test_permit(
        Uuid::new_v4(),
        Uuid::new_v4(),
        vec![
            maya_mini::types::ActionClass::ExportData,
            maya_mini::types::ActionClass::DeleteHistory,
        ],
        maya_mini::types::ApprovalScope::default(),
        maya_mini::types::Digest::ZERO,
        vec![],
        (
            i64::MAX,
            std::time::Instant::now() + std::time::Duration::from_secs(3600),
        ),
    );

    let export_art = mem_service
        .export(
            maya_mini::types::ExportSelection {
                format: maya_mini::types::ExportFormat::Json,
                settings: true,
                tasks: vec![],
                memory: vec![item1.id, item2.id],
                reports: vec![],
                include_project_paths: false,
                all_tasks: None,
            },
            &permit,
        )
        .await
        .expect("Failed to export memory to JSON");

    assert_eq!(export_art.kind, maya_mini::types::ArtifactKind::Export);
    assert_eq!(export_art.mime, "application/json");

    // 4. Purge memory items
    let receipt = mem_service
        .purge(
            maya_mini::types::DeleteSelection {
                kind: maya_mini::types::DeleteSelectionKind::MemoryItems,
                ids: vec![item1.id],
                include_subtasks: false,
            },
            &permit,
        )
        .await
        .expect("Failed to purge memory items");

    assert_eq!(receipt.deleted_count, 1);
    let search_after = mem_service
        .search(maya_mini::types::MemoryQuery {
            q: Some("dark mode".to_string()),
            kind: None,
            project_id: None,
            conversation_id: None,
            limit: 10,
            cursor: None,
        })
        .await
        .expect("Search after purge should succeed");
    assert_eq!(search_after.items.len(), 0);
}

#[tokio::test]
async fn test_report_service_daily_and_weekly() {
    let (_dir, db) = test_db().await;
    let task_service = maya_mini::tasks::TaskService::new(db.clone());
    let report_service = maya_mini::reports::ReportService::new(db.clone());

    let actor = Actor {
        kind: ActorKind::Owner,
        id: None,
    };

    let tz: Tz = "UTC".parse().unwrap();
    let today = chrono::Utc::now().date_naive();
    let now_ms = chrono::Utc::now().timestamp_millis();

    // Create a task and complete it today
    let task1 = task_service
        .create(
            TaskDraft {
                title: "Completed Task 1".to_string(),
                description: "Done today".to_string(),
                project_id: None,
                parent_task_id: None,
                priority: Priority::High,
                due_at: Some(now_ms),
                timezone: "UTC".to_string(),
                estimated_minutes: None,
                reminder: None,
            },
            actor.clone(),
        )
        .await
        .expect("Failed to create task 1");

    task_service
        .update(
            task1.id,
            maya_mini::types::TaskPatch {
                revision: task1.revision,
                title: None,
                description: None,
                project_id: None,
                parent_task_id: None,
                priority: None,
                due_at: None,
                timezone: None,
                estimated_minutes: None,
                status: Some(maya_mini::types::TaskStatus::Done),
                progress_pct: Some(Some(100)),
                progress_source: Some(Some(maya_mini::types::ProgressSource::Manual)),
                checklist: None,
            },
            actor.clone(),
        )
        .await
        .expect("Failed to complete task 1");

    // Add a blocker event on task 1
    task_service
        .event(
            task1.id,
            maya_mini::types::EventPayload {
                text: Some("Had an initial blocker that was resolved".to_string()),
                percent: None,
                source: Some(maya_mini::types::ProgressSource::Manual),
                from_status: None,
                to_status: None,
                due_at: None,
            },
            maya_mini::types::TaskEventKind::Blocker,
            actor.clone(),
        )
        .await
        .expect("Failed to record blocker event");

    // Generate daily report
    let daily_rep = report_service
        .daily(today, tz)
        .await
        .expect("Failed to generate daily report");

    assert_eq!(daily_rep.completed.len(), 1);
    assert_eq!(daily_rep.completed[0], task1.id);
    assert_eq!(daily_rep.blockers.len(), 1);

    // Generate weekly report (2026-W25)
    let iso_week = today.iso_week();
    let weekly_rep = report_service
        .weekly(iso_week, tz)
        .await
        .expect("Failed to generate weekly report");

    assert_eq!(weekly_rep.completed.len(), 1);
    assert_eq!(weekly_rep.blockers.len(), 1);

    // Check markdown generation
    let md = report_service.markdown(&daily_rep);
    assert!(md.contains("# Factual Activity Report"));
    assert!(md.contains("## Completed Tasks (1)"));
    assert!(md.contains("## Blockers Encountered (1)"));
}
