use crate::approval::ApprovalService;
use crate::artifacts::ArtifactStore;
use crate::assistant::AssistantService;
use crate::audit::{AuditLog, EventHub};
use crate::auth::AuthService;
use crate::backup::BackupService;
use crate::browser::BrowserService;
use crate::checkpoint::CheckpointService;
use crate::codex::CodexRunner;
use crate::coding::CodingService;
use crate::config::RuntimeConfig;
use crate::conversation::ConversationService;
use crate::db::Db;
use crate::desktop::DesktopService;
use crate::error::{AppError, ErrorCode, Result};
use crate::fileops::FileService;
use crate::http;
use crate::memory::MemoryService;
use crate::policy::Policy;
use crate::process::ProcessSupervisor;
use crate::project::ProjectService;
use crate::provider::ProviderRegistry;
use crate::providers;
use crate::reports::ReportService;
use crate::sandbox::SandboxBuilder;
use crate::schedule::{ScheduleWake, Scheduler, SystemClock};
use crate::secrets::{AppSecretStore, SecretStore};
use crate::state::{AppState, ServiceHandles};
use crate::tasks::TaskService;
use crate::telemetry::ResourceMonitor;
use crate::types::ToolKind;
use crate::voice::VoiceService;
use crate::workflow::WorkflowEngine;
use rustix::fs::{FlockOperation, flock};
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, UnixListener, UnixStream};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

