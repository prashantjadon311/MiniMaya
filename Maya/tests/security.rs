mod common;

use common::{repo, test_db};
use maya_mini::approval::{ApprovalDecision, ApprovalRequest, ApprovalService};
use maya_mini::auth::AuthService;
use maya_mini::config::validate_settings;
use maya_mini::error::ErrorCode;
use maya_mini::policy::{ActionContext, ActionRequest, Policy};
use maya_mini::scope::{open_read, register_root, snapshot};
use maya_mini::secrets::{AppSecretStore, SecretStore, SecretString};
use maya_mini::types::{
    ActionClass, Actor, ActorKind, ApprovalDecisionKind, Digest, Plan, PlanStep, ScanLimits,
    SecretMode, Settings,
};
use uuid::Uuid;

#[tokio::test]
async fn extended_03_credential_redaction_and_canary() {
    let (_dir, db) = test_db().await;
    let policy = Policy::new(db);

    let canary = "sk-live1234567890abcdefghijklmnopqrstuvwxyz";
    let input = format!("Error with token {} and \x1B[31mred text\x1B[0m", canary);
    let redacted = policy.redact(&input);

    assert!(!redacted.contains(canary), "Secret canary must be redacted");
    assert!(
        !redacted.contains("\x1B[31m"),
        "ANSI color escapes must be stripped"
    );
    assert!(redacted.contains("[REDACTED SECRET]"));
}

#[tokio::test]
async fn extended_09_bootstrap_and_session_expiry() {
    let auth = AuthService::new(8765);

    // Issue launch link
    let link = auth.issue_launch_link(8765).await.unwrap();
    let url = link.url.expose_str();
    assert!(url.contains("#bootstrap="));

    let token = url.split("#bootstrap=").nth(1).unwrap().to_string();

    // 1. Wrong origin rejected
    let res_bad_origin = auth
        .exchange(SecretString::new(&token), "http://evil.com")
        .await;
    assert!(res_bad_origin.is_err());
    assert_eq!(res_bad_origin.unwrap_err().code, ErrorCode::OriginDenied);

    // 2. Successful exchange
    let grant = auth
        .exchange(SecretString::new(&token), "http://127.0.0.1:8765")
        .await
        .unwrap();
    assert_eq!(grant.session_token.len(), 43); // 32 bytes base64url

    // 3. Replay rejected
    let res_replay = auth
        .exchange(SecretString::new(&token), "http://127.0.0.1:8765")
        .await;
    assert!(res_replay.is_err());
    assert_eq!(res_replay.unwrap_err().code, ErrorCode::BootstrapInvalid);

    // 4. Authenticate session
    let ctx = auth.authenticate_token(&grant.session_token).await.unwrap();
    assert_eq!(ctx.id, grant.session_id);

    // 5. CSRF validation
    let mut headers = axum::http::HeaderMap::new();
    headers.insert("X-CSRF-Token", grant.csrf_token.parse().unwrap());
    assert!(auth.csrf(&ctx, &headers).is_ok());

    let mut bad_headers = axum::http::HeaderMap::new();
    bad_headers.insert("X-CSRF-Token", "wrong_token".parse().unwrap());
    assert!(auth.csrf(&ctx, &bad_headers).is_err());

    // 6. Revoke session
    auth.revoke(grant.session_id).await.unwrap();
    let res_after_revoke = auth.authenticate_token(&grant.session_token).await;
    assert!(res_after_revoke.is_err());
}

