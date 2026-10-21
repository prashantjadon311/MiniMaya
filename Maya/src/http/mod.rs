pub mod events;
pub mod routes;

use crate::error::{AppError, ErrorCode, Result};
use crate::protocol::{ApiEnvelope, ApiErrorEnvelope, ApiMeta};
use crate::state::AppState;
use crate::types::Digest;
use axum::body::{Body, to_bytes};
use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, patch, post, put};
use axum::{Json, Router};
use rusqlite::params;
use serde::Serialize;
use serde::de::{DeserializeSeed, Error as _, MapAccess, SeqAccess, Visitor};
use std::collections::HashSet;
use std::fmt;
use std::sync::Arc;
use tower_http::set_header::SetResponseHeaderLayer;
use uuid::Uuid;

pub const API_VERSION: &str = "v1";

pub struct ApiResponse<T> {
    status: StatusCode,
    data: T,
    request_id: Uuid,
}

impl<T> ApiResponse<T> {
    pub fn ok(data: T) -> Self {
        Self {
            status: StatusCode::OK,
            data,
            request_id: Uuid::new_v4(),
        }
    }

    pub fn created(data: T) -> Self {
        Self {
            status: StatusCode::CREATED,
            data,
            request_id: Uuid::new_v4(),
        }
    }

    pub fn accepted(data: T) -> Self {
        Self {
            status: StatusCode::ACCEPTED,
            data,
            request_id: Uuid::new_v4(),
        }
    }
}

impl<T: Serialize> IntoResponse for ApiResponse<T> {
    fn into_response(self) -> Response {
        let envelope = ApiEnvelope {
            data: self.data,
            meta: ApiMeta::new(self.request_id),
        };
        (self.status, Json(envelope)).into_response()
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let request_id = self.request_id.unwrap_or_else(Uuid::new_v4);
        let status = self.status();
        let envelope = ApiErrorEnvelope {
            error: self.public_error(),
            meta: ApiMeta::new(request_id),
        };
        let mut response = (status, Json(envelope)).into_response();
        if status == StatusCode::TOO_MANY_REQUESTS {
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from_static("1"));
        }
        response
    }
}

