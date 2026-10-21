mod common;

use common::test_db;
use maya_mini::artifacts::ArtifactStore;
use maya_mini::audit::{AuditLog, EventHub};
use maya_mini::auth::SessionContext;
use maya_mini::backup::BackupService;
use maya_mini::browser::BrowserService;
use maya_mini::config::RuntimeConfig;
use maya_mini::desktop::DesktopService;
use maya_mini::error::ErrorCode;
use maya_mini::fileops::FileService;
use maya_mini::policy::ExecutionPermit;
use maya_mini::process::ProcessSupervisor;
use maya_mini::project::ProjectService;
use maya_mini::sandbox::SandboxBuilder;
use maya_mini::secrets::SessionSecrets;
use maya_mini::telemetry::ResourceMonitor;
use maya_mini::types::{
    ActionClass, ApprovalScope, AudioInput, BrowserAction, BrowserActionKind, ClipboardInput,
    Digest, FileQuery, Language, LaunchAppTarget, LaunchInput, ProjectInput, SpeechInput,
};
use maya_mini::voice::VoiceService;
use std::os::unix::fs::MetadataExt;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

fn make_permit(action_class: ActionClass) -> ExecutionPermit {
    ExecutionPermit::new_test_permit(
        Uuid::new_v4(),
        Uuid::new_v4(),
        vec![action_class],
        ApprovalScope {
            action_classes: vec![action_class],
            project_id: None,
            project_revision: None,
            paths: vec![],
            command_ids: vec![],
            model_ids: vec![],
            tool_id: None,
            artifact_ids: vec![],
            operation_digest: None,
        },
        Digest::ZERO,
        vec![Uuid::new_v4()],
        (
            chrono::Utc::now().timestamp_millis() + 60_000,
            Instant::now() + Duration::from_secs(60),
        ),
    )
}

fn test_config(dir: &std::path::Path) -> Arc<RuntimeConfig> {
    Arc::new(RuntimeConfig {
        data_dir: dir.to_path_buf(),
        cache_dir: dir.join("cache"),
        runtime_dir: dir.join("run"),
        port: 0,
        open_browser: false,
        headless: true,
        print_launch_link: false,
        test_mode: true,
    })
}

#[tokio::test]
async fn test_x31_desktop_clipboard_and_launch() {
    let (_tmp, db) = test_db().await;
    let desktop = DesktopService::new(db);

    // 1. Clipboard write and read within 64 KiB limit
    let permit = make_permit(ActionClass::ClipboardWrite);
    let write_res = desktop
        .clipboard(
            ClipboardInput {
                text: Some("Maya Mini Secure Clipboard Test".to_string()),
            },
            &permit,
        )
        .await
        .expect("Clipboard write should succeed");
    assert!(write_res.message.contains("Copied"));

    let read_permit = make_permit(ActionClass::ClipboardRead);
    let read_res = desktop
        .clipboard(ClipboardInput { text: None }, &read_permit)
        .await
        .expect("Clipboard read should succeed");
    assert_eq!(
        read_res.text.as_deref(),
        Some("Maya Mini Secure Clipboard Test")
    );

    // 2. Oversized clipboard write (> 64 KiB) rejected with InvalidRequest
    let large_text = "x".repeat(65 * 1024);
    let oversized_err = desktop
        .clipboard(
            ClipboardInput {
                text: Some(large_text),
            },
            &permit,
        )
        .await
        .expect_err("Oversized clipboard content must be rejected");
    assert_eq!(oversized_err.code(), ErrorCode::InvalidRequest);

    // 3. Launch requires permit
    let unpermitted = make_permit(ActionClass::ReadProject);
    let launch_err = desktop
        .launch(
            LaunchInput {
                app: LaunchAppTarget::Browser,
                url: Some("https://example.com".to_string()),
                folder: None,
                project_id: None,
                tool_id: None,
            },
            &unpermitted,
        )
        .await
        .expect_err("Launch without LaunchApp permit must fail");
    assert_eq!(launch_err.code(), ErrorCode::ActionDenied);
}