#[tokio::test]
async fn extended_12_plan_request_digest_and_expiry() {
    let (_dir, db) = test_db().await;
    let approvals = ApprovalService::new(db.clone());
    let policy = Policy::new(db.clone());

    let job_id = Uuid::new_v4();
    let project_id = Uuid::new_v4();
    let plan_id = Uuid::new_v4();
    let plan = Plan {
        id: plan_id,
        job_id,
        revision: 1,
        goal: "test goal".to_string(),
        summary: "Edit the approved project path".to_string(),
        steps: vec![PlanStep {
            position: 1,
            description: "Update src/lib.rs".to_string(),
            expected_paths: vec!["src/lib.rs".to_string()],
            verification_command_ids: vec![],
        }],
        allowed_paths: vec!["src/lib.rs".to_string()],
        actions: vec![ActionClass::ProjectEdit],
        command_ids: vec![],
        context_artifact_ids: vec![],
        selected_model_id: None,
        selected_tool_id: None,
        project_revision: 1,
        baseline_digest: Digest::ZERO,
        timeout_ms: 60_000,
        risks: vec![],
        digest: Digest::of(b"immutable test plan"),
        created_at: 1000,
    };
    let plan_json = serde_json::to_string(&plan).unwrap();
    let plan_digest = plan.digest.to_hex();
    // Insert a project-bound job and immutable plan. Project-edit approval must never
    // be created from caller-supplied scope alone.
    db.write({
        let j_id = job_id;
        let p_id = project_id;
        move |tx| {
            tx.execute(
                r#"INSERT INTO projects
                   (id,name,root,root_device,root_inode,allowed_paths_json,excluded_paths_json,
                    notes,archived,revision,created_at,updated_at)
                   VALUES (?1,'approval-test','/tmp/maya-approval-test',1,1,'["src"]','[]','',0,1,1000,1000)"#,
                rusqlite::params![p_id.to_string()],
            ).unwrap();
            tx.execute(
                r#"INSERT INTO jobs (id,kind,project_id,goal,state,revision,verification,input_json,created_at,updated_at)
                   VALUES (?1,'coding',?2,'test goal','planned',1,'not_run','{}',1000,1000)"#,
                rusqlite::params![j_id.to_string(), p_id.to_string()],
            ).unwrap();
            tx.execute(
                r#"INSERT INTO plans
                   (id,job_id,revision,body_json,digest,project_revision,baseline_digest,created_at)
                   VALUES (?1,?2,1,?3,?4,1,?5,1000)"#,
                rusqlite::params![
                    plan_id.to_string(),
                    j_id.to_string(),
                    plan_json,
                    plan_digest,
                    Digest::ZERO.to_hex()
                ],
            ).unwrap();
            tx.execute(
                "UPDATE jobs SET plan_id=?1 WHERE id=?2",
                rusqlite::params![plan_id.to_string(), j_id.to_string()],
            ).unwrap();
            Ok(())
        }
    }).await.unwrap();

    let req = ApprovalRequest {
        job_revision: 1,
        action: ActionClass::ProjectEdit,
        plan_id: Some(plan_id),
        change_digest: Some(plan.digest),
        ttl_minutes: 5,
    };

    let approval = approvals.request(job_id, req).await.unwrap();
    assert_eq!(approval.decision, ApprovalDecisionKind::Pending);

    // Context without approval -> fails
    let ctx_no_approval = ActionContext {
        actor: Actor {
            kind: ActorKind::Owner,
            id: None,
        },
        job_id,
        step_id: Uuid::new_v4(),
        project_id: Some(project_id),
        plan_id: Some(plan_id),
        approval_ids: vec![],
        policy_version: 1,
        now: chrono::Utc::now().timestamp_millis(),
    };

    let act_req = ActionRequest {
        class: ActionClass::ProjectEdit,
        scope: approval.scope.clone(),
        request_digest: approval.request_digest,
        change_digest: Some(plan.digest),
    };

    let res_denied = policy.authorize(&ctx_no_approval, &act_req).await;
    assert!(res_denied.is_err());
    assert_eq!(res_denied.unwrap_err().code, ErrorCode::ApprovalRequired);

    // Decide: approve
    let session_ctx = maya_mini::auth::SessionContext {
        id: Uuid::new_v4(),
        owner_uid: 1000,
        expires_at: chrono::Utc::now().timestamp_millis() + 3_600_000,
        csrf_hash: Digest::ZERO,
    };

    let decision = ApprovalDecision {
        revision: 1,
        request_digest: approval.request_digest,
        decision: ApprovalDecisionKind::Approved,
        owner_ack: true,
    };

    let approved = approvals
        .decide(approval.id, decision, &session_ctx)
        .await
        .unwrap();
    assert_eq!(approved.decision, ApprovalDecisionKind::Approved);

    // Authorize with valid approval
    let ctx_approved = ActionContext {
        actor: Actor {
            kind: ActorKind::Owner,
            id: None,
        },
        job_id,
        step_id: Uuid::new_v4(),
        project_id: Some(project_id),
        plan_id: Some(plan_id),
        approval_ids: vec![approval.id],
        policy_version: 1,
        now: chrono::Utc::now().timestamp_millis(),
    };

    let permit = policy.authorize(&ctx_approved, &act_req).await.unwrap();
    assert!(permit.permits(&ActionClass::ProjectEdit, None).is_ok());
    assert!(
        permit
            .permits(&ActionClass::DeleteProjectFiles, None)
            .is_err()
    );

    // A valid approval cannot be replayed against another job, plan, project, or
    // caller-expanded scope.
    let mut wrong_job = ctx_approved.clone();
    wrong_job.job_id = Uuid::new_v4();
    let error = policy.authorize(&wrong_job, &act_req).await.unwrap_err();
    assert_eq!(error.code, ErrorCode::ActionDenied);

    let mut expanded = act_req.clone();
    expanded.scope.paths.push("secrets".to_string());
    let error = policy
        .authorize(&ctx_approved, &expanded)
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::ActionDenied);

    let export = ActionRequest {
        class: ActionClass::ExportData,
        scope: Default::default(),
        request_digest: Digest::ZERO,
        change_digest: None,
    };
    let error = policy
        .authorize(&ctx_no_approval, &export)
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::ApprovalRequired);
}

