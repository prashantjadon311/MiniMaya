use crate::coding::CodingRunner;
use crate::error::{AppError, ErrorCode, Result};
use crate::policy::ExecutionPermit;
use crate::process::ProcessSupervisor;
use crate::provider::ProviderRegistry;
use crate::secrets::{SecretStore, SecretString};
use crate::types::{
    ActionClass, Capability, CapabilityDetails, CapabilityState, ChangeSet, CodingOutcome,
    CodingRequest, Digest, Model, NetworkMode, ProcessCategory, ProcessLimits, ProcessSpec,
    Provider, SandboxSpec, Tool,
};
use futures_util::future::BoxFuture;
use sha2::{Digest as ShaDigest, Sha256};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Clone)]
pub struct RelaySession {
    pub token: SecretString,
    pub port: u16,
    pub max_requests: u32,
    pub deadline: std::time::Instant,
    pub provider: Provider,
    pub model: Model,
    pub secrets: Arc<dyn SecretStore>,
}

pub struct RelayPolicy {
    pub max_requests: u32,
}

impl Default for RelayPolicy {
    fn default() -> Self {
        Self { max_requests: 128 }
    }
}

pub struct CodexRunner {
    supervisor: Arc<ProcessSupervisor>,
    provider_registry: Arc<ProviderRegistry>,
    secrets: Arc<dyn SecretStore>,
}

impl CodexRunner {
    pub fn new(
        supervisor: Arc<ProcessSupervisor>,
        provider_registry: Arc<ProviderRegistry>,
        secrets: Arc<dyn SecretStore>,
    ) -> Self {
        Self {
            supervisor,
            provider_registry,
            secrets,
        }
    }
}

impl CodingRunner for CodexRunner {
    fn probe<'a>(&'a self, tool: &'a Tool) -> BoxFuture<'a, Result<Capability>> {
        Box::pin(async move {
            let exe_path = Path::new(&tool.executable);
            if !exe_path.exists() {
                return Ok(Capability {
                    key: "codex".to_string(),
                    state: CapabilityState::Unavailable,
                    reason: Some(format!("Executable does not exist: {}", tool.executable)),
                    version: None,
                    checked_at: chrono::Utc::now().timestamp_millis(),
                    details: CapabilityDetails::default(),
                });
            }

            // Probe with --version
            let output = tokio::process::Command::new(&tool.executable)
                .arg("--version")
                .output()
                .await;

            match output {
                Ok(out) => {
                    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
                    if out.status.success() {
                        Ok(Capability {
                            key: "codex".to_string(),
                            state: CapabilityState::Available,
                            reason: Some("Codex CLI verified".to_string()),
                            version: Some(stdout),
                            checked_at: chrono::Utc::now().timestamp_millis(),
                            details: CapabilityDetails::default(),
                        })
                    } else {
                        Ok(Capability {
                            key: "codex".to_string(),
                            state: CapabilityState::Unavailable,
                            reason: Some(format!(
                                "Codex CLI exited with non-zero: {:?}",
                                out.status.code()
                            )),
                            version: Some(stdout),
                            checked_at: chrono::Utc::now().timestamp_millis(),
                            details: CapabilityDetails::default(),
                        })
                    }
                }
                Err(e) => Ok(Capability {
                    key: "codex".to_string(),
                    state: CapabilityState::Unavailable,
                    reason: Some(format!("Failed to spawn Codex CLI: {e}")),
                    version: None,
                    checked_at: chrono::Utc::now().timestamp_millis(),
                    details: CapabilityDetails::default(),
                }),
            }
        })
    }

