use crate::error::{AppError, ErrorCode, Result};
use crate::policy::ExecutionPermit;
use crate::types::{
    ActionClass, Digest, FileChange, FileChangeKind, FileReceipt, ScanLimits, SkippedPath,
    Snapshot, SnapshotEntry,
};
use rustix::fs::{Mode, OFlags, ResolveFlags, openat2};
use sha2::{Digest as _, Sha256};
use std::fs::{self, File};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::io::AsFd;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelativePath(String);

impl RelativePath {
    pub fn new(path: &str) -> Result<Self> {
        validate_relative(path)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

pub fn validate_relative(path: &str) -> Result<RelativePath> {
    if path.is_empty() || path.contains('\0') {
        return Err(AppError::new(
            ErrorCode::ScopeDenied,
            "Path cannot be empty or contain NUL bytes",
        ));
    }
    let p = Path::new(path);
    if p.is_absolute() {
        return Err(AppError::new(
            ErrorCode::ScopeDenied,
            "Path must be relative, not absolute",
        ));
    }
    for comp in p.components() {
        match comp {
            std::path::Component::ParentDir => {
                return Err(AppError::new(
                    ErrorCode::ScopeDenied,
                    "Path traversal ('..') is strictly denied",
                ));
            }
            std::path::Component::RootDir | std::path::Component::Prefix(_) => {
                return Err(AppError::new(
                    ErrorCode::ScopeDenied,
                    "Path must be purely relative",
                ));
            }
            std::path::Component::CurDir | std::path::Component::Normal(_) => {}
        }
    }
    let normalized = p.to_string_lossy().to_string();
    Ok(RelativePath(normalized))
}

#[derive(Clone)]
pub struct RootHandle {
    pub dir: Arc<File>,
    pub canonical: PathBuf,
    pub device: u64,
    pub inode: u64,
    pub allowed: Vec<RelativePath>,
    pub excluded: Vec<RelativePath>,
}

pub fn register_root(path: &Path, allowed: &[String]) -> Result<RootHandle> {
    if !path.exists() {
        return Err(AppError::new(
            ErrorCode::NotFound,
            format!("Project root {:?} does not exist", path),
        ));
    }
    if !path.is_dir() {
        return Err(AppError::new(
            ErrorCode::ScopeDenied,
            format!("Project root {:?} is not a directory", path),
        ));
    }

    let canonical = path.canonicalize().map_err(|e| {
        AppError::new(
            ErrorCode::ScopeDenied,
            format!("Failed to canonicalize root path: {e}"),
        )
    })?;

    // Deny sensitive system roots and personal roots
    let canon_str = canonical.to_string_lossy();
    if canon_str == "/" || canon_str == "/root" || canon_str == "/home" || canon_str == "/etc" {
        return Err(AppError::new(
            ErrorCode::ScopeDenied,
            "Registering root filesystem or system directory as project root is denied",
        ));
    }

    let dir_file = File::open(&canonical).map_err(|e| {
        AppError::new(
            ErrorCode::ScopeDenied,
            format!("Failed to open directory file descriptor: {e}"),
        )
    })?;

    let meta = dir_file.metadata().map_err(|e| {
        AppError::new(
            ErrorCode::ScopeDenied,
            format!("Failed to read metadata for root directory: {e}"),
        )
    })?;

    let device = meta.dev();
    let inode = meta.ino();

    let mut allowed_rel = Vec::new();
    for a in allowed {
        allowed_rel.push(validate_relative(a)?);
    }

    Ok(RootHandle {
        dir: Arc::new(dir_file),
        canonical,
        device,
        inode,
        allowed: allowed_rel,
        excluded: Vec::new(),
    })
}

pub fn verify_root(root: &RootHandle) -> Result<()> {
    let meta = root.dir.metadata().map_err(|e| {
        AppError::new(
            ErrorCode::ScopeDenied,
            format!("Root directory is no longer accessible: {e}"),
        )
    })?;

    if meta.dev() != root.device || meta.ino() != root.inode {
        return Err(AppError::new(
            ErrorCode::ScopeDenied,
            "Root directory device/inode identity has changed",
        ));
    }
    Ok(())
}

pub fn open_read(root: &RootHandle, relative_path: &str) -> Result<File> {
    verify_root(root)?;
    let rel = validate_relative(relative_path)?;

    // Check against allowed scopes if any are registered
    if !root.allowed.is_empty() {
        let is_allowed = root.allowed.iter().any(|a| {
            rel.as_str() == a.as_str() || rel.as_str().starts_with(&format!("{}/", a.as_str()))
        });
        if !is_allowed {
            return Err(AppError::new(
                ErrorCode::ScopeDenied,
                format!("Path '{}' is outside allowed project scopes", relative_path),
            ));
        }
    }

    // Use openat2 with RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS | RESOLVE_NO_XDEV
    let oflags = OFlags::RDONLY | OFlags::CLOEXEC;
    let resolve_flags = ResolveFlags::BENEATH
        | ResolveFlags::NO_SYMLINKS
        | ResolveFlags::NO_MAGICLINKS
        | ResolveFlags::NO_XDEV;

    let fd = openat2(
        root.dir.as_fd(),
        rel.as_str(),
        oflags,
        Mode::empty(),
        resolve_flags,
    )
    .map_err(|e| {
        AppError::new(
            ErrorCode::ScopeDenied,
            format!(
                "Failed to securely open path '{}' beneath root: {e}",
                relative_path
            ),
        )
    })?;

    let file = File::from(fd);
    let meta = file.metadata().map_err(|e| {
        AppError::new(
            ErrorCode::ScopeDenied,
            format!("Failed to read metadata for '{}': {e}", relative_path),
        )
    })?;

    // Deny non-regular files and hardlinks with nlink > 1
    if !meta.is_file() {
        return Err(AppError::new(
            ErrorCode::ScopeDenied,
            format!("Path '{}' is not a regular file", relative_path),
        ));
    }
    if meta.nlink() > 1 {
        return Err(AppError::new(
            ErrorCode::ScopeDenied,
            format!(
                "Path '{}' has multiple hardlinks (nlink > 1)",
                relative_path
            ),
        ));
    }

    Ok(file)
}

pub fn snapshot(root: &RootHandle, limits: ScanLimits) -> Result<Snapshot> {
    verify_root(root)?;

    struct WalkState {
        entries: Vec<SnapshotEntry>,
        skipped: Vec<SkippedPath>,
        total_bytes: u64,
        entry_count: u32,
        truncated: bool,
    }

    let mut state = WalkState {
        entries: Vec::new(),
        skipped: Vec::new(),
        total_bytes: 0,
        entry_count: 0,
        truncated: false,
    };

    fn walk(dir: &Path, root: &RootHandle, limits: &ScanLimits, state: &mut WalkState) {
        if state.truncated || state.entry_count >= limits.entries {
            state.truncated = true;
            return;
        }

        let read_dir = match fs::read_dir(dir) {
            Ok(rd) => rd,
            Err(e) => {
                let rel = dir
                    .strip_prefix(&root.canonical)
                    .unwrap_or(dir)
                    .to_string_lossy()
                    .to_string();
                state.skipped.push(SkippedPath {
                    path: rel,
                    reason: format!("ReadDir error: {e}"),
                });
                return;
            }
        };

        for item in read_dir {
            if state.entry_count >= limits.entries {
                state.truncated = true;
                break;
            }

            let entry = match item {
                Ok(e) => e,
                Err(_) => continue,
            };

            let path = entry.path();
            let file_name = entry.file_name().to_string_lossy().to_string();

            // Exclude sensitive patterns
            if file_name == ".git"
                || file_name == ".codex"
                || file_name == ".agents"
                || file_name == ".ssh"
                || file_name == ".gnupg"
                || file_name.starts_with(".env")
                || file_name.starts_with(".maya-recovery-")
            {
                continue;
            }

            let file_type = match entry.file_type() {
                Ok(ft) => ft,
                Err(_) => continue,
            };

            if file_type.is_symlink() {
                let rel = path
                    .strip_prefix(&root.canonical)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .to_string();
                state.skipped.push(SkippedPath {
                    path: rel,
                    reason: "Symlink denied".to_string(),
                });
                continue;
            }

            if file_type.is_dir() {
                walk(&path, root, limits, state);
            } else if file_type.is_file() {
                let meta = match entry.metadata() {
                    Ok(m) => m,
                    Err(_) => continue,
                };

                let rel = match path.strip_prefix(&root.canonical) {
                    Ok(p) => p.to_string_lossy().to_string(),
                    Err(_) => continue,
                };

                // Check allowed scope if configured
                if !root.allowed.is_empty() {
                    let allowed = root
                        .allowed
                        .iter()
                        .any(|a| rel == a.as_str() || rel.starts_with(&format!("{}/", a.as_str())));
                    if !allowed {
                        continue;
                    }
                }

                if meta.nlink() > 1 {
                    state.skipped.push(SkippedPath {
                        path: rel,
                        reason: "Hardlink nlink > 1 denied".to_string(),
                    });
                    continue;
                }

                let size = meta.len();
                if size > limits.per_file_bytes {
                    state.skipped.push(SkippedPath {
                        path: rel,
                        reason: format!("File exceeds per-file limit ({} bytes)", size),
                    });
                    continue;
                }

                if state.total_bytes + size > limits.total_bytes {
                    state.truncated = true;
                    break;
                }

                // Compute hash
                let content = match fs::read(&path) {
                    Ok(b) => b,
                    Err(e) => {
                        state.skipped.push(SkippedPath {
                            path: rel,
                            reason: format!("Read error: {e}"),
                        });
                        continue;
                    }
                };

                let sha = Digest::of(&content);
                let mode = meta.permissions().mode();

                state.entries.push(SnapshotEntry {
                    path: rel,
                    size_bytes: size,
                    sha256: sha,
                    mode,
                });

                state.total_bytes += size;
                state.entry_count += 1;
            }
        }
    }

    walk(&root.canonical, root, &limits, &mut state);

    let WalkState {
        mut entries,
        skipped,
        total_bytes: _,
        entry_count: _,
        truncated,
    } = state;

    // Sort entries lexicographically for deterministic digest
    entries.sort_by(|a, b| a.path.cmp(&b.path));

    let mut hasher = Sha256::new();
    for e in &entries {
        hasher.update(e.path.as_bytes());
        hasher.update(e.size_bytes.to_le_bytes());
        hasher.update(e.sha256.0);
    }
    let res = hasher.finalize();
    let mut digest_bytes = [0u8; 32];
    digest_bytes.copy_from_slice(&res);

    Ok(Snapshot {
        root_device: root.device,
        root_inode: root.inode,
        entries,
        digest: Digest(digest_bytes),
        skipped,
        truncated,
    })
}

pub fn write_stage_file(stage: &Path, rel_path: &str, content: &[u8]) -> Result<()> {
    let rel = validate_relative(rel_path)?;
    let dest = stage.join(rel.as_str());
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            AppError::new(
                ErrorCode::Internal,
                format!("Failed to create stage directories: {e}"),
            )
        })?;
    }
    fs::write(&dest, content).map_err(|e| {
        AppError::new(
            ErrorCode::Internal,
            format!("Failed to write stage file: {e}"),
        )
    })?;
    Ok(())
}

