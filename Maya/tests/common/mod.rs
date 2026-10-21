#![allow(dead_code)]

use maya_mini::db::Db;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use tempfile::TempDir;

pub struct TempProject {
    pub dir: TempDir,
    pub path: PathBuf,
}

impl TempProject {
    pub fn new() -> Self {
        let dir = tempfile::tempdir().expect("Failed to create temp project dir");
        let path = dir
            .path()
            .canonicalize()
            .unwrap_or_else(|_| dir.path().to_path_buf());
        Self { dir, path }
    }

    pub fn write_file(&self, relative_path: &str, content: &[u8]) {
        let full = self.path.join(relative_path);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).expect("Failed to create parent dirs");
        }
        fs::write(full, content).expect("Failed to write file");
    }

    pub fn read_file(&self, relative_path: &str) -> Vec<u8> {
        let full = self.path.join(relative_path);
        fs::read(full).expect("Failed to read file")
    }
}

pub fn repo() -> TempProject {
    TempProject::new()
}

pub async fn test_db() -> (TempDir, Db) {
    let dir = tempfile::tempdir().expect("Failed to create temp dir for DB");
    let db_path = dir.path().join("test.sqlite3");
    let db = Db::open(&db_path).await.expect("Failed to open test DB");
    (dir, db)
}

#[derive(Clone)]
pub struct FakeClock {
    now: Arc<AtomicI64>,
}

impl FakeClock {
    pub fn new(initial_ms: i64) -> Self {
        Self {
            now: Arc::new(AtomicI64::new(initial_ms)),
        }
    }

    pub fn now_ms(&self) -> i64 {
        self.now.load(Ordering::SeqCst)
    }

    pub fn advance_ms(&self, ms: i64) {
        self.now.fetch_add(ms, Ordering::SeqCst);
    }

    pub fn set_ms(&self, ms: i64) {
        self.now.store(ms, Ordering::SeqCst);
    }
}

pub fn fake_clock(ms: i64) -> FakeClock {
    FakeClock::new(ms)
}

impl maya_mini::schedule::Clock for FakeClock {
    fn now_ms(&self) -> i64 {
        self.now.load(Ordering::SeqCst)
    }
}

use futures_util::future::BoxFuture;
use maya_mini::schedule::{Notice, NotificationReceipt, NotificationStatusKind, Notifier};
use std::sync::Mutex;

#[derive(Clone, Default)]
pub struct FakeNotifier {
    pub notices: Arc<Mutex<Vec<Notice>>>,
    pub fail_next: Arc<Mutex<bool>>,
    pub crash_after_attempt: Arc<Mutex<bool>>,
}

impl FakeNotifier {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Notifier for FakeNotifier {
    fn notify<'a>(
        &'a self,
        notice: Notice,
    ) -> BoxFuture<'a, maya_mini::error::Result<NotificationReceipt>> {
        Box::pin(async move {
            if *self.fail_next.lock().unwrap() {
                return Ok(NotificationReceipt {
                    status: NotificationStatusKind::Failed,
                    desktop_id: None,
                });
            }
            if *self.crash_after_attempt.lock().unwrap() {
                return Err(maya_mini::error::AppError::new(
                    maya_mini::error::ErrorCode::Internal,
                    "Simulated crash after attempt",
                ));
            }
            self.notices.lock().unwrap().push(notice);
            Ok(NotificationReceipt {
                status: NotificationStatusKind::Accepted,
                desktop_id: Some(42),
            })
        })
    }
}