#[tokio::test]
async fn test_x36_browser_service_probe_actions_and_ssrf() {
    let (tmp, db) = test_db().await;
    let artifacts_dir = tmp.path().join("artifacts");
    let artifacts = Arc::new(ArtifactStore::new(artifacts_dir, db.clone()));
    let browser = BrowserService::new(db, artifacts);
    let cancel = CancellationToken::new();

    // 1. Probe returns capability
    let cap = browser.probe().await.expect("Browser probe must succeed");
    assert_eq!(cap.key, "browser");

    // 2. SSRF Protection: Attempt to open AWS/cloud metadata IP must be denied
    let permit = make_permit(ActionClass::OpenUrl);
    let ssrf_err = browser
        .action(
            BrowserAction {
                kind: BrowserActionKind::Open,
                url: Some("http://169.254.169.254/latest/meta-data/".to_string()),
                site: None,
                query: None,
                prompt: None,
                model_id: None,
                owner_ack: true,
                tab_id: None,
            },
            &permit,
            cancel.clone(),
        )
        .await
        .expect_err("Opening cloud metadata SSRF URL must be denied");
    assert_eq!(ssrf_err.code(), ErrorCode::ScopeDenied);

    // 3. A legitimate HTTPS URL either executes through an available controller or
    // reports the missing capability honestly. It must never fabricate a tab.
    let open_result = browser
        .action(
            BrowserAction {
                kind: BrowserActionKind::Open,
                url: Some("https://duckduckgo.com".to_string()),
                site: None,
                query: None,
                prompt: None,
                model_id: None,
                owner_ack: true,
                tab_id: None,
            },
            &permit,
            cancel.clone(),
        )
        .await;
    if cap.state == maya_mini::types::CapabilityState::Available {
        let open_res = open_result.expect("Available browser controller must open the URL");
        assert_eq!(open_res.kind, "browser_open");
        assert!(open_res.tab_id.is_some());
    } else {
        let err = open_result.expect_err("Unavailable browser must report capability failure");
        assert_eq!(err.code(), ErrorCode::UnsupportedCapability);
    }

    // 4. Search obeys the same capability contract.
    let search_permit = make_permit(ActionClass::BrowserRead);
    let search_result = browser
        .action(
            BrowserAction {
                kind: BrowserActionKind::Search,
                url: None,
                site: None,
                query: Some("rust lang".to_string()),
                prompt: None,
                model_id: None,
                owner_ack: true,
                tab_id: None,
            },
            &search_permit,
            cancel.clone(),
        )
        .await;
    if cap.state == maya_mini::types::CapabilityState::Available {
        assert_eq!(
            search_result
                .expect("Available browser controller must search")
                .kind,
            "browser_search"
        );
    } else {
        assert_eq!(
            search_result
                .expect_err("Unavailable browser must reject search")
                .code(),
            ErrorCode::UnsupportedCapability
        );
    }

    // 5. Submit action on generic unadapted site returns UnsupportedCapability
    let submit_permit = make_permit(ActionClass::BrowserSubmit);
    let submit_err = browser
        .action(
            BrowserAction {
                kind: BrowserActionKind::Submit,
                url: None,
                site: None,
                query: None,
                prompt: None,
                model_id: None,
                owner_ack: true,
                tab_id: Some("tab-1".to_string()),
            },
            &submit_permit,
            cancel.clone(),
        )
        .await
        .expect_err("Generic DOM submit must return UnsupportedCapability");
    assert_eq!(submit_err.code(), ErrorCode::UnsupportedCapability);
}

