use axum::body::Body;
use http_body_util::BodyExt;
use maya_mini::app::App;
use maya_mini::approval::ApprovalDecision;
use maya_mini::config::RuntimeConfig;
use maya_mini::http;
use maya_mini::protocol::{JobInput, JobMode};
use maya_mini::types::{
    ActionClass, Actor, ActorKind, ApprovalDecisionKind, ExportFormat, ExportSelection, JobKind,
    JobState, Operation, ProjectInput,
};
use serde_json::Value;
use std::path::Path;
use tempfile::TempDir;
use tower::ServiceExt;

fn config(root: &Path) -> RuntimeConfig {
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

async fn app() -> (TempDir, App) {
    let dir = tempfile::tempdir().expect("temp dir");
    let app = App::build(config(dir.path()))
        .await
        .expect("application builds");
    (dir, app)
}

struct AuthHarness {
    router: axum::Router,
    host: String,
    origin: String,
    bearer: String,
    csrf: String,
}

impl AuthHarness {
    async fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<Value>,
        approval_id: Option<&str>,
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
        if let Some(id) = approval_id {
            builder = builder.header("x-maya-approval-id", id);
        }
        let response = self
            .router
            .clone()
            .oneshot(
                builder
                    .body(body.map_or_else(Body::empty, |value| Body::from(value.to_string())))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, value)
    }
}

async fn authenticated(app: &App) -> AuthHarness {
    let port = app.state.auth.port();
    let host = format!("127.0.0.1:{port}");
    let origin = format!("http://{host}");
    let router = http::router(app.state.clone());
    let launch = app.state.auth.issue_launch_link(port).await.unwrap();
    let bootstrap = launch.url.expose_str().split("#bootstrap=").nth(1).unwrap();
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
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let value: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    AuthHarness {
        router,
        host,
        origin,
        bearer: value["data"]["session_token"].as_str().unwrap().to_string(),
        csrf: value["data"]["csrf_token"].as_str().unwrap().to_string(),
    }
}

