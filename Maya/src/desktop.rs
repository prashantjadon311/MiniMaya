use crate::db::Db;
use crate::error::{AppError, ErrorCode, Result};
use crate::policy::ExecutionPermit;
use crate::schedule::{Notice, NotificationReceipt, NotificationStatusKind, Notifier};
use crate::types::{ActionClass, ClipboardInput, LaunchAppTarget, LaunchInput, OperationResult};
use futures_util::future::BoxFuture;
use std::sync::{Arc, Mutex};
use tokio::process::Command;

#[derive(Clone)]
pub struct DesktopService {
    db: Db,
    // In-memory clipboard store for sterile/headless test environments
    in_memory_clipboard: Arc<Mutex<Option<String>>>,
}

impl DesktopService {
    pub fn new(db: Db) -> Self {
        Self {
            db,
            in_memory_clipboard: Arc::new(Mutex::new(None)),
        }
    }

    pub fn db(&self) -> &Db {
        &self.db
    }

    pub async fn launch(
        &self,
        input: LaunchInput,
        permit: &ExecutionPermit,
    ) -> Result<OperationResult> {
        permit.permits(&ActionClass::LaunchApp, None)?;

        match input.app {
            LaunchAppTarget::Browser => {
                let url = input.url.as_deref().unwrap_or("https://localhost");
                if !url.starts_with("http://") && !url.starts_with("https://") {
                    return Err(AppError::new(
                        ErrorCode::InvalidRequest,
                        "Browser URL must be http:// or https://",
                    ));
                }

                // Attempt to launch via xdg-open
                let status = Command::new("xdg-open").arg(url).status().await;

                match status {
                    Ok(exit) if exit.success() => Ok(OperationResult {
                        kind: "launch_browser".to_string(),
                        message: format!("Opened browser to {}", url),
                        artifact_id: None,
                        text: None,
                        tab_id: None,
                        exit: None,
                    }),
                    Ok(exit) => Err(AppError::new(
                        ErrorCode::ToolFailed,
                        format!("xdg-open returned exit code {:?}", exit.code()),
                    )),
                    Err(e) => Err(AppError::new(
                        ErrorCode::UnsupportedCapability,
                        format!("Desktop browser launcher unavailable: {e}"),
                    )),
                }
            }
            LaunchAppTarget::Editor => {
                let editor = std::env::var("EDITOR").unwrap_or_else(|_| "nano".to_string());
                let mut cmd = Command::new(&editor);
                if let Some(ref folder) = input.folder {
                    cmd.arg(folder);
                }

                match cmd.spawn() {
                    Ok(_) => Ok(OperationResult {
                        kind: "launch_editor".to_string(),
                        message: format!("Launched editor '{}'", editor),
                        artifact_id: None,
                        text: None,
                        tab_id: None,
                        exit: None,
                    }),
                    Err(e) => Err(AppError::new(
                        ErrorCode::UnsupportedCapability,
                        format!("Failed to launch editor '{}': {}", editor, e),
                    )),
                }
            }
            LaunchAppTarget::InstalledTool => {
                if let Some(tool_id) = input.tool_id {
                    // Look up tool in tools table
                    let id_str = tool_id.to_string();
                    let (executable, enabled): (String, i64) = self
                        .db
                        .read(move |conn| {
                            let mut stmt = conn
                                .prepare("SELECT executable, enabled FROM tools WHERE id = ?1")?;
                            let res = stmt.query_row(rusqlite::params![id_str], |r| {
                                Ok((r.get(0)?, r.get(1)?))
                            })?;
                            Ok(res)
                        })
                        .await
                        .map_err(|_| {
                            AppError::new(
                                ErrorCode::NotFound,
                                format!("Tool {} not found", tool_id),
                            )
                        })?;

                    if enabled == 0 {
                        return Err(AppError::new(
                            ErrorCode::ActionDenied,
                            format!("Tool {} is disabled", tool_id),
                        ));
                    }

                    match Command::new(&executable).spawn() {
                        Ok(_) => Ok(OperationResult {
                            kind: "launch_tool".to_string(),
                            message: format!("Launched tool '{}'", executable),
                            artifact_id: None,
                            text: None,
                            tab_id: None,
                            exit: None,
                        }),
                        Err(e) => Err(AppError::new(
                            ErrorCode::UnsupportedCapability,
                            format!("Failed to launch tool '{}': {}", executable, e),
                        )),
                    }
                } else {
                    Err(AppError::new(
                        ErrorCode::InvalidRequest,
                        "tool_id required for installed tool launch",
                    ))
                }
            }
        }
    }