pub struct App {
    pub state: Arc<AppState>,
    listener: TcpListener,
    control_listener: UnixListener,
    shutdown: CancellationToken,
    scheduler_wake: mpsc::Sender<ScheduleWake>,
    scheduler_receiver: mpsc::Receiver<ScheduleWake>,
    config: Arc<RuntimeConfig>,
    _instance_lock: File,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum ControlRequest {
    IssueLaunchLink,
    Status,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ControlResponse {
    Link { url: String, expires_at: i64 },
    Status { ready: bool, port: u16 },
    Error { code: ErrorCode, message: String },
}

#[derive(Serialize)]
struct LaunchMetadata {
    port: u16,
    pid: u32,
    started_at: i64,
}

impl App {
    pub async fn build(config: RuntimeConfig) -> Result<Self> {
        let config = Arc::new(config);
        secure_runtime_directories(&config)?;
        let instance_lock = acquire_instance_lock(&config.instance_lock_path())?;
        let db = Db::open(&config.db_path()).await?;
        db.migrate().await?;
        db.write(|tx| {
            tx.execute(
                "UPDATE providers SET secret_ref=NULL,credential_generation=credential_generation+1,revision=revision+1 WHERE secret_ref='session' OR secret_ref LIKE 'session:%'",
                [],
            )
            .map_err(|error| AppError::new(ErrorCode::DbUnavailable, error.to_string()))?;
            tx.execute(
                "UPDATE idempotency_keys SET state='uncertain' WHERE state='pending'",
                [],
            )
            .map_err(|error| AppError::new(ErrorCode::DbUnavailable, error.to_string()))?;
            Ok(())
        })
        .await?;

        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, config.port))
            .await
            .map_err(|error| {
                AppError::new(ErrorCode::Internal, "Failed to bind loopback HTTP listener")
                    .with_source(error)
            })?;
        let port = listener
            .local_addr()
            .map_err(|error| AppError::new(ErrorCode::Internal, error.to_string()))?
            .port();
        let auth = Arc::new(AuthService::new(port));

        let control_path = config.control_socket_path();
        if control_path.exists() {
            std::fs::remove_file(&control_path).map_err(|error| {
                AppError::new(
                    ErrorCode::Internal,
                    "Failed to replace stale control socket",
                )
                .with_source(error)
            })?;
        }
        let control_listener = UnixListener::bind(&control_path).map_err(|error| {
            AppError::new(ErrorCode::Internal, "Failed to bind control socket").with_source(error)
        })?;
        std::fs::set_permissions(&control_path, std::fs::Permissions::from_mode(0o600))
            .map_err(|error| AppError::new(ErrorCode::Internal, error.to_string()))?;

        let events = Arc::new(EventHub::new());
        let audit = Arc::new(AuditLog::new(db.clone(), events.clone()));
        let artifacts = Arc::new(ArtifactStore::new(config.artifacts_dir(), db.clone()));
        let sandbox = Arc::new(SandboxBuilder::new());
        let process = Arc::new(ProcessSupervisor::new(
            db.clone(),
            sandbox.clone(),
            artifacts.clone(),
            audit.clone(),
        ));
        let secrets: Arc<dyn SecretStore> = Arc::new(AppSecretStore::new());
        let providers = Arc::new(ProviderRegistry::new(
            db.clone(),
            secrets.clone(),
            providers::create,
        ));
        let checkpoint = Arc::new(CheckpointService::new(
            db.clone(),
            artifacts.clone(),
            config.runtime_dir.clone(),
        ));
        let mut coding_service = CodingService::new(checkpoint, providers.clone());
        coding_service.register_runner(
            ToolKind::Codex,
            Arc::new(CodexRunner::new(
                process.clone(),
                providers.clone(),
                secrets.clone(),
            )),
        );
        let coding = Arc::new(coding_service);
        let policy = Arc::new(Policy::new(db.clone()));
        let approval = Arc::new(ApprovalService::with_artifacts(
            db.clone(),
            artifacts.clone(),
        ));
        let projects = Arc::new(ProjectService::new(db.clone(), artifacts.clone()));
        let tasks = Arc::new(TaskService::new(db.clone()));
        let desktop = Arc::new(DesktopService::new(db.clone()));
        let scheduler = Arc::new(Scheduler::new(
            db.clone(),
            Arc::new(SystemClock),
            desktop.clone(),
        ));
        let conversations = Arc::new(ConversationService::new(db.clone()));
        let memory = Arc::new(MemoryService::new(db.clone(), artifacts.clone()));
        let reports = Arc::new(ReportService::new(db.clone()));
        let files = Arc::new(FileService::new(db.clone(), process.clone()));
        let workflow = Arc::new(WorkflowEngine::new(
            db.clone(),
            approval.clone(),
            projects.clone(),
            providers.clone(),
            coding.clone(),
            policy.clone(),
            artifacts.clone(),
            files.clone(),
            conversations.clone(),
            events.clone(),
        ));
        let assistant = Arc::new(AssistantService::new(
            conversations.clone(),
            tasks.clone(),
            providers.clone(),
            workflow.clone(),
        ));
        let browser = Arc::new(BrowserService::new(db.clone(), artifacts.clone()));
        let voice = Arc::new(VoiceService::new(
            db.clone(),
            secrets.clone(),
            Some(artifacts.clone()),
        ));
        let telemetry = Arc::new(ResourceMonitor::new(db.clone(), config.clone()));
        let backup = Arc::new(BackupService::new(db.clone(), config.clone()));
        let (scheduler_wake, scheduler_receiver) = mpsc::channel(32);
        let state = Arc::new(AppState::new(ServiceHandles {
            db,
            auth,
            policy,
            approval,
            providers,
            projects,
            tasks,
            scheduler,
            conversations,
            assistant,
            workflow,
            memory,
            reports,
            artifacts,
            audit,
            desktop,
            browser,
            voice,
            telemetry,
            backup,
            files,
            coding,
            process,
            sandbox,
            secrets,
            events,
            schedule_wake: scheduler_wake.clone(),
        }));
        state.workflow.recover().await?;
        state.workflow.resume_safe_chat_jobs().await?;
        write_launch_metadata(&config.launch_json_path(), state.auth.port())?;
        Ok(Self {
            state,
            listener,
            control_listener,
            shutdown: CancellationToken::new(),
            scheduler_wake,
            scheduler_receiver,
            config,
            _instance_lock: instance_lock,
        })
    }