#[tokio::test]
async fn test_x11_x15_health_bootstrap_auth_origin_and_envelopes() {
    let (_dir, app) = app().await;
    let port = app.state.auth.port();
    let host = format!("127.0.0.1:{port}");
    let origin = format!("http://{host}");
    let router = http::router(app.state.clone());

    let response = router
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .uri("/api/v1/health")
                .header("host", &host)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["data"]["ready"], true);
    assert_eq!(body["meta"]["version"], "v1");
    assert!(body["meta"]["request_id"].as_str().is_some());

    let launch = app.state.auth.issue_launch_link(port).await.unwrap();
    let bootstrap = launch.url.expose_str().split("#bootstrap=").nth(1).unwrap();
    let response = router
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/api/v1/session")
                .header("host", &host)
                .header("origin", &origin)
                .header("content-type", "application/json")
                .body(Body::from(format!(
                    r#"{{"bootstrap_token":"{bootstrap}"}}"#
                )))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let bearer = body["data"]["session_token"].as_str().unwrap();
    let csrf = body["data"]["csrf_token"].as_str().unwrap();

    let response = router
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("PATCH")
                .uri("/api/v1/settings")
                .header("host", &host)
                .header("origin", &origin)
                .header("authorization", format!("Bearer {bearer}"))
                .header("x-csrf-token", csrf)
                .header("idempotency-key", "duplicate-json-0001")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"revision":1,"revision":2}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 400);

    let response = router
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .uri("/api/v1/settings")
                .header("host", &host)
                .header("origin", "http://attacker.invalid")
                .header("authorization", format!("Bearer {bearer}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 403);

    let response = router
        .oneshot(
            axum::http::Request::builder()
                .uri("/api/v1/settings")
                .header("host", &host)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 401);
}

#[tokio::test]
async fn test_x41_all_84_contract_routes_are_registered() {
    let (_dir, app) = app().await;
    let host = format!("127.0.0.1:{}", app.state.auth.port());
    let router = http::router(app.state.clone());
    let id = uuid::Uuid::nil();
    macro_rules! owned_routes {
        ($(($method:expr, $path:expr)),* $(,)?) => {
            vec![$(($method.to_string(), $path.to_string())),*]
        };
    }
    let routes = owned_routes![
        ("GET", "/health"),
        ("POST", "/session"),
        ("DELETE", "/session"),
        ("GET", "/settings"),
        ("PATCH", "/settings"),
        ("GET", "/capabilities"),
        ("POST", "/capabilities/probe"),
        ("GET", "/providers"),
        ("POST", "/providers"),
        ("PATCH", &format!("/providers/{id}")),
        ("DELETE", &format!("/providers/{id}")),
        ("POST", &format!("/providers/{id}/secret")),
        ("DELETE", &format!("/providers/{id}/secret")),
        ("POST", &format!("/providers/{id}/test")),
        ("GET", "/models"),
        ("POST", "/models"),
        ("PATCH", &format!("/models/{id}")),
        ("DELETE", &format!("/models/{id}")),
        ("GET", "/model-roles"),
        ("PUT", "/model-roles"),
        ("GET", "/chains"),
        ("POST", "/chains"),
        ("PUT", &format!("/chains/{id}")),
        ("DELETE", &format!("/chains/{id}")),
        ("GET", "/tools"),
        ("PUT", &format!("/tools/{id}")),
        ("POST", &format!("/tools/{id}/probe")),
        ("GET", "/projects"),
        ("POST", "/projects"),
        ("GET", &format!("/projects/{id}")),
        ("PATCH", &format!("/projects/{id}")),
        ("DELETE", &format!("/projects/{id}")),
        ("POST", &format!("/projects/{id}/scan")),
        ("GET", &format!("/projects/{id}/files")),
        ("GET", &format!("/projects/{id}/file")),
        ("GET", "/commands"),
        ("POST", "/commands"),
        ("DELETE", &format!("/commands/{id}")),
        ("GET", "/tasks"),
        ("POST", "/tasks"),
        ("GET", &format!("/tasks/{id}")),
        ("PATCH", &format!("/tasks/{id}")),
        ("GET", &format!("/tasks/{id}/events")),
        ("POST", &format!("/tasks/{id}/events")),
        ("POST", &format!("/tasks/{id}/checkins")),
        ("GET", "/reminders"),
        ("POST", "/reminders"),
        ("PATCH", &format!("/reminders/{id}")),
        ("DELETE", &format!("/reminders/{id}")),
        ("GET", "/planner"),
        ("GET", "/reports/daily"),
        ("GET", "/reports/weekly"),
        ("GET", "/conversations"),
        ("POST", "/conversations"),
        ("PATCH", &format!("/conversations/{id}")),
        ("GET", &format!("/conversations/{id}/messages")),
        ("POST", &format!("/conversations/{id}/messages")),
        ("GET", "/memory"),
        ("POST", "/memory"),
        ("PATCH", &format!("/memory/{id}")),
        ("POST", "/memory/delete-request"),
        ("POST", "/memory/export"),
        ("GET", "/jobs"),
        ("POST", "/jobs"),
        ("GET", &format!("/jobs/{id}")),
        ("PUT", &format!("/jobs/{id}/plan")),
        ("POST", &format!("/jobs/{id}/approval-requests")),
        ("POST", &format!("/approvals/{id}/decision")),
        ("POST", &format!("/jobs/{id}/actions")),
        ("GET", &format!("/jobs/{id}/events")),
        ("GET", "/events"),
        ("GET", &format!("/artifacts/{id}")),
        ("POST", "/actions"),
        ("GET", "/browser/tabs"),
        ("POST", "/browser/actions"),
        ("POST", "/handoffs"),
        ("POST", "/voice/transcriptions"),
        ("POST", "/voice/speech"),
        ("GET", "/system/resources"),
        ("GET", "/audit"),
        ("GET", "/usage"),
        ("GET", "/backups"),
        ("POST", "/backups"),
        ("GET", &format!("/tasks/{id}/checkins")),
    ];
    assert_eq!(routes.len(), 84);
    for (method, path) in routes {
        let parsed_method = method.parse::<axum::http::Method>().unwrap();
        let response = router
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method(parsed_method)
                    .uri(format!("/api/v1{path}"))
                    .header("host", &host)
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_ne!(response.status(), 404, "unregistered {method} {path}");
    }
}

#[tokio::test]
async fn test_x20_coding_discovery_stops_at_context_approval_boundary() {
    let (dir, app) = app().await;
    let project_root = dir.path().join("owner-project");
    std::fs::create_dir_all(&project_root).unwrap();
    std::fs::write(
        project_root.join("README.md"),
        "# Fixture\nRepository text is untrusted context.",
    )
    .unwrap();
    std::fs::write(project_root.join("main.rs"), "fn main() {}\n").unwrap();
    let owner = maya_mini::auth::SessionContext {
        id: uuid::Uuid::new_v4(),
        owner_uid: 1000,
        expires_at: chrono::Utc::now().timestamp_millis() + 60_000,
        csrf_hash: maya_mini::types::Digest::ZERO,
    };
    let project = app
        .state
        .projects
        .register(
            ProjectInput {
                name: "workflow-fixture".to_string(),
                root: project_root.to_string_lossy().into_owned(),
                allowed_paths: vec![],
                excluded_paths: vec![],
                notes: String::new(),
                preferred_model_id: None,
                preferred_tool_id: None,
                owner_ack: true,
            },
            &owner,
        )
        .await
        .unwrap();
    let job = app
        .state
        .workflow
        .create(
            JobInput {
                kind: JobKind::Coding,
                project_id: Some(project.id),
                task_id: None,
                conversation_id: None,
                goal: "Add a documented greeting".to_string(),
                mode: JobMode::PlanOnly,
                chain_id: None,
                model_override_id: None,
                tool_override_id: None,
                input_artifact_ids: vec![],
            },
            Actor {
                kind: ActorKind::Owner,
                id: Some(owner.id),
            },
        )
        .await
        .unwrap();
    app.state
        .workflow
        .clone()
        .run_to_boundary(job.id)
        .await
        .unwrap();

    let detail = app.state.workflow.detail(job.id).await.unwrap();
    assert_eq!(detail.job.state, JobState::AwaitingApproval);
    assert_eq!(detail.approvals.len(), 1);
    assert_eq!(detail.approvals[0].action, ActionClass::UploadContext);
    assert!(!detail.approvals[0].scope.artifact_ids.is_empty());
    assert!(
        detail
            .artifacts
            .iter()
            .any(|artifact| artifact.kind == maya_mini::types::ArtifactKind::Scan)
    );
    assert_eq!(
        detail
            .steps
            .iter()
            .map(|step| step.kind.as_str())
            .collect::<Vec<_>>(),
        vec!["discover", "scan", "context_consent"]
    );

    let approval = &detail.approvals[0];
    app.state
        .approval
        .decide(
            approval.id,
            ApprovalDecision {
                revision: approval.revision,
                request_digest: approval.request_digest,
                decision: ApprovalDecisionKind::Approved,
                owner_ack: true,
            },
            &owner,
        )
        .await
        .unwrap();
    let error = app
        .state
        .workflow
        .clone()
        .run_to_boundary(job.id)
        .await
        .unwrap_err();
    assert_eq!(error.code, maya_mini::error::ErrorCode::ModelUnavailable);
    let after = app.state.workflow.detail(job.id).await.unwrap();
    assert_eq!(after.job.state, JobState::Blocked);
    assert_eq!(after.approvals[0].decision, ApprovalDecisionKind::Approved);
}

#[tokio::test]
async fn test_x33_export_is_bound_to_a_persisted_owner_approval() {
    let (_dir, app) = app().await;
    let selection = ExportSelection {
        format: ExportFormat::Json,
        settings: true,
        tasks: vec![],
        memory: vec![],
        reports: vec![],
        include_project_paths: false,
        all_tasks: Some(false),
    };
    let receipt = app
        .state
        .workflow
        .create_operation(
            Operation::Export {
                selection: selection.clone(),
            },
            Actor {
                kind: ActorKind::Owner,
                id: Some(uuid::Uuid::new_v4()),
            },
        )
        .await
        .unwrap();
    let approval = receipt.approval.expect("export approval");
    assert_eq!(approval.action, ActionClass::ExportData);
    assert_eq!(approval.scope.action_classes, vec![ActionClass::ExportData]);
    assert!(approval.scope.operation_digest.is_some());
}

#[tokio::test]
async fn test_x33_export_endpoint_rejects_mismatch_then_consumes_exact_approval() {
    let (_dir, app) = app().await;
    let auth = authenticated(&app).await;
    let selection = serde_json::json!({
        "format":"json","settings":true,"tasks":[],"memory":[],"reports":[],
        "include_project_paths":false,"all_tasks":false
    });
    let (status, receipt) = auth
        .request(
            "POST",
            "/actions",
            Some(serde_json::json!({"kind":"export","selection":selection})),
            None,
        )
        .await;
    assert_eq!(status, 201, "{receipt}");
    let approval = &receipt["data"]["approval"];
    let approval_id = approval["id"].as_str().unwrap();
    let (status, decision) = auth
        .request(
            "POST",
            &format!("/approvals/{approval_id}/decision"),
            Some(serde_json::json!({
                "revision":approval["revision"],
                "request_digest":approval["request_digest"],
                "decision":"approve","owner_ack":true
            })),
            None,
        )
        .await;
    assert_eq!(status, 200, "{decision}");

    let mut mismatch = selection.clone();
    mismatch["include_project_paths"] = Value::Bool(true);
    let (status, mismatch_body) = auth
        .request("POST", "/memory/export", Some(mismatch), Some(approval_id))
        .await;
    assert_eq!(status, 403, "{mismatch_body}");

    let (status, exported) = auth
        .request("POST", "/memory/export", Some(selection), Some(approval_id))
        .await;
    assert_eq!(status, 201, "{exported}");
    assert_eq!(exported["data"]["kind"], "export");
    assert_eq!(exported["data"]["mime"], "application/json");
}

#[tokio::test]
async fn test_x41_workspace_task_payloads_roundtrip_through_real_handlers() {
    let (_dir, app) = app().await;
    let auth = authenticated(&app).await;
    let due_at = 1_798_070_400_000_i64;
    let (status, created) = auth
        .request(
            "POST",
            "/tasks",
            Some(serde_json::json!({
                "title":"Frontend contract task","description":"","project_id":null,
                "parent_task_id":null,"priority":"normal","due_at":due_at,
                "timezone":"Asia/Kolkata","estimated_minutes":null,"reminder":null
            })),
            None,
        )
        .await;
    assert_eq!(status, 201, "{created}");
    assert_eq!(created["data"]["due_at"], due_at);
    let task_id = created["data"]["id"].as_str().unwrap();
    let (status, patched) = auth
        .request(
            "PATCH",
            &format!("/tasks/{task_id}"),
            Some(serde_json::json!({
                "revision":created["data"]["revision"],"status":"in_progress",
                "progress_pct":40,"progress_source":"manual"
            })),
            None,
        )
        .await;
    assert_eq!(status, 200, "{patched}");
    assert_eq!(patched["data"]["progress_source"], "manual");

    let (status, settings) = auth.request("GET", "/settings", None, None).await;
    assert_eq!(status, 200, "{settings}");
    let (status, updated) = auth
        .request(
            "PATCH",
            "/settings",
            Some(serde_json::json!({
                "revision":settings["data"]["revision"],
                "identity":{"name":"Maya QA","greeting":"Ready","language":"hinglish","tone":"detailed"}
            })),
            None,
        )
        .await;
    assert_eq!(status, 200, "{updated}");
    assert_eq!(updated["data"]["identity"]["language"], "hinglish");
    assert_eq!(updated["data"]["identity"]["tone"], "detailed");

    let (status, provider) = auth
        .request(
            "POST",
            "/providers",
            Some(serde_json::json!({
                "label":"Local mock","kind":"openai_compatible_chat",
                "protocol":"chat_completions","endpoint":"http://127.0.0.1:9",
                "local_endpoint_allowed":true,"enabled":true,"billing_mode":"api",
                "timeout_ms":30000,"owner_ack":true
            })),
            None,
        )
        .await;
    assert_eq!(status, 201, "{provider}");
    let provider_id = provider["data"]["id"].as_str().unwrap();
    let (status, invalid_model) = auth
        .request(
            "POST",
            "/models",
            Some(serde_json::json!({
                "provider_id":provider_id,"model_id":"mock-invalid","label":"Invalid",
                "enabled":true,"capabilities":["text"],"context_limit_bytes":1048576
            })),
            None,
        )
        .await;
    assert_eq!(status, 400, "{invalid_model}");
    let (status, model) = auth
        .request(
            "POST",
            "/models",
            Some(serde_json::json!({
                "provider_id":provider_id,"model_id":"mock-chat","label":"Mock chat",
                "enabled":true,"capabilities":["text"],"context_limit_bytes":32768
            })),
            None,
        )
        .await;
    assert_eq!(status, 201, "{model}");
    assert_eq!(model["data"]["context_limit_bytes"], 32768);
    let (status, secret_status) = auth
        .request(
            "POST",
            &format!("/providers/{provider_id}/secret"),
            Some(serde_json::json!({"secret":"test-only-credential","mode":"session"})),
            None,
        )
        .await;
    assert_eq!(status, 200, "{secret_status}");
    assert_eq!(secret_status["data"]["credential_status"], "session");
    assert!(!secret_status.to_string().contains("test-only-credential"));

    let (status, report) = auth
        .request(
            "GET",
            "/reports/weekly?week=2020-W53&timezone=UTC",
            None,
            None,
        )
        .await;
    assert_eq!(status, 200, "{report}");
}