pub fn router(state: Arc<AppState>) -> Router {
    let public = Router::new()
        .route("/health", get(routes::health))
        .route("/session", post(routes::session_exchange));

    let protected = Router::new()
        .route("/session", delete(routes::session_revoke))
        .route(
            "/settings",
            get(routes::settings_get).patch(routes::settings_patch),
        )
        .route("/capabilities", get(routes::capabilities_get))
        .route("/capabilities/probe", post(routes::capabilities_probe))
        .route(
            "/providers",
            get(routes::providers_list).post(routes::providers_create),
        )
        .route(
            "/providers/{id}",
            patch(routes::providers_patch).delete(routes::providers_disable),
        )
        .route(
            "/providers/{id}/secret",
            post(routes::secret_write).delete(routes::secret_remove),
        )
        .route("/providers/{id}/test", post(routes::provider_test))
        .route(
            "/models",
            get(routes::models_list).post(routes::models_create),
        )
        .route(
            "/models/{id}",
            patch(routes::models_patch).delete(routes::models_disable),
        )
        .route(
            "/model-roles",
            get(routes::roles_get).put(routes::roles_put),
        )
        .route(
            "/chains",
            get(routes::chains_list).post(routes::chains_create),
        )
        .route(
            "/chains/{id}",
            put(routes::chains_put).delete(routes::chains_disable),
        )
        .route("/tools", get(routes::tools_list))
        .route("/tools/{id}", put(routes::tools_put))
        .route("/tools/{id}/probe", post(routes::tools_probe))
        .route(
            "/projects",
            get(routes::projects_list).post(routes::projects_create),
        )
        .route(
            "/projects/{id}",
            get(routes::projects_get)
                .patch(routes::projects_patch)
                .delete(routes::projects_archive),
        )
        .route("/projects/{id}/scan", post(routes::projects_scan))
        .route("/projects/{id}/files", get(routes::files_find))
        .route("/projects/{id}/file", get(routes::file_preview))
        .route(
            "/commands",
            get(routes::commands_list).post(routes::commands_create),
        )
        .route("/commands/{id}", delete(routes::commands_disable))
        .route("/tasks", get(routes::tasks_list).post(routes::tasks_create))
        .route(
            "/tasks/{id}",
            get(routes::tasks_get).patch(routes::tasks_patch),
        )
        .route(
            "/tasks/{id}/events",
            get(routes::task_events).post(routes::task_event_create),
        )
        .route(
            "/tasks/{id}/checkins",
            get(routes::checkins_list).post(routes::checkin_reply),
        )
        .route(
            "/reminders",
            get(routes::schedules_list).post(routes::schedules_create),
        )
        .route(
            "/reminders/{id}",
            patch(routes::schedules_patch).delete(routes::schedules_disable),
        )
        .route("/planner", get(routes::planner_get))
        .route("/reports/daily", get(routes::reports_daily))
        .route("/reports/weekly", get(routes::reports_weekly))
        .route(
            "/conversations",
            get(routes::conversations_list).post(routes::conversations_create),
        )
        .route("/conversations/{id}", patch(routes::conversations_patch))
        .route(
            "/conversations/{id}/messages",
            get(routes::messages_list).post(routes::message_send),
        )
        .route(
            "/memory",
            get(routes::memory_search).post(routes::memory_create),
        )
        .route("/memory/{id}", patch(routes::memory_patch))
        .route(
            "/memory/delete-request",
            post(routes::memory_delete_preview),
        )
        .route("/memory/export", post(routes::memory_export))
        .route("/jobs", get(routes::jobs_list).post(routes::jobs_create))
        .route("/jobs/{id}", get(routes::jobs_get))
        .route("/jobs/{id}/plan", put(routes::plan_revise))
        .route(
            "/jobs/{id}/approval-requests",
            post(routes::approval_request),
        )
        .route("/approvals/{id}/decision", post(routes::approval_decide))
        .route("/jobs/{id}/actions", post(routes::job_control))
        .route("/jobs/{id}/events", get(events::stream_job))
        .route("/events", get(events::stream_global))
        .route("/artifacts/{id}", get(routes::artifact_get))
        .route("/actions", post(routes::operation_create))
        .route("/browser/tabs", get(routes::browser_tabs))
        .route("/browser/actions", post(routes::browser_action_create))
        .route("/handoffs", post(routes::handoff_create))
        .route(
            "/voice/transcriptions",
            post(routes::voice_transcribe).layer(DefaultBodyLimit::max(8 * 1024 * 1024)),
        )
        .route("/voice/speech", post(routes::voice_synthesize))
        .route("/system/resources", get(routes::resources_get))
        .route("/audit", get(routes::audit_list))
        .route("/usage", get(routes::usage_list))
        .route(
            "/backups",
            get(routes::backups_list).post(routes::backup_create),
        )
        .route_layer(middleware::from_fn_with_state(state.clone(), protect));

    Router::new()
        .route("/", get(index))
        .route("/web/style.css", get(style))
        .route("/web/js/{name}", get(script))
        .nest("/api/v1", public.merge(protected))
        .layer(DefaultBodyLimit::max(256 * 1024))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static("default-src 'self'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self' data:; media-src 'self' blob:; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'self'"),
        ))
        .with_state(state)
}

async fn protect(
    State(state): State<Arc<AppState>>,
    mut request: Request,
    next: Next,
) -> Result<Response> {
    validate_host_and_origin(&state, &request)?;
    let owner = state.auth.authenticate(request.headers()).await?;
    if is_mutation(request.method()) {
        state.auth.csrf(&owner, request.headers())?;
        state.auth.admit_mutation(owner.id).await?;
        validate_mutation_headers(&request)?;
        let key = request
            .headers()
            .get("idempotency-key")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let method = request.method().to_string();
        let route = request.uri().path().to_string();
        let (parts, body) = request.into_parts();
        let max = if route == "/api/v1/voice/transcriptions" {
            8 * 1024 * 1024
        } else {
            256 * 1024
        };
        let bytes = to_bytes(body, max).await.map_err(|_| {
            AppError::new(
                ErrorCode::BodyTooLarge,
                "Request body exceeds the allowed size",
            )
        })?;
        let digest = request_digest(
            &method,
            &route,
            &bytes,
            parts
                .headers
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
        )?;
        if let Some(response) = begin_idempotency(&state, owner.id, &key, &digest).await? {
            return Ok(response);
        }
        request = Request::from_parts(parts, Body::from(bytes));
        request.extensions_mut().insert(owner.clone());
        let response = next.run(request).await;
        return finish_idempotency(&state, owner.id, &key, &digest, response).await;
    }
    request.extensions_mut().insert(owner);
    Ok(next.run(request).await)
}

