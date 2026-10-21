use crate::error::{AppError, ErrorCode, Result};
use rusqlite::{Connection, Transaction, TransactionBehavior};
use sha2::{Digest as _, Sha256};
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{SyncSender, sync_channel};
use std::thread;
use tokio::sync::oneshot;

pub const INITIAL_SCHEMA_SQL: &str = include_str!("../migrations/0001_initial.sql");

type ReadCommand = Box<dyn FnOnce(&Connection) -> Result<()> + Send>;
type WriteCommand = Box<dyn FnOnce(&mut Connection) -> Result<()> + Send>;

enum DbMessage {
    Read(ReadCommand),
    Write(WriteCommand),
    Restore {
        source: PathBuf,
        safety_copy: PathBuf,
        response: oneshot::Sender<Result<()>>,
    },
    Close(oneshot::Sender<()>),
}

#[derive(Clone, Debug)]
pub struct Db {
    sender: SyncSender<DbMessage>,
    path: PathBuf,
}

impl Db {
    pub async fn open(path: &Path) -> Result<Self> {
        let path_buf = path.to_path_buf();
        let (tx, rx) = sync_channel::<DbMessage>(128);

        let (init_tx, init_rx) = oneshot::channel::<Result<()>>();

        let path_clone = path_buf.clone();
        thread::Builder::new()
            .name("maya-db-worker".to_string())
            .spawn(move || {
                let conn_res = Self::open_connection(&path_clone);
                let mut conn = match conn_res {
                    Ok(c) => {
                        let _ = init_tx.send(Ok(()));
                        c
                    }
                    Err(e) => {
                        let _ = init_tx.send(Err(e));
                        return;
                    }
                };

                while let Ok(msg) = rx.recv() {
                    match msg {
                        DbMessage::Read(f) => {
                            let _ = f(&conn);
                        }
                        DbMessage::Write(f) => {
                            let _ = f(&mut conn);
                        }
                        DbMessage::Restore {
                            source,
                            safety_copy,
                            response,
                        } => {
                            let replacement = match Connection::open_in_memory() {
                                Ok(replacement) => replacement,
                                Err(error) => {
                                    let _ = response.send(Err(AppError::new(
                                        ErrorCode::DbUnavailable,
                                        format!("Failed to prepare database restore: {error}"),
                                    )));
                                    continue;
                                }
                            };
                            let live = std::mem::replace(&mut conn, replacement);
                            match Self::replace_database(live, &path_clone, &source, &safety_copy) {
                                Ok(restored) => {
                                    conn = restored;
                                    let _ = response.send(Ok(()));
                                }
                                Err(error) => {
                                    if let Ok(reopened) = Self::open_connection(&path_clone) {
                                        conn = reopened;
                                    }
                                    let _ = response.send(Err(error));
                                }
                            }
                        }
                        DbMessage::Close(ack) => {
                            let _ = ack.send(());
                            break;
                        }
                    }
                }
            })
            .map_err(|e| {
                AppError::new(
                    ErrorCode::DbUnavailable,
                    format!("Failed to spawn DB thread: {e}"),
                )
            })?;

        init_rx.await.map_err(|_| {
            AppError::new(
                ErrorCode::DbUnavailable,
                "DB thread panicked during startup",
            )
        })??;

        let db = Db {
            sender: tx,
            path: path_buf,
        };

        db.migrate().await?;

        Ok(db)
    }

    fn open_connection(path: &Path) -> Result<Connection> {
        let conn = Connection::open(path).map_err(|e| {
            AppError::new(
                ErrorCode::DbUnavailable,
                format!("Failed to open SQLite database at {:?}: {e}", path),
            )
        })?;

        // Apply PRAGMAs from BP §5:
        // foreign_keys=ON, journal_mode=WAL, synchronous=FULL, busy_timeout=5000,
        // cache_size=-8192, mmap_size=0, temp_store=FILE, wal_autocheckpoint=1000,
        // trusted_schema=OFF, secure_delete=ON
        conn.execute_batch(
            r#"
            PRAGMA foreign_keys = ON;
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous = FULL;
            PRAGMA busy_timeout = 5000;
            PRAGMA cache_size = -8192;
            PRAGMA mmap_size = 0;
            PRAGMA temp_store = FILE;
            PRAGMA wal_autocheckpoint = 1000;
            PRAGMA trusted_schema = OFF;
            PRAGMA secure_delete = ON;
            "#,
        )
        .map_err(|e| {
            AppError::new(
                ErrorCode::DbUnavailable,
                format!("Failed to configure SQLite pragmas: {e}"),
            )
        })?;

        Ok(conn)
    }

