use crate::error::{AppError, ErrorCode, Result};
use crate::types::{
    Capability, CapabilityDetails, CapabilityState, NetworkMode, ProcessLimits, SandboxSpec,
    SpawnSpec,
};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug)]
pub struct SandboxBuilder {
    bwrap_path: Option<PathBuf>,
    cached_capability: Arc<RwLock<Option<Capability>>>,
}

impl Default for SandboxBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl SandboxBuilder {
    pub fn new() -> Self {
        let bwrap_path = Self::find_bwrap();
        Self {
            bwrap_path,
            cached_capability: Arc::new(RwLock::new(None)),
        }
    }

    fn find_bwrap() -> Option<PathBuf> {
        let candidates = ["/usr/bin/bwrap", "/bin/bwrap", "/usr/local/bin/bwrap"];
        for path in &candidates {
            let p = PathBuf::from(path);
            if p.is_file() {
                return Some(p);
            }
        }
        // Fall back to PATH lookup
        if let Ok(path_var) = std::env::var("PATH") {
            for dir in std::env::split_paths(&path_var) {
                let candidate = dir.join("bwrap");
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
        None
    }

    pub fn bwrap_path(&self) -> Option<&Path> {
        self.bwrap_path.as_deref()
    }

    pub async fn probe(&self) -> Result<Capability> {
        {
            let read = self.cached_capability.read().await;
            if let Some(cap) = &*read {
                return Ok(cap.clone());
            }
        }

        let cap = self.probe_internal();
        let mut write = self.cached_capability.write().await;
        *write = Some(cap.clone());
        Ok(cap)
    }

    pub fn probe_sync(&self) -> Capability {
        self.probe_internal()
    }

    fn probe_internal(&self) -> Capability {
        let now = chrono::Utc::now().timestamp_millis();
        let bwrap = match &self.bwrap_path {
            Some(p) => p,
            None => {
                return Capability {
                    key: "sandbox".to_string(),
                    state: CapabilityState::Unavailable,
                    reason: Some("bwrap binary not found in standard system paths".to_string()),
                    version: None,
                    checked_at: now,
                    details: CapabilityDetails {
                        limitations: vec!["no bubblewrap binary".to_string()],
                        sandbox_verified: false,
                        ..Default::default()
                    },
                };
            }
        };

        // 1. Get version
        let version_out = Command::new(bwrap).arg("--version").output();
        let version_str = match version_out {
            Ok(out) if out.status.success() => {
                String::from_utf8_lossy(&out.stdout).trim().to_string()
            }
            _ => "unknown".to_string(),
        };

        // 2. Test minimal unshare invocation
        let test_out = Command::new(bwrap)
            .args([
                "--unshare-user",
                "--unshare-ipc",
                "--unshare-pid",
                "--ro-bind-try",
                "/bin",
                "/bin",
                "--ro-bind-try",
                "/usr",
                "/usr",
                "--dev",
                "/dev",
                "--proc",
                "/proc",
                "true",
            ])
            .output();

        match test_out {
            Ok(out) if out.status.success() => Capability {
                key: "sandbox".to_string(),
                state: CapabilityState::Available,
                reason: None,
                version: Some(version_str),
                checked_at: now,
                details: CapabilityDetails {
                    sandbox_verified: true,
                    ..Default::default()
                },
            },
            Ok(out) => {
                let err_text = String::from_utf8_lossy(&out.stderr).trim().to_string();
                let reason = if err_text.is_empty() {
                    format!("bwrap probe exited with code {:?}", out.status.code())
                } else {
                    err_text
                };
                Capability {
                    key: "sandbox".to_string(),
                    state: CapabilityState::Unavailable,
                    reason: Some(reason),
                    version: Some(version_str),
                    checked_at: now,
                    details: CapabilityDetails {
                        limitations: vec!["unshare namespace creation restricted".to_string()],
                        sandbox_verified: false,
                        ..Default::default()
                    },
                }
            }
            Err(e) => Capability {
                key: "sandbox".to_string(),
                state: CapabilityState::Unavailable,
                reason: Some(format!("Failed to execute bwrap probe: {e}")),
                version: Some(version_str),
                checked_at: now,
                details: CapabilityDetails {
                    limitations: vec![e.to_string()],
                    sandbox_verified: false,
                    ..Default::default()
                },
            },
        }
    }

    pub fn wrap(
        &self,
        spec: &SandboxSpec,
        executable: &Path,
        args: &[OsString],
        cwd: &Path,
        env: &[(OsString, OsString)],
        limits: ProcessLimits,
    ) -> Result<SpawnSpec> {
        let bwrap_bin = self.bwrap_path.as_ref().ok_or_else(|| {
            AppError::new(
                ErrorCode::SandboxUnavailable,
                "Bubblewrap (bwrap) binary is not installed on this system",
            )
        })?;

        let probe = self.probe_sync();
        if probe.state != CapabilityState::Available {
            let reason = probe
                .reason
                .unwrap_or_else(|| "Unknown sandbox failure".to_string());
            return Err(AppError::new(
                ErrorCode::SandboxUnavailable,
                format!("Bubblewrap sandbox is unavailable: {reason}"),
            ));
        }

        // 1. Core isolation
        let mut bwrap_args: Vec<OsString> = vec![
            "--die-with-parent".into(),
            "--new-session".into(),
            "--unshare-user".into(),
            "--unshare-ipc".into(),
            "--unshare-pid".into(),
            "--unshare-uts".into(),
            "--unshare-cgroup".into(),
        ];

        // 2. Network policy
        if spec.network == NetworkMode::Denied {
            bwrap_args.push("--unshare-net".into());
        }

        // 3. System read-only mounts
        bwrap_args.push("--ro-bind".into());
        bwrap_args.push("/usr".into());
        bwrap_args.push("/usr".into());

        for try_bind in &["/lib", "/lib64", "/bin", "/sbin"] {
            if Path::new(try_bind).exists() {
                bwrap_args.push("--ro-bind-try".into());
                bwrap_args.push((*try_bind).into());
                bwrap_args.push((*try_bind).into());
            }
        }

        // 4. Minimal /etc mounts (never bind full /etc)
        for etc_file in &[
            "/etc/resolv.conf",
            "/etc/ssl",
            "/etc/ca-certificates",
            "/etc/passwd",
        ] {
            if Path::new(etc_file).exists() {
                bwrap_args.push("--ro-bind-try".into());
                bwrap_args.push((*etc_file).into());
                bwrap_args.push((*etc_file).into());
            }
        }

        // 5. Minimal pseudo-filesystems
        bwrap_args.push("--dev".into());
        bwrap_args.push("/dev".into());
        bwrap_args.push("--proc".into());
        bwrap_args.push("/proc".into());
        bwrap_args.push("--tmpfs".into());
        bwrap_args.push("/tmp".into());
        bwrap_args.push("--tmpfs".into());
        bwrap_args.push("/run".into());

        // 6. Configured read roots
        for rroot in &spec.read_roots {
            bwrap_args.push("--ro-bind".into());
            bwrap_args.push(rroot.as_os_str().to_os_string());
            bwrap_args.push(rroot.as_os_str().to_os_string());
        }

        // 7. Writable stage is the only persistent writable tree
        bwrap_args.push("--bind".into());
        bwrap_args.push(spec.stage.as_os_str().to_os_string());
        bwrap_args.push(spec.stage.as_os_str().to_os_string());

        for wroot in &spec.writable_roots {
            if wroot != &spec.stage {
                bwrap_args.push("--bind".into());
                bwrap_args.push(wroot.as_os_str().to_os_string());
                bwrap_args.push(wroot.as_os_str().to_os_string());
            }
        }

        // 8. Synthetic private HOME
        bwrap_args.push("--dir".into());
        bwrap_args.push("/tmp/home".into());
        bwrap_args.push("--setenv".into());
        bwrap_args.push("HOME".into());
        bwrap_args.push("/tmp/home".into());

        // 9. Sanitized minimal environment
        bwrap_args.push("--setenv".into());
        bwrap_args.push("TMPDIR".into());
        bwrap_args.push("/tmp".into());

        bwrap_args.push("--setenv".into());
        bwrap_args.push("LANG".into());
        bwrap_args.push("C.UTF-8".into());

        for (k, v) in env {
            bwrap_args.push("--setenv".into());
            bwrap_args.push(k.clone());
            bwrap_args.push(v.clone());
        }

        // 10. Chdir
        bwrap_args.push("--chdir".into());
        bwrap_args.push(cwd.as_os_str().to_os_string());

        // 11. Command separator and argv
        bwrap_args.push("--".into());
        bwrap_args.push(executable.as_os_str().to_os_string());
        for arg in args {
            bwrap_args.push(arg.clone());
        }

        Ok(SpawnSpec {
            executable: bwrap_bin.clone(),
            args: bwrap_args,
            cwd: spec.stage.clone(),
            env: Vec::new(),
            limits,
        })
    }
}
