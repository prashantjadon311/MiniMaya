mod common;

use common::test_db;
use maya_mini::artifacts::ArtifactStore;
use maya_mini::checkpoint::CheckpointService;
use maya_mini::policy::ExecutionPermit;
use maya_mini::types::{ActionClass, ApprovalScope, Digest, FileChangeKind, Project};
use rusqlite::params;
use std::fs;
use std::sync::Arc;
use std::time::Duration;
use tempfile::tempdir;
use uuid::Uuid;

#[tokio::test]
async fn extended_18_checkpoint_create_stage_and_changes() {
    let (_dir, db) = test_db().await;
    let artifacts_dir = tempdir().unwrap();
    let runtime_dir = tempdir().unwrap();
    let artifacts = Arc::new(ArtifactStore::new(
        artifacts_dir.path().to_path_buf(),
        db.clone(),
    ));
    let checkpoint_svc = CheckpointService::new(
        db.clone(),
        artifacts.clone(),
        runtime_dir.path().to_path_buf(),
    );

    // 1. Create a dummy project on disk
    let proj_dir = tempdir().unwrap();
    let file1 = proj_dir.path().join("main.rs");
    let file2 = proj_dir.path().join("lib.rs");
    fs::write(&file1, "fn main() { println!(\"v1\"); }").unwrap();
    fs::write(&file2, "pub fn add(a: i32, b: i32) -> i32 { a + b }").unwrap();

    let project_id = Uuid::new_v4();
    let job_id = Uuid::new_v4();
    let step_id = Uuid::new_v4();

    // Insert project and job in SQLite
    let p_root = proj_dir.path().to_str().unwrap().to_string();
    db.write({
        let pid = project_id.to_string();
        let jid = job_id.to_string();
        let root = p_root.clone();
        move |tx| {
            tx.execute(
                r#"INSERT INTO projects (id, name, root, root_device, root_inode, allowed_paths_json, excluded_paths_json, notes, archived, revision, created_at, updated_at)
                   VALUES (?1, 'test-proj', ?2, 1, 1, '[]', '[]', '', 0, 1, 1000, 1000)"#,
                params![pid, root],
            ).unwrap();
            tx.execute(
                r#"INSERT INTO jobs (id, kind, project_id, goal, state, revision, verification, input_json, created_at, updated_at)
                   VALUES (?1, 'coding', ?2, 'build feature', 'executing', 1, 'not_run', '{}', 1000, 1000)"#,
                params![jid, pid],
            ).unwrap();
            Ok(())
        }
    }).await.unwrap();

    let project = Project {
        id: project_id,
        name: "test-proj".to_string(),
        root: proj_dir.path().to_str().unwrap().to_string(),
        root_device: 0,
        root_inode: 0,
        allowed_paths: vec![],
        excluded_paths: vec![],
        notes: String::new(),
        preferred_model_id: None,
        preferred_tool_id: None,
        archived: false,
        revision: 1,
    };

    let permit = ExecutionPermit::new_test_permit(
        job_id,
        step_id,
        vec![
            ActionClass::ReadProject,
            ActionClass::ProjectEdit,
            ActionClass::DeleteProjectFiles,
            ActionClass::RestoreCheckpoint,
        ],
        ApprovalScope::default(),
        Digest::ZERO,
        vec![],
        (
            chrono::Utc::now().timestamp_millis() + 60_000,
            std::time::Instant::now() + Duration::from_secs(60),
        ),
    );

    // 2. Create checkpoint
    let checkpoint = checkpoint_svc
        .create(&project, job_id, &permit)
        .await
        .unwrap();
    assert_ne!(checkpoint.baseline_digest, Digest::ZERO);

    // 3. Stage the project
    let stage = checkpoint_svc
        .stage_with_project(&checkpoint, &project)
        .await
        .unwrap();
    assert!(stage.path.exists());
    assert!(stage.path.join("main.rs").exists());
    assert!(stage.path.join("lib.rs").exists());

    // 4. Modify stage (edit main.rs, create new file extra.rs, delete lib.rs)
    fs::write(
        stage.path.join("main.rs"),
        "fn main() { println!(\"v2\"); }",
    )
    .unwrap();
    fs::write(
        stage.path.join("extra.rs"),
        "pub const VERSION: &str = \"0.2.0\";",
    )
    .unwrap();
    fs::remove_file(stage.path.join("lib.rs")).unwrap();

    // 5. Compute changes
    let changes = checkpoint_svc.changes(&stage).await.unwrap();
    assert_eq!(changes.changes.len(), 3);

    let main_ch = changes
        .changes
        .iter()
        .find(|c| c.path == "main.rs")
        .unwrap();
    assert_eq!(main_ch.kind, FileChangeKind::Modify);

    let extra_ch = changes
        .changes
        .iter()
        .find(|c| c.path == "extra.rs")
        .unwrap();
    assert_eq!(extra_ch.kind, FileChangeKind::Create);

    let lib_ch = changes.changes.iter().find(|c| c.path == "lib.rs").unwrap();
    assert_eq!(lib_ch.kind, FileChangeKind::Delete);

    // 6. Promote changes to real project
    let promo_receipt = checkpoint_svc
        .promote(&project, &changes, &permit)
        .await
        .unwrap();
    assert!(promo_receipt.complete);
    assert_eq!(promo_receipt.files.len(), 3);

    // Verify disk content
    assert_eq!(
        fs::read_to_string(&file1).unwrap(),
        "fn main() { println!(\"v2\"); }"
    );
    assert!(proj_dir.path().join("extra.rs").exists());
    assert!(!file2.exists());

    // 7. Restore checkpoint
    let restore_receipt = checkpoint_svc
        .restore(checkpoint.id, changes.digest, &project, &permit)
        .await
        .unwrap();
    assert!(restore_receipt.complete);

    // Verify restored state
    assert_eq!(
        fs::read_to_string(&file1).unwrap(),
        "fn main() { println!(\"v1\"); }"
    );
    assert!(file2.exists());
    assert!(!proj_dir.path().join("extra.rs").exists());
}