    fn replace_database(
        live: Connection,
        live_path: &Path,
        source_path: &Path,
        safety_copy: &Path,
    ) -> Result<Connection> {
        if live_path == source_path || !source_path.is_file() {
            return Err(AppError::new(
                ErrorCode::IntegrityFailure,
                "Restore source is missing or aliases the live database",
            ));
        }

        let parent = match live_path.parent() {
            Some(parent) => parent,
            None => {
                return Err(AppError::new(
                    ErrorCode::IntegrityFailure,
                    "Live database path has no parent directory",
                ));
            }
        };
        let nonce = uuid::Uuid::new_v4();
        let staged_path = parent.join(format!(".maya-restore-{nonce}.sqlite3"));
        let rollback_path = parent.join(format!(".maya-rollback-{nonce}.sqlite3"));

        if let Err(error) = fs::copy(source_path, &staged_path) {
            return Err(AppError::new(
                ErrorCode::IntegrityFailure,
                format!("Failed to stage verified database backup: {error}"),
            ));
        }
        if let Err(error) = File::open(&staged_path).and_then(|file| file.sync_all()) {
            let _ = fs::remove_file(&staged_path);
            return Err(AppError::new(
                ErrorCode::IntegrityFailure,
                format!("Failed to sync staged database backup: {error}"),
            ));
        }

        let staged_check = (|| -> Result<()> {
            let staged = Connection::open_with_flags(
                &staged_path,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )?;
            let integrity: String =
                staged.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
            if integrity != "ok" {
                return Err(AppError::new(
                    ErrorCode::IntegrityFailure,
                    "Restore source failed SQLite integrity_check",
                ));
            }
            let foreign_key_errors: i64 =
                staged.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |row| {
                    row.get(0)
                })?;
            if foreign_key_errors != 0 {
                return Err(AppError::new(
                    ErrorCode::IntegrityFailure,
                    "Restore source failed SQLite foreign_key_check",
                ));
            }
            Ok(())
        })();
        if let Err(error) = staged_check {
            let _ = fs::remove_file(&staged_path);
            return Err(error);
        }

