use crate::artifacts::{ArtifactInput, ArtifactStore};
use crate::audit::AuditLog;
use crate::db::Db;
use crate::error::{AppError, ErrorCode, Result};
use crate::policy::ExecutionPermit;
use crate::sandbox::SandboxBuilder;
use crate::types::{ActionClass, ArtifactKind, ProcessResult, ProcessSpec, Provenance};
use rusqlite::params;
use rustix::process::{
    Pid, Resource, Rlimit, Signal, getpgid, kill_process_group, setpgid, setrlimit,
};
use std::fs;
use std::io::Cursor;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::time::{Instant, sleep};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Debug)]
pub struct ProcessSupervisor {
    db: Db,
    sandbox: Arc<SandboxBuilder>,
    artifacts: Arc<ArtifactStore>,
    audit: Arc<AuditLog>,
    boot_id: String,
}

impl ProcessSupervisor {
    pub fn new(
        db: Db,
        sandbox: Arc<SandboxBuilder>,
        artifacts: Arc<ArtifactStore>,
        audit: Arc<AuditLog>,
    ) -> Self {
        let boot_id = fs::read_to_string("/proc/sys/kernel/random/boot_id")
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|_| Uuid::new_v4().to_string());

        Self {
            db,
            sandbox,
            artifacts,
            audit,
            boot_id,
        }
    }

    pub fn boot_id(&self) -> &str {
        &self.boot_id
    }

    pub fn audit(&self) -> &Arc<AuditLog> {
        &self.audit
    }

    pub async fn run(
        &self,
        spec: ProcessSpec,
        permit: &ExecutionPermit,
        cancel: CancellationToken,
    ) -> Result<ProcessResult> {
        // 1. Validate policy permit for command execution
        permit
            .permits(&ActionClass::RunCommand, None)
            .or_else(|_| permit.permits(&ActionClass::RunTest, None))?;

        let now = chrono::Utc::now().timestamp_millis();
        let lease_id = Uuid::new_v4();
        let job_id_str = spec.job_id.map(|id| id.to_string());
        let cat_str = spec.category.as_str().to_string();
        let boot_id = self.boot_id.clone();

        // 2. Persist starting lease in process_leases table
        let lid_str = lease_id.to_string();
        self.db
            .write({
                let lid = lid_str.clone();
                let jid = job_id_str.clone();
                let cat = cat_str.clone();
                let bid = boot_id.clone();
                move |tx| {
                    tx.execute(
                        r#"INSERT INTO process_leases
                           (id, job_id, category, pid, start_ticks, pgid, boot_id, owned, state, created_at, updated_at)
                           VALUES (?1, ?2, ?3, NULL, NULL, NULL, ?4, 1, 'starting', ?5, ?5)"#,
                        params![lid, jid, cat, bid, now],
                    )
                    .map_err(|e| AppError::new(ErrorCode::DbUnavailable, e.to_string()))?;
                    Ok(())
                }
            })
            .await?;

        // 3. Wrap with bubblewrap sandbox
        let spawn_spec = match self.sandbox.wrap(
            &spec.sandbox,
            &spec.executable,
            &spec.args,
            &spec.cwd,
            &spec.env,
            spec.limits,
        ) {
            Ok(s) => s,
            Err(e) => {
                // Update lease to unknown/exited
                let _ = self
                    .db
                    .write({
                        let lid = lid_str.clone();
                        let n = chrono::Utc::now().timestamp_millis();
                        move |tx| {
                            tx.execute(
                                "UPDATE process_leases SET state = 'exited', updated_at = ?1 WHERE id = ?2",
                                params![n, lid],
                            )
                            .map_err(|e| AppError::new(ErrorCode::DbUnavailable, e.to_string()))?;
                            Ok(())
                        }
                    })
                    .await;
                return Err(e);
            }
        };

        // 4. Construct command with process group and rlimits
        let limits = spawn_spec.limits;
        let mut cmd = Command::new(&spawn_spec.executable);
        cmd.args(&spawn_spec.args);
        cmd.current_dir(&spawn_spec.cwd);
        for (k, v) in &spawn_spec.env {
            cmd.env(k, v);
        }
        cmd.kill_on_drop(true);
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        if spec.stdin.is_some() {
            cmd.stdin(std::process::Stdio::piped());
        } else {
            cmd.stdin(std::process::Stdio::null());
        }

        // Apply pre_exec for process group and resource limits
        unsafe {
            cmd.pre_exec(move || {
                // New process group
                setpgid(None, None)
                    .map_err(|e| std::io::Error::from_raw_os_error(e.raw_os_error()))?;

                // NOFILE limit
                let _ = setrlimit(
                    Resource::Nofile,
                    Rlimit {
                        current: Some(limits.max_open_files),
                        maximum: Some(limits.max_open_files),
                    },
                );

                // NPROC limit
                let _ = setrlimit(
                    Resource::Nproc,
                    Rlimit {
                        current: Some(limits.max_processes),
                        maximum: Some(limits.max_processes),
                    },
                );

                // FSIZE limit
                let _ = setrlimit(
                    Resource::Fsize,
                    Rlimit {
                        current: Some(limits.max_file_bytes),
                        maximum: Some(limits.max_file_bytes),
                    },
                );

                // AS (address space) limit if specified
                if let Some(as_bytes) = limits.address_space_bytes {
                    let _ = setrlimit(
                        Resource::As,
                        Rlimit {
                            current: Some(as_bytes),
                            maximum: Some(as_bytes),
                        },
                    );
                }

                Ok(())
            });
        }

        // 5. Spawn child
        let mut child = cmd.spawn().map_err(|e| {
            AppError::new(
                ErrorCode::Internal,
                format!("Failed to spawn child process: {e}"),
            )
        })?;

        let pid = match child.id() {
            Some(p) => p as i32,
            None => {
                return Err(AppError::new(
                    ErrorCode::Internal,
                    "Child process exited immediately without PID",
                ));
            }
        };

        let pid_val = Pid::from_raw(pid);
        let pgid_val = getpgid(pid_val).unwrap_or(pid_val.unwrap());
        let pgid = pgid_val.as_raw_nonzero().get();
        let start_ticks = Self::read_start_ticks(pid).unwrap_or(now);

        // Update lease state to running
        let lid_clone = lid_str.clone();
        let running_now = chrono::Utc::now().timestamp_millis();
        self.db
            .write(move |tx| {
                tx.execute(
                    r#"UPDATE process_leases
                       SET state = 'running', pid = ?1, start_ticks = ?2, pgid = ?3, updated_at = ?4
                       WHERE id = ?5"#,
                    params![pid as i64, start_ticks, pgid as i64, running_now, lid_clone],
                )
                .map_err(|e| AppError::new(ErrorCode::DbUnavailable, e.to_string()))?;
                Ok(())
            })
            .await?;

        // 6. Write stdin if provided
        if let Some(stdin_bytes) = spec.stdin
            && let Some(mut stdin) = child.stdin.take()
        {
            use tokio::io::AsyncWriteExt;
            let _ = stdin.write_all(&stdin_bytes).await;
            drop(stdin);
        }

        // 7. Concurrently drain stdout and stderr with output capping
        let stdout_pipe = child.stdout.take();
        let stderr_pipe = child.stderr.take();

        let max_output = limits.max_output_bytes;

        let stdout_task = tokio::spawn(async move {
            let mut buf = Vec::new();
            let mut truncated = false;
            if let Some(mut pipe) = stdout_pipe {
                let mut chunk = [0u8; 4096];
                loop {
                    match pipe.read(&mut chunk).await {
                        Ok(0) => break,
                        Ok(n) => {
                            if (buf.len() + n) as u64 <= max_output {
                                buf.extend_from_slice(&chunk[..n]);
                            } else {
                                let remaining = (max_output as usize).saturating_sub(buf.len());
                                if remaining > 0 {
                                    buf.extend_from_slice(&chunk[..remaining]);
                                }
                                truncated = true;
                            }
                        }
                        Err(_) => break,
                    }
                }
            }
            (buf, truncated)
        });

        let stderr_task = tokio::spawn(async move {
            let mut buf = Vec::new();
            let mut truncated = false;
            if let Some(mut pipe) = stderr_pipe {
                let mut chunk = [0u8; 4096];
                loop {
                    match pipe.read(&mut chunk).await {
                        Ok(0) => break,
                        Ok(n) => {
                            if (buf.len() + n) as u64 <= max_output {
                                buf.extend_from_slice(&chunk[..n]);
                            } else {
                                let remaining = (max_output as usize).saturating_sub(buf.len());
                                if remaining > 0 {
                                    buf.extend_from_slice(&chunk[..remaining]);
                                }
                                truncated = true;
                            }
                        }
                        Err(_) => break,
                    }
                }
            }
            (buf, truncated)
        });

        // 8. Wait for child with timeout and cancellation
        let mut timed_out = false;
        let mut cancelled = false;

        let status = tokio::select! {
            res = child.wait() => {
                res.ok()
            }
            _ = sleep(spec.timeout) => {
                timed_out = true;
                Self::terminate_process_group(pgid).await;
                child.wait().await.ok()
            }
            _ = cancel.cancelled() => {
                cancelled = true;
                Self::terminate_process_group(pgid).await;
                child.wait().await.ok()
            }
        };

        // 9. Collect outputs
        let (raw_stdout, stdout_trunc) = stdout_task.await.unwrap_or_default();
        let (raw_stderr, stderr_trunc) = stderr_task.await.unwrap_or_default();
        let truncated = stdout_trunc || stderr_trunc;

        // Redact secrets and strip ANSI from outputs
        let stdout_str = String::from_utf8_lossy(&raw_stdout);
        let redacted_stdout = crate::policy::Policy::redact_static(&stdout_str);

        let stderr_str = String::from_utf8_lossy(&raw_stderr);
        let redacted_stderr = crate::policy::Policy::redact_static(&stderr_str);

        // 10. Update lease state to exited
        let exit_now = chrono::Utc::now().timestamp_millis();
        let _ = self
            .db
            .write({
                let lid = lid_str.clone();
                move |tx| {
                    tx.execute(
                        "UPDATE process_leases SET state = 'exited', updated_at = ?1 WHERE id = ?2",
                        params![exit_now, lid],
                    )
                    .map_err(|e| AppError::new(ErrorCode::DbUnavailable, e.to_string()))?;
                    Ok(())
                }
            })
            .await;

        // 11. Persist stdout and stderr into ArtifactStore
        let stdout_bytes = redacted_stdout.into_bytes();
        let stderr_bytes = redacted_stderr.into_bytes();

        let stdout_artifact = self
            .artifacts
            .put(
                ArtifactInput {
                    job_id: spec.job_id,
                    task_id: None,
                    kind: ArtifactKind::Log,
                    mime: "text/plain".to_string(),
                    provenance: Provenance::system(),
                    max_bytes: limits.max_output_bytes,
                    expires_at: None,
                },
                Box::pin(Cursor::new(stdout_bytes)),
            )
            .await?
            .id;

        let stderr_artifact = self
            .artifacts
            .put(
                ArtifactInput {
                    job_id: spec.job_id,
                    task_id: None,
                    kind: ArtifactKind::Log,
                    mime: "text/plain".to_string(),
                    provenance: Provenance::system(),
                    max_bytes: limits.max_output_bytes,
                    expires_at: None,
                },
                Box::pin(Cursor::new(stderr_bytes)),
            )
            .await?
            .id;

        let exit_code = status.as_ref().and_then(|s| s.code());
        #[cfg(unix)]
        let signal = {
            use std::os::unix::process::ExitStatusExt;
            status.as_ref().and_then(|s| s.signal())
        };
        #[cfg(not(unix))]
        let signal = None;

        Ok(ProcessResult {
            exit_code,
            signal,
            timed_out,
            cancelled,
            stdout_artifact,
            stderr_artifact,
            truncated,
            children_reaped: true,
        })
    }

    async fn terminate_process_group(pgid: i32) {
        if pgid <= 0 {
            return;
        }

        if let Some(pgid_pid) = Pid::from_raw(pgid) {
            let _ = kill_process_group(pgid_pid, Signal::TERM);

            let sig_zero = unsafe { Signal::from_raw_unchecked(0) };

            let start = Instant::now();
            while start.elapsed() < Duration::from_secs(3) {
                sleep(Duration::from_millis(100)).await;
                if kill_process_group(pgid_pid, sig_zero).is_err() {
                    return;
                }
            }

            let _ = kill_process_group(pgid_pid, Signal::KILL);

            let kill_start = Instant::now();
            while kill_start.elapsed() < Duration::from_secs(5) {
                sleep(Duration::from_millis(100)).await;
                if kill_process_group(pgid_pid, sig_zero).is_err() {
                    return;
                }
            }
        }
    }

    fn read_start_ticks(pid: i32) -> Option<i64> {
        let stat_path = format!("/proc/{pid}/stat");
        let content = fs::read_to_string(stat_path).ok()?;
        // The starttime is field 22 (1-based index) after the command name in parentheses
        let close_paren = content.rfind(')')?;
        let after_paren = &content[close_paren + 1..];
        let fields: Vec<&str> = after_paren.split_whitespace().collect();
        // field 3 is state (index 0 in fields), so field 22 is index 19 in fields
        if fields.len() > 19 {
            fields[19].parse::<i64>().ok()
        } else {
            None
        }
    }
}