async fn begin_idempotency(
    state: &AppState,
    session_id: Uuid,
    key: &str,
    digest: &Digest,
) -> Result<Option<Response>> {
    let sid = session_id.to_string();
    let key = key.to_string();
    let hash = digest.to_hex();
    let current = chrono::Utc::now().timestamp_millis();
    state.db.write(move|tx|{
        let existing=tx.query_row("SELECT request_digest,state,response_json,status_code FROM idempotency_keys WHERE session_id=?1 AND key=?2",params![sid,key],|row|Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,Option<String>>(2)?,row.get::<_,Option<u16>>(3)?))).ok();
        if let Some((stored,state,response,status))=existing{
            if stored!=hash{return Err(AppError::new(ErrorCode::Conflict,"Idempotency key was reused for a different request"));}
            return match(state.as_str(),response,status){("complete",Some(body),Some(code))=>Ok(Some(Response::builder().status(code).header(header::CONTENT_TYPE,"application/json").header(header::CACHE_CONTROL,"no-store").body(Body::from(body)).map_err(|error|AppError::new(ErrorCode::Internal,error.to_string()))?)),("pending",_,_)=>Err(AppError::new(ErrorCode::Conflict,"An identical request is still in progress")),_=>Err(AppError::new(ErrorCode::NeedsReconciliation,"The prior request outcome is uncertain"))};
        }
        tx.execute("INSERT INTO idempotency_keys(session_id,key,request_digest,state,response_json,status_code,resource_id,created_at,expires_at)VALUES(?1,?2,?3,'pending',NULL,NULL,NULL,?4,?5)",params![sid,key,hash,current,current+86_400_000]).map_err(|error|AppError::new(ErrorCode::Conflict,error.to_string()))?;Ok(None)
    }).await
}

async fn finish_idempotency(
    state: &AppState,
    session_id: Uuid,
    key: &str,
    digest: &Digest,
    response: Response,
) -> Result<Response> {
    let mut status = response.status();
    let mut headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .map_err(|_| {
            AppError::new(
                ErrorCode::OutputLimit,
                "Response exceeds idempotency storage limit",
            )
        })?;
    let mut text = String::from_utf8(bytes.to_vec())
        .map_err(|_| AppError::new(ErrorCode::Internal, "JSON response was not UTF-8"))?;
    if serde_json::from_str::<serde_json::Value>(&text).is_err() {
        let error = if status.is_client_error() {
            AppError::new(
                ErrorCode::InvalidRequest,
                "Request body failed schema validation",
            )
        } else {
            AppError::new(
                ErrorCode::IntegrityFailure,
                "Mutation handler returned an invalid response",
            )
        };
        status = error.status();
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        text = serde_json::to_string(&ApiErrorEnvelope {
            error: error.public_error(),
            meta: ApiMeta::new(Uuid::new_v4()),
        })
        .map_err(|encode| AppError::new(ErrorCode::Internal, encode.to_string()))?;
    }
    let sid = session_id.to_string();
    let key = key.to_string();
    let hash = digest.to_hex();
    let stored = text.clone();
    state.db.write(move|tx|{let changed=tx.execute("UPDATE idempotency_keys SET state='complete',response_json=?1,status_code=?2 WHERE session_id=?3 AND key=?4 AND request_digest=?5 AND state='pending'",params![stored,status.as_u16(),sid,key,hash]).map_err(|error|AppError::new(ErrorCode::DbUnavailable,error.to_string()))?;if changed!=1{return Err(AppError::new(ErrorCode::NeedsReconciliation,"Idempotency record changed unexpectedly"));}Ok(())}).await?;
    let mut builder = Response::builder().status(status);
    if let Some(value) = headers.get(header::CONTENT_TYPE) {
        builder = builder.header(header::CONTENT_TYPE, value);
    }
    builder
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from(text))
        .map_err(|error| AppError::new(ErrorCode::Internal, error.to_string()))
}