#[tokio::test]
async fn extended_14_keyring_and_session_secrets() {
    let secrets = AppSecretStore::new();
    let id = Uuid::new_v4();
    let sec = SecretString::new("api-key-test-value");

    // Session put and get
    let status = secrets
        .put(id, sec.clone(), SecretMode::Session)
        .await
        .unwrap();
    assert_eq!(status, maya_mini::types::CredentialStatus::Session);

    let retrieved = secrets.get(id).await.unwrap();
    assert_eq!(retrieved.expose_str(), "api-key-test-value");

    secrets.remove(id).await.unwrap();
    let after_remove = secrets.get(id).await;
    assert!(after_remove.is_err());
}

#[tokio::test]
async fn extended_16_scope_path_confinement_and_openat2() {
    let temp = repo();
    temp.write_file("src/main.rs", b"fn main() {}");
    temp.write_file("docs/readme.txt", b"readme");

    let root = register_root(&temp.path, &[]).unwrap();

    // 1. Valid file open
    let f = open_read(&root, "src/main.rs");
    assert!(f.is_ok(), "Opening normal file within root must succeed");

    // 2. Traversal attempt
    let f_traversal = open_read(&root, "../test.txt");
    assert!(f_traversal.is_err(), "Traversal .. must be denied");
    assert_eq!(f_traversal.unwrap_err().code, ErrorCode::ScopeDenied);

    // 3. Absolute path attempt
    let f_abs = open_read(&root, "/etc/passwd");
    assert!(f_abs.is_err(), "Absolute path must be denied");
    assert_eq!(f_abs.unwrap_err().code, ErrorCode::ScopeDenied);

    // 4. NUL byte attempt
    let f_nul = open_read(&root, "src\0main.rs");
    assert!(f_nul.is_err(), "NUL byte in path must be denied");
    assert_eq!(f_nul.unwrap_err().code, ErrorCode::ScopeDenied);

    // 5. Symlink to outside target
    let outside = tempfile::tempdir().unwrap();
    let outside_file = outside.path().join("secret.txt");
    std::fs::write(&outside_file, b"secret").unwrap();

    let symlink_path = temp.path.join("leak_link");
    let _ = std::os::unix::fs::symlink(&outside_file, &symlink_path);

    let f_symlink = open_read(&root, "leak_link");
    assert!(f_symlink.is_err(), "Symlink traversal must be denied");
    assert_eq!(f_symlink.unwrap_err().code, ErrorCode::ScopeDenied);
}

#[tokio::test]
async fn extended_17_snapshot_exclusions() {
    let temp = repo();
    temp.write_file("src/lib.rs", b"pub fn hello() {}");
    temp.write_file(".git/config", b"git config");
    temp.write_file(".env", b"SECRET=123");
    temp.write_file(".ssh/id_rsa", b"private key");

    let root = register_root(&temp.path, &[]).unwrap();
    let snap = snapshot(&root, ScanLimits::default()).unwrap();

    let paths: Vec<String> = snap.entries.into_iter().map(|e| e.path).collect();
    assert!(paths.contains(&"src/lib.rs".to_string()));
    assert!(
        !paths.contains(&".git/config".to_string()),
        ".git must be excluded"
    );
    assert!(
        !paths.contains(&".env".to_string()),
        ".env must be excluded"
    );
    assert!(
        !paths.contains(&".ssh/id_rsa".to_string()),
        ".ssh must be excluded"
    );
}

#[tokio::test]
async fn extended_27_config_security_invariants() {
    let mut settings = Settings {
        revision: 1,
        identity: Default::default(),
        voice: Default::default(),
        assistant: Default::default(),
        security: Default::default(),
        browser: Default::default(),
    };

    // 1. require_plan_approval cannot be disabled
    settings.security.require_plan_approval = false;
    let res = validate_settings(&settings);
    assert!(res.is_err());
    assert_eq!(res.unwrap_err().code, ErrorCode::ActionDenied);

    // 2. critical_always_confirm cannot be disabled
    settings.security.require_plan_approval = true;
    settings.security.critical_always_confirm = false;
    let res2 = validate_settings(&settings);
    assert!(res2.is_err());
    assert_eq!(res2.unwrap_err().code, ErrorCode::ActionDenied);

    // 3. Valid settings pass
    settings.security.critical_always_confirm = true;
    assert!(validate_settings(&settings).is_ok());
}