#[tokio::test]
async fn extended_19_concurrent_edit_conflict_and_recovery() {
    let (_dir, db) = test_db().await;
    let artifacts_dir = tempdir().unwrap();
    let runtime_dir = tempdir().unwrap();
    let artifacts = Arc::new(ArtifactStore::new(
        artifacts_dir.path().to_path_buf(),
        db.clone(),
    ));
    let checkpoint_svc = CheckpointService::new(
        db.clone(),
        artifacts.clone(),
        runtime_dir.path().to_path_buf(),
    );

    let proj_dir = tempdir().unwrap();
    let target_file = proj_dir.path().join("data.txt");
    fs::write(&target_file, "initial-content").unwrap();

    let project_id = Uuid::new_v4();
    let job_id = Uuid::new_v4();
    let step_id = Uuid::new_v4();

    let p_root = proj_dir.path().to_str().unwrap().to_string();
    db.write({
        let pid = project_id.to_string();
        let jid = job_id.to_string();
        let root = p_root.clone();
        move |tx| {
            tx.execute(
                r#"INSERT INTO projects (id, name, root, root_device, root_inode, allowed_paths_json, excluded_paths_json, notes, archived, revision, created_at, updated_at)
                   VALUES (?1, 'test-proj2', ?2, 1, 1, '[]', '[]', '', 0, 1, 1000, 1000)"#,
                params![pid, root],
            ).unwrap();
            tx.execute(
                r#"INSERT INTO jobs (id, kind, project_id, goal, state, revision, verification, input_json, created_at, updated_at)
                   VALUES (?1, 'coding', ?2, 'build feature', 'executing', 1, 'not_run', '{}', 1000, 1000)"#,
                params![jid, pid],
            ).unwrap();
            Ok(())
        }
    }).await.unwrap();

    let project = Project {
        id: project_id,
        name: "test-proj2".to_string(),
        root: proj_dir.path().to_str().unwrap().to_string(),
        root_device: 0,
        root_inode: 0,
        allowed_paths: vec![],
        excluded_paths: vec![],
        notes: String::new(),
        preferred_model_id: None,
        preferred_tool_id: None,
        archived: false,
        revision: 1,
    };

    let permit = ExecutionPermit::new_test_permit(
        job_id,
        step_id,
        vec![
            ActionClass::ReadProject,
            ActionClass::ProjectEdit,
            ActionClass::DeleteProjectFiles,
            ActionClass::RestoreCheckpoint,
        ],
        ApprovalScope::default(),
        Digest::ZERO,
        vec![],
        (
            chrono::Utc::now().timestamp_millis() + 60_000,
            std::time::Instant::now() + Duration::from_secs(60),
        ),
    );

    let checkpoint = checkpoint_svc
        .create(&project, job_id, &permit)
        .await
        .unwrap();
    let stage = checkpoint_svc
        .stage_with_project(&checkpoint, &project)
        .await
        .unwrap();

    // Stage changes: modify data.txt
    fs::write(stage.path.join("data.txt"), "ai-modified-content").unwrap();
    let changes = checkpoint_svc.changes(&stage).await.unwrap();

    // INJECT CONCURRENT OWNER EDIT before promotion
    fs::write(&target_file, "owner-concurrent-edit").unwrap();

    // Promote: must detect divergent preimage and refuse to destroy owner bytes!
    let promo_receipt = checkpoint_svc
        .promote(&project, &changes, &permit)
        .await
        .unwrap();
    assert!(!promo_receipt.complete);
    let f_receipt = &promo_receipt.files[0];
    assert!(f_receipt.conflict);
    assert!(!f_receipt.applied);

    // Verify owner bytes were preserved untouched!
    assert_eq!(
        fs::read_to_string(&target_file).unwrap(),
        "owner-concurrent-edit"
    );
}
