#![allow(dead_code)]

use axum::body::Body;
use http_body_util::BodyExt;
use maya_mini::db::Db;
use maya_mini::{app::App, config::RuntimeConfig, http};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use tempfile::TempDir;
use tower::ServiceExt;

pub fn runtime_config(root: &std::path::Path) -> RuntimeConfig {
    RuntimeConfig {
        data_dir: root.join("data"),
        cache_dir: root.join("cache"),
        runtime_dir: root.join("runtime"),
        port: 0,
        open_browser: false,
        headless: true,
        print_launch_link: false,
        test_mode: true,
    }
}

pub async fn test_app() -> (TempDir, App) {
    let dir = tempfile::tempdir().expect("temp app dir");
    let app = App::build(runtime_config(dir.path()))
        .await
        .expect("application builds");
    (dir, app)
}

#[derive(Clone)]
pub struct AuthHarness {
    pub router: axum::Router,
    pub host: String,
    pub origin: String,
    pub bearer: String,
    pub csrf: String,
}

impl AuthHarness {
    pub async fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> (axum::http::StatusCode, Value) {
        self.request_with_headers(method, path, body, &[]).await
    }

    pub async fn request_with_headers(
        &self,
        method: &str,
        path: &str,
        body: Option<Value>,
        headers: &[(&str, &str)],
    ) -> (axum::http::StatusCode, Value) {
        let mut builder = axum::http::Request::builder()
            .method(method)
            .uri(format!("/api/v1{path}"))
            .header("host", &self.host)
            .header("authorization", format!("Bearer {}", self.bearer));
        if !matches!(method, "GET" | "HEAD") {
            builder = builder
                .header("origin", &self.origin)
                .header("x-csrf-token", &self.csrf)
                .header("idempotency-key", uuid::Uuid::new_v4().to_string())
                .header("content-type", "application/json");
        }
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        let response = self
            .router
            .clone()
            .oneshot(
                builder
                    .body(body.map_or_else(Body::empty, |value| Body::from(value.to_string())))
                    .expect("request builds"),
            )
            .await
            .expect("request completes");
        let status = response.status();
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("response body")
            .to_bytes();
        let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, value)
    }
}

pub async fn authenticated(app: &App) -> AuthHarness {
    let port = app.state.auth.port();
    let host = format!("127.0.0.1:{port}");
    let origin = format!("http://{host}");
    let router = http::router(app.state.clone());
    let launch = app
        .state
        .auth
        .issue_launch_link(port)
        .await
        .expect("launch link");
    let bootstrap = launch
        .url
        .expose_str()
        .split("#bootstrap=")
        .nth(1)
        .expect("bootstrap token");
    let response = router
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/api/v1/session")
                .header("host", &host)
                .header("origin", &origin)
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"bootstrap_token":bootstrap}).to_string(),
                ))
                .expect("session request"),
        )
        .await
        .expect("session exchange");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let value: Value = serde_json::from_slice(
        &response
            .into_body()
            .collect()
            .await
            .expect("session body")
            .to_bytes(),
    )
    .expect("session JSON");
    AuthHarness {
        router,
        host,
        origin,
        bearer: value["data"]["session_token"]
            .as_str()
            .expect("session token")
            .to_string(),
        csrf: value["data"]["csrf_token"]
            .as_str()
            .expect("CSRF token")
            .to_string(),
    }
}

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