pub fn apply_one(
    root: &RootHandle,
    change: &FileChange,
    content: Option<&mut File>,
    permit: &ExecutionPermit,
) -> Result<FileReceipt> {
    verify_root(root)?;
    let rel = validate_relative(&change.path)?;

    match change.kind {
        FileChangeKind::Create => {
            permit.permits(&ActionClass::ProjectEdit, Some(&change.path))?;
            let dest = root.canonical.join(rel.as_str());

            if dest.exists() {
                return Ok(FileReceipt {
                    path: change.path.clone(),
                    before: None,
                    after: None,
                    applied: false,
                    recovery_path: None,
                    conflict: true,
                });
            }

            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent).map_err(|e| {
                    AppError::new(
                        ErrorCode::ScopeDenied,
                        format!("Failed to create directory {:?}: {e}", parent),
                    )
                })?;
            }

            let in_file = content.ok_or_else(|| {
                AppError::new(
                    ErrorCode::InvalidRequest,
                    "Missing content file for create change",
                )
            })?;

            use std::io::{Read, Seek, SeekFrom};
            in_file
                .seek(SeekFrom::Start(0))
                .map_err(|e| AppError::new(ErrorCode::Internal, e.to_string()))?;
            let mut buf = Vec::new();
            in_file
                .read_to_end(&mut buf)
                .map_err(|e| AppError::new(ErrorCode::Internal, e.to_string()))?;

            let after_digest = Digest::of(&buf);
            if let Some(exp) = change.after
                && exp != after_digest
            {
                return Err(AppError::new(
                    ErrorCode::Conflict,
                    "Generated postimage digest mismatch",
                ));
            }

            // Write temporary file in same directory then rename
            let temp_name = format!(".maya-temp-{}", Uuid::new_v4());
            let temp_path = dest.parent().unwrap().join(temp_name);
            fs::write(&temp_path, &buf)
                .map_err(|e| AppError::new(ErrorCode::ScopeDenied, e.to_string()))?;

            fs::rename(&temp_path, &dest).map_err(|e| {
                let _ = fs::remove_file(&temp_path);
                AppError::new(ErrorCode::ScopeDenied, e.to_string())
            })?;

            Ok(FileReceipt {
                path: change.path.clone(),
                before: None,
                after: Some(after_digest),
                applied: true,
                recovery_path: None,
                conflict: false,
            })
        }
        FileChangeKind::Modify => {
            permit.permits(&ActionClass::ProjectEdit, Some(&change.path))?;
            let dest = root.canonical.join(rel.as_str());

            if !dest.exists() {
                return Ok(FileReceipt {
                    path: change.path.clone(),
                    before: None,
                    after: None,
                    applied: false,
                    recovery_path: None,
                    conflict: true,
                });
            }

            let live_bytes = fs::read(&dest)
                .map_err(|e| AppError::new(ErrorCode::ScopeDenied, e.to_string()))?;
            let live_digest = Digest::of(&live_bytes);

            if let Some(exp_before) = change.before
                && exp_before != live_digest
            {
                return Ok(FileReceipt {
                    path: change.path.clone(),
                    before: Some(live_digest),
                    after: None,
                    applied: false,
                    recovery_path: None,
                    conflict: true,
                });
            }

            let in_file = content.ok_or_else(|| {
                AppError::new(
                    ErrorCode::InvalidRequest,
                    "Missing content file for modify change",
                )
            })?;

            use std::io::{Read, Seek, SeekFrom};
            in_file
                .seek(SeekFrom::Start(0))
                .map_err(|e| AppError::new(ErrorCode::Internal, e.to_string()))?;
            let mut buf = Vec::new();
            in_file
                .read_to_end(&mut buf)
                .map_err(|e| AppError::new(ErrorCode::Internal, e.to_string()))?;

            let after_digest = Digest::of(&buf);
            if let Some(exp_after) = change.after
                && exp_after != after_digest
            {
                return Err(AppError::new(
                    ErrorCode::Conflict,
                    "Generated postimage digest mismatch",
                ));
            }

            // Reserve recovery path
            let recovery_name = format!(".maya-recovery-{}", Uuid::new_v4());
            let recovery_path = dest.parent().unwrap().join(&recovery_name);

            // Move original to recovery path
            fs::rename(&dest, &recovery_path).map_err(|e| {
                AppError::new(
                    ErrorCode::ScopeDenied,
                    format!("Failed to move to recovery path: {e}"),
                )
            })?;

            // Write new file
            if let Err(e) = fs::write(&dest, &buf) {
                // Restore original from recovery path
                let _ = fs::rename(&recovery_path, &dest);
                return Err(AppError::new(ErrorCode::ScopeDenied, e.to_string()));
            }

            Ok(FileReceipt {
                path: change.path.clone(),
                before: Some(live_digest),
                after: Some(after_digest),
                applied: true,
                recovery_path: Some(recovery_name),
                conflict: false,
            })
        }
        FileChangeKind::Delete => {
            permit.permits(&ActionClass::DeleteProjectFiles, Some(&change.path))?;
            let dest = root.canonical.join(rel.as_str());

            if !dest.exists() {
                return Ok(FileReceipt {
                    path: change.path.clone(),
                    before: None,
                    after: None,
                    applied: false,
                    recovery_path: None,
                    conflict: true,
                });
            }

            let live_bytes = fs::read(&dest)
                .map_err(|e| AppError::new(ErrorCode::ScopeDenied, e.to_string()))?;
            let live_digest = Digest::of(&live_bytes);

            if let Some(exp_before) = change.before
                && exp_before != live_digest
            {
                return Ok(FileReceipt {
                    path: change.path.clone(),
                    before: Some(live_digest),
                    after: None,
                    applied: false,
                    recovery_path: None,
                    conflict: true,
                });
            }

            let recovery_name = format!(".maya-recovery-{}", Uuid::new_v4());
            let recovery_path = dest.parent().unwrap().join(&recovery_name);

            fs::rename(&dest, &recovery_path).map_err(|e| {
                AppError::new(
                    ErrorCode::ScopeDenied,
                    format!("Failed to move to recovery path: {e}"),
                )
            })?;

            Ok(FileReceipt {
                path: change.path.clone(),
                before: Some(live_digest),
                after: None,
                applied: true,
                recovery_path: Some(recovery_name),
                conflict: false,
            })
        }
    }
}