#[tokio::test]
async fn test_x37_voice_service_transcribe_synthesize_and_capabilities() {
    let (tmp, db) = test_db().await;
    let artifacts_dir = tmp.path().join("artifacts");
    let artifacts = Arc::new(ArtifactStore::new(artifacts_dir, db.clone()));
    let secrets = Arc::new(SessionSecrets::new());
    let voice = VoiceService::new(db, secrets, Some(artifacts));

    // 1. Capabilities probe returns 3 capabilities
    let caps = voice
        .capabilities()
        .await
        .expect("Capabilities query must succeed");
    assert_eq!(caps.len(), 3);
    assert!(caps.iter().any(|c| c.key == "voice_browser_tts"));
    assert!(caps.iter().any(|c| c.key == "voice_cloud_stt"));
    assert!(caps.iter().any(|c| c.key == "voice_cloud_tts"));

    // 2. Synthesize in browser mode (no model_id) returns browser mode
    let synth_permit = make_permit(ActionClass::VoiceUpload);
    let cancel = CancellationToken::new();
    let browser_speech = voice
        .synthesize(
            SpeechInput {
                text: "Hello from Maya Mini".to_string(),
                model_id: None,
                voice: "system-default".to_string(),
                rate: 1.0,
                language: Language::English,
                approval_id: None,
            },
            &synth_permit,
            cancel.clone(),
        )
        .await
        .expect("Browser speech synthesis should succeed");
    assert_eq!(browser_speech.text.as_deref(), Some("Hello from Maya Mini"));

    // 3. Synthesize with text > 8 KiB is rejected
    let large_speech_text = "a".repeat(8193);
    let large_synth_err = voice
        .synthesize(
            SpeechInput {
                text: large_speech_text,
                model_id: None,
                voice: "default".to_string(),
                rate: 1.0,
                language: Language::English,
                approval_id: None,
            },
            &synth_permit,
            cancel.clone(),
        )
        .await
        .expect_err("Speech text exceeding 8 KiB must be rejected");
    assert_eq!(large_synth_err.code(), ErrorCode::InvalidRequest);

    // 4. Transcribe requires VoiceUpload permit
    let wrong_permit = make_permit(ActionClass::ReadProject);
    let permit_err = voice
        .transcribe(
            AudioInput {
                data: vec![0u8; 100],
                mime: "audio/webm".to_string(),
                language: Language::English,
                model_id: Uuid::new_v4(),
                approval_id: None,
                payload_digest: Digest::ZERO,
            },
            &wrong_permit,
            cancel.clone(),
        )
        .await
        .expect_err("Transcribe without VoiceUpload permit must fail");
    assert_eq!(permit_err.code(), ErrorCode::ActionDenied);

    // 5. Transcribe with unsupported MIME is rejected
    let mime_err = voice
        .transcribe(
            AudioInput {
                data: vec![1, 2, 3],
                mime: "image/png".to_string(),
                language: Language::English,
                model_id: Uuid::new_v4(),
                approval_id: None,
                payload_digest: Digest::ZERO,
            },
            &synth_permit,
            cancel.clone(),
        )
        .await
        .expect_err("Unsupported audio MIME must be rejected");
    assert_eq!(mime_err.code(), ErrorCode::UnsupportedMedia);

    // 6. Transcribe with corrupt digest is rejected
    let digest_err = voice
        .transcribe(
            AudioInput {
                data: vec![1, 2, 3],
                mime: "audio/wav".to_string(),
                language: Language::English,
                model_id: Uuid::new_v4(),
                approval_id: None,
                payload_digest: Digest::of(b"different_data"),
            },
            &synth_permit,
            cancel.clone(),
        )
        .await
        .expect_err("Audio digest mismatch must be rejected");
    assert_eq!(digest_err.code(), ErrorCode::IntegrityFailure);
}

