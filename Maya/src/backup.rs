use crate::config::RuntimeConfig;
use crate::db::Db;
use crate::error::{AppError, ErrorCode, Result};
use crate::policy::ExecutionPermit;
use crate::types::{ActionClass, BackupRecord, BackupRecordState, Digest, Id, RestoreReceipt};
use rusqlite::params;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Clone)]
pub struct BackupService {
    db: Db,
    config: Arc<RuntimeConfig>,
}

impl BackupService {
    pub fn new(db: Db, config: Arc<RuntimeConfig>) -> Self {
        Self { db, config }
    }

    pub fn db(&self) -> &Db {
        &self.db
    }

    pub fn config(&self) -> &RuntimeConfig {
        &self.config
    }

    pub async fn create(&self) -> Result<BackupRecord> {
        let backups_dir = self.config.backups_dir();
        std::fs::create_dir_all(&backups_dir).map_err(|e| {
            AppError::new(
                ErrorCode::Internal,
                format!("Failed to create backups directory: {e}"),
            )
        })?;

        let id = Uuid::new_v4();
        let storage_name = format!("backup-{id}.sqlite3");
        let temp_path = backups_dir.join(format!("backup-{id}.tmp"));
        let final_path = backups_dir.join(&storage_name);

        // 1. Online backup from live SQLite connection into temporary file
        self.db.backup_to(&temp_path).await?;

        // 2. Sanitize the backup database: clear idempotency, secrets, approvals, in-flight jobs
        let sanitize_res = (|| -> Result<()> {
            let sanitized = rusqlite::Connection::open(&temp_path).map_err(|e| {
                AppError::new(
                    ErrorCode::Internal,
                    format!("Failed to open temp backup DB: {e}"),
                )
            })?;

            sanitized
                .execute_batch(
                    "DELETE FROM idempotency_keys;
                     UPDATE providers SET secret_ref = 'redacted', credential_generation = 0;
                     UPDATE approvals SET decision = 'revoked' WHERE decision IN ('pending', 'approved');
                     UPDATE jobs SET state = 'needs_reconciliation' WHERE state IN ('inspecting', 'reviewing', 'planned', 'checkpointing', 'executing', 'verifying', 'promoting');
                     VACUUM;",
                )
                .map_err(|e| {
                    AppError::new(
                        ErrorCode::Internal,
                        format!("Failed to sanitize backup DB: {e}"),
                    )
                })?;

            drop(sanitized);

            std::fs::rename(&temp_path, &final_path).map_err(|e| {
                AppError::new(
                    ErrorCode::Internal,
                    format!("Failed to finalize backup file: {e}"),
                )
            })?;

            Ok(())
        })();

        if let Err(e) = sanitize_res {
            let _ = std::fs::remove_file(&temp_path);
            let _ = std::fs::remove_file(&final_path);
            return Err(e);
        }

        // 3. Compute SHA-256 digest and size
        let bytes = std::fs::read(&final_path).map_err(|e| {
            AppError::new(
                ErrorCode::Internal,
                format!("Failed to read finalized backup: {e}"),
            )
        })?;
        let digest = Digest::of(&bytes);
        let size_bytes = bytes.len() as u64;

        // 4. Insert into backups table and enforce retention limit
        let now = chrono::Utc::now().timestamp_millis();
        let retention = 7usize; // default retention 7 backups
        let storage_name_clone = storage_name.clone();
        let digest_hex = digest.to_hex();

        let pruned_files = self
            .db
            .write(move |tx| {
                tx.execute(
                    "INSERT INTO backups (id, storage_name, schema_version, manifest_sha256, size_bytes, state, created_at, verified_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        id.to_string(),
                        storage_name_clone,
                        1,
                        digest_hex,
                        size_bytes as i64,
                        "verified",
                        now,
                        now,
                    ],
                )?;

                // Query older backups beyond retention cap
                let mut stmt = tx.prepare(
                    "SELECT id, storage_name FROM backups
                     WHERE state = 'verified'
                     ORDER BY created_at DESC
                     LIMIT -1 OFFSET ?1",
                )?;

                let old_backups = stmt
                    .query_map(params![retention as i64], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                    })?
                    .collect::<std::result::Result<Vec<_>, rusqlite::Error>>()?;

                let mut to_delete = Vec::new();
                for (old_id, old_storage) in old_backups {
                    tx.execute("DELETE FROM backups WHERE id = ?1", params![old_id])?;
                    to_delete.push(old_storage);
                }

                Ok(to_delete)
            })
            .await?;

        // 5. Remove pruned backup files from disk
        for old_storage in pruned_files {
            let p = backups_dir.join(old_storage);
            let _ = std::fs::remove_file(p);
        }

        Ok(BackupRecord {
            id,
            storage_name,
            schema_version: 1,
            manifest_sha256: digest,
            size_bytes,
            state: BackupRecordState::Verified,
            created_at: now,
            verified_at: Some(now),
        })
    }

    pub async fn verify(&self, id: Id) -> Result<BackupRecord> {
        let (storage_name, schema_version, manifest_sha256, size_bytes, _state, created_at) =
            self.db
                .read(move |conn| {
                    let mut stmt = conn.prepare(
                        "SELECT storage_name, schema_version, manifest_sha256, size_bytes, state, created_at
                         FROM backups WHERE id = ?1",
                    )?;

                    stmt.query_row(params![id.to_string()], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, i64>(3)?,
                            row.get::<_, String>(4)?,
                            row.get::<_, i64>(5)?,
                        ))
                    })
                    .map_err(|e| match e {
                        rusqlite::Error::QueryReturnedNoRows => AppError::new(
                            ErrorCode::NotFound,
                            format!("Backup {id} not found"),
                        ),
                        other => AppError::from(other),
                    })
                })
                .await?;

        let backup_path = self.config.backups_dir().join(&storage_name);
        if !backup_path.exists() {
            self.db
                .write(move |tx| {
                    tx.execute(
                        "UPDATE backups SET state = 'failed' WHERE id = ?1",
                        params![id.to_string()],
                    )?;
                    Ok(())
                })
                .await?;

            return Err(AppError::new(
                ErrorCode::IntegrityFailure,
                format!("Backup file {storage_name} missing on disk"),
            ));
        }

        let bytes = std::fs::read(&backup_path).map_err(|e| {
            AppError::new(
                ErrorCode::Internal,
                format!("Failed to read backup file {storage_name}: {e}"),
            )
        })?;

        let computed_digest = Digest::of(&bytes);
        if computed_digest.to_hex() != manifest_sha256 {
            self.db
                .write(move |tx| {
                    tx.execute(
                        "UPDATE backups SET state = 'failed' WHERE id = ?1",
                        params![id.to_string()],
                    )?;
                    Ok(())
                })
                .await?;

            return Err(AppError::new(
                ErrorCode::IntegrityFailure,
                format!(
                    "Backup manifest SHA-256 mismatch: recorded {}, computed {}",
                    manifest_sha256,
                    computed_digest.to_hex()
                ),
            ));
        }

        let now = chrono::Utc::now().timestamp_millis();
        self.db
            .write(move |tx| {
                tx.execute(
                    "UPDATE backups SET verified_at = ?1, state = 'verified' WHERE id = ?2",
                    params![now, id.to_string()],
                )?;
                Ok(())
            })
            .await?;

        Ok(BackupRecord {
            id,
            storage_name,
            schema_version,
            manifest_sha256: computed_digest,
            size_bytes: size_bytes as u64,
            state: BackupRecordState::Verified,
            created_at,
            verified_at: Some(now),
        })
    }

    pub async fn restore(&self, id: Id, permit: &ExecutionPermit) -> Result<RestoreReceipt> {
        permit.permits(&ActionClass::RestoreDatabase, None)?;

        // 1. Verify backup integrity before restoring
        let record = self.verify(id).await?;
        let backup_path = self.config.backups_dir().join(&record.storage_name);

        // 2. Pre-restore safety snapshot of the live database
        let now = chrono::Utc::now().timestamp_millis();
        let pre_restore_path = self
            .config
            .backups_dir()
            .join(format!("pre-restore-{}.sqlite3", now));

        self.db
            .restore_from(&backup_path, &pre_restore_path)
            .await?;

        let reconciled_job_ids = self
            .db
            .write(move |tx| {
                tx.execute(
                    "UPDATE approvals
                     SET decision = 'revoked', decided_at = COALESCE(decided_at, ?1)
                     WHERE decision IN ('pending', 'approved')",
                    params![now],
                )?;
                tx.execute(
                    "UPDATE jobs SET state = 'needs_reconciliation', updated_at = ?1
                     WHERE state IN ('inspecting', 'reviewing', 'planned', 'approved', 'checkpointing', 'executing', 'verifying', 'promoting', 'cancelling')",
                    params![now],
                )?;

                let mut stmt = tx.prepare("SELECT id FROM jobs WHERE state = 'needs_reconciliation'")?;
                let rows = stmt
                    .query_map([], |row| {
                        let value: String = row.get(0)?;
                        Ok(Uuid::parse_str(&value).unwrap_or_default())
                    })?
                    .collect::<std::result::Result<Vec<_>, rusqlite::Error>>()?;
                Ok(rows)
            })
            .await?;

        Ok(RestoreReceipt {
            backup_id: id,
            restored_at: now,
            reconciliation_job_ids: reconciled_job_ids,
        })
    }

    pub async fn list(&self) -> Result<Vec<BackupRecord>> {
        self.db
            .read(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, storage_name, schema_version, manifest_sha256, size_bytes, state, created_at, verified_at
                     FROM backups
                     ORDER BY created_at DESC",
                )?;

                let rows = stmt
                    .query_map([], |row| {
                        let id_str: String = row.get(0)?;
                        let storage_name: String = row.get(1)?;
                        let schema_version: i64 = row.get(2)?;
                        let manifest_sha256: String = row.get(3)?;
                        let size_bytes: i64 = row.get(4)?;
                        let state_str: String = row.get(5)?;
                        let created_at: i64 = row.get(6)?;
                        let verified_at: Option<i64> = row.get(7)?;

                        let state = match state_str.as_str() {
                            "creating" => BackupRecordState::Creating,
                            "verified" => BackupRecordState::Verified,
                            _ => BackupRecordState::Failed,
                        };

                        Ok(BackupRecord {
                            id: Uuid::parse_str(&id_str).unwrap_or_default(),
                            storage_name,
                            schema_version,
                            manifest_sha256: Digest::from_hex(&manifest_sha256).unwrap_or(Digest::ZERO),
                            size_bytes: size_bytes as u64,
                            state,
                            created_at,
                            verified_at,
                        })
                    })?
                    .collect::<std::result::Result<Vec<_>, rusqlite::Error>>()?;

                Ok(rows)
            })
            .await
    }
}
