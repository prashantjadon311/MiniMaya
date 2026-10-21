use crate::artifacts::{ArtifactInput, ArtifactStore};
use crate::auth::SessionContext;
use crate::db::Db;
use crate::error::{AppError, ErrorCode, Result};
use crate::policy::Policy;
use crate::scope;
use crate::types::{
    Actor, Artifact, ArtifactKind, Id, ListQuery, Page, Project, ProjectInput, ProjectPatch,
    Provenance, ProvenanceSource, Revision, ScanLimits,
};
use chrono::Utc;
use rusqlite::params;
use std::fs::File;
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const MAX_SCAN_ENTRIES: u32 = 10_000;
const MAX_SCAN_TOTAL_BYTES: u64 = 50 * 1024 * 1024; // 50 MB
const MAX_CONTEXT_FILE_BYTES: usize = 512 * 1024; // 512 KiB per file

#[derive(Clone)]
pub struct ProjectService {
    db: Db,
    artifacts: Arc<ArtifactStore>,
}

impl ProjectService {
    pub fn new(db: Db, artifacts: Arc<ArtifactStore>) -> Self {
        Self { db, artifacts }
    }

    pub fn db(&self) -> &Db {
        &self.db
    }

    pub fn artifacts(&self) -> &Arc<ArtifactStore> {
        &self.artifacts
    }

    pub async fn register(
        &self,
        input: ProjectInput,
        _session: &SessionContext,
    ) -> Result<Project> {
        if !input.owner_ack {
            return Err(AppError::new(
                ErrorCode::InvalidRequest,
                "owner_ack must be true to register a project",
            ));
        }

        let raw_root = input.root.trim();
        if raw_root.is_empty() {
            return Err(AppError::new(
                ErrorCode::InvalidRequest,
                "Project root cannot be empty",
            ));
        }

        let path = PathBuf::from(raw_root);
        let canonical = path.canonicalize().map_err(|e| {
            AppError::new(
                ErrorCode::ScopeDenied,
                format!("Invalid project root path '{}': {}", raw_root, e),
            )
        })?;

        if !canonical.is_dir() {
            return Err(AppError::new(
                ErrorCode::ScopeDenied,
                "Project root must be a directory",
            ));
        }

        // Validate forbidden roots
        validate_root_safety(&canonical)?;

        let meta = canonical
            .metadata()
            .map_err(|e| AppError::new(ErrorCode::ScopeDenied, format!("Cannot stat root: {e}")))?;

        let root_device = meta.dev();
        let root_inode = meta.ino();
        let root_str = canonical.to_string_lossy().to_string();

        let id = Uuid::new_v4();
        let id_str = id.to_string();
        let now = Utc::now().timestamp_millis();
        let allowed_json =
            serde_json::to_string(&input.allowed_paths).unwrap_or_else(|_| "[]".to_string());
        let excluded_json =
            serde_json::to_string(&input.excluded_paths).unwrap_or_else(|_| "[]".to_string());

        let created_project = self
            .db
            .write(move |tx| {
                // Check if device+inode already registered
                let mut check_stmt = tx.prepare(
                    "SELECT id FROM projects WHERE root_device = ?1 AND root_inode = ?2 AND archived = 0",
                )?;
                let mut rows = check_stmt.query(params![root_device as i64, root_inode as i64])?;
                if rows.next()?.is_some() {
                    return Err(AppError::new(
                        ErrorCode::Conflict,
                        "Project with this root directory is already registered",
                    ));
                }

                tx.execute(
                    "INSERT INTO projects (
                        id, name, root, root_device, root_inode,
                        allowed_paths_json, excluded_paths_json, notes,
                        preferred_model_id, preferred_tool_id, archived, revision,
                        created_at, updated_at
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 0, 1, ?11, ?11)",
                    params![
                        id_str,
                        input.name.trim(),
                        root_str,
                        root_device as i64,
                        root_inode as i64,
                        allowed_json,
                        excluded_json,
                        input.notes,
                        input.preferred_model_id.map(|m| m.to_string()),
                        input.preferred_tool_id.map(|t| t.to_string()),
                        now,
                    ],
                )?;

                Ok(Project {
                    id,
                    name: input.name.trim().to_string(),
                    root: root_str,
                    root_device,
                    root_inode,
                    allowed_paths: input.allowed_paths,
                    excluded_paths: input.excluded_paths,
                    notes: input.notes,
                    preferred_model_id: input.preferred_model_id,
                    preferred_tool_id: input.preferred_tool_id,
                    archived: false,
                    revision: 1,
                })
            })
            .await?;

