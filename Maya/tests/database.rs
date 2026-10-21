mod common;

use common::test_db;
use maya_mini::error::ErrorCode;
use rusqlite::params;
use uuid::Uuid;

#[tokio::test]
async fn extended_01_schema_and_migrations() {
    let (_dir, db) = test_db().await;

    // 1. Verify 30 tables exist
    db.read(|conn| {
        let mut stmt = conn
            .prepare("SELECT count(*) FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'")
            .map_err(|e| maya_mini::error::AppError::new(ErrorCode::DbUnavailable, e.to_string()))?;
        let count: i64 = stmt.query_row([], |r| r.get(0)).unwrap();
        assert_eq!(count, 30, "Expected exactly 30 application tables");

        // Integrity check
        let mut pragma = conn
            .prepare("PRAGMA quick_check")
            .map_err(|e| maya_mini::error::AppError::new(ErrorCode::DbUnavailable, e.to_string()))?;
        let check: String = pragma.query_row([], |r| r.get(0)).unwrap();
        assert_eq!(check, "ok");

        // Foreign keys check
        let mut fk_check = conn
            .prepare("PRAGMA foreign_key_check")
            .map_err(|e| maya_mini::error::AppError::new(ErrorCode::DbUnavailable, e.to_string()))?;
        let violations = fk_check
            .query_map([], |_| Ok(()))
            .unwrap()
            .count();
        assert_eq!(violations, 0, "Foreign key check must return zero violations");

        // Verify initial 7 model roles seeded
        let mut roles_stmt = conn
            .prepare("SELECT role, model_id, tool_id FROM model_roles")
            .map_err(|e| maya_mini::error::AppError::new(ErrorCode::DbUnavailable, e.to_string()))?;
        let roles: Vec<String> = roles_stmt
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(roles.len(), 7);
        assert!(roles.contains(&"chat".to_string()));
        assert!(roles.contains(&"planning".to_string()));
        assert!(roles.contains(&"review".to_string()));
        assert!(roles.contains(&"coding".to_string()));
        assert!(roles.contains(&"fallback".to_string()));
        assert!(roles.contains(&"stt".to_string()));
        assert!(roles.contains(&"tts".to_string()));

        Ok(())
    })
    .await
    .unwrap();

    // 2. Test trigger: plans_no_update
    let job_id = Uuid::new_v4().to_string();
    let plan_id = Uuid::new_v4().to_string();
    let baseline_digest = "0".repeat(64);
    let plan_digest = "1".repeat(64);

    db.write(move |tx| {
        // Insert job first
        tx.execute(
            r#"INSERT INTO jobs (id, kind, goal, state, revision, verification, input_json, created_at, updated_at)
               VALUES (?1, 'coding', 'test goal', 'planned', 1, 'not_run', '{}', 1000, 1000)"#,
            params![job_id],
        ).unwrap();

        // Insert plan
        tx.execute(
            r#"INSERT INTO plans (id, job_id, revision, body_json, digest, project_revision, baseline_digest, created_at)
               VALUES (?1, ?2, 1, '{"summary":"test"}', ?3, 1, ?4, 1000)"#,
            params![plan_id, job_id, plan_digest, baseline_digest],
        ).unwrap();

        // Attempt to update plan -> trigger should abort
        let res = tx.execute(
            "UPDATE plans SET body_json = '{\"summary\":\"changed\"}' WHERE id = ?1",
            params![plan_id],
        );
        assert!(res.is_err(), "Trigger plans_no_update must reject UPDATE");

        Ok(())
    })
    .await
    .unwrap();

    // 3. Test trigger: approvals_no_rebind
    let approval_id = Uuid::new_v4().to_string();
    let req_digest = "2".repeat(64);
    let job_id2 = Uuid::new_v4().to_string();

    db.write(move |tx| {
        tx.execute(
            r#"INSERT INTO jobs (id, kind, goal, state, revision, verification, input_json, created_at, updated_at)
               VALUES (?1, 'coding', 'test goal 2', 'awaiting_approval', 1, 'not_run', '{}', 1000, 1000)"#,
            params![job_id2],
        ).unwrap();

        tx.execute(
            r#"INSERT INTO approvals (id, job_id, action, request_digest, scope_json, policy_version, decision, requested_at, expires_at, revision)
               VALUES (?1, ?2, 'project_edit', ?3, '{}', 1, 'pending', 1000, 2000, 1)"#,
            params![approval_id, job_id2, req_digest],
        ).unwrap();

        // Update immutable field (e.g. action or request_digest)
        let res = tx.execute(
            "UPDATE approvals SET action = 'run_test' WHERE id = ?1",
            params![approval_id],
        );
        assert!(res.is_err(), "Trigger approvals_no_rebind must reject action update");

        // Updating decision is allowed
        let res_ok = tx.execute(
            "UPDATE approvals SET decision = 'approved', decided_at = 1500 WHERE id = ?1",
            params![approval_id],
        );
        assert!(res_ok.is_ok(), "Updating decision on approval is allowed");

        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn extended_02_wal_backup() {
    let (_dir, db) = test_db().await;

    // Insert some records
    db.write(|tx| {
        tx.execute(
            "INSERT INTO memory_items (id, kind, text, source, revision, created_at, updated_at) VALUES (?1, 'preference', 'dark mode', 'owner', 1, 1000, 1000)",
            params![Uuid::new_v4().to_string()],
        ).unwrap();
        Ok(())
    }).await.unwrap();

    let backup_dir = tempfile::tempdir().unwrap();
    let backup_path = backup_dir.path().join("backup.sqlite3");

    db.backup_to(&backup_path).await.unwrap();

    assert!(backup_path.exists());
    let backup_conn = rusqlite::Connection::open(&backup_path).unwrap();
    let count: i64 = backup_conn
        .query_row("SELECT count(*) FROM memory_items", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn extended_04_task_invariants() {
    let (_dir, db) = test_db().await;

    let proj_id = Uuid::new_v4().to_string();
    db.write({
        let p_id = proj_id.clone();
        move |tx| {
            tx.execute(
                r#"INSERT INTO projects (id, name, root, root_device, root_inode, allowed_paths_json, excluded_paths_json, notes, archived, revision, created_at, updated_at)
                   VALUES (?1, 'Test Project', '/tmp/test_proj', 1, 1, '[]', '[]', '', 0, 1, 1000, 1000)"#,
                params![p_id],
            ).unwrap();
            Ok(())
        }
    }).await.unwrap();

    let t1 = Uuid::new_v4().to_string();
    let p_id = proj_id.clone();
    db.write({
        let t1 = t1.clone();
        move |tx| {
            // Insert task
            tx.execute(
                r#"INSERT INTO tasks (id, project_id, title, description, priority, status, timezone, revision, created_at, updated_at)
                   VALUES (?1, ?2, 'Task 1', 'desc', 'normal', 'todo', 'Asia/Kolkata', 1, 1000, 1000)"#,
                params![t1, p_id],
            ).unwrap();
            Ok(())
        }
    }).await.unwrap();

    // Check done invariant: status='done' requires completed_at IS NOT NULL
    let res = db
        .write({
            let t1 = t1.clone();
            move |tx| {
                tx.execute(
                    "UPDATE tasks SET status = 'done' WHERE id = ?1",
                    params![t1],
                )
                .map_err(|e| {
                    maya_mini::error::AppError::new(ErrorCode::InvalidRequest, e.to_string())
                })?;
                Ok(())
            }
        })
        .await;
    assert!(
        res.is_err(),
        "status=done without completed_at must violate check constraint"
    );

    // Setting both status='done' and completed_at succeeds
    db.write({
        let t1 = t1.clone();
        move |tx| {
            tx.execute(
                "UPDATE tasks SET status = 'done', completed_at = 2000 WHERE id = ?1",
                params![t1],
            )
            .unwrap();
            Ok(())
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn extended_32_memory_and_chat_retention() {
    let (_dir, db) = test_db().await;

    let conv_id = Uuid::new_v4().to_string();
    db.write({
        let c_id = conv_id.clone();
        move |tx| {
            tx.execute(
                "INSERT INTO conversations (id, summary, revision, created_at, updated_at) VALUES (?1, 'Summary', 1, 1000, 1000)",
                params![c_id],
            ).unwrap();

            tx.execute(
                "INSERT INTO messages (id, conversation_id, role, text, created_at) VALUES (?1, ?2, 'owner', 'Hello', 1000)",
                params![Uuid::new_v4().to_string(), c_id],
            ).unwrap();

            tx.execute(
                "INSERT INTO memory_items (id, kind, conversation_id, text, source, revision, created_at, updated_at) VALUES (?1, 'note', ?2, 'fact 1', 'owner', 1, 1000, 1000)",
                params![Uuid::new_v4().to_string(), c_id],
            ).unwrap();

            Ok(())
        }
    }).await.unwrap();

    // Deleting conversation cascades to messages and sets conversation_id NULL in memory_items
    db.write({
        let c_id = conv_id.clone();
        move |tx| {
            tx.execute("DELETE FROM conversations WHERE id = ?1", params![c_id])
                .unwrap();
            Ok(())
        }
    })
    .await
    .unwrap();

    db.read(|conn| {
        let msg_count: i64 = conn
            .query_row("SELECT count(*) FROM messages", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            msg_count, 0,
            "Messages must cascade delete with conversation"
        );

        let mem_count: i64 = conn
            .query_row("SELECT count(*) FROM memory_items", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            mem_count, 1,
            "Owner memory item must remain after conversation deletion"
        );
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn extended_38_backup_restore_journal() {
    let (_dir, db) = test_db().await;

    let backup_id = Uuid::new_v4().to_string();
    let manifest_sha = "a".repeat(64);
    db.write(move |tx| {
        tx.execute(
            r#"INSERT INTO backups (id, storage_name, schema_version, manifest_sha256, size_bytes, state, created_at)
               VALUES (?1, 'backup-001.sqlite3', 1, ?2, 4096, 'verified', 1000)"#,
            params![backup_id, manifest_sha],
        ).unwrap();
        Ok(())
    }).await.unwrap();

    db.read(|conn| {
        let count: i64 = conn
            .query_row(
                "SELECT count(*) FROM backups WHERE state = 'verified'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
        Ok(())
    })
    .await
    .unwrap();
}
