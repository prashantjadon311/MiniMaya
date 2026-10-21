use crate::artifacts::{ArtifactInput, ArtifactStore};
use crate::db::Db;
use crate::error::{AppError, ErrorCode, Result};
use crate::policy::ExecutionPermit;
use crate::scope::{apply_one, register_root, snapshot};
use crate::types::{
    ActionClass, ArtifactKind, ChangeSet, Checkpoint, Digest, FileChange, FileChangeKind,
    FileReceipt, Id, Project, PromotionReceipt, ScanLimits, Snapshot, SnapshotEntry, StagedProject,
};
use rusqlite::params;
use sha2::{Digest as _, Sha256};
use std::collections::HashMap;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use uuid::Uuid;

type WorkspaceChangeRow = (
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
);

#[derive(Debug)]
pub struct CheckpointService {
    db: Db,
    artifacts: Arc<ArtifactStore>,
    runtime_dir: PathBuf,
}

impl CheckpointService {
    pub fn new(db: Db, artifacts: Arc<ArtifactStore>, runtime_dir: PathBuf) -> Self {
        Self {
            db,
            artifacts,
            runtime_dir,
        }
    }

    pub async fn create(
        &self,
        project: &Project,
        job_id: Id,
        permit: &ExecutionPermit,
    ) -> Result<Checkpoint> {
        permit.permits(&ActionClass::ProjectEdit, None)?;
        self.create_snapshot(project, job_id).await
    }

    pub async fn create_command_stage(
        &self,
        project: &Project,
        job_id: Id,
        action: ActionClass,
        permit: &ExecutionPermit,
    ) -> Result<(Checkpoint, StagedProject)> {
        if !matches!(action, ActionClass::RunCommand | ActionClass::RunTest) {
            return Err(AppError::new(
                ErrorCode::ActionDenied,
                "Command stage requires a command or test permit",
            ));
        }
        permit.permits(&action, None)?;
        let checkpoint = self.create_snapshot(project, job_id).await?;
        let stage = self.stage_with_project(&checkpoint, project).await?;
        Ok((checkpoint, stage))
    }

    async fn create_snapshot(&self, project: &Project, job_id: Id) -> Result<Checkpoint> {
        let root_handle = register_root(Path::new(&project.root), &project.allowed_paths)?;

        // 1. Snapshot the project root
        let snap = snapshot(&root_handle, ScanLimits::default())?;

        // 2. Check git status if repository
        let (git_head, dirty_paths) = Self::detect_git_status(Path::new(&project.root));

        // 3. Serialize snapshot and save as Checkpoint artifact
        let snap_json = serde_json::to_vec(&snap).map_err(|e| {
            AppError::new(
                ErrorCode::Internal,
                format!("Failed to serialize snapshot: {e}"),
            )
        })?;

        let artifact = self
            .artifacts
            .put(
                ArtifactInput {
                    job_id: Some(job_id),
                    task_id: None,
                    kind: ArtifactKind::Checkpoint,
                    mime: "application/json".to_string(),
                    provenance: crate::types::Provenance::system(),
                    max_bytes: snap_json.len() as u64 + 1024,
                    expires_at: None,
                },
                Box::pin(Cursor::new(snap_json)),
            )
            .await?;

        Ok(Checkpoint {
            id: artifact.id,
            baseline_digest: snap.digest,
            project_revision: project.revision,
            git_head,
            dirty_paths,
        })
    }