        Ok(created_project)
    }

    pub async fn get(&self, id: Id) -> Result<Project> {
        let id_str = id.to_string();
        self.db
            .read(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, name, root, root_device, root_inode, allowed_paths_json, excluded_paths_json, notes, preferred_model_id, preferred_tool_id, archived, revision
                     FROM projects WHERE id = ?1",
                )?;
                stmt.query_row(params![id_str], crate::fileops::parse_project_row)
                    .map_err(|e| match e {
                        rusqlite::Error::QueryReturnedNoRows => {
                            AppError::new(ErrorCode::NotFound, format!("Project {} not found", id))
                        }
                        other => AppError::from(other),
                    })
            })
            .await
    }

    pub async fn list(&self, q: ListQuery) -> Result<Page<Project>> {
        let limit = q.limit.unwrap_or(50).clamp(1, 100) as usize;
        let cursor_id = q.cursor.as_deref().and_then(|s| Uuid::parse_str(s).ok());

        self.db
            .read(move |conn| {
                let mut sql = "SELECT id, name, root, root_device, root_inode, allowed_paths_json, excluded_paths_json, notes, preferred_model_id, preferred_tool_id, archived, revision FROM projects WHERE archived = 0".to_string();
                let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

                if let Some(cid) = cursor_id {
                    sql.push_str(" AND id > ?1 ORDER BY id ASC LIMIT ?2");
                    params_vec.push(Box::new(cid.to_string()));
                    params_vec.push(Box::new((limit + 1) as i64));
                } else {
                    sql.push_str(" ORDER BY id ASC LIMIT ?1");
                    params_vec.push(Box::new((limit + 1) as i64));
                }

                let mut stmt = conn.prepare(&sql)?;
                let rusqlite_params: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|b| b.as_ref()).collect();
                let mut rows = stmt.query(rusqlite_params.as_slice())?;
                let mut items = Vec::new();

                while let Some(row) = rows.next()? {
                    items.push(crate::fileops::parse_project_row(row)?);
                }

                let next_cursor = if items.len() > limit {
                    items.pop();
                    items.last().map(|p| p.id.to_string())
                } else {
                    None
                };

                Ok(Page {
                    items,
                    next_cursor,
                })
            })
            .await
    }

    pub async fn patch(&self, id: Id, patch: ProjectPatch, _actor: Actor) -> Result<Project> {
        if !patch.owner_ack {
            return Err(AppError::new(
                ErrorCode::InvalidRequest,
                "owner_ack must be true to patch a project",
            ));
        }

        let id_str = id.to_string();

        self.db
            .write(move |tx| {
                let current = {
                    let mut stmt = tx.prepare(
                        "SELECT id, name, root, root_device, root_inode, allowed_paths_json, excluded_paths_json, notes, preferred_model_id, preferred_tool_id, archived, revision
                         FROM projects WHERE id = ?1",
                    )?;
                    stmt.query_row(params![id_str], crate::fileops::parse_project_row)
                        .map_err(|e| match e {
                            rusqlite::Error::QueryReturnedNoRows => {
                                AppError::new(ErrorCode::NotFound, format!("Project {} not found", id))
                            }
                            other => AppError::from(other),
                        })?
                };

                if current.revision != patch.revision {
                    return Err(AppError::new(
                        ErrorCode::Conflict,
                        format!(
                            "Stale revision: current is {}, patch expected {}",
                            current.revision, patch.revision
                        ),
                    ));
                }

                let new_name = patch.name.unwrap_or(current.name);
                let new_allowed = patch.allowed_paths.unwrap_or(current.allowed_paths);
                let new_excluded = patch.excluded_paths.unwrap_or(current.excluded_paths);
                let new_notes = patch.notes.unwrap_or(current.notes);
                let new_model = match patch.preferred_model_id {
                    Some(m) => m,
                    None => current.preferred_model_id,
                };
                let new_tool = match patch.preferred_tool_id {
                    Some(t) => t,
                    None => current.preferred_tool_id,
                };
                let new_archived = patch.archived.unwrap_or(current.archived);
                let new_rev = current.revision + 1;

                let allowed_json = serde_json::to_string(&new_allowed).unwrap_or_else(|_| "[]".to_string());
                let excluded_json = serde_json::to_string(&new_excluded).unwrap_or_else(|_| "[]".to_string());

                tx.execute(
                    "UPDATE projects SET
                        name = ?1, allowed_paths_json = ?2, excluded_paths_json = ?3,
                        notes = ?4, preferred_model_id = ?5, preferred_tool_id = ?6,
                        archived = ?7, revision = ?8
                     WHERE id = ?9 AND revision = ?10",
                    params![
                        new_name,
                        allowed_json,
                        excluded_json,
                        new_notes,
                        new_model.map(|m| m.to_string()),
                        new_tool.map(|t| t.to_string()),
                        if new_archived { 1 } else { 0 },
                        new_rev,
                        id.to_string(),
                        current.revision
                    ],
                )?;

                Ok(Project {
                    id,
                    name: new_name,
                    root: current.root,
                    root_device: current.root_device,
                    root_inode: current.root_inode,
                    allowed_paths: new_allowed,
                    excluded_paths: new_excluded,
                    notes: new_notes,
                    preferred_model_id: new_model,
                    preferred_tool_id: new_tool,
                    archived: new_archived,
                    revision: new_rev,
                })
            })
            .await
    }

    pub async fn archive(&self, id: Id, expected_revision: Revision, _actor: Actor) -> Result<()> {
        let current = self.get(id).await?;
        if current.revision != expected_revision {
            return Err(AppError::new(
                ErrorCode::Conflict,
                format!(
                    "Stale revision: current is {}, expected {}",
                    current.revision, expected_revision
                ),
            ));
        }

        let id_str = id.to_string();
        self.db
            .write(move |tx| {
                // Check if active jobs reference this project
                let mut stmt = tx.prepare(
                    "SELECT COUNT(*) FROM jobs WHERE project_id = ?1 AND state NOT IN ('completed', 'completed_with_issues', 'failed', 'cancelled')",
                )?;
                let active_count: i64 = stmt.query_row(params![id_str], |r| r.get(0))?;
                if active_count > 0 {
                    return Err(AppError::new(
                        ErrorCode::ResourceInUse,
                        "Cannot archive project with active jobs",
                    ));
                }

                tx.execute(
                    "UPDATE projects SET archived = 1, revision = revision + 1 WHERE id = ?1",
                    params![id_str],
                )?;
                Ok(())
            })
            .await
    }

    pub async fn scan(&self, id: Id, cancel: CancellationToken) -> Result<Artifact> {
        let project = self.get(id).await?;
        if project.archived {
            return Err(AppError::new(
                ErrorCode::InvalidRequest,
                "Cannot scan archived project",
            ));
        }

        if cancel.is_cancelled() {
            return Err(AppError::new(ErrorCode::Cancelled, "Scan was cancelled"));
        }

        let root_handle = scope::register_root(Path::new(&project.root), &project.allowed_paths)?;
        let limits = ScanLimits {
            entries: MAX_SCAN_ENTRIES,
            per_file_bytes: 10 * 1024 * 1024,
            total_bytes: MAX_SCAN_TOTAL_BYTES,
        };
        let snapshot = scope::snapshot(&root_handle, limits)?;

        // Analyze file types / languages
        let mut extensions_map = std::collections::BTreeMap::new();
        let mut total_files = 0u64;
        let mut total_bytes = 0u64;

        for entry in &snapshot.entries {
            total_files += 1;
            total_bytes += entry.size_bytes;
            let ext = Path::new(&entry.path)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("no_extension")
                .to_string();
            *extensions_map.entry(ext).or_insert(0u64) += 1;
        }

        // Check for README
        let readme_snippet = {
            let mut found_snippet = None;
            for name in &["README.md", "README.txt", "readme.md", "README"] {
                if let Ok(mut f) = scope::open_read(&root_handle, name) {
                    let mut buf = vec![0u8; 4096];
                    if let Ok(n) = f.read(&mut buf) {
                        buf.truncate(n);
                        found_snippet = Some(Policy::redact_static(&String::from_utf8_lossy(&buf)));
                        break;
                    }
                }
            }
            found_snippet
        };

        // Git status read-only
        let git_status = get_git_status_readonly(Path::new(&project.root));

        let scan_summary = serde_json::json!({
            "project_id": project.id,
            "project_name": project.name,
            "root": project.root,
            "total_files": total_files,
            "total_bytes": total_bytes,
            "extensions": extensions_map,
            "readme_preview": readme_snippet,
            "git": git_status,
            "skipped_paths": snapshot.skipped,
            "truncated": snapshot.truncated,
        });

        let data = serde_json::to_vec_pretty(&scan_summary).map_err(|e| {
            AppError::new(
                ErrorCode::Internal,
                format!("Failed to serialize scan: {e}"),
            )
        })?;

        let now = Utc::now().timestamp_millis();
        let artifact_input = ArtifactInput {
            job_id: None,
            task_id: None,
            kind: ArtifactKind::Scan,
            mime: "application/json".to_string(),
            provenance: Provenance {
                source: ProvenanceSource::System,
                provider_id: None,
                model_id: None,
                url: None,
                observed_at: now,
            },
            max_bytes: MAX_SCAN_TOTAL_BYTES,
            expires_at: None,
        };

        self.artifacts.put_bytes(artifact_input, &data).await
    }

    pub async fn context(&self, id: Id, paths: Vec<String>) -> Result<Artifact> {
        let project = self.get(id).await?;
        if project.archived {
            return Err(AppError::new(
                ErrorCode::InvalidRequest,
                "Cannot gather context for archived project",
            ));
        }

        let root_handle = scope::register_root(Path::new(&project.root), &project.allowed_paths)?;
        let mut context_files = Vec::new();

        for p_str in paths {
            let rel = scope::validate_relative(&p_str)?;
            let mut file = match scope::open_read(&root_handle, rel.as_str()) {
                Ok(f) => f,
                Err(_) => continue, // skip missing/forbidden paths safely
            };

            let mut buf = vec![0u8; MAX_CONTEXT_FILE_BYTES];
            let read_bytes = file.read(&mut buf).unwrap_or(0);
            buf.truncate(read_bytes);

            // Redact context text: no credentials, tokens or secrets
            let text = String::from_utf8_lossy(&buf);
            let safe_text = Policy::redact_static(&text);

            context_files.push(serde_json::json!({
                "path": rel.as_str(),
                "content": safe_text,
                "bytes": read_bytes,
            }));
        }

        let bundle = serde_json::json!({
            "project_id": project.id,
            "root": project.root,
            "files": context_files,
            "gathered_at": Utc::now().timestamp_millis(),
        });

        let data = serde_json::to_vec_pretty(&bundle).map_err(|e| {
            AppError::new(
                ErrorCode::Internal,
                format!("Failed to serialize context: {e}"),
            )
        })?;

        let now = Utc::now().timestamp_millis();
        let artifact_input = ArtifactInput {
            job_id: None,
            task_id: None,
            kind: ArtifactKind::Context,
            mime: "application/json".to_string(),
            provenance: Provenance {
                source: ProvenanceSource::System,
                provider_id: None,
                model_id: None,
                url: None,
                observed_at: now,
            },
            max_bytes: 20 * 1024 * 1024,
            expires_at: None,
        };

        self.artifacts.put_bytes(artifact_input, &data).await
    }
}

