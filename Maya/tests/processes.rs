mod common;

use common::test_db;
use maya_mini::artifacts::ArtifactStore;
use maya_mini::audit::AuditLog;
use maya_mini::error::ErrorCode;
use maya_mini::policy::ExecutionPermit;
use maya_mini::process::ProcessSupervisor;
use maya_mini::sandbox::SandboxBuilder;
use maya_mini::types::{
    ActionClass, ApprovalScope, CapabilityState, Digest, NetworkMode, ProcessCategory,
    ProcessLimits, ProcessSpec, SandboxSpec,
};
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tempfile::tempdir;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[tokio::test]
async fn extended_24_sandbox_probe_and_wrap_fail_closed() {
    let sandbox = SandboxBuilder::new();
    let cap = sandbox.probe().await.unwrap();

    assert_eq!(cap.key, "sandbox");

    let stage_dir = tempdir().unwrap();
    let spec = SandboxSpec {
        stage: stage_dir.path().to_path_buf(),
        read_roots: vec![PathBuf::from("/usr")],
        writable_roots: vec![stage_dir.path().to_path_buf()],
        network: NetworkMode::Denied,
        deadline: chrono::Utc::now().timestamp_millis() + 60_000,
    };

    let limits = ProcessLimits::default();
    let executable = PathBuf::from("/bin/echo");
    let args = vec![OsString::from("hello")];
    let cwd = stage_dir.path().to_path_buf();
    let env = vec![];

    if cap.state == CapabilityState::Available {
        let spawn_spec = sandbox
            .wrap(&spec, &executable, &args, &cwd, &env, limits)
            .unwrap();
        // Check core isolation arguments
        let args_str: Vec<String> = spawn_spec
            .args
            .iter()
            .map(|a| a.to_string_lossy().to_string())
            .collect();
        assert!(args_str.contains(&"--unshare-user".to_string()));
        assert!(args_str.contains(&"--unshare-pid".to_string()));
        assert!(args_str.contains(&"--unshare-net".to_string()));
        assert!(args_str.contains(&"--die-with-parent".to_string()));
        assert!(args_str.contains(&"--new-session".to_string()));
    } else {
        // Fail-closed test: If sandbox unavailable on host, wrap MUST fail with SandboxUnavailable
        let err = sandbox
            .wrap(&spec, &executable, &args, &cwd, &env, limits)
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::SandboxUnavailable);
    }
}

#[tokio::test]
async fn extended_25_process_supervisor_lease_and_bounds() {
    let (_dir, db) = test_db().await;
    let artifacts_dir = tempdir().unwrap();
    let artifacts = Arc::new(ArtifactStore::new(
        artifacts_dir.path().to_path_buf(),
        db.clone(),
    ));
    let audit = Arc::new(AuditLog::new(
        db.clone(),
        Arc::new(maya_mini::audit::EventHub::new()),
    ));
    let sandbox = Arc::new(SandboxBuilder::new());

    let supervisor = ProcessSupervisor::new(db.clone(), sandbox.clone(), artifacts, audit);
    assert!(!supervisor.boot_id().is_empty());

    let stage_dir = tempdir().unwrap();
    let job_id = Uuid::new_v4();
    let step_id = Uuid::new_v4();

    let limits = ProcessLimits {
        max_output_bytes: 1024,
        max_line_bytes: 256,
        max_open_files: 64,
        max_processes: 16,
        max_file_bytes: 1024 * 1024,
        address_space_bytes: None,
    };

    let spec = ProcessSpec {
        executable: PathBuf::from("/bin/echo"),
        args: vec![OsString::from("test-output-stream")],
        cwd: stage_dir.path().to_path_buf(),
        env: vec![],
        stdin: None,
        timeout: Duration::from_secs(5),
        limits,
        sandbox: SandboxSpec {
            stage: stage_dir.path().to_path_buf(),
            read_roots: vec![],
            writable_roots: vec![stage_dir.path().to_path_buf()],
            network: NetworkMode::Denied,
            deadline: chrono::Utc::now().timestamp_millis() + 10_000,
        },
        category: ProcessCategory::Coding,
        job_id: None,
    };

    let permit = ExecutionPermit::new_test_permit(
        job_id,
        step_id,
        vec![ActionClass::RunCommand],
        ApprovalScope::default(),
        Digest::ZERO,
        vec![],
        (
            chrono::Utc::now().timestamp_millis() + 60_000,
            std::time::Instant::now() + Duration::from_secs(60),
        ),
    );

    let cancel = CancellationToken::new();
    let res = supervisor.run(spec, &permit, cancel).await;

    let cap = sandbox.probe().await.unwrap();
    if cap.state == CapabilityState::Available {
        let p_res = res.unwrap();
        assert_eq!(p_res.exit_code, Some(0));
        assert!(!p_res.timed_out);
        assert!(!p_res.cancelled);
        assert!(p_res.children_reaped);
    } else {
        // If sandbox unavailable, it MUST return SandboxUnavailable
        assert_eq!(res.unwrap_err().code, ErrorCode::SandboxUnavailable);
    }
}

#[tokio::test]
async fn extended_26_process_supervisor_cancellation_and_reap() {
    let (_dir, db) = test_db().await;
    let artifacts_dir = tempdir().unwrap();
    let artifacts = Arc::new(ArtifactStore::new(
        artifacts_dir.path().to_path_buf(),
        db.clone(),
    ));
    let audit = Arc::new(AuditLog::new(
        db.clone(),
        Arc::new(maya_mini::audit::EventHub::new()),
    ));
    let sandbox = Arc::new(SandboxBuilder::new());

    let supervisor = ProcessSupervisor::new(db.clone(), sandbox.clone(), artifacts, audit);
    let stage_dir = tempdir().unwrap();
    let job_id = Uuid::new_v4();
    let step_id = Uuid::new_v4();

    let limits = ProcessLimits::default();
    let spec = ProcessSpec {
        executable: PathBuf::from("/bin/sleep"),
        args: vec![OsString::from("10")],
        cwd: stage_dir.path().to_path_buf(),
        env: vec![],
        stdin: None,
        timeout: Duration::from_secs(30),
        limits,
        sandbox: SandboxSpec {
            stage: stage_dir.path().to_path_buf(),
            read_roots: vec![],
            writable_roots: vec![stage_dir.path().to_path_buf()],
            network: NetworkMode::Denied,
            deadline: chrono::Utc::now().timestamp_millis() + 60_000,
        },
        category: ProcessCategory::Coding,
        job_id: None,
    };

    let permit = ExecutionPermit::new_test_permit(
        job_id,
        step_id,
        vec![ActionClass::RunCommand],
        ApprovalScope::default(),
        Digest::ZERO,
        vec![],
        (
            chrono::Utc::now().timestamp_millis() + 60_000,
            std::time::Instant::now() + Duration::from_secs(60),
        ),
    );

    let cancel = CancellationToken::new();
    let cancel_trigger = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        cancel_trigger.cancel();
    });

    let res = supervisor.run(spec, &permit, cancel).await;
    let cap = sandbox.probe().await.unwrap();
    if cap.state == CapabilityState::Available {
        let p_res = res.unwrap();
        assert!(p_res.cancelled);
        assert!(p_res.children_reaped);
    } else {
        assert_eq!(res.unwrap_err().code, ErrorCode::SandboxUnavailable);
    }
}
