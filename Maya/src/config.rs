use crate::error::{AppError, ErrorCode, Result};
use crate::types::{
    AssistantSettings, BrowserSettings, Identity, SecuritySettings, Settings, VoiceSettings,
};
use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default)]
pub struct CliArgs {
    pub port: Option<u16>,
    pub open_browser: bool,
    pub headless: bool,
    pub print_launch_link: bool,
    pub data_dir: Option<PathBuf>,
    pub cache_dir: Option<PathBuf>,
    pub runtime_dir: Option<PathBuf>,
    pub test_mode: bool,
}

impl CliArgs {
    pub fn parse_from<I, T>(args: I) -> Result<Self>
    where
        I: IntoIterator<Item = T>,
        T: Into<String>,
    {
        let mut cli = Self::default();
        let items: Vec<String> = args.into_iter().map(Into::into).collect();
        let mut i = 1; // skip binary name if present
        while i < items.len() {
            match items[i].as_str() {
                "--open" => cli.open_browser = true,
                "--headless" => cli.headless = true,
                "--print-launch-link" => cli.print_launch_link = true,
                "--test-mode" => cli.test_mode = true,
                "--port" => {
                    i += 1;
                    if i >= items.len() {
                        return Err(AppError::new(
                            ErrorCode::InvalidRequest,
                            "Missing value for --port",
                        ));
                    }
                    let p: u16 = items[i].parse().map_err(|_| {
                        AppError::new(ErrorCode::InvalidRequest, "Invalid port number")
                    })?;
                    cli.port = Some(p);
                }
                "--data-dir" => {
                    i += 1;
                    if i >= items.len() {
                        return Err(AppError::new(
                            ErrorCode::InvalidRequest,
                            "Missing value for --data-dir",
                        ));
                    }
                    cli.data_dir = Some(PathBuf::from(&items[i]));
                }
                "--cache-dir" => {
                    i += 1;
                    if i >= items.len() {
                        return Err(AppError::new(
                            ErrorCode::InvalidRequest,
                            "Missing value for --cache-dir",
                        ));
                    }
                    cli.cache_dir = Some(PathBuf::from(&items[i]));
                }
                "--runtime-dir" => {
                    i += 1;
                    if i >= items.len() {
                        return Err(AppError::new(
                            ErrorCode::InvalidRequest,
                            "Missing value for --runtime-dir",
                        ));
                    }
                    cli.runtime_dir = Some(PathBuf::from(&items[i]));
                }
                arg if arg.starts_with("--port=") => {
                    let val = &arg[7..];
                    let p: u16 = val.parse().map_err(|_| {
                        AppError::new(ErrorCode::InvalidRequest, "Invalid port number")
                    })?;
                    cli.port = Some(p);
                }
                arg if arg.starts_with("--data-dir=") => {
                    cli.data_dir = Some(PathBuf::from(&arg[11..]));
                }
                arg if arg.starts_with("--cache-dir=") => {
                    cli.cache_dir = Some(PathBuf::from(&arg[12..]));
                }
                arg if arg.starts_with("--runtime-dir=") => {
                    cli.runtime_dir = Some(PathBuf::from(&arg[14..]));
                }
                other => {
                    return Err(AppError::new(
                        ErrorCode::InvalidRequest,
                        format!("Unknown argument: {other}"),
                    ));
                }
            }
            i += 1;
        }
        Ok(cli)
    }
}

#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub data_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub runtime_dir: PathBuf,
    pub port: u16,
    pub open_browser: bool,
    pub headless: bool,
    pub print_launch_link: bool,
    pub test_mode: bool,
}

impl RuntimeConfig {
    pub fn db_path(&self) -> PathBuf {
        self.data_dir.join("maya.sqlite3")
    }

    pub fn artifacts_dir(&self) -> PathBuf {
        self.data_dir.join("artifacts")
    }

    pub fn checkpoints_dir(&self) -> PathBuf {
        self.data_dir.join("checkpoints")
    }

    pub fn backups_dir(&self) -> PathBuf {
        self.data_dir.join("backups")
    }

    pub fn stages_dir(&self) -> PathBuf {
        self.cache_dir.join("stages")
    }

    pub fn audio_dir(&self) -> PathBuf {
        self.cache_dir.join("audio")
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.cache_dir.join("logs")
    }

    pub fn control_socket_path(&self) -> PathBuf {
        self.runtime_dir.join("control.sock")
    }

    pub fn instance_lock_path(&self) -> PathBuf {
        self.runtime_dir.join("instance.lock")
    }

    pub fn launch_json_path(&self) -> PathBuf {
        self.runtime_dir.join("launch.json")
    }
}

pub fn defaults() -> Settings {
    Settings {
        revision: 1,
        identity: Identity::default(),
        voice: VoiceSettings::default(),
        assistant: AssistantSettings::default(),
        security: SecuritySettings::default(),
        browser: BrowserSettings::default(),
    }
}