    fn execute<'a>(
        &'a self,
        request: CodingRequest,
        permit: &'a ExecutionPermit,
        cancel: CancellationToken,
    ) -> BoxFuture<'a, Result<CodingOutcome>> {
        Box::pin(async move {
            permit.permits(&ActionClass::ProjectEdit, None)?;

            let tool = request.tool.clone().ok_or_else(|| {
                AppError::new(
                    ErrorCode::InvalidRequest,
                    "No tool specified for Codex runner",
                )
            })?;

            let model = request.model.clone().ok_or_else(|| {
                AppError::new(
                    ErrorCode::ModelUnavailable,
                    "No model specified for Codex runner",
                )
            })?;

            let provider = self
                .provider_registry
                .get_provider(model.provider_id)
                .await?;

            // Generate ephemeral relay token
            let raw_token = format!("maya_relay_{}", Uuid::new_v4().simple());
            let relay_token = SecretString::new(raw_token.clone());

            // Bind relay listener
            let listener = TcpListener::bind("127.0.0.1:0").await.map_err(|e| {
                AppError::new(
                    ErrorCode::Internal,
                    format!("Failed to bind relay port: {e}"),
                )
            })?;
            let relay_port = listener
                .local_addr()
                .map_err(|e| {
                    AppError::new(
                        ErrorCode::Internal,
                        format!("Failed to get relay addr: {e}"),
                    )
                })?
                .port();

            let session = RelaySession {
                token: relay_token,
                port: relay_port,
                max_requests: 128,
                deadline: std::time::Instant::now() + std::time::Duration::from_secs(1200),
                provider: provider.clone(),
                model: model.clone(),
                secrets: self.secrets.clone(),
            };

            let relay_cancel = CancellationToken::new();
            let relay_cancel_clone = relay_cancel.clone();
            let session_clone = session.clone();

            let relay_handle = tokio::spawn(async move {
                let _ = serve_relay_listener(listener, session_clone, relay_cancel_clone).await;
            });

            // Set up sterile CODEX_HOME
            let codex_home = tempfile::tempdir().map_err(|e| {
                AppError::new(
                    ErrorCode::Internal,
                    format!("Failed to create CODEX_HOME: {e}"),
                )
            })?;
            let config_path = codex_home.path().join("config.toml");
            let config_content = format!(
                r#"[custom_providers.maya_relay]
base_url = "http://127.0.0.1:{relay_port}"
token = "{raw_token}"
wire_api = "responses"
requires_openai_auth = false
"#
            );
            fs::write(&config_path, config_content).map_err(|e| {
                AppError::new(
                    ErrorCode::Internal,
                    format!("Failed to write CODEX config: {e}"),
                )
            })?;

            // Build ProcessSpec
            let spec = ProcessSpec {
                executable: PathBuf::from(&tool.executable),
                args: vec![
                    OsString::from("exec"),
                    OsString::from("--json"),
                    OsString::from("--sandbox"),
                    OsString::from("workspace-write"),
                    OsString::from("--provider=maya_relay"),
                    OsString::from(format!("--model={}", model.model_id)),
                    OsString::from(&request.plan.goal),
                ],
                cwd: request.stage.path.clone(),
                env: vec![
                    (
                        OsString::from("CODEX_HOME"),
                        codex_home.path().as_os_str().to_os_string(),
                    ),
                    (
                        OsString::from("HOME"),
                        codex_home.path().as_os_str().to_os_string(),
                    ),
                ],
                stdin: None,
                timeout: std::time::Duration::from_secs(300),
                limits: ProcessLimits::default(),
                sandbox: SandboxSpec {
                    stage: request.stage.path.clone(),
                    read_roots: vec![codex_home.path().to_path_buf()],
                    writable_roots: vec![
                        request.stage.path.clone(),
                        codex_home.path().to_path_buf(),
                    ],
                    network: NetworkMode::CliParentOnly,
                    deadline: chrono::Utc::now().timestamp_millis() + 300_000,
                },
                category: ProcessCategory::Coding,
                job_id: Some(request.job_id),
            };

            let proc_result = self.supervisor.run(spec, permit, cancel).await;

            // Stop relay
            relay_cancel.cancel();
            let _ = relay_handle.await;

            let result = proc_result?;

            let change_set = ChangeSet {
                baseline_digest: request.stage.baseline_digest,
                changes: Vec::new(),
                digest: compute_sha256(b"codex_changes"),
            };

            Ok(CodingOutcome {
                process: Some(result),
                changes: change_set,
                summary: "Codex CLI execution completed".to_string(),
            })
        })
    }
}

