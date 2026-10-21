use crate::db::Db;
use crate::error::{AppError, ErrorCode, Result};
use crate::policy::ExecutionPermit;
use crate::process::ProcessSupervisor;
use crate::scope;
use crate::types::{
    ActionClass, Digest, FileEntry, FileEntryKind, FilePreview, FileQuery, Id, NetworkMode, Page,
    ProcessCategory, ProcessLimits, ProcessResult, ProcessSpec, Project, SandboxSpec, ScanLimits,
    StagedProject,
};
use chrono::Utc;
use rusqlite::params;
use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const MAX_PREVIEW_BYTES: usize = 256 * 1024; // 256 KiB

#[derive(Clone)]
pub struct FileService {
    db: Db,
    process: Arc<ProcessSupervisor>,
}

impl FileService {
    pub fn new(db: Db, process: Arc<ProcessSupervisor>) -> Self {
        Self { db, process }
    }

    pub fn db(&self) -> &Db {
        &self.db
    }

    pub fn process(&self) -> &Arc<ProcessSupervisor> {
        &self.process
    }

    pub async fn find(&self, project_id: Id, query: FileQuery) -> Result<Page<FileEntry>> {
        let project = self.get_project(project_id).await?;
        if project.archived {
            return Err(AppError::new(
                ErrorCode::InvalidRequest,
                "Project is archived",
            ));
        }

        let root_handle = scope::register_root(Path::new(&project.root), &project.allowed_paths)?;
        let limits = ScanLimits {
            entries: 10_000,
            per_file_bytes: 10 * 1024 * 1024,
            total_bytes: 100 * 1024 * 1024,
        };
        let snapshot = scope::snapshot(&root_handle, limits)?;

        let limit = query.limit.unwrap_or(50).clamp(1, 100) as usize;
        let mut entries = Vec::new();

        for entry in snapshot.entries {
            // Apply path filter
            if let Some(ref path_prefix) = query.path
                && !entry.path.starts_with(path_prefix)
            {
                continue;
            }

            // Apply extension filter
            if let Some(ref ext) = query.extension {
                let lower_ext = ext.trim_start_matches('.').to_lowercase();
                let p = Path::new(&entry.path);
                let file_ext = p
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|s| s.to_lowercase())
                    .unwrap_or_default();
                if file_ext != lower_ext {
                    continue;
                }
            }

            // Apply text/substring query
            if let Some(ref q) = query.q
                && !entry.path.to_lowercase().contains(&q.to_lowercase())
            {
                continue;
            }

            let mime = detect_mime(&entry.path);

            entries.push(FileEntry {
                path: entry.path,
                kind: FileEntryKind::File,
                size_bytes: Some(entry.size_bytes),
                mime: Some(mime),
            });
        }

        // Sort deterministic
        entries.sort_by(|a, b| a.path.cmp(&b.path));

        // Apply cursor
        if let Some(ref cur) = query.cursor
            && let Some(idx) = entries.iter().position(|e| &e.path == cur)
        {
            if idx + 1 < entries.len() {
                entries = entries.split_off(idx + 1);
            } else {
                entries.clear();
            }
        }

        let next_cursor = if entries.len() > limit {
            entries.truncate(limit);
            entries.last().map(|e| e.path.clone())
        } else {
            None
        };