fn request_digest(
    method: &str,
    route: &str,
    body: &[u8],
    content_type: Option<&str>,
) -> Result<Digest> {
    let normalized =
        if content_type.is_some_and(|value| value.eq_ignore_ascii_case("application/json")) {
            reject_duplicate_json_keys(body)?;
            let value: serde_json::Value = serde_json::from_slice(body).map_err(|_| {
                AppError::new(ErrorCode::InvalidRequest, "Request body is not valid JSON")
            })?;
            serde_json::to_vec(&value)
                .map_err(|error| AppError::new(ErrorCode::Internal, error.to_string()))?
        } else {
            body.to_vec()
        };
    let mut bytes = Vec::with_capacity(method.len() + route.len() + normalized.len() + 2);
    bytes.extend_from_slice(method.as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(route.as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(&normalized);
    Ok(Digest::of(&bytes))
}

struct UniqueJson;

impl<'de> DeserializeSeed<'de> for UniqueJson {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> std::result::Result<(), D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(UniqueJsonVisitor)
    }
}

struct UniqueJsonVisitor;

impl<'de> Visitor<'de> for UniqueJsonVisitor {
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON value with unique object keys")
    }

    fn visit_map<A>(self, mut map: A) -> std::result::Result<(), A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut keys = HashSet::new();
        while let Some(key) = map.next_key::<String>()? {
            if !keys.insert(key.clone()) {
                return Err(A::Error::custom(format!("duplicate object key: {key}")));
            }
            map.next_value_seed(UniqueJson)?;
        }
        Ok(())
    }

    fn visit_seq<A>(self, mut sequence: A) -> std::result::Result<(), A::Error>
    where
        A: SeqAccess<'de>,
    {
        while sequence.next_element_seed(UniqueJson)?.is_some() {}
        Ok(())
    }

    fn visit_bool<E>(self, _: bool) -> std::result::Result<(), E> {
        Ok(())
    }
    fn visit_i64<E>(self, _: i64) -> std::result::Result<(), E> {
        Ok(())
    }
    fn visit_u64<E>(self, _: u64) -> std::result::Result<(), E> {
        Ok(())
    }
    fn visit_f64<E>(self, _: f64) -> std::result::Result<(), E> {
        Ok(())
    }
    fn visit_str<E>(self, _: &str) -> std::result::Result<(), E> {
        Ok(())
    }
    fn visit_string<E>(self, _: String) -> std::result::Result<(), E> {
        Ok(())
    }
    fn visit_none<E>(self) -> std::result::Result<(), E> {
        Ok(())
    }
    fn visit_some<D>(self, deserializer: D) -> std::result::Result<(), D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        UniqueJson.deserialize(deserializer)
    }
    fn visit_unit<E>(self) -> std::result::Result<(), E> {
        Ok(())
    }
}

fn reject_duplicate_json_keys(bytes: &[u8]) -> Result<()> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    UniqueJson.deserialize(&mut deserializer).map_err(|_| {
        AppError::new(
            ErrorCode::InvalidRequest,
            "Request body contains invalid JSON or duplicate object keys",
        )
    })?;
    deserializer.end().map_err(|_| {
        AppError::new(
            ErrorCode::InvalidRequest,
            "Request body contains trailing JSON data",
        )
    })
}