#[tokio::test]
async fn test_x38_backup_create_verify_and_restore_cycle() {
    let (tmp, db) = test_db().await;
    let config = test_config(tmp.path());
    let backup_service = BackupService::new(db.clone(), config.clone());

    let before_json = r#"{"name":"before-restore"}"#.to_string();
    db.write(move |tx| {
        tx.execute(
            "UPDATE settings SET value_json = ?1, revision = revision + 1, updated_at = ?2
             WHERE key = 'identity'",
            rusqlite::params![before_json, chrono::Utc::now().timestamp_millis()],
        )?;
        Ok(())
    })
    .await
    .expect("Seed pre-backup value");

    // 1. Create a backup of the current database
    let record = backup_service
        .create()
        .await
        .expect("Creating backup must succeed");
    assert_eq!(record.schema_version, 1);
    assert!(record.size_bytes > 0);

    // 2. Verify intact backup succeeds
    let verified = backup_service
        .verify(record.id)
        .await
        .expect("Verifying intact backup must succeed");
    assert_eq!(verified.manifest_sha256, record.manifest_sha256);

    // 3. Verify corrupted backup file fails with IntegrityFailure
    let backup_file = config.backups_dir().join(&record.storage_name);
    let mut file_bytes = std::fs::read(&backup_file).expect("Read backup file");
    file_bytes[0] ^= 0xFF; // corrupt one byte
    std::fs::write(&backup_file, file_bytes).expect("Write corrupted backup");

    let corrupt_err = backup_service
        .verify(record.id)
        .await
        .expect_err("Verifying corrupted backup must fail");
    assert_eq!(corrupt_err.code(), ErrorCode::IntegrityFailure);

    // 4. Create a new valid backup for restore testing
    let valid_backup = backup_service
        .create()
        .await
        .expect("Creating second backup must succeed");

    db.write(|tx| {
        tx.execute(
            "UPDATE settings SET value_json = ?1, revision = revision + 1, updated_at = ?2
             WHERE key = 'identity'",
            rusqlite::params![
                r#"{"name":"after-backup"}"#,
                chrono::Utc::now().timestamp_millis()
            ],
        )?;
        Ok(())
    })
    .await
    .expect("Mutate live value after backup");

    // 5. Restore requires RestoreDatabase permit
    let wrong_permit = make_permit(ActionClass::ReadProject);
    let unpermitted_err = backup_service
        .restore(valid_backup.id, &wrong_permit)
        .await
        .expect_err("Restore without RestoreDatabase permit must fail");
    assert_eq!(unpermitted_err.code(), ErrorCode::ActionDenied);

    // 6. Restore with valid permit succeeds and returns RestoreReceipt
    let restore_permit = make_permit(ActionClass::RestoreDatabase);
    let receipt = backup_service
        .restore(valid_backup.id, &restore_permit)
        .await
        .expect("Restore with valid permit must succeed");
    assert_eq!(receipt.backup_id, valid_backup.id);
    assert!(receipt.restored_at > 0);

    let restored_value = db
        .read(|conn| {
            conn.query_row(
                "SELECT value_json FROM settings WHERE key = 'identity'",
                [],
                |row| row.get::<_, String>(0),
            )
            .map_err(Into::into)
        })
        .await
        .expect("Read restored identity value");
    assert_eq!(restored_value, r#"{"name":"before-restore"}"#);
}

#[tokio::test]
async fn test_x34_telemetry_resource_monitor_sampling_and_subscription() {
    let (tmp, db) = test_db().await;
    let config = test_config(tmp.path());
    let monitor = ResourceMonitor::new(db, config);

    // 1. On-demand sampling returns valid metrics
    let sample = monitor
        .sample(true)
        .await
        .expect("Resource monitor sample must succeed");

    assert!(sample.core.pids.contains(&std::process::id()));
    assert!(sample.core.rss_bytes.unwrap_or(0) > 0);
    assert!(sample.system.total_ram_bytes > 0);
    assert!(sample.system.storage_total_bytes > 0);
    assert!(sample.system.process_count > 0);

    // 2. Subscription channel creates a receiver and cancels cleanly
    let cancel = CancellationToken::new();
    let mut rx = monitor
        .subscribe(cancel.clone())
        .await
        .expect("Subscription must succeed");

    // Cancel subscription
    cancel.cancel();
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn test_fileops_preview_and_find() {
    let (tmp, db) = test_db().await;
    let artifacts_dir = tmp.path().join("artifacts");
    let artifacts = Arc::new(ArtifactStore::new(artifacts_dir, db.clone()));
    let hub = Arc::new(EventHub::new());
    let audit = Arc::new(AuditLog::new(db.clone(), hub));
    let sandbox = Arc::new(SandboxBuilder::new());
    let supervisor = Arc::new(ProcessSupervisor::new(
        db.clone(),
        sandbox,
        artifacts,
        audit,
    ));
    let fileops = FileService::new(db.clone(), supervisor);

    // Write a test project with files
    let project_dir = tmp.path().join("my_proj");
    std::fs::create_dir_all(&project_dir).unwrap();
    std::fs::write(
        project_dir.join("main.rs"),
        "fn main() { println!(\"Hello\"); }",
    )
    .unwrap();
    std::fs::write(project_dir.join("README.md"), "# Project Readme").unwrap();

    let project_id = Uuid::new_v4();
    let now = chrono::Utc::now().timestamp_millis();
    let metadata = std::fs::metadata(&project_dir).unwrap();
    let root_device = metadata.dev() as i64;
    let root_inode = metadata.ino() as i64;
    db.write(move |tx| {
        tx.execute(
            "INSERT INTO projects (id, name, root, root_device, root_inode, allowed_paths_json, excluded_paths_json, notes, revision, created_at, updated_at, archived)
             VALUES (?1, 'TestProj', ?2, ?3, ?4, '[]', '[]', '', 1, ?5, ?5, 0)",
            rusqlite::params![project_id.to_string(), project_dir.to_str().unwrap(), root_device, root_inode, now],
        )?;
        Ok(())
    })
    .await
    .unwrap();

    // 1. Find files
    let page = fileops
        .find(
            project_id,
            FileQuery {
                q: Some("main".to_string()),
                extension: Some("rs".to_string()),
                path: None,
                limit: Some(10),
                cursor: None,
            },
        )
        .await
        .expect("Find files should succeed");
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].path, "main.rs");

    // 2. Preview file
    let preview = fileops
        .preview(project_id, "main.rs".to_string())
        .await
        .expect("Preview main.rs should succeed");
    assert!(preview.text.contains("fn main()"));
    assert!(!preview.truncated);
    assert_eq!(preview.size_bytes, 32);
}