fn validate_root_safety(canonical: &Path) -> Result<()> {
    let canonical_str = canonical.to_string_lossy();

    // Reject root filesystem
    if canonical_str == "/" {
        return Err(AppError::new(
            ErrorCode::ScopeDenied,
            "Root filesystem '/' cannot be registered as a project root",
        ));
    }

    // Reject system paths
    let forbidden_prefixes = [
        "/bin", "/boot", "/dev", "/etc", "/lib", "/lib64", "/proc", "/root", "/run", "/sbin",
        "/sys", "/usr", "/var",
    ];

    for prefix in &forbidden_prefixes {
        if canonical_str == *prefix || canonical_str.starts_with(&format!("{}/", prefix)) {
            return Err(AppError::new(
                ErrorCode::ScopeDenied,
                format!(
                    "System directory '{}' cannot be registered as a project root",
                    prefix
                ),
            ));
        }
    }

    // Reject user home root itself (e.g. /home/user)
    if let Ok(home) = std::env::var("HOME")
        && let Ok(home_canon) = PathBuf::from(home).canonicalize()
        && canonical == home_canon
    {
        return Err(AppError::new(
            ErrorCode::ScopeDenied,
            "User home directory cannot be registered as a project root directly; use a subdirectory",
        ));
    }

    Ok(())
}

fn get_git_status_readonly(root: &Path) -> serde_json::Value {
    let git_dir = root.join(".git");
    if !git_dir.exists() {
        return serde_json::json!({
            "is_repo": false
        });
    }

    // Read git head if possible
    let head_file = git_dir.join("HEAD");
    let head = if let Ok(mut f) = File::open(head_file) {
        let mut s = String::new();
        let _ = f.read_to_string(&mut s);
        Some(s.trim().to_string())
    } else {
        None
    };

    serde_json::json!({
        "is_repo": true,
        "head": head,
    })
}