pub fn validate_settings(settings: &Settings) -> Result<()> {
    if settings.identity.name.trim().is_empty() || settings.identity.name.len() > 80 {
        return Err(AppError::new(
            ErrorCode::InvalidRequest,
            "Identity name must be between 1 and 80 characters",
        ));
    }
    if !settings.security.require_plan_approval {
        return Err(AppError::new(
            ErrorCode::ActionDenied,
            "Security setting require_plan_approval cannot be disabled",
        ));
    }
    if !settings.security.critical_always_confirm {
        return Err(AppError::new(
            ErrorCode::ActionDenied,
            "Security setting critical_always_confirm cannot be disabled",
        ));
    }
    if settings.security.chat_retention_days < 1 || settings.security.chat_retention_days > 3650 {
        return Err(AppError::new(
            ErrorCode::InvalidRequest,
            "chat_retention_days must be between 1 and 3650",
        ));
    }
    if settings.security.event_retention_days < 30 || settings.security.event_retention_days > 3650
    {
        return Err(AppError::new(
            ErrorCode::InvalidRequest,
            "event_retention_days must be between 30 and 3650",
        ));
    }
    if settings.security.artifact_retention_days < 1
        || settings.security.artifact_retention_days > 365
    {
        return Err(AppError::new(
            ErrorCode::InvalidRequest,
            "artifact_retention_days must be between 1 and 365",
        ));
    }
    if settings.security.backup_keep < 1 || settings.security.backup_keep > 30 {
        return Err(AppError::new(
            ErrorCode::InvalidRequest,
            "backup_keep must be between 1 and 30",
        ));
    }
    if settings.voice.rate < 0.5 || settings.voice.rate > 2.0 {
        return Err(AppError::new(
            ErrorCode::InvalidRequest,
            "Voice rate must be between 0.5 and 2.0",
        ));
    }
    for day in &settings.assistant.work_days {
        if *day < 1 || *day > 7 {
            return Err(AppError::new(
                ErrorCode::InvalidRequest,
                "Work days must be between 1 and 7 (ISO)",
            ));
        }
    }
    Ok(())
}

fn secure_dir(path: &Path) -> Result<()> {
    if path.is_symlink() {
        return Err(AppError::new(
            ErrorCode::ScopeDenied,
            format!("Path {:?} cannot be a symlink", path),
        ));
    }
    if !path.exists() {
        fs::create_dir_all(path).map_err(|e| {
            AppError::new(
                ErrorCode::Internal,
                format!("Failed to create directory {:?}: {e}", path),
            )
        })?;
    }
    let perms = fs::Permissions::from_mode(0o700);
    fs::set_permissions(path, perms).map_err(|e| {
        AppError::new(
            ErrorCode::Internal,
            format!("Failed to set permissions 0700 on {:?}: {e}", path),
        )
    })?;
    Ok(())
}

pub fn load(args: CliArgs) -> Result<RuntimeConfig> {
    let port = args.port.unwrap_or(8765);

    let home = env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    let home_path = PathBuf::from(home);

    let data_dir = args.data_dir.unwrap_or_else(|| {
        if let Ok(xdg_data) = env::var("XDG_DATA_HOME") {
            PathBuf::from(xdg_data).join("maya-mini")
        } else {
            home_path.join(".local/share/maya-mini")
        }
    });

    let cache_dir = args.cache_dir.unwrap_or_else(|| {
        if let Ok(xdg_cache) = env::var("XDG_CACHE_HOME") {
            PathBuf::from(xdg_cache).join("maya-mini")
        } else {
            home_path.join(".cache/maya-mini")
        }
    });

    let runtime_dir = args.runtime_dir.unwrap_or_else(|| {
        if let Ok(xdg_runtime) = env::var("XDG_RUNTIME_DIR") {
            PathBuf::from(xdg_runtime).join("maya-mini")
        } else {
            let uid = rustix::process::getuid().as_raw();
            PathBuf::from(format!("/tmp/maya-mini-{}", uid))
        }
    });

    // Create directories with mode 0700
    secure_dir(&data_dir)?;
    secure_dir(&data_dir.join("artifacts"))?;
    secure_dir(&data_dir.join("checkpoints"))?;
    secure_dir(&data_dir.join("backups"))?;

    secure_dir(&cache_dir)?;
    secure_dir(&cache_dir.join("stages"))?;
    secure_dir(&cache_dir.join("audio"))?;
    secure_dir(&cache_dir.join("logs"))?;

    secure_dir(&runtime_dir)?;

    Ok(RuntimeConfig {
        data_dir,
        cache_dir,
        runtime_dir,
        port,
        open_browser: args.open_browser,
        headless: args.headless,
        print_launch_link: args.print_launch_link,
        test_mode: args.test_mode,
    })
}