    pub async fn stage(&self, checkpoint: &Checkpoint) -> Result<StagedProject> {
        let (root_str, allowed_str, excluded_str): (String, String, String) = self
            .db
            .read({
                let chk_id = checkpoint.id.to_string();
                move |conn| {
                    let mut stmt = conn
                        .prepare(
                            r#"SELECT p.root, p.allowed_paths_json, p.excluded_paths_json
                               FROM artifacts a
                               JOIN jobs j ON a.job_id = j.id
                               JOIN projects p ON j.project_id = p.id
                               WHERE a.id = ?1"#,
                        )
                        .map_err(|e| AppError::new(ErrorCode::DbUnavailable, e.to_string()))?;

                    stmt.query_row(params![chk_id], |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, String>(2)?,
                        ))
                    })
                    .map_err(|_| {
                        AppError::new(ErrorCode::NotFound, "Project for checkpoint not found")
                    })
                }
            })
            .await?;

        let allowed: Vec<String> = serde_json::from_str(&allowed_str).unwrap_or_default();
        let excluded: Vec<String> = serde_json::from_str(&excluded_str).unwrap_or_default();

        let dummy_proj = Project {
            id: Uuid::nil(),
            name: "temp".to_string(),
            root: root_str,
            root_device: 0,
            root_inode: 0,
            allowed_paths: allowed,
            excluded_paths: excluded,
            notes: String::new(),
            preferred_model_id: None,
            preferred_tool_id: None,
            archived: false,
            revision: 1,
        };

        self.stage_with_project(checkpoint, &dummy_proj).await
    }

    pub async fn stage_with_project(
        &self,
        checkpoint: &Checkpoint,
        project: &Project,
    ) -> Result<StagedProject> {
        let stage_dir = self
            .runtime_dir
            .join("stages")
            .join(checkpoint.id.to_string());
        if stage_dir.exists() {
            let _ = fs::remove_dir_all(&stage_dir);
        }
        fs::create_dir_all(&stage_dir).map_err(|e| {
            AppError::new(
                ErrorCode::Internal,
                format!("Failed to create stage dir: {e}"),
            )
        })?;

        // Read snapshot from artifact
        let snap_file = self.artifacts.verified_file(checkpoint.id).await?;
        let snap: Snapshot = serde_json::from_reader(snap_file).map_err(|e| {
            AppError::new(
                ErrorCode::Internal,
                format!("Failed to decode checkpoint snapshot: {e}"),
            )
        })?;

        // Copy all entries from project root into stage
        let root_p = Path::new(&project.root);
        for entry in &snap.entries {
            let src_path = root_p.join(&entry.path);
            let dst_path = stage_dir.join(&entry.path);

            if let Some(parent) = dst_path.parent() {
                fs::create_dir_all(parent).map_err(|e| {
                    AppError::new(
                        ErrorCode::Internal,
                        format!("Failed to create stage parent: {e}"),
                    )
                })?;
            }

            fs::copy(&src_path, &dst_path).map_err(|e| {
                AppError::new(
                    ErrorCode::Internal,
                    format!("Failed to copy file to stage: {e}"),
                )
            })?;
        }

        Ok(StagedProject {
            path: stage_dir,
            checkpoint_id: checkpoint.id,
            baseline_digest: checkpoint.baseline_digest,
        })
    }

    pub fn existing_stage(&self, checkpoint: &Checkpoint) -> Result<StagedProject> {
        let path = self
            .runtime_dir
            .join("stages")
            .join(checkpoint.id.to_string());
        let metadata = fs::symlink_metadata(&path).map_err(|_| {
            AppError::new(
                ErrorCode::NeedsReconciliation,
                "The staged workspace is unavailable; inspect the checkpoint before retrying",
            )
        })?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(AppError::new(
                ErrorCode::ScopeDenied,
                "The staged workspace is not a safe directory",
            ));
        }
        Ok(StagedProject {
            path,
            checkpoint_id: checkpoint.id,
            baseline_digest: checkpoint.baseline_digest,
        })
    }

    pub async fn changes(&self, stage: &StagedProject) -> Result<ChangeSet> {
        // Read original baseline snapshot
        let snap_file = self.artifacts.verified_file(stage.checkpoint_id).await?;
        let snap: Snapshot = serde_json::from_reader(snap_file).map_err(|e| {
            AppError::new(
                ErrorCode::Internal,
                format!("Failed to decode checkpoint snapshot: {e}"),
            )
        })?;

        let mut baseline_map: HashMap<String, SnapshotEntry> = HashMap::new();
        for entry in snap.entries {
            baseline_map.insert(entry.path.clone(), entry);
        }

        // Snapshot current stage directory
        let stage_root_handle = register_root(&stage.path, &[])?;
        let current_snap = snapshot(&stage_root_handle, ScanLimits::default())?;

        let mut current_map: HashMap<String, SnapshotEntry> = HashMap::new();
        for entry in current_snap.entries {
            current_map.insert(entry.path.clone(), entry);
        }

        let mut changes: Vec<FileChange> = Vec::new();

        // 1. Detect Created and Modified files
        for (path, cur_entry) in &current_map {
            // Ignore recovery files and system metadata in stage
            if path.contains(".maya-recovery-") || path.contains(".maya-temp-") {
                continue;
            }

            match baseline_map.get(path) {
                None => {
                    // Create
                    let file_content = fs::read(stage.path.join(path)).map_err(|e| {
                        AppError::new(
                            ErrorCode::Internal,
                            format!("Failed to read staged create file: {e}"),
                        )
                    })?;
                    let art = self
                        .artifacts
                        .put(
                            ArtifactInput {
                                job_id: None,
                                task_id: None,
                                kind: ArtifactKind::Changes,
                                mime: "application/octet-stream".to_string(),
                                provenance: crate::types::Provenance::system(),
                                max_bytes: file_content.len() as u64 + 1024,
                                expires_at: None,
                            },
                            Box::pin(Cursor::new(file_content)),
                        )
                        .await?;

                    changes.push(FileChange {
                        path: path.clone(),
                        kind: FileChangeKind::Create,
                        before: None,
                        after: Some(cur_entry.sha256),
                        artifact_id: Some(art.id),
                        mode: cur_entry.mode,
                    });
                }
                Some(base_entry) => {
                    if base_entry.sha256 != cur_entry.sha256 {
                        // Modify
                        let file_content = fs::read(stage.path.join(path)).map_err(|e| {
                            AppError::new(
                                ErrorCode::Internal,
                                format!("Failed to read staged modify file: {e}"),
                            )
                        })?;
                        let art = self
                            .artifacts
                            .put(
                                ArtifactInput {
                                    job_id: None,
                                    task_id: None,
                                    kind: ArtifactKind::Changes,
                                    mime: "application/octet-stream".to_string(),
                                    provenance: crate::types::Provenance::system(),
                                    max_bytes: file_content.len() as u64 + 1024,
                                    expires_at: None,
                                },
                                Box::pin(Cursor::new(file_content)),
                            )
                            .await?;

                        changes.push(FileChange {
                            path: path.clone(),
                            kind: FileChangeKind::Modify,
                            before: Some(base_entry.sha256),
                            after: Some(cur_entry.sha256),
                            artifact_id: Some(art.id),
                            mode: cur_entry.mode,
                        });
                    }
                }
            }
        }

        // 2. Detect Deleted files
        for (path, base_entry) in &baseline_map {
            if !current_map.contains_key(path) {
                changes.push(FileChange {
                    path: path.clone(),
                    kind: FileChangeKind::Delete,
                    before: Some(base_entry.sha256),
                    after: None,
                    artifact_id: None,
                    mode: base_entry.mode,
                });
            }
        }

        // Sort deterministically by path
        changes.sort_by(|a, b| a.path.cmp(&b.path));

        // Compute canonical change digest
        let mut hasher = Sha256::new();
        for ch in &changes {
            hasher.update(ch.path.as_bytes());
            hasher.update(match ch.kind {
                FileChangeKind::Create => b"create",
                FileChangeKind::Modify => b"modify",
                FileChangeKind::Delete => b"delete",
            });
            if let Some(b) = ch.before {
                hasher.update(b.as_bytes());
            }
            if let Some(a) = ch.after {
                hasher.update(a.as_bytes());
            }
        }
        let change_digest = Digest::new(hasher.finalize().into());

        Ok(ChangeSet {
            baseline_digest: stage.baseline_digest,
            changes,
            digest: change_digest,
        })
    }

    pub async fn promote(
        &self,
        project: &Project,
        changes: &ChangeSet,
        permit: &ExecutionPermit,
    ) -> Result<PromotionReceipt> {
        permit.permits(&ActionClass::ProjectEdit, None)?;

        let root_handle = register_root(Path::new(&project.root), &project.allowed_paths)?;

        let job_id = permit.job_id();
        let job_id_str = job_id.to_string();

        let mut receipts: Vec<FileReceipt> = Vec::new();
        let mut all_applied = true;

        for (idx, change) in changes.changes.iter().enumerate() {
            let ordinal = (idx + 1) as i64;
            let change_id = Uuid::new_v4().to_string();
            let now = chrono::Utc::now().timestamp_millis();
            let path_str = change.path.clone();
            let kind_str = match change.kind {
                FileChangeKind::Create => "create",
                FileChangeKind::Modify => "modify",
                FileChangeKind::Delete => "delete",
            };
            let preimage_str = change.before.map(|d| d.to_hex());
            let postimage_str = change.after.map(|d| d.to_hex());
            let content_art_id_str = change.artifact_id.map(|id| id.to_string());

            // 1. Insert prepared row in workspace_changes
            self.db
                .write({
                    let cid = change_id.clone();
                    let jid = job_id_str.clone();
                    let pth = path_str.clone();
                    let knd = kind_str;
                    let pre = preimage_str.clone();
                    let post = postimage_str.clone();
                    let cart = content_art_id_str.clone();
                    move |tx| {
                        tx.execute(
                            r#"INSERT INTO workspace_changes
                               (id, job_id, checkpoint_id, ordinal, path, kind, preimage, postimage, content_artifact_id, state, updated_at)
                               VALUES (?1, ?2, (SELECT id FROM artifacts WHERE job_id = ?2 AND kind = 'checkpoint' ORDER BY created_at DESC LIMIT 1), ?3, ?4, ?5, ?6, ?7, ?8, 'prepared', ?9)"#,
                            params![cid, jid, ordinal, pth, knd, pre, post, cart, now],
                        )
                        .map_err(|e| AppError::new(ErrorCode::DbUnavailable, e.to_string()))?;
                        Ok(())
                    }
                })
                .await?;

            // 2. Transition to applying
            self.db
                .write({
                    let cid = change_id.clone();
                    move |tx| {
                        tx.execute(
                            "UPDATE workspace_changes SET state = 'applying', updated_at = ?1 WHERE id = ?2",
                            params![now, cid],
                        )
                        .map_err(|e| AppError::new(ErrorCode::DbUnavailable, e.to_string()))?;
                        Ok(())
                    }
                })
                .await?;

            // 3. Open content file if required
            let mut content_file = if let Some(art_id) = change.artifact_id {
                Some(self.artifacts.verified_file(art_id).await?)
            } else {
                None
            };

            // 4. Apply one filesystem change atomically
            let receipt = apply_one(&root_handle, change, content_file.as_mut(), permit)?;

            // 5. Update workspace_changes row based on receipt
            let new_state = if receipt.conflict {
                all_applied = false;
                "conflict"
            } else if receipt.applied {
                "applied"
            } else {
                all_applied = false;
                "uncertain"
            };

            let rec_path = receipt.recovery_path.clone();
            let upd_now = chrono::Utc::now().timestamp_millis();

            self.db
                .write({
                    let cid = change_id.clone();
                    let rp = rec_path.clone();
                    move |tx| {
                        tx.execute(
                            "UPDATE workspace_changes SET state = ?1, recovery_path = ?2, updated_at = ?3 WHERE id = ?4",
                            params![new_state, rp, upd_now, cid],
                        )
                        .map_err(|e| AppError::new(ErrorCode::DbUnavailable, e.to_string()))?;
                        Ok(())
                    }
                })
                .await?;

            receipts.push(receipt);
        }

        Ok(PromotionReceipt {
            job_id,
            change_digest: changes.digest,
            files: receipts,
            complete: all_applied,
        })
    }

    pub async fn restore(
        &self,
        checkpoint_id: Id,
        change_digest: Digest,
        project: &Project,
        permit: &ExecutionPermit,
    ) -> Result<PromotionReceipt> {
        permit.permits(&ActionClass::RestoreCheckpoint, None)?;

        let root_p = Path::new(&project.root);
        let _root_handle = register_root(root_p, &project.allowed_paths)?;

        // Fetch applied workspace_changes for this checkpoint
        let rows: Vec<WorkspaceChangeRow> = self
            .db
            .read({
                let chk_str = checkpoint_id.to_string();
                move |conn| {
                    let mut stmt = conn
                        .prepare(
                            r#"SELECT id, path, kind, preimage, postimage, recovery_path
                               FROM workspace_changes
                               WHERE checkpoint_id = ?1 AND state = 'applied'
                               ORDER BY ordinal DESC"#,
                        )
                        .map_err(|e| AppError::new(ErrorCode::DbUnavailable, e.to_string()))?;

                    let mapped = stmt
                        .query_map(params![chk_str], |r| {
                            Ok((
                                r.get::<_, String>(0)?,
                                r.get::<_, String>(1)?,
                                r.get::<_, String>(2)?,
                                r.get::<_, Option<String>>(3)?,
                                r.get::<_, Option<String>>(4)?,
                                r.get::<_, Option<String>>(5)?,
                            ))
                        })
                        .map_err(|e| AppError::new(ErrorCode::DbUnavailable, e.to_string()))?
                        .collect::<std::result::Result<Vec<_>, _>>()
                        .map_err(|e| AppError::new(ErrorCode::DbUnavailable, e.to_string()))?;

                    Ok(mapped)
                }
            })
            .await?;

        let mut receipts: Vec<FileReceipt> = Vec::new();
        let mut all_restored = true;

        for (row_id, path_str, kind_str, _pre_opt, post_opt, rec_path_opt) in rows {
            let target_path = root_p.join(&path_str);

            // Verify current postimage matches recorded postimage
            if let Some(post_hex) = post_opt
                && target_path.exists()
            {
                let cur_bytes = fs::read(&target_path).unwrap_or_default();
                let cur_hash = format!("{:x}", Sha256::digest(&cur_bytes));
                if cur_hash != post_hex {
                    // Current file diverged from Maya postimage! Reject overwrite.
                    receipts.push(FileReceipt {
                        path: path_str,
                        before: None,
                        after: None,
                        applied: false,
                        recovery_path: None,
                        conflict: true,
                    });
                    all_restored = false;
                    continue;
                }
            }

            // Restore displaced file
            let mut applied = false;
            let mut conflict = false;

            match kind_str.as_str() {
                "create" => {
                    // Reverse of create is delete
                    if target_path.exists() {
                        let _ = fs::remove_file(&target_path);
                        applied = true;
                    }
                }
                "modify" => {
                    // Restore from recovery_path if present
                    if let Some(rec_rel) = rec_path_opt {
                        let rec_abs = root_p.join(&rec_rel);
                        if rec_abs.exists() {
                            if let Ok(()) = fs::rename(&rec_abs, &target_path) {
                                applied = true;
                            } else {
                                conflict = true;
                            }
                        } else {
                            conflict = true;
                        }
                    } else {
                        conflict = true;
                    }
                }
                "delete" => {
                    // Restore deleted file from recovery_path
                    if let Some(rec_rel) = rec_path_opt {
                        let rec_abs = root_p.join(&rec_rel);
                        if rec_abs.exists() {
                            if let Ok(()) = fs::rename(&rec_abs, &target_path) {
                                applied = true;
                            } else {
                                conflict = true;
                            }
                        } else {
                            conflict = true;
                        }
                    } else {
                        conflict = true;
                    }
                }
                _ => {}
            }

            if conflict {
                all_restored = false;
            }

            let new_state = if applied { "restored" } else { "conflict" };
            let now = chrono::Utc::now().timestamp_millis();
            let _ = self
                .db
                .write({
                    let rid = row_id.clone();
                    move |tx| {
                        tx.execute(
                            "UPDATE workspace_changes SET state = ?1, updated_at = ?2 WHERE id = ?3",
                            params![new_state, now, rid],
                        )
                        .map_err(|e| AppError::new(ErrorCode::DbUnavailable, e.to_string()))?;
                        Ok(())
                    }
                })
                .await;

            receipts.push(FileReceipt {
                path: path_str,
                before: None,
                after: None,
                applied,
                recovery_path: None,
                conflict,
            });
        }

        Ok(PromotionReceipt {
            job_id: permit.job_id(),
            change_digest,
            files: receipts,
            complete: all_restored,
        })
    }

    fn detect_git_status(root: &Path) -> (Option<String>, Vec<String>) {
        let git_dir = root.join(".git");
        if !git_dir.exists() {
            return (None, Vec::new());
        }

        // Read .git/HEAD
        let head_path = git_dir.join("HEAD");
        let head_commit = if let Ok(content) = fs::read_to_string(&head_path) {
            let trimmed = content.trim();
            if let Some(target) = trimmed.strip_prefix("ref: ") {
                let ref_path = git_dir.join(target);
                fs::read_to_string(ref_path)
                    .map(|c| c.trim().to_string())
                    .ok()
            } else {
                Some(trimmed.to_string())
            }
        } else {
            None
        };

        (head_commit, Vec::new())
    }
}
