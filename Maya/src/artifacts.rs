use crate::db::Db;
use crate::error::{AppError, ErrorCode, Result};
use crate::policy::ExecutionPermit;
use crate::types::{ActionClass, Artifact, ArtifactKind, Digest, Id, Provenance, UtcMs};
use rusqlite::params;
use sha2::{Digest as _, Sha256};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct ArtifactInput {
    pub job_id: Option<Id>,
    pub task_id: Option<Id>,
    pub kind: ArtifactKind,
    pub mime: String,
    pub provenance: Provenance,
    pub max_bytes: u64,
    pub expires_at: Option<UtcMs>,
}

#[derive(Clone, Debug)]
pub struct ArtifactStore {
    dir: PathBuf,
    db: Db,
}

impl ArtifactStore {
    pub fn new(dir: PathBuf, db: Db) -> Self {
        Self { dir, db }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub async fn put(
        &self,
        meta: ArtifactInput,
        mut reader: std::pin::Pin<Box<dyn tokio::io::AsyncRead + Send + Unpin>>,
    ) -> Result<Artifact> {
        use tokio::io::AsyncReadExt;
        let mut buf = Vec::new();
        reader.read_to_end(&mut buf).await.map_err(|e| {
            AppError::new(ErrorCode::Internal, format!("Failed to read artifact: {e}"))
        })?;
        self.put_bytes(meta, &buf).await
    }

    pub async fn put_bytes(&self, meta: ArtifactInput, data: &[u8]) -> Result<Artifact> {
        let size_bytes = data.len() as u64;
        if size_bytes > meta.max_bytes {
            return Err(AppError::new(
                ErrorCode::OutputLimit,
                format!(
                    "Artifact exceeds maximum allowed bytes (limit: {})",
                    meta.max_bytes
                ),
            ));
        }

        let sha256 = Digest::of(data);
        let id = Uuid::new_v4();
        let storage_name = format!("{}_{}", id, sha256.to_hex());
        let target_path = self.dir.join(&storage_name);
        let temp_path = self.dir.join(format!(".tmp_{}", Uuid::new_v4()));

        tokio::task::spawn_blocking({
            let temp = temp_path.clone();
            let target = target_path.clone();
            let bytes = data.to_vec();
            move || -> Result<()> {
                fs::create_dir_all(temp.parent().ok_or_else(|| {
                    AppError::new(ErrorCode::Internal, "Artifact directory has no parent")
                })?)
                .map_err(|e| {
                    AppError::new(
                        ErrorCode::Internal,
                        format!("Failed to create artifact directory: {e}"),
                    )
                })?;
                fs::set_permissions(
                    temp.parent().ok_or_else(|| {
                        AppError::new(ErrorCode::Internal, "Artifact directory has no parent")
                    })?,
                    fs::Permissions::from_mode(0o700),
                )
                .map_err(|e| {
                    AppError::new(
                        ErrorCode::Internal,
                        format!("Failed to secure artifact directory: {e}"),
                    )
                })?;
                let mut f = File::create(&temp).map_err(|e| {
                    AppError::new(
                        ErrorCode::Internal,
                        format!("Failed to create temp artifact: {e}"),
                    )
                })?;
                f.write_all(&bytes).map_err(|e| {
                    AppError::new(
                        ErrorCode::Internal,
                        format!("Failed to write artifact: {e}"),
                    )
                })?;
                f.sync_all().map_err(|e| {
                    AppError::new(
                        ErrorCode::Internal,
                        format!("Failed to fsync artifact: {e}"),
                    )
                })?;
                fs::rename(&temp, &target).map_err(|e| {
                    let _ = fs::remove_file(&temp);
                    AppError::new(
                        ErrorCode::Internal,
                        format!("Failed to commit artifact: {e}"),
                    )
                })?;
                Ok(())
            }
        })
        .await
        .map_err(|e| AppError::new(ErrorCode::Internal, e.to_string()))??;

        let now = chrono::Utc::now().timestamp_millis();
        let prov_json = serde_json::to_string(&meta.provenance).unwrap_or_default();
        let kind_str = serde_json::to_string(&meta.kind)
            .unwrap_or_default()
            .trim_matches('"')
            .to_string();

        let artifact = Artifact {
            id,
            job_id: meta.job_id,
            task_id: meta.task_id,
            kind: meta.kind,
            storage_name: storage_name.clone(),
            sha256,
            size_bytes,
            mime: meta.mime.clone(),
            provenance: meta.provenance,
            created_at: now,
            redacted: false,
            expires_at: meta.expires_at,
        };

        let art_clone = artifact.clone();
        self.db
            .write(move |tx| {
                tx.execute(
                    r#"INSERT INTO artifacts (id, job_id, task_id, kind, storage_name, sha256, size_bytes, mime, provenance_json, redacted, expires_at, created_at)
                       VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, ?10, ?11)"#,
                    params![
                        art_clone.id.to_string(),
                        art_clone.job_id.map(|id| id.to_string()),
                        art_clone.task_id.map(|id| id.to_string()),
                        kind_str,
                        art_clone.storage_name,
                        art_clone.sha256.to_hex(),
                        art_clone.size_bytes,
                        art_clone.mime,
                        prov_json,
                        art_clone.expires_at,
                        art_clone.created_at,
                    ],
                )
                .map_err(|e| AppError::new(ErrorCode::DbUnavailable, e.to_string()))?;
                Ok(())
            })
            .await?;

        Ok(artifact)
    }