    pub async fn run(self) -> Result<()> {
        let App {
            state,
            listener,
            control_listener,
            shutdown,
            scheduler_wake,
            scheduler_receiver,
            config,
            _instance_lock,
        } = self;
        let scheduler = (*state.scheduler).clone();
        let scheduler_cancel = shutdown.child_token();
        let scheduler_task =
            tokio::spawn(async move { scheduler.run(scheduler_cancel, scheduler_receiver).await });

        let control_state = state.clone();
        let control_cancel = shutdown.child_token();
        let control_task = tokio::spawn(async move {
            serve_control_listener(control_listener, control_state, control_cancel).await
        });

        let link = state.auth.issue_launch_link(state.auth.port()).await?;
        if config.print_launch_link || !config.headless {
            println!("{}", link.url.expose_str());
        }
        if config.open_browser {
            let _ = tokio::process::Command::new("xdg-open")
                .arg(link.url.expose_str())
                .spawn();
        }
        let server_cancel = shutdown.clone();
        let router = http::router(state.clone());
        let server = axum::serve(listener, router)
            .with_graceful_shutdown(async move { server_cancel.cancelled().await });
        tokio::select! {
            result=server=>result.map_err(|error|AppError::new(ErrorCode::Internal,"HTTP server failed").with_source(error))?,
            _=shutdown_signal()=>shutdown.cancel(),
        }
        shutdown.cancel();
        let _ = scheduler_wake.send(ScheduleWake::Shutdown).await;
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), scheduler_task).await;
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), control_task).await;
        let _ = std::fs::remove_file(config.control_socket_path());
        let _ = std::fs::remove_file(config.launch_json_path());
        drop(_instance_lock);
        Ok(())
    }

    pub async fn serve_control(&self, cancel: CancellationToken) -> Result<()> {
        let path = self.config.control_socket_path();
        if path.exists() {
            std::fs::remove_file(&path)
                .map_err(|error| AppError::new(ErrorCode::Internal, error.to_string()))?;
        }
        let listener = UnixListener::bind(path)
            .map_err(|error| AppError::new(ErrorCode::Internal, error.to_string()))?;
        serve_control_listener(listener, self.state.clone(), cancel).await
    }
}