    pub async fn clipboard(
        &self,
        input: ClipboardInput,
        permit: &ExecutionPermit,
    ) -> Result<OperationResult> {
        if let Some(text) = input.text {
            if text.len() > 65536 {
                return Err(AppError::new(
                    ErrorCode::InvalidRequest,
                    "Clipboard text exceeds 64 KiB limit",
                ));
            }
            // Write to clipboard
            permit.permits(&ActionClass::ClipboardWrite, None)?;

            // Try wl-copy or xclip, fallback to in-memory
            let mut written = false;

            // Try wl-copy
            if let Ok(mut child) = Command::new("wl-copy")
                .stdin(std::process::Stdio::piped())
                .spawn()
            {
                if let Some(mut stdin) = child.stdin.take() {
                    use tokio::io::AsyncWriteExt;
                    let _ = stdin.write_all(text.as_bytes()).await;
                }
                if let Ok(status) = child.wait().await
                    && status.success()
                {
                    written = true;
                }
            }

            // If wl-copy not available or failed, try xclip
            if !written
                && let Ok(mut child) = Command::new("xclip")
                    .args(["-selection", "clipboard"])
                    .stdin(std::process::Stdio::piped())
                    .spawn()
            {
                if let Some(mut stdin) = child.stdin.take() {
                    use tokio::io::AsyncWriteExt;
                    let _ = stdin.write_all(text.as_bytes()).await;
                }
                if let Ok(status) = child.wait().await
                    && status.success()
                {
                    written = true;
                }
            }

            // Always update in-memory clipboard store as backup/test harness
            *self.in_memory_clipboard.lock().unwrap() = Some(text);

            Ok(OperationResult {
                kind: "clipboard_write".to_string(),
                message: if written {
                    "Copied to system clipboard".to_string()
                } else {
                    "Copied to session clipboard (system clipboard tool unavailable)".to_string()
                },
                artifact_id: None,
                text: None,
                tab_id: None,
                exit: None,
            })
        } else {
            // Read from clipboard
            permit.permits(&ActionClass::ClipboardRead, None)?;

            // Try wl-paste
            let mut read_text = None;
            if let Ok(output) = Command::new("wl-paste").output().await
                && output.status.success()
            {
                read_text = Some(String::from_utf8_lossy(&output.stdout).to_string());
            }

            // Try xclip -o
            if read_text.is_none()
                && let Ok(output) = Command::new("xclip")
                    .args(["-selection", "clipboard", "-o"])
                    .output()
                    .await
                && output.status.success()
            {
                read_text = Some(String::from_utf8_lossy(&output.stdout).to_string());
            }

            // Fallback to in-memory clipboard
            if read_text.is_none() {
                read_text = self.in_memory_clipboard.lock().unwrap().clone();
            }

            let text = read_text.unwrap_or_default();
            Ok(OperationResult {
                kind: "clipboard_read".to_string(),
                message: "Read from clipboard".to_string(),
                artifact_id: None,
                text: Some(text),
                tab_id: None,
                exit: None,
            })
        }
    }
}

impl Notifier for DesktopService {
    fn notify<'a>(&'a self, notice: Notice) -> BoxFuture<'a, Result<NotificationReceipt>> {
        Box::pin(async move {
            // Sanitize notification text (plain text <= 1000 chars, no ANSI)
            let safe_title: String = notice
                .title
                .chars()
                .filter(|c| !c.is_control() || *c == ' ')
                .take(200)
                .collect();

            let safe_body: String = notice
                .body
                .chars()
                .filter(|c| !c.is_control() || *c == '\n' || *c == ' ')
                .take(1000)
                .collect();

            // Try notify-send
            match Command::new("notify-send")
                .arg(&safe_title)
                .arg(&safe_body)
                .status()
                .await
            {
                Ok(status) if status.success() => Ok(NotificationReceipt {
                    status: NotificationStatusKind::Accepted,
                    desktop_id: Some(1),
                }),
                _ => {
                    // Desktop notification daemon is unavailable on headless/sandbox
                    Ok(NotificationReceipt {
                        status: NotificationStatusKind::Unavailable,
                        desktop_id: None,
                    })
                }
            }
        })
    }
}