        if let Err(error) = Self::backup_connection(&live, safety_copy) {
            let _ = fs::remove_file(&staged_path);
            return Err(error);
        }
        if let Err(error) = live.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);") {
            let _ = fs::remove_file(&staged_path);
            return Err(AppError::new(
                ErrorCode::IntegrityFailure,
                format!("Failed to checkpoint live database before restore: {error}"),
            ));
        }
        if let Err((_live, error)) = live.close() {
            let _ = fs::remove_file(&staged_path);
            return Err(AppError::new(
                ErrorCode::IntegrityFailure,
                format!("Failed to close live database before restore: {error}"),
            ));
        }

        if let Err(error) = fs::rename(live_path, &rollback_path) {
            let _ = fs::remove_file(&staged_path);
            return Err(AppError::new(
                ErrorCode::IntegrityFailure,
                format!("Failed to preserve live database during restore: {error}"),
            ));
        }

        if let Err(error) = fs::rename(&staged_path, live_path) {
            let _ = fs::rename(&rollback_path, live_path);
            return Err(AppError::new(
                ErrorCode::IntegrityFailure,
                format!("Failed to install restored database: {error}"),
            ));
        }

        if let Ok(directory) = OpenOptions::new().read(true).open(parent) {
            let _ = directory.sync_all();
        }

        match Self::open_connection(live_path) {
            Ok(restored) => {
                let _ = fs::remove_file(&rollback_path);
                Ok(restored)
            }
            Err(error) => {
                let failed_path = parent.join(format!(".maya-failed-restore-{nonce}.sqlite3"));
                let _ = fs::rename(live_path, failed_path);
                let _ = fs::rename(&rollback_path, live_path);
                Err(error)
            }
        }
    }

    fn backup_connection(conn: &Connection, destination: &Path) -> Result<()> {
        if destination.exists() {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "Pre-restore safety backup already exists",
            ));
        }
        let mut destination_conn = Connection::open(destination)?;
        let backup = rusqlite::backup::Backup::new(conn, &mut destination_conn)?;
        backup
            .run_to_completion(100, std::time::Duration::from_millis(10), None)
            .map_err(AppError::from)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub async fn read<T: Send + 'static>(
        &self,
        f: impl FnOnce(&Connection) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let (resp_tx, resp_rx) = oneshot::channel();
        let task = Box::new(move |conn: &Connection| {
            let res = f(conn);
            let _ = resp_tx.send(res);
            Ok(())
        });

        self.sender
            .try_send(DbMessage::Read(task))
            .map_err(|e| match e {
                std::sync::mpsc::TrySendError::Full(_) => {
                    AppError::new(ErrorCode::Busy, "Database queue full")
                }
                std::sync::mpsc::TrySendError::Disconnected(_) => {
                    AppError::new(ErrorCode::DbUnavailable, "Database thread disconnected")
                }
            })?;

        resp_rx
            .await
            .map_err(|_| AppError::new(ErrorCode::DbUnavailable, "Database read cancelled"))?
    }

    pub async fn write<T: Send + 'static>(
        &self,
        f: impl FnOnce(&Transaction<'_>) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let (resp_tx, resp_rx) = oneshot::channel();
        let task = Box::new(move |conn: &mut Connection| {
            let tx = conn
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(|e| {
                    AppError::new(
                        ErrorCode::DbUnavailable,
                        format!("Failed to begin transaction: {e}"),
                    )
                })?;

            let res = f(&tx);
            match res {
                Ok(val) => {
                    tx.commit().map_err(|e| {
                        AppError::new(
                            ErrorCode::DbUnavailable,
                            format!("Failed to commit transaction: {e}"),
                        )
                    })?;
                    let _ = resp_tx.send(Ok(val));
                }
                Err(err) => {
                    let _ = tx.rollback();
                    let _ = resp_tx.send(Err(err));
                }
            }
            Ok(())
        });

        self.sender
            .try_send(DbMessage::Write(task))
            .map_err(|e| match e {
                std::sync::mpsc::TrySendError::Full(_) => {
                    AppError::new(ErrorCode::Busy, "Database queue full")
                }
                std::sync::mpsc::TrySendError::Disconnected(_) => {
                    AppError::new(ErrorCode::DbUnavailable, "Database thread disconnected")
                }
            })?;

        resp_rx
            .await
            .map_err(|_| AppError::new(ErrorCode::DbUnavailable, "Database write cancelled"))?
    }

    pub async fn raw_write<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let (resp_tx, resp_rx) = oneshot::channel();
        let task = Box::new(move |conn: &mut Connection| {
            let res = f(conn);
            let _ = resp_tx.send(res);
            Ok(())
        });

        self.sender
            .try_send(DbMessage::Write(task))
            .map_err(|e| match e {
                std::sync::mpsc::TrySendError::Full(_) => {
                    AppError::new(ErrorCode::Busy, "Database queue full")
                }
                std::sync::mpsc::TrySendError::Disconnected(_) => {
                    AppError::new(ErrorCode::DbUnavailable, "Database thread disconnected")
                }
            })?;

        resp_rx
            .await
            .map_err(|_| AppError::new(ErrorCode::DbUnavailable, "Database write cancelled"))?
    }

    pub async fn migrate(&self) -> Result<()> {
        let expected_sha = {
            let mut hasher = Sha256::new();
            hasher.update(INITIAL_SCHEMA_SQL.as_bytes());
            hex::encode(hasher.finalize())
        };

        self.write(move |tx| {
            // Check if schema_migrations table exists
            let mut check_stmt = tx
                .prepare(
                    "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='schema_migrations'",
                )
                .map_err(|e| AppError::new(ErrorCode::DbUnavailable, e.to_string()))?;
            let has_migrations_table: i64 = check_stmt
                .query_row([], |r| r.get(0))
                .unwrap_or(0);

            if has_migrations_table > 0 {
                // Check applied migrations
                let mut stmt = tx
                    .prepare("SELECT version, sha256 FROM schema_migrations ORDER BY version ASC")
                    .map_err(|e| AppError::new(ErrorCode::DbUnavailable, e.to_string()))?;
                let rows: Vec<(i64, String)> = stmt
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                    .map_err(|e| AppError::new(ErrorCode::DbUnavailable, e.to_string()))?
                    .collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(|e| AppError::new(ErrorCode::DbUnavailable, e.to_string()))?;

                if let Some((v, hash)) = rows.first() {
                    if *v > 1 {
                        return Err(AppError::new(
                            ErrorCode::SchemaTooNew,
                            "Database schema version is newer than supported",
                        ));
                    }
                    if hash != &expected_sha {
                        return Err(AppError::new(
                            ErrorCode::MigrationFailed,
                            "Database schema migration hash mismatch",
                        ));
                    }
                    // Version 1 already applied and matches hash
                    return Ok(());
                }
            }

            // Apply 0001_initial.sql
            tx.execute_batch(INITIAL_SCHEMA_SQL).map_err(|e| {
                AppError::new(
                    ErrorCode::MigrationFailed,
                    format!("Failed to execute initial schema: {e}"),
                )
            })?;

            let now = chrono::Utc::now().timestamp_millis();
            tx.execute(
                "INSERT INTO schema_migrations (version, sha256, applied_at) VALUES (1, ?1, ?2)",
                rusqlite::params![expected_sha, now],
            )
            .map_err(|e| {
                AppError::new(
                    ErrorCode::MigrationFailed,
                    format!("Failed to record migration: {e}"),
                )
            })?;

            Ok(())
        })
        .await
    }

    pub async fn backup_to(&self, dest_path: &Path) -> Result<()> {
        let dest = dest_path.to_path_buf();
        self.read(move |conn| {
            let mut dst_conn = Connection::open(&dest).map_err(|e| {
                AppError::new(
                    ErrorCode::DbUnavailable,
                    format!("Failed to open backup destination database: {e}"),
                )
            })?;
            let backup = rusqlite::backup::Backup::new(conn, &mut dst_conn).map_err(|e| {
                AppError::new(
                    ErrorCode::DbUnavailable,
                    format!("Failed to initialize SQLite backup: {e}"),
                )
            })?;
            backup
                .run_to_completion(100, std::time::Duration::from_millis(50), None)
                .map_err(|e| {
                    AppError::new(
                        ErrorCode::DbUnavailable,
                        format!("SQLite backup execution failed: {e}"),
                    )
                })?;
            Ok(())
        })
        .await
    }

    pub async fn restore_from(&self, source_path: &Path, safety_copy: &Path) -> Result<()> {
        let (response, receive) = oneshot::channel();
        self.sender
            .try_send(DbMessage::Restore {
                source: source_path.to_path_buf(),
                safety_copy: safety_copy.to_path_buf(),
                response,
            })
            .map_err(|error| match error {
                std::sync::mpsc::TrySendError::Full(_) => {
                    AppError::new(ErrorCode::Busy, "Database queue full")
                }
                std::sync::mpsc::TrySendError::Disconnected(_) => {
                    AppError::new(ErrorCode::DbUnavailable, "Database thread disconnected")
                }
            })?;
        receive
            .await
            .map_err(|_| AppError::new(ErrorCode::DbUnavailable, "Database restore cancelled"))?
    }

    pub async fn close(self) -> Result<()> {
        let (ack_tx, ack_rx) = oneshot::channel();
        let _ = self.sender.send(DbMessage::Close(ack_tx));
        let _ = ack_rx.await;
        Ok(())
    }
}