fn validate_host_and_origin(state: &AppState, request: &Request) -> Result<()> {
    let expected = format!("127.0.0.1:{}", state.auth.port());
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok());
    if host != Some(expected.as_str()) {
        return Err(AppError::new(
            ErrorCode::OriginDenied,
            "Host is not the Maya loopback origin",
        ));
    }
    let expected_origin = format!("http://{expected}");
    let origin = request
        .headers()
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok());
    if let Some(value) = origin {
        if value == "null" || value != expected_origin {
            return Err(AppError::new(
                ErrorCode::OriginDenied,
                "Request origin is not allowed",
            ));
        }
    } else if is_mutation(request.method()) {
        return Err(AppError::new(
            ErrorCode::OriginDenied,
            "Mutation requires an Origin header",
        ));
    }
    if let Some(site) = request
        .headers()
        .get("sec-fetch-site")
        .and_then(|value| value.to_str().ok())
        && !matches!(site, "same-origin" | "none")
    {
        return Err(AppError::new(
            ErrorCode::OriginDenied,
            "Cross-site request denied",
        ));
    }
    Ok(())
}

fn validate_mutation_headers(request: &Request) -> Result<()> {
    let key = request
        .headers()
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| {
            AppError::new(ErrorCode::InvalidRequest, "Missing Idempotency-Key header")
        })?;
    if key.len() < 16
        || key.len() > 128
        || !key
            .bytes()
            .all(|byte| byte.is_ascii_graphic() || byte == b' ')
    {
        return Err(AppError::new(
            ErrorCode::InvalidRequest,
            "Invalid Idempotency-Key header",
        ));
    }
    let path = request.uri().path();
    let multipart = path == "/api/v1/voice/transcriptions";
    let content_type = request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if multipart {
        if !content_type.starts_with("multipart/form-data;") {
            return Err(AppError::new(
                ErrorCode::UnsupportedMedia,
                "Voice upload must use multipart/form-data",
            ));
        }
    } else if !content_type.eq_ignore_ascii_case("application/json") {
        return Err(AppError::new(
            ErrorCode::UnsupportedMedia,
            "Mutation body must be application/json",
        ));
    }
    Ok(())
}

fn is_mutation(method: &Method) -> bool {
    matches!(
        *method,
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    )
}

async fn index() -> Response {
    static_response("/", include_str!("../../web/index.html"))
}
async fn style() -> Response {
    static_response("/web/style.css", include_str!("../../web/style.css"))
}
async fn script(axum::extract::Path(name): axum::extract::Path<String>) -> Response {
    let body = match name.as_str() {
        "api.js" => include_str!("../../web/js/api.js"),
        "dom.js" => include_str!("../../web/js/dom.js"),
        "voice.js" => include_str!("../../web/js/voice.js"),
        "chat.js" => include_str!("../../web/js/chat.js"),
        "tasks.js" => include_str!("../../web/js/tasks.js"),
        "activity.js" => include_str!("../../web/js/activity.js"),
        "workspace.js" => include_str!("../../web/js/workspace.js"),
        "app.js" => include_str!("../../web/js/app.js"),
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    static_response(".js", body)
}

pub fn static_asset(path: &str) -> Result<Response> {
    let body = match path {
        "/" | "/index.html" => include_str!("../../web/index.html"),
        "/web/style.css" => include_str!("../../web/style.css"),
        "/web/js/api.js" => include_str!("../../web/js/api.js"),
        "/web/js/dom.js" => include_str!("../../web/js/dom.js"),
        "/web/js/voice.js" => include_str!("../../web/js/voice.js"),
        "/web/js/chat.js" => include_str!("../../web/js/chat.js"),
        "/web/js/tasks.js" => include_str!("../../web/js/tasks.js"),
        "/web/js/activity.js" => include_str!("../../web/js/activity.js"),
        "/web/js/workspace.js" => include_str!("../../web/js/workspace.js"),
        "/web/js/app.js" => include_str!("../../web/js/app.js"),
        _ => return Err(AppError::new(ErrorCode::NotFound, "Static asset not found")),
    };
    Ok(static_response(path, body))
}

fn static_response(path: &str, body: &'static str) -> Response {
    let content_type = if path.ends_with(".css") {
        "text/css; charset=utf-8"
    } else if path.ends_with(".js") {
        "text/javascript; charset=utf-8"
    } else {
        "text/html; charset=utf-8"
    };
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from(body))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}