pub async fn serve_relay(session: RelaySession, cancel: CancellationToken) -> Result<()> {
    let listener = TcpListener::bind(format!("127.0.0.1:{}", session.port))
        .await
        .map_err(|e| AppError::new(ErrorCode::Internal, format!("Failed to bind relay: {e}")))?;
    serve_relay_listener(listener, session, cancel).await
}

pub async fn serve_relay_listener(
    listener: TcpListener,
    session: RelaySession,
    cancel: CancellationToken,
) -> Result<()> {
    let request_count = Arc::new(AtomicU32::new(0));

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                break;
            }
            conn_res = listener.accept() => {
                let (mut socket, _) = match conn_res {
                    Ok(pair) => pair,
                    Err(_) => break,
                };

                let session = session.clone();
                let req_count = request_count.clone();

                tokio::spawn(async move {
                    let mut buf = vec![0u8; 8192];
                    let n = match socket.read(&mut buf).await {
                        Ok(n) if n > 0 => n,
                        _ => return,
                    };

                    let req_str = String::from_utf8_lossy(&buf[..n]);

                    let mut lines = req_str.lines();
                    let request_line = match lines.next() {
                        Some(l) => l,
                        None => {
                            let _ = socket.write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n").await;
                            return;
                        }
                    };

                    let parts: Vec<&str> = request_line.split_whitespace().collect();
                    if parts.len() < 2 {
                        let _ = socket.write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n").await;
                        return;
                    }

                    let method = parts[0];
                    let path = parts[1];

                    if method != "POST" {
                        let _ = socket
                            .write_all(b"HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                            .await;
                        return;
                    }

                    if path != "/responses" {
                        let _ = socket
                            .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                            .await;
                        return;
                    }

                    let expected_header = format!("authorization: bearer {}", session.token.as_str());
                    let has_auth = req_str
                        .lines()
                        .any(|l| l.trim().to_ascii_lowercase() == expected_header);
                    if !has_auth {
                        let _ = socket
                            .write_all(b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                            .await;
                        return;
                    }

                    let current_count = req_count.fetch_add(1, Ordering::SeqCst);
                    if current_count >= session.max_requests {
                        let _ = socket
                            .write_all(b"HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                            .await;
                        return;
                    }

                    let body_start = req_str.find("\r\n\r\n").map(|idx| idx + 4).unwrap_or(n);
                    let body = &req_str[body_start..];

                    let secret = match session.secrets.get(session.provider.id).await {
                        Ok(s) => s,
                        Err(_) => {
                            let _ = socket.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n").await;
                            return;
                        }
                    };

                    let base = session.provider.endpoint.trim_end_matches('/');
                    let upstream_url = format!("{base}/responses");

                    let client = reqwest::Client::new();
                    let upstream_resp = client
                        .post(&upstream_url)
                        .header("Authorization", format!("Bearer {}", secret.as_str()))
                        .header("Content-Type", "application/json")
                        .body(body.to_string())
                        .send()
                        .await;

                    match upstream_resp {
                        Ok(resp) => {
                            let status = resp.status().as_u16();
                            let status_text = resp.status().canonical_reason().unwrap_or("OK");
                            let bytes = resp.bytes().await.unwrap_or_default();

                            let header_str = format!(
                                "HTTP/1.1 {status} {status_text}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                bytes.len()
                            );

                            let _ = socket.write_all(header_str.as_bytes()).await;
                            let _ = socket.write_all(&bytes).await;
                        }
                        Err(_) => {
                            let _ = socket.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n").await;
                        }
                    }
                });
            }
        }
    }

    Ok(())
}

fn compute_sha256(bytes: &[u8]) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    Digest::new(hasher.finalize().into())
}