    pub async fn get_metadata(&self, id: Id) -> Result<Artifact> {
        let id_str = id.to_string();
        self.db
            .read(move |conn| {
                let mut stmt = conn
                    .prepare(
                        r#"SELECT id, job_id, task_id, kind, storage_name, sha256, size_bytes, mime, provenance_json, redacted, expires_at, created_at
                           FROM artifacts WHERE id = ?1"#,
                    )
                    .map_err(|e| AppError::new(ErrorCode::DbUnavailable, e.to_string()))?;

                stmt.query_row(params![id_str], |r| {
                    let id_s: String = r.get(0)?;
                    let job_s: Option<String> = r.get(1)?;
                    let task_s: Option<String> = r.get(2)?;
                    let kind_s: String = r.get(3)?;
                    let storage_name: String = r.get(4)?;
                    let sha_s: String = r.get(5)?;
                    let size_bytes: u64 = r.get(6)?;
                    let mime: String = r.get(7)?;
                    let prov_s: String = r.get(8)?;
                    let redacted_i: i64 = r.get(9)?;
                    let expires_at: Option<i64> = r.get(10)?;
                    let created_at: i64 = r.get(11)?;

                    let kind: ArtifactKind =
                        serde_json::from_str(&format!("\"{}\"", kind_s)).unwrap_or(ArtifactKind::Context);
                    let provenance: Provenance = serde_json::from_str(&prov_s).unwrap_or(Provenance {
                        source: crate::types::ProvenanceSource::System,
                        provider_id: None,
                        model_id: None,
                        url: None,
                        observed_at: created_at,
                    });
                    let sha256 = Digest::from_hex(&sha_s).unwrap_or(Digest::ZERO);

                    Ok(Artifact {
                        id: id_s.parse::<Id>().unwrap(),
                        job_id: job_s.and_then(|s| s.parse::<Id>().ok()),
                        task_id: task_s.and_then(|s| s.parse::<Id>().ok()),
                        kind,
                        storage_name,
                        sha256,
                        size_bytes,
                        mime,
                        provenance,
                        created_at,
                        redacted: redacted_i != 0,
                        expires_at,
                    })
                })
                .map_err(|_| AppError::new(ErrorCode::NotFound, "Artifact not found"))
            })
            .await
    }

    pub async fn verified_file(&self, id: Id) -> Result<File> {
        let meta = self.get_metadata(id).await?;
        let target_path = self.dir.join(&meta.storage_name);

        tokio::task::spawn_blocking(move || {
            let mut file = File::open(&target_path).map_err(|e| {
                AppError::new(
                    ErrorCode::NotFound,
                    format!("Artifact storage file missing: {e}"),
                )
            })?;

            let mut hasher = Sha256::new();
            let mut buf = [0u8; 8192];
            loop {
                let n = file
                    .read(&mut buf)
                    .map_err(|e| AppError::new(ErrorCode::Internal, e.to_string()))?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
            }

            let res = hasher.finalize();
            let mut sha_bytes = [0u8; 32];
            sha_bytes.copy_from_slice(&res);

            if Digest(sha_bytes) != meta.sha256 {
                return Err(AppError::new(
                    ErrorCode::IntegrityFailure,
                    "Artifact checksum verification failed",
                ));
            }

            // Re-open for reading from start
            File::open(&target_path).map_err(|e| AppError::new(ErrorCode::Internal, e.to_string()))
        })
        .await
        .map_err(|e| AppError::new(ErrorCode::Internal, e.to_string()))?
    }

    pub async fn read_bytes(&self, id: Id) -> Result<Vec<u8>> {
        let meta = self.get_metadata(id).await?;
        let target_path = self.dir.join(&meta.storage_name);

        tokio::task::spawn_blocking(move || {
            let bytes = fs::read(&target_path).map_err(|e| {
                AppError::new(
                    ErrorCode::NotFound,
                    format!("Artifact file read error: {e}"),
                )
            })?;
            let digest = Digest::of(&bytes);
            if digest != meta.sha256 {
                return Err(AppError::new(
                    ErrorCode::IntegrityFailure,
                    "Artifact checksum verification failed",
                ));
            }
            Ok(bytes)
        })
        .await
        .map_err(|e| AppError::new(ErrorCode::Internal, e.to_string()))?
    }

    pub async fn delete(&self, id: Id, permit: &ExecutionPermit) -> Result<()> {
        permit
            .permits(&ActionClass::DeleteProjectFiles, None)
            .or_else(|_| permit.permits(&ActionClass::DeleteHistory, None))?;

        let meta = self.get_metadata(id).await?;
        let target_path = self.dir.join(&meta.storage_name);
        let id_str = id.to_string();

        self.db
            .write(move |tx| {
                tx.execute("DELETE FROM artifacts WHERE id = ?1", params![id_str])
                    .map_err(|e| AppError::new(ErrorCode::DbUnavailable, e.to_string()))?;
                Ok(())
            })
            .await?;

        let _ = fs::remove_file(&target_path);
        Ok(())
    }
}