fn write_launch_metadata(path: &Path, port: u16) -> Result<()> {
    let metadata = LaunchMetadata {
        port,
        pid: std::process::id(),
        started_at: chrono::Utc::now().timestamp_millis(),
    };
    let bytes = serde_json::to_vec(&metadata)
        .map_err(|error| AppError::new(ErrorCode::Internal, error.to_string()))?;
    let temporary = path.with_extension(format!("json.{}.tmp", uuid::Uuid::new_v4()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)
        .map_err(|error| {
            AppError::new(ErrorCode::Internal, "Failed to create launch metadata")
                .with_source(error)
        })?;
    file.write_all(&bytes).map_err(|error| {
        AppError::new(ErrorCode::Internal, "Failed to write launch metadata").with_source(error)
    })?;
    file.sync_all().map_err(|error| {
        AppError::new(ErrorCode::Internal, "Failed to sync launch metadata").with_source(error)
    })?;
    std::fs::rename(&temporary, path).map_err(|error| {
        let _ = std::fs::remove_file(&temporary);
        AppError::new(ErrorCode::Internal, "Failed to publish launch metadata").with_source(error)
    })?;
    Ok(())
}

async fn serve_control_listener(
    listener: UnixListener,
    state: Arc<AppState>,
    cancel: CancellationToken,
) -> Result<()> {
    loop {
        let (stream, _) = tokio::select! {value=listener.accept()=>value.map_err(|error|AppError::new(ErrorCode::Internal,error.to_string()))?,_ = cancel.cancelled()=>return Ok(())};
        let peer = stream
            .peer_cred()
            .map_err(|error| AppError::new(ErrorCode::AuthRequired, error.to_string()))?;
        if peer.uid() != rustix::process::geteuid().as_raw() {
            continue;
        }
        let state = state.clone();
        tokio::spawn(async move {
            let _ = handle_control(stream, state).await;
        });
    }
}

async fn handle_control(mut stream: UnixStream, state: Arc<AppState>) -> Result<()> {
    let length = tokio::time::timeout(std::time::Duration::from_secs(2), stream.read_u32())
        .await
        .map_err(|_| AppError::new(ErrorCode::Timeout, "Control request timed out"))?
        .map_err(|error| AppError::new(ErrorCode::InvalidRequest, error.to_string()))?
        as usize;
    if length > 4096 {
        return Err(AppError::new(
            ErrorCode::BodyTooLarge,
            "Control request is too large",
        ));
    }
    let mut bytes = vec![0; length];
    stream
        .read_exact(&mut bytes)
        .await
        .map_err(|error| AppError::new(ErrorCode::InvalidRequest, error.to_string()))?;
    let request: ControlRequest = serde_json::from_slice(&bytes)
        .map_err(|_| AppError::new(ErrorCode::InvalidRequest, "Invalid control request"))?;
    let response = match request {
        ControlRequest::Status => ControlResponse::Status {
            ready: true,
            port: state.auth.port(),
        },
        ControlRequest::IssueLaunchLink => {
            match state.auth.issue_launch_link(state.auth.port()).await {
                Ok(link) => ControlResponse::Link {
                    url: link.url.expose_str().to_string(),
                    expires_at: link.expires_at,
                },
                Err(error) => ControlResponse::Error {
                    code: error.code(),
                    message: error.message().to_string(),
                },
            }
        }
    };
    let encoded = serde_json::to_vec(&response)
        .map_err(|error| AppError::new(ErrorCode::Internal, error.to_string()))?;
    stream
        .write_u32(encoded.len() as u32)
        .await
        .map_err(|error| AppError::new(ErrorCode::Internal, error.to_string()))?;
    stream
        .write_all(&encoded)
        .await
        .map_err(|error| AppError::new(ErrorCode::Internal, error.to_string()))?;
    stream
        .shutdown()
        .await
        .map_err(|error| AppError::new(ErrorCode::Internal, error.to_string()))?;
    Ok(())
}

fn secure_runtime_directories(config: &RuntimeConfig) -> Result<()> {
    for path in [&config.data_dir, &config.cache_dir, &config.runtime_dir] {
        if path
            .symlink_metadata()
            .is_ok_and(|meta| meta.file_type().is_symlink())
        {
            return Err(AppError::new(
                ErrorCode::ScopeDenied,
                "Maya directory cannot be a symlink",
            ));
        }
        std::fs::create_dir_all(path)
            .map_err(|error| AppError::new(ErrorCode::Internal, error.to_string()))?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .map_err(|error| AppError::new(ErrorCode::Internal, error.to_string()))?;
    }
    Ok(())
}
fn acquire_instance_lock(path: &Path) -> Result<File> {
    let mut file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(path)
        .map_err(|error| AppError::new(ErrorCode::Internal, error.to_string()))?;
    flock(&file, FlockOperation::NonBlockingLockExclusive)
        .map_err(|_| AppError::new(ErrorCode::Busy, "Another Maya Mini instance is running"))?;
    file.set_len(0)
        .map_err(|error| AppError::new(ErrorCode::Internal, error.to_string()))?;
    write!(file, "{}", std::process::id())
        .map_err(|error| AppError::new(ErrorCode::Internal, error.to_string()))?;
    file.sync_all()
        .map_err(|error| AppError::new(ErrorCode::Internal, error.to_string()))?;
    Ok(file)
}
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            signal.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {_=ctrl_c=>{},_=terminate=>{}}
}