        Ok(Page {
            items: entries,
            next_cursor,
        })
    }

    pub async fn preview(&self, project_id: Id, path: String) -> Result<FilePreview> {
        let project = self.get_project(project_id).await?;
        if project.archived {
            return Err(AppError::new(
                ErrorCode::InvalidRequest,
                "Project is archived",
            ));
        }

        let rel_path = scope::validate_relative(&path)?;
        let root_handle = scope::register_root(Path::new(&project.root), &project.allowed_paths)?;

        let mut file = scope::open_read(&root_handle, rel_path.as_str())?;
        let metadata = file
            .metadata()
            .map_err(|e| AppError::new(ErrorCode::NotFound, format!("File not accessible: {e}")))?;

        let file_size = metadata.len();
        let read_size = (file_size as usize).min(MAX_PREVIEW_BYTES);
        let mut buffer = vec![0u8; read_size];
        file.read_exact(&mut buffer)
            .map_err(|e| AppError::new(ErrorCode::Internal, format!("Failed to read file: {e}")))?;

        let truncated = file_size > MAX_PREVIEW_BYTES as u64;
        let mime = detect_mime(rel_path.as_str());

        // Text extraction
        let text = if mime == "application/pdf" {
            // PDF: extract plain text without active content (no JS / launch / script)
            extract_pdf_text_safe(&buffer)
        } else {
            String::from_utf8_lossy(&buffer).to_string()
        };

        let sha256 = Digest::of(&buffer);

        Ok(FilePreview {
            project_id,
            path: rel_path.as_str().to_string(),
            mime,
            text,
            truncated,
            sha256,
            size_bytes: file_size,
        })
    }

    pub async fn command(
        &self,
        id: Id,
        stage: &StagedProject,
        permit: &ExecutionPermit,
        cancel: CancellationToken,
    ) -> Result<ProcessResult> {
        let (executable, args, timeout_ms, _network, kind) = self.get_command_spec(id).await?;
        let required = if matches!(
            kind,
            crate::types::CommandKind::Test | crate::types::CommandKind::Build
        ) {
            ActionClass::RunTest
        } else {
            ActionClass::RunCommand
        };
        permit.permits(&required, None)?;

        let spec = ProcessSpec {
            executable: PathBuf::from(executable),
            args: args.into_iter().map(OsString::from).collect(),
            cwd: stage.path.clone(),
            env: vec![(OsString::from("PATH"), OsString::from("/usr/bin:/bin"))],
            stdin: None,
            timeout: Duration::from_millis(timeout_ms),
            limits: ProcessLimits::default(),
            sandbox: SandboxSpec {
                stage: stage.path.clone(),
                read_roots: vec![],
                writable_roots: vec![],
                network: NetworkMode::Denied,
                deadline: Utc::now().timestamp_millis() + timeout_ms as i64,
            },
            category: ProcessCategory::Coding,
            job_id: Some(permit.job_id()),
        };

        self.process.run(spec, permit, cancel).await
    }

    async fn get_project(&self, id: Id) -> Result<Project> {
        let id_str = id.to_string();
        self.db
            .read(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, name, root, root_device, root_inode, allowed_paths_json, excluded_paths_json, notes, preferred_model_id, preferred_tool_id, archived, revision
                     FROM projects WHERE id = ?1",
                )?;
                stmt.query_row(params![id_str], parse_project_row)
                    .map_err(|e| match e {
                        rusqlite::Error::QueryReturnedNoRows => {
                            AppError::new(ErrorCode::NotFound, format!("Project {} not found", id))
                        }
                        other => AppError::from(other),
                    })
            })
            .await
    }

    async fn get_command_spec(
        &self,
        id: Id,
    ) -> Result<(String, Vec<String>, u64, bool, crate::types::CommandKind)> {
        let id_str = id.to_string();
        self.db
            .read(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT executable, args_json, timeout_ms, network, enabled, kind
                     FROM command_specs WHERE id = ?1",
                )?;
                stmt.query_row(params![id_str], |row| {
                    let exe: String = row.get(0)?;
                    let args_json: String = row.get(1)?;
                    let timeout: i64 = row.get(2)?;
                    let net: i64 = row.get(3)?;
                    let enabled: i64 = row.get(4)?;
                    if enabled == 0 {
                        return Err(rusqlite::Error::FromSqlConversionFailure(
                            4,
                            rusqlite::types::Type::Integer,
                            Box::new(std::io::Error::other("Command is disabled")),
                        ));
                    }
                    let args: Vec<String> = serde_json::from_str(&args_json).unwrap_or_default();
                    let kind_text: String = row.get(5)?;
                    let kind =
                        serde_json::from_str(&format!("\"{kind_text}\"")).map_err(|error| {
                            rusqlite::Error::FromSqlConversionFailure(
                                5,
                                rusqlite::types::Type::Text,
                                Box::new(error),
                            )
                        })?;
                    Ok((exe, args, timeout as u64, net != 0, kind))
                })
                .map_err(|e| match e {
                    rusqlite::Error::QueryReturnedNoRows => {
                        AppError::new(ErrorCode::NotFound, format!("Command {} not found", id))
                    }
                    _ => AppError::new(ErrorCode::ActionDenied, "Command disabled or invalid"),
                })
            })
            .await
    }
}

pub fn parse_project_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Project> {
    let id_str: String = row.get(0)?;
    let name: String = row.get(1)?;
    let root: String = row.get(2)?;
    let root_device: u64 = row.get(3)?;
    let root_inode: u64 = row.get(4)?;
    let allowed_json: String = row.get(5)?;
    let excluded_json: String = row.get(6)?;
    let notes: String = row.get(7)?;
    let model_str: Option<String> = row.get(8)?;
    let tool_str: Option<String> = row.get(9)?;
    let archived: i64 = row.get(10)?;
    let revision: i64 = row.get(11)?;

    let allowed_paths: Vec<String> = serde_json::from_str(&allowed_json).unwrap_or_default();
    let excluded_paths: Vec<String> = serde_json::from_str(&excluded_json).unwrap_or_default();

    Ok(Project {
        id: Uuid::parse_str(&id_str).unwrap_or_default(),
        name,
        root,
        root_device,
        root_inode,
        allowed_paths,
        excluded_paths,
        notes,
        preferred_model_id: model_str.and_then(|s| Uuid::parse_str(&s).ok()),
        preferred_tool_id: tool_str.and_then(|s| Uuid::parse_str(&s).ok()),
        archived: archived != 0,
        revision,
    })
}

fn detect_mime(path_str: &str) -> String {
    let p = Path::new(path_str);
    let ext = p
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_lowercase())
        .unwrap_or_default();

    match ext.as_str() {
        "rs" => "text/x-rust".to_string(),
        "md" => "text/markdown".to_string(),
        "txt" => "text/plain".to_string(),
        "json" => "application/json".to_string(),
        "toml" => "application/toml".to_string(),
        "yaml" | "yml" => "application/yaml".to_string(),
        "html" | "htm" => "text/html".to_string(),
        "css" => "text/css".to_string(),
        "js" | "mjs" => "text/javascript".to_string(),
        "pdf" => "application/pdf".to_string(),
        "sh" => "application/x-sh".to_string(),
        "sql" => "application/sql".to_string(),
        _ => "application/octet-stream".to_string(),
    }
}

fn extract_pdf_text_safe(bytes: &[u8]) -> String {
    // Check for suspicious active content markers and strip/omit them
    let text = String::from_utf8_lossy(bytes);
    let mut safe_lines = Vec::new();

    for line in text.lines() {
        // Disallow lines with active JavaScript / executable actions
        if line.contains("/JavaScript")
            || line.contains("/JS")
            || line.contains("/Launch")
            || line.contains("/Action")
        {
            continue;
        }

        // Keep printable ASCII / UTF-8 text lines
        let printable: String = line
            .chars()
            .filter(|c| c.is_ascii_graphic() || c.is_whitespace())
            .collect();

        if !printable.trim().is_empty() {
            safe_lines.push(printable);
        }
    }

    if safe_lines.is_empty() {
        "[PDF document: binary stream without plain text]".to_string()
    } else {
        safe_lines.join("\n")
    }
}