#[tokio::test]
async fn test_project_service_register_and_scan() {
    let (tmp, db) = test_db().await;
    let artifacts_dir = tmp.path().join("artifacts");
    let artifacts = Arc::new(ArtifactStore::new(artifacts_dir, db.clone()));
    let project_service = ProjectService::new(db.clone(), artifacts);

    let session = SessionContext {
        id: Uuid::new_v4(),
        owner_uid: 1000,
        expires_at: chrono::Utc::now().timestamp_millis() + 3_600_000,
        csrf_hash: Digest::ZERO,
    };

    // 1. Registering root directory `/` or system paths must be rejected
    let root_err = project_service
        .register(
            ProjectInput {
                name: "Root".to_string(),
                root: "/".to_string(),
                allowed_paths: vec![],
                excluded_paths: vec![],
                notes: "root".to_string(),
                preferred_model_id: None,
                preferred_tool_id: None,
                owner_ack: true,
            },
            &session,
        )
        .await
        .expect_err("Registering / must be rejected");
    assert_eq!(root_err.code(), ErrorCode::ScopeDenied);

    let etc_err = project_service
        .register(
            ProjectInput {
                name: "Etc".to_string(),
                root: "/etc".to_string(),
                allowed_paths: vec![],
                excluded_paths: vec![],
                notes: "etc".to_string(),
                preferred_model_id: None,
                preferred_tool_id: None,
                owner_ack: true,
            },
            &session,
        )
        .await
        .expect_err("Registering /etc must be rejected");
    assert_eq!(etc_err.code(), ErrorCode::ScopeDenied);

    // 2. Registering a valid scoped project directory succeeds
    let valid_proj_path = tmp.path().join("valid_project");
    std::fs::create_dir_all(&valid_proj_path).unwrap();
    std::fs::write(valid_proj_path.join("lib.rs"), "pub fn test() {}").unwrap();

    let proj = project_service
        .register(
            ProjectInput {
                name: "ValidProj".to_string(),
                root: valid_proj_path.to_str().unwrap().to_string(),
                allowed_paths: vec![],
                excluded_paths: vec![],
                notes: "valid".to_string(),
                preferred_model_id: None,
                preferred_tool_id: None,
                owner_ack: true,
            },
            &session,
        )
        .await
        .expect("Registering valid project must succeed");
    assert_eq!(proj.name, "ValidProj");

    // 3. Scan project generates scan artifact
    let cancel = CancellationToken::new();
    let scan_art = project_service
        .scan(proj.id, cancel.clone())
        .await
        .expect("Scanning project must succeed");
    assert_eq!(scan_art.kind, maya_mini::types::ArtifactKind::Scan);
    assert!(scan_art.size_bytes > 0);
}
