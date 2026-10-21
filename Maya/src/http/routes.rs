use super::ApiResponse;
use crate::approval::{ApprovalDecision, ApprovalRequest};
use crate::artifacts::ArtifactInput;
use crate::audit::AuditQuery;
use crate::auth::SessionContext;
use crate::config::validate_settings;
use crate::error::{AppError, ErrorCode, Result};
use crate::protocol::*;
use crate::schedule::ScheduleWake as SchedulerWake;
use crate::secrets::SecretString;
use crate::state::AppState;
use crate::types::*;
use axum::Extension;
use axum::Json;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::Response;
use chrono::{Datelike, NaiveDate};
use chrono_tz::Tz;
use rusqlite::{Row, params, types::Value};
use std::io::Cursor;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

pub async fn health(State(state): State<Arc<AppState>>) -> Result<ApiResponse<Health>> {
    state
        .db
        .read(|conn| conn.query_row("SELECT 1", [], |_| Ok(())).map_err(db_error))
        .await?;
    Ok(ApiResponse::ok(Health {
        ready: true,
        version: env!("CARGO_PKG_VERSION"),
    }))
}

pub async fn session_exchange(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<SessionExchange>,
) -> Result<ApiResponse<SessionInfo>> {
    let origin = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| {
            AppError::new(
                ErrorCode::OriginDenied,
                "Bootstrap exchange requires Origin",
            )
        })?;
    let grant = state
        .auth
        .exchange(SecretString::new(body.bootstrap_token), origin)
        .await?;
    Ok(ApiResponse::ok(SessionInfo {
        session_id: grant.session_id,
        session_token: grant.session_token,
        csrf_token: grant.csrf_token,
        expires_at: grant.expires_at,
    }))
}

pub async fn session_revoke(
    State(state): State<Arc<AppState>>,
    Extension(owner): Extension<SessionContext>,
    Json(_): Json<Empty>,
) -> Result<ApiResponse<Receipt>> {
    state.auth.revoke(owner.id).await?;
    Ok(ApiResponse::ok(receipt(
        Some(owner.id),
        ReceiptResult::Revoked,
        "Session revoked",
    )))
}

pub async fn settings_get(
    State(state): State<Arc<AppState>>,
    Extension(_): Extension<SessionContext>,
) -> Result<ApiResponse<Settings>> {
    Ok(ApiResponse::ok(load_settings(&state).await?))
}

pub async fn settings_patch(
    State(state): State<Arc<AppState>>,
    Extension(_): Extension<SessionContext>,
    Json(body): Json<SettingsPatch>,
) -> Result<ApiResponse<Settings>> {
    let current = load_settings(&state).await?;
    let next = body.apply(current)?;
    save_settings(&state, &next).await?;
    Ok(ApiResponse::ok(next))
}

pub async fn capabilities_get(
    State(state): State<Arc<AppState>>,
    Extension(_): Extension<SessionContext>,
) -> Result<ApiResponse<Capabilities>> {
    let mut items = vec![state.browser.probe().await?, state.sandbox.probe().await?];
    items.extend(state.voice.capabilities().await?);
    Ok(ApiResponse::ok(Capabilities { items }))
}

pub async fn capabilities_probe(
    State(state): State<Arc<AppState>>,
    Extension(_): Extension<SessionContext>,
    Json(body): Json<ProbeInput>,
) -> Result<ApiResponse<Capabilities>> {
    let items = match body.kind {
        ProbeKind::All => {
            return capabilities_get(
                State(state),
                Extension(SessionContext {
                    id: Uuid::nil(),
                    owner_uid: 0,
                    expires_at: 0,
                    csrf_hash: Digest::ZERO,
                }),
            )
            .await;
        }
        ProbeKind::Browser => vec![state.browser.probe().await?],
        ProbeKind::Voice => state.voice.capabilities().await?,
        ProbeKind::Desktop => vec![capability(
            "desktop",
            CapabilityState::Available,
            Some("Desktop actions require explicit approval"),
        )],
        ProbeKind::Provider => {
            let id = body.id.ok_or_else(|| {
                AppError::new(ErrorCode::InvalidRequest, "Provider probe requires id")
            })?;
            vec![provider_capability(&state, id).await?]
        }
        ProbeKind::Tool => {
            let id = body.id.ok_or_else(|| {
                AppError::new(ErrorCode::InvalidRequest, "Tool probe requires id")
            })?;
            vec![tool_capability(&state, id).await?]
        }
    };
    Ok(ApiResponse::ok(Capabilities { items }))
}

pub async fn providers_list(
    State(state): State<Arc<AppState>>,
    Extension(_): Extension<SessionContext>,
    Query(q): Query<ListQuery>,
) -> Result<ApiResponse<Page<Provider>>> {
    let limit = checked_limit(q.limit)? as usize;
    let mut items = state.providers.list_providers().await?;
    items.truncate(limit);
    Ok(ApiResponse::ok(Page {
        items,
        next_cursor: None,
    }))
}
pub async fn providers_create(
    State(state): State<Arc<AppState>>,
    Extension(_): Extension<SessionContext>,
    Json(body): Json<ProviderInput>,
) -> Result<ApiResponse<Provider>> {
    Ok(ApiResponse::created(
        state.providers.create_provider(body).await?,
    ))
}
pub async fn providers_patch(
    State(state): State<Arc<AppState>>,
    Extension(_): Extension<SessionContext>,
    Path(p): Path<IdPath>,
    Json(body): Json<ProviderPatch>,
) -> Result<ApiResponse<Provider>> {
    Ok(ApiResponse::ok(
        state.providers.patch_provider(p.id, body).await?,
    ))
}
pub async fn providers_disable(
    State(state): State<Arc<AppState>>,
    Extension(_): Extension<SessionContext>,
    Path(p): Path<IdPath>,
    Json(body): Json<ConfirmRevision>,
) -> Result<ApiResponse<Receipt>> {
    require_ack(body.owner_ack)?;
    state
        .providers
        .disable_or_delete_provider(p.id, body.revision)
        .await?;
    Ok(ApiResponse::ok(receipt(
        Some(p.id),
        ReceiptResult::Disabled,
        "Provider disabled",
    )))
}

pub async fn secret_write(
    State(state): State<Arc<AppState>>,
    Extension(_): Extension<SessionContext>,
    Path(p): Path<IdPath>,
    Json(body): Json<SecretWrite>,
) -> Result<ApiResponse<SecretStatus>> {
    if body.secret.is_empty() || body.secret.len() > 8192 {
        return Err(AppError::new(
            ErrorCode::InvalidRequest,
            "Secret must be between 1 and 8192 bytes",
        ));
    }
    state.providers.get_provider(p.id).await?;
    let mode = body.mode;
    let credential_status = state
        .secrets
        .put(p.id, SecretString::new(body.secret), body.mode)
        .await?;
    let reference = match mode {
        SecretMode::Session => format!("session:{}", p.id),
        SecretMode::Keyring => format!("keyring:{}", p.id),
    };
    state.db.write(move|tx|{tx.execute("UPDATE providers SET secret_ref=?1,credential_generation=credential_generation+1,revision=revision+1,updated_at=?2 WHERE id=?3",params![reference,now(),p.id.to_string()]).map_err(db_error)?;Ok(())}).await?;
    Ok(ApiResponse::ok(SecretStatus { credential_status }))
}
pub async fn secret_remove(
    State(state): State<Arc<AppState>>,
    Extension(_): Extension<SessionContext>,
    Path(p): Path<IdPath>,
    Json(body): Json<ConfirmRevision>,
) -> Result<ApiResponse<SecretStatus>> {
    require_ack(body.owner_ack)?;
    let provider = state.providers.get_provider(p.id).await?;
    if provider.revision != body.revision {
        return Err(AppError::new(
            ErrorCode::StaleRevision,
            "Provider revision is stale",
        ));
    }
    state.secrets.remove(p.id).await?;
    state.db.write(move|tx|{tx.execute("UPDATE providers SET secret_ref=NULL,credential_generation=credential_generation+1,revision=revision+1,updated_at=?1 WHERE id=?2",params![now(),p.id.to_string()]).map_err(db_error)?;Ok(())}).await?;
    Ok(ApiResponse::ok(SecretStatus {
        credential_status: CredentialStatus::Missing,
    }))
}
pub async fn provider_test(
    State(state): State<Arc<AppState>>,
    Extension(_): Extension<SessionContext>,
    Path(p): Path<IdPath>,
    Json(_): Json<Empty>,
) -> Result<ApiResponse<Capability>> {
    Ok(ApiResponse::ok(provider_capability(&state, p.id).await?))
}

pub async fn models_list(
    State(state): State<Arc<AppState>>,
    Extension(_): Extension<SessionContext>,
    Query(q): Query<ModelQuery>,
) -> Result<ApiResponse<Page<Model>>> {
    let limit = checked_limit(q.limit)? as usize;
    let mut items = state.providers.list_models(q.provider_id).await?;
    items.truncate(limit);
    Ok(ApiResponse::ok(Page {
        items,
        next_cursor: None,
    }))
}
pub async fn models_create(
    State(state): State<Arc<AppState>>,
    Extension(_): Extension<SessionContext>,
    Json(body): Json<ModelInput>,
) -> Result<ApiResponse<Model>> {
    Ok(ApiResponse::created(
        state.providers.create_model(body).await?,
    ))
}
pub async fn models_patch(
    State(state): State<Arc<AppState>>,
    Extension(_): Extension<SessionContext>,
    Path(p): Path<IdPath>,
    Json(body): Json<ModelPatch>,
) -> Result<ApiResponse<Model>> {
    Ok(ApiResponse::ok(
        state.providers.patch_model(p.id, body).await?,
    ))
}
pub async fn models_disable(
    State(state): State<Arc<AppState>>,
    Extension(_): Extension<SessionContext>,
    Path(p): Path<IdPath>,
    Json(body): Json<ConfirmRevision>,
) -> Result<ApiResponse<Receipt>> {
    require_ack(body.owner_ack)?;
    state
        .providers
        .disable_or_delete_model(p.id, body.revision)
        .await?;
    Ok(ApiResponse::ok(receipt(
        Some(p.id),
        ReceiptResult::Disabled,
        "Model disabled",
    )))
}
pub async fn roles_get(
    State(state): State<Arc<AppState>>,
    Extension(_): Extension<SessionContext>,
) -> Result<ApiResponse<RolesView>> {
    Ok(ApiResponse::ok(state.providers.get_roles().await?))
}
pub async fn roles_put(
    State(state): State<Arc<AppState>>,
    Extension(_): Extension<SessionContext>,
    Json(body): Json<RolesPut>,
) -> Result<ApiResponse<RolesView>> {
    Ok(ApiResponse::ok(state.providers.put_roles(body).await?))
}
pub async fn chains_list(
    State(state): State<Arc<AppState>>,
    Extension(_): Extension<SessionContext>,
    Query(q): Query<ListQuery>,
) -> Result<ApiResponse<Page<Chain>>> {
    let limit = checked_limit(q.limit)? as usize;
    let mut items = state.providers.list_chains().await?;
    items.truncate(limit);
    Ok(ApiResponse::ok(Page {
        items,
        next_cursor: None,
    }))
}
pub async fn chains_create(
    State(state): State<Arc<AppState>>,
    Extension(_): Extension<SessionContext>,
    Json(body): Json<ChainInput>,
) -> Result<ApiResponse<Chain>> {
    Ok(ApiResponse::created(
        state.providers.create_chain(body).await?,
    ))
}
pub async fn chains_put(
    State(state): State<Arc<AppState>>,
    Extension(_): Extension<SessionContext>,
    Path(p): Path<IdPath>,
    Json(body): Json<ChainPut>,
) -> Result<ApiResponse<Chain>> {
    Ok(ApiResponse::ok(
        state.providers.put_chain(p.id, body).await?,
    ))
}
pub async fn chains_disable(
    State(state): State<Arc<AppState>>,
    Extension(_): Extension<SessionContext>,
    Path(p): Path<IdPath>,
    Json(body): Json<ConfirmRevision>,
) -> Result<ApiResponse<Receipt>> {
    require_ack(body.owner_ack)?;
    let current = state
        .providers
        .list_chains()
        .await?
        .into_iter()
        .find(|item| item.id == p.id)
        .ok_or_else(|| AppError::new(ErrorCode::NotFound, "Chain not found"))?;
    if current.revision != body.revision {
        return Err(AppError::new(
            ErrorCode::StaleRevision,
            "Chain revision is stale",
        ));
    }
    state
        .providers
        .put_chain(
            p.id,
            ChainPut {
                revision: body.revision,
                name: current.name,
                enabled: false,
                steps: current.steps,
            },
        )
        .await?;
    Ok(ApiResponse::ok(receipt(
        Some(p.id),
        ReceiptResult::Disabled,
        "Chain disabled",
    )))
}

pub async fn tools_list(
    State(state): State<Arc<AppState>>,
    Extension(_): Extension<SessionContext>,
    Query(q): Query<ListQuery>,
) -> Result<ApiResponse<Page<Tool>>> {
    Ok(ApiResponse::ok(list_tools(&state, q).await?))
}
pub async fn tools_put(
    State(state): State<Arc<AppState>>,
    Extension(_): Extension<SessionContext>,
    Path(p): Path<IdPath>,
    Json(body): Json<ToolPut>,
) -> Result<ApiResponse<Tool>> {
    Ok(ApiResponse::ok(upsert_tool(&state, p.id, body).await?))
}
pub async fn tools_probe(
    State(state): State<Arc<AppState>>,
    Extension(_): Extension<SessionContext>,
    Path(p): Path<IdPath>,
    Json(_): Json<Empty>,
) -> Result<ApiResponse<Capability>> {
    Ok(ApiResponse::ok(tool_capability(&state, p.id).await?))
}

pub async fn projects_list(
    State(state): State<Arc<AppState>>,
    Extension(_): Extension<SessionContext>,
    Query(q): Query<ListQuery>,
) -> Result<ApiResponse<Page<Project>>> {
    Ok(ApiResponse::ok(state.projects.list(q).await?))
}
pub async fn projects_create(
    State(state): State<Arc<AppState>>,
    Extension(owner): Extension<SessionContext>,
    Json(body): Json<ProjectInput>,
) -> Result<ApiResponse<Project>> {
    Ok(ApiResponse::created(
        state.projects.register(body, &owner).await?,
    ))
}
pub async fn projects_get(
    State(state): State<Arc<AppState>>,
    Extension(_): Extension<SessionContext>,
    Path(p): Path<IdPath>,
) -> Result<ApiResponse<Project>> {
    Ok(ApiResponse::ok(state.projects.get(p.id).await?))
}
pub async fn projects_patch(
    State(state): State<Arc<AppState>>,
    Extension(owner): Extension<SessionContext>,
    Path(p): Path<IdPath>,
    Json(body): Json<ProjectPatch>,
) -> Result<ApiResponse<Project>> {
    Ok(ApiResponse::ok(
        state
            .projects
            .patch(p.id, body, owner_actor(&owner))
            .await?,
    ))
}
pub async fn projects_archive(
    State(state): State<Arc<AppState>>,
    Extension(owner): Extension<SessionContext>,
    Path(p): Path<IdPath>,
    Json(body): Json<ConfirmRevision>,
) -> Result<ApiResponse<Receipt>> {
    require_ack(body.owner_ack)?;
    state
        .projects
        .archive(p.id, body.revision, owner_actor(&owner))
        .await?;
    Ok(ApiResponse::ok(receipt(
        Some(p.id),
        ReceiptResult::Archived,
        "Project archived",
    )))
}
pub async fn projects_scan(
    State(state): State<Arc<AppState>>,
    Extension(_): Extension<SessionContext>,
    Path(p): Path<IdPath>,
    Json(_): Json<Empty>,
) -> Result<ApiResponse<Artifact>> {
    Ok(ApiResponse::created(
        state.projects.scan(p.id, CancellationToken::new()).await?,
    ))
}
pub async fn files_find(
    State(state): State<Arc<AppState>>,
    Extension(_): Extension<SessionContext>,
    Path(p): Path<IdPath>,
    Query(q): Query<FileQuery>,
) -> Result<ApiResponse<Page<FileEntry>>> {
    Ok(ApiResponse::ok(state.files.find(p.id, q).await?))
}
pub async fn file_preview(
    State(state): State<Arc<AppState>>,
    Extension(_): Extension<SessionContext>,
    Path(p): Path<IdPath>,
    Query(q): Query<FileReadQuery>,
) -> Result<ApiResponse<FilePreview>> {
    Ok(ApiResponse::ok(state.files.preview(p.id, q.path).await?))
}

pub async fn commands_list(
    State(state): State<Arc<AppState>>,
    Extension(_): Extension<SessionContext>,
    Query(q): Query<CommandQuery>,
) -> Result<ApiResponse<Page<CommandSpec>>> {
    Ok(ApiResponse::ok(list_commands(&state, q).await?))
}
pub async fn commands_create(
    State(state): State<Arc<AppState>>,
    Extension(_): Extension<SessionContext>,
    Json(body): Json<CommandInput>,
) -> Result<ApiResponse<CommandSpec>> {
    Ok(ApiResponse::created(create_command(&state, body).await?))
}
pub async fn commands_disable(
    State(state): State<Arc<AppState>>,
    Extension(_): Extension<SessionContext>,
    Path(p): Path<IdPath>,
    Json(body): Json<ConfirmRevision>,
) -> Result<ApiResponse<Receipt>> {
    require_ack(body.owner_ack)?;
    disable_command(&state, p.id, body.revision).await?;
    Ok(ApiResponse::ok(receipt(
        Some(p.id),
        ReceiptResult::Disabled,
        "Command disabled",
    )))
}

pub async fn tasks_list(
    State(state): State<Arc<AppState>>,
    Extension(_): Extension<SessionContext>,
    Query(mut q): Query<TaskQuery>,
) -> Result<ApiResponse<Page<Task>>> {
    if q.limit == 0 {
        q.limit = 50;
    }
    Ok(ApiResponse::ok(state.tasks.list(q).await?))
}
pub async fn tasks_create(
    State(state): State<Arc<AppState>>,
    Extension(owner): Extension<SessionContext>,
    Json(body): Json<TaskInput>,
) -> Result<ApiResponse<Task>> {
    Ok(ApiResponse::created(
        state
            .tasks
            .create(body.try_into()?, owner_actor(&owner))
            .await?,
    ))
}
pub async fn tasks_get(
    State(state): State<Arc<AppState>>,
    Extension(_): Extension<SessionContext>,
    Path(p): Path<IdPath>,
) -> Result<ApiResponse<Task>> {
    Ok(ApiResponse::ok(state.tasks.get(p.id).await?))
}
pub async fn tasks_patch(
    State(state): State<Arc<AppState>>,
    Extension(owner): Extension<SessionContext>,
    Path(p): Path<IdPath>,
    Json(body): Json<TaskPatchInput>,
) -> Result<ApiResponse<Task>> {
    if matches!(body.progress_source, Some(Some(source)) if source != ProgressSource::Manual) {
        return Err(AppError::new(
            ErrorCode::ActionDenied,
            "Owner task updates may only record manual progress",
        ));
    }
    Ok(ApiResponse::ok(
        state
            .tasks
            .update(p.id, body.try_into()?, owner_actor(&owner))
            .await?,
    ))
}
pub async fn task_events(
    State(state): State<Arc<AppState>>,
    Extension(_): Extension<SessionContext>,
    Path(p): Path<IdPath>,
    Query(q): Query<ListQuery>,
) -> Result<ApiResponse<Page<TaskEvent>>> {
    Ok(ApiResponse::ok(list_task_events(&state, p.id, q).await?))
}
pub async fn task_event_create(
    State(state): State<Arc<AppState>>,
    Extension(owner): Extension<SessionContext>,
    Path(p): Path<IdPath>,
    Json(body): Json<EventInput>,
) -> Result<ApiResponse<TaskEvent>> {
    if matches!(
        body.kind,
        TaskEventKind::Evidence
            | TaskEventKind::Status
            | TaskEventKind::Reschedule
            | TaskEventKind::Checkin
    ) {
        return Err(AppError::new(
            ErrorCode::ActionDenied,
            "This event kind is host-controlled",
        ));
    }
    let payload = EventPayload {
        text: body.text,
        percent: body.percent,
        source: body.percent.map(|_| ProgressSource::Manual),
        ..Default::default()
    };
    Ok(ApiResponse::created(
        state
            .tasks
            .event(p.id, payload, body.kind, owner_actor(&owner))
            .await?,
    ))
}
pub async fn checkin_reply(
    State(state): State<Arc<AppState>>,
    Extension(_): Extension<SessionContext>,
    Path(p): Path<IdPath>,
    Json(body): Json<CheckinReply>,
) -> Result<ApiResponse<Checkin>> {
    let checkin_id = body
        .checkin_id
        .ok_or_else(|| AppError::new(ErrorCode::InvalidRequest, "checkin_id is required"))?;
    verify_checkin_task(&state, checkin_id, p.id).await?;
    Ok(ApiResponse::ok(
        state.tasks.checkin(checkin_id, body).await?,
    ))
}
pub async fn checkins_list(
    State(state): State<Arc<AppState>>,
    Extension(_): Extension<SessionContext>,
    Path(p): Path<IdPath>,
    Query(mut q): Query<CheckinQuery>,
) -> Result<ApiResponse<Page<CheckinStatus>>> {
    if q.limit == 0 {
        q.limit = 50;
    }
    Ok(ApiResponse::ok(state.tasks.checkins(p.id, q).await?))
}

pub async fn schedules_list(
    State(state): State<Arc<AppState>>,
    Extension(_): Extension<SessionContext>,
    Query(q): Query<ScheduleQuery>,
) -> Result<ApiResponse<Page<Schedule>>> {
    let limit = checked_limit(q.limit)? as usize;
    let mut items = state.scheduler.list_schedules().await?;
    items.retain(|s| {
        q.task_id.is_none_or(|id| s.task_id == Some(id)) && q.kind.is_none_or(|kind| s.kind == kind)
    });
    items.truncate(limit);
    Ok(ApiResponse::ok(Page {
        items,
        next_cursor: None,
    }))
}
pub async fn schedules_create(
    State(state): State<Arc<AppState>>,
    Extension(_): Extension<SessionContext>,
    Json(body): Json<ScheduleInput>,
) -> Result<ApiResponse<Schedule>> {
    let schedule = state.scheduler.create_schedule(body.try_into()?).await?;
    let _ = state
        .schedule_wake
        .send(SchedulerWake::Changed(schedule.id))
        .await;
    Ok(ApiResponse::created(schedule))
}
pub async fn schedules_patch(
    State(state): State<Arc<AppState>>,
    Extension(_): Extension<SessionContext>,
    Path(p): Path<IdPath>,
    Json(body): Json<SchedulePatchInput>,
) -> Result<ApiResponse<Schedule>> {
    let schedule = state
        .scheduler
        .patch_schedule(p.id, body.try_into()?)
        .await?;
    let _ = state
        .schedule_wake
        .send(SchedulerWake::Changed(schedule.id))
        .await;
    Ok(ApiResponse::ok(schedule))
}
pub async fn schedules_disable(
    State(state): State<Arc<AppState>>,
    Extension(_): Extension<SessionContext>,
    Path(p): Path<IdPath>,
    Json(body): Json<ConfirmRevision>,
) -> Result<ApiResponse<Receipt>> {
    require_ack(body.owner_ack)?;
    state
        .scheduler
        .disable_or_delete_schedule(p.id, body.revision)
        .await?;
    let _ = state.schedule_wake.send(SchedulerWake::Changed(p.id)).await;
    Ok(ApiResponse::ok(receipt(
        Some(p.id),
        ReceiptResult::Disabled,
        "Reminder disabled",
    )))
}
pub async fn planner_get(
    State(state): State<Arc<AppState>>,
    Extension(_): Extension<SessionContext>,
    Query(q): Query<PlannerQuery>,
) -> Result<ApiResponse<Planner>> {
    let date = parse_date(&q.date)?;
    let tz = parse_tz(&q.timezone)?;
    Ok(ApiResponse::ok(state.tasks.planner(date, tz).await?))
}
pub async fn reports_daily(
    State(state): State<Arc<AppState>>,
    Extension(_): Extension<SessionContext>,
    Query(q): Query<DailyReportQuery>,
) -> Result<ApiResponse<Report>> {
    let page = ReportPageQuery {
        section: q.section,
        cursor: q.cursor,
        limit: q.limit.unwrap_or(50),
    };
    Ok(ApiResponse::ok(
        state
            .reports
            .daily_page(parse_date(&q.date)?, parse_tz(&q.timezone)?, page)
            .await?,
    ))
}
pub async fn reports_weekly(
    State(state): State<Arc<AppState>>,
    Extension(_): Extension<SessionContext>,
    Query(q): Query<WeeklyReportQuery>,
) -> Result<ApiResponse<Report>> {
    let week = parse_week(&q.week)?;
    let page = ReportPageQuery {
        section: q.section,
        cursor: q.cursor,
        limit: q.limit.unwrap_or(50),
    };
    Ok(ApiResponse::ok(
        state
            .reports
            .weekly_page(week, parse_tz(&q.timezone)?, page)
            .await?,
    ))
}

pub async fn conversations_list(
    State(state): State<Arc<AppState>>,
    Extension(_): Extension<SessionContext>,
    Query(q): Query<ListQuery>,
) -> Result<ApiResponse<Page<Conversation>>> {
    Ok(ApiResponse::ok(state.conversations.list(q).await?))
}
pub async fn conversations_create(
    State(state): State<Arc<AppState>>,
    Extension(owner): Extension<SessionContext>,
    Json(body): Json<ConversationInput>,
) -> Result<ApiResponse<Conversation>> {
    Ok(ApiResponse::created(
        state
            .conversations
            .create(body, owner_actor(&owner))
            .await?,
    ))
}
pub async fn conversations_patch(
    State(state): State<Arc<AppState>>,
    Extension(owner): Extension<SessionContext>,
    Path(p): Path<IdPath>,
    Json(body): Json<ConversationPatch>,
) -> Result<ApiResponse<Conversation>> {
    Ok(ApiResponse::ok(
        state
            .conversations
            .patch(p.id, body, owner_actor(&owner))
            .await?,
    ))
}
pub async fn messages_list(
    State(state): State<Arc<AppState>>,
    Extension(_): Extension<SessionContext>,
    Path(p): Path<IdPath>,
    Query(q): Query<ListQuery>,
) -> Result<ApiResponse<Page<Message>>> {
    Ok(ApiResponse::ok(
        state.conversations.list_messages(p.id, q).await?,
    ))
}
pub async fn message_send(
    State(state): State<Arc<AppState>>,
    Extension(owner): Extension<SessionContext>,
    Path(p): Path<IdPath>,
    Json(body): Json<UserMessageInput>,
) -> Result<ApiResponse<MessageReceipt>> {
    let receipt = state
        .assistant
        .receive(p.id, body.try_into()?, &owner)
        .await?;
    if let Some(job_id) = receipt.job_id {
        let workflow = state.workflow.clone();
        tokio::spawn(async move {
            let _ = workflow.run_chat(job_id).await;
        });
        Ok(ApiResponse::accepted(receipt))
    } else {
        Ok(ApiResponse::ok(receipt))
    }
}

pub async fn memory_search(
    State(state): State<Arc<AppState>>,
    Extension(_): Extension<SessionContext>,
    Query(mut q): Query<MemoryQuery>,
) -> Result<ApiResponse<Page<MemoryItem>>> {
    if q.limit == 0 {
        q.limit = 50;
    }
    Ok(ApiResponse::ok(state.memory.search(q).await?))
}
pub async fn memory_create(
    State(state): State<Arc<AppState>>,
    Extension(_): Extension<SessionContext>,
    Json(body): Json<MemoryInput>,
) -> Result<ApiResponse<MemoryItem>> {
    Ok(ApiResponse::created(state.memory.upsert(body).await?))
}
pub async fn memory_patch(
    State(state): State<Arc<AppState>>,
    Extension(owner): Extension<SessionContext>,
    Path(p): Path<IdPath>,
    Json(body): Json<MemoryPatch>,
) -> Result<ApiResponse<MemoryItem>> {
    Ok(ApiResponse::ok(
        state.memory.patch(p.id, body, owner_actor(&owner)).await?,
    ))
}
pub async fn memory_delete_preview(
    State(state): State<Arc<AppState>>,
    Extension(owner): Extension<SessionContext>,
    Json(body): Json<DeleteSelection>,
) -> Result<ApiResponse<OperationReceipt>> {
    Ok(ApiResponse::created(
        state
            .workflow
            .create_delete(body, owner_actor(&owner))
            .await?,
    ))
}
pub async fn memory_export(
    State(state): State<Arc<AppState>>,
    Extension(owner): Extension<SessionContext>,
    headers: HeaderMap,
    Json(body): Json<ExportSelection>,
) -> Result<ApiResponse<Artifact>> {
    let approval_id = headers
        .get("x-maya-approval-id")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<Id>().ok())
        .ok_or_else(|| {
            AppError::new(
                ErrorCode::ApprovalRequired,
                "Export requires an approved export_data operation",
            )
        })?;
    verify_export_binding(&state, approval_id, &body).await?;
    let permit = permit_for_approval(&state, &owner, approval_id, ActionClass::ExportData).await?;
    let artifact = state.memory.export(body, &permit).await?;
    complete_approval_job(&state, approval_id).await?;
    Ok(ApiResponse::created(artifact))
}

pub async fn jobs_list(
    State(state): State<Arc<AppState>>,
    Extension(_): Extension<SessionContext>,
    Query(q): Query<JobQuery>,
) -> Result<ApiResponse<Page<Job>>> {
    Ok(ApiResponse::ok(state.workflow.list(q).await?))
}
pub async fn jobs_create(
    State(state): State<Arc<AppState>>,
    Extension(owner): Extension<SessionContext>,
    Json(body): Json<JobInput>,
) -> Result<ApiResponse<Job>> {
    let job = state.workflow.create(body, owner_actor(&owner)).await?;
    let workflow = state.workflow.clone();
    let job_id = job.id;
    tokio::spawn(async move {
        let _ = workflow.run_to_boundary(job_id).await;
    });
    Ok(ApiResponse::accepted(job))
}
pub async fn jobs_get(
    State(state): State<Arc<AppState>>,
    Extension(_): Extension<SessionContext>,
    Path(p): Path<IdPath>,
) -> Result<ApiResponse<JobDetail>> {
    Ok(ApiResponse::ok(state.workflow.detail(p.id).await?))
}
pub async fn plan_revise(
    State(state): State<Arc<AppState>>,
    Extension(_): Extension<SessionContext>,
    Path(p): Path<IdPath>,
    Json(body): Json<PlanPut>,
) -> Result<ApiResponse<Plan>> {
    Ok(ApiResponse::ok(
        state.workflow.revise_plan(p.id, body).await?,
    ))
}
pub async fn approval_request(
    State(state): State<Arc<AppState>>,
    Extension(_): Extension<SessionContext>,
    Path(p): Path<IdPath>,
    Json(body): Json<ApprovalRequestBody>,
) -> Result<ApiResponse<Approval>> {
    Ok(ApiResponse::created(
        state
            .approval
            .request(p.id, ApprovalRequest::from(body))
            .await?,
    ))
}
pub async fn approval_decide(
    State(state): State<Arc<AppState>>,
    Extension(owner): Extension<SessionContext>,
    Path(p): Path<IdPath>,
    Json(body): Json<ApprovalDecisionBody>,
) -> Result<ApiResponse<Approval>> {
    let decision = ApprovalDecision::try_from(body)?;
    let approval = state.approval.decide(p.id, decision, &owner).await?;
    if approval.decision == ApprovalDecisionKind::Approved
        && approval.action == ActionClass::UploadContext
    {
        let workflow = state.workflow.clone();
        let job_id = approval.job_id;
        tokio::spawn(async move {
            let _ = workflow.run_to_boundary(job_id).await;
        });
    }
    Ok(ApiResponse::ok(approval))
}
pub async fn job_control(
    State(state): State<Arc<AppState>>,
    Extension(owner): Extension<SessionContext>,
    Path(p): Path<IdPath>,
    Json(body): Json<JobAction>,
) -> Result<ApiResponse<Job>> {
    let job = state.workflow.get(p.id).await?;
    if job.kind == JobKind::Operation && body.action == JobControl::Start {
        return Ok(ApiResponse::ok(
            execute_effect(&state, &owner, job, body.revision).await?,
        ));
    }
    let job = state.workflow.control(p.id, body, &owner).await?;
    if matches!(
        job.state,
        JobState::Reviewing
            | JobState::Checkpointing
            | JobState::Executing
            | JobState::Verifying
            | JobState::Promoting
            | JobState::Reporting
    ) {
        let workflow = state.workflow.clone();
        let job_id = job.id;
        tokio::spawn(async move {
            let _ = workflow.run_to_boundary(job_id).await;
        });
    }
    Ok(ApiResponse::ok(job))
}

pub async fn artifact_get(
    State(state): State<Arc<AppState>>,
    Extension(_): Extension<SessionContext>,
    Path(p): Path<IdPath>,
) -> Result<Response> {
    let meta = state.artifacts.get_metadata(p.id).await?;
    let bytes = state.artifacts.read_bytes(p.id).await?;
    let name = format!("maya-{}", p.id);
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, meta.mime)
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{name}\""),
        )
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from(bytes))
        .map_err(|error| AppError::new(ErrorCode::Internal, error.to_string()))
}
pub async fn operation_create(
    State(state): State<Arc<AppState>>,
    Extension(owner): Extension<SessionContext>,
    Json(body): Json<Operation>,
) -> Result<ApiResponse<OperationReceipt>> {
    Ok(ApiResponse::created(
        state
            .workflow
            .create_operation(body, owner_actor(&owner))
            .await?,
    ))
}
pub async fn browser_tabs(
    State(state): State<Arc<AppState>>,
    Extension(_): Extension<SessionContext>,
) -> Result<ApiResponse<Vec<BrowserTab>>> {
    Ok(ApiResponse::ok(state.browser.tabs().await?))
}
pub async fn browser_action_create(
    State(state): State<Arc<AppState>>,
    Extension(owner): Extension<SessionContext>,
    Json(body): Json<BrowserAction>,
) -> Result<ApiResponse<OperationReceipt>> {
    Ok(ApiResponse::created(
        state
            .workflow
            .create_browser(body, owner_actor(&owner))
            .await?,
    ))
}
pub async fn handoff_create(
    State(state): State<Arc<AppState>>,
    Extension(_): Extension<SessionContext>,
    Json(body): Json<HandoffInput>,
) -> Result<ApiResponse<Artifact>> {
    require_ack(body.owner_ack)?;
    if body.text.is_empty() || body.text.len() > 65536 {
        return Err(AppError::new(
            ErrorCode::OutputLimit,
            "Handoff text exceeds 64 KiB",
        ));
    }
    let provenance = Provenance {
        source: ProvenanceSource::ManualHandoff,
        provider_id: None,
        model_id: None,
        url: body.source_url,
        observed_at: now(),
    };
    let bytes = body.text.into_bytes();
    let artifact = state
        .artifacts
        .put(
            ArtifactInput {
                job_id: body.job_id,
                task_id: body.task_id,
                kind: ArtifactKind::Handoff,
                mime: "text/plain; charset=utf-8".to_string(),
                provenance,
                max_bytes: 65536,
                expires_at: None,
            },
            Box::pin(Cursor::new(bytes)),
        )
        .await?;
    Ok(ApiResponse::created(artifact))
}
pub async fn voice_transcribe(
    State(state): State<Arc<AppState>>,
    Extension(owner): Extension<SessionContext>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<ApiResponse<Transcript>> {
    let (audio, mime, meta) = parse_voice_multipart(&headers, &body)?;
    if Digest::of(&audio) != meta.payload_digest {
        return Err(AppError::new(
            ErrorCode::InvalidRequest,
            "Audio payload digest mismatch",
        ));
    }
    verify_voice_binding(
        &state,
        meta.approval_id,
        meta.model_id,
        meta.payload_digest,
        audio.len() as u64,
        VoiceUploadMode::Stt,
    )
    .await?;
    let permit =
        permit_for_approval(&state, &owner, meta.approval_id, ActionClass::VoiceUpload).await?;
    let transcript = state
        .voice
        .transcribe(
            AudioInput {
                data: audio,
                mime,
                language: meta.language,
                model_id: meta.model_id,
                approval_id: Some(meta.approval_id),
                payload_digest: meta.payload_digest,
            },
            &permit,
            CancellationToken::new(),
        )
        .await?;
    complete_approval_job(&state, meta.approval_id).await?;
    Ok(ApiResponse::ok(transcript))
}
pub async fn voice_synthesize(
    State(state): State<Arc<AppState>>,
    Extension(owner): Extension<SessionContext>,
    Json(body): Json<SpeechInput>,
) -> Result<ApiResponse<AudioOutput>> {
    if body.text.is_empty() || body.text.len() > 8192 {
        return Err(AppError::new(
            ErrorCode::InvalidRequest,
            "Speech text must be between 1 and 8192 bytes",
        ));
    }
    if body.model_id.is_none() {
        return Ok(ApiResponse::ok(AudioOutput {
            mode: AudioOutputMode::Browser,
            audio_artifact_id: None,
            mime: None,
            text: Some(body.text),
        }));
    }
    let model_id = body
        .model_id
        .ok_or_else(|| AppError::new(ErrorCode::ModelUnavailable, "TTS model is required"))?;
    let approval_id = body.approval_id.ok_or_else(|| {
        AppError::new(
            ErrorCode::ApprovalRequired,
            "Cloud speech requires a matching voice approval",
        )
    })?;
    let digest = Digest::of(body.text.as_bytes());
    verify_voice_binding(
        &state,
        approval_id,
        model_id,
        digest,
        body.text.len() as u64,
        VoiceUploadMode::Tts,
    )
    .await?;
    let permit = permit_for_approval(&state, &owner, approval_id, ActionClass::VoiceUpload).await?;
    let output = state
        .voice
        .synthesize(body, &permit, CancellationToken::new())
        .await?;
    complete_approval_job(&state, approval_id).await?;
    Ok(ApiResponse::ok(output))
}
pub async fn resources_get(
    State(state): State<Arc<AppState>>,
    Extension(_): Extension<SessionContext>,
    Query(q): Query<ResourceQuery>,
) -> Result<ApiResponse<ResourceSnapshot>> {
    Ok(ApiResponse::ok(
        state.telemetry.sample(q.include_processes).await?,
    ))
}
pub async fn audit_list(
    State(state): State<Arc<AppState>>,
    Extension(_): Extension<SessionContext>,
    Query(q): Query<AuditQueryWire>,
) -> Result<ApiResponse<Page<AuditEvent>>> {
    Ok(ApiResponse::ok(
        state
            .audit
            .list(AuditQuery {
                limit: q.limit,
                cursor: q.cursor,
                job_id: q.job_id,
                action: q.action,
                since: q.since,
            })
            .await?,
    ))
}
pub async fn usage_list(
    State(state): State<Arc<AppState>>,
    Extension(_): Extension<SessionContext>,
    Query(q): Query<UsageQuery>,
) -> Result<ApiResponse<Page<UsageRecord>>> {
    Ok(ApiResponse::ok(list_usage(&state, q).await?))
}
pub async fn backups_list(
    State(state): State<Arc<AppState>>,
    Extension(_): Extension<SessionContext>,
    Query(q): Query<ListQuery>,
) -> Result<ApiResponse<Page<BackupRecord>>> {
    let limit = checked_limit(q.limit)? as usize;
    let mut items = state.backup.list().await?;
    items.truncate(limit);
    Ok(ApiResponse::ok(Page {
        items,
        next_cursor: None,
    }))
}
pub async fn backup_create(
    State(state): State<Arc<AppState>>,
    Extension(_): Extension<SessionContext>,
    Json(_): Json<Empty>,
) -> Result<ApiResponse<BackupRecord>> {
    Ok(ApiResponse::created(state.backup.create().await?))
}

#[derive(Debug, serde::Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct AuditQueryWire {
    pub limit: Option<u16>,
    pub cursor: Option<String>,
    pub job_id: Option<Id>,
    pub action: Option<String>,
    pub since: Option<UtcMs>,
}

fn owner_actor(owner: &SessionContext) -> Actor {
    Actor {
        kind: ActorKind::Owner,
        id: Some(owner.id),
    }
}
fn receipt(id: Option<Id>, result: ReceiptResult, message: &str) -> Receipt {
    Receipt {
        id,
        result,
        message: message.to_string(),
    }
}
fn now() -> UtcMs {
    chrono::Utc::now().timestamp_millis()
}
fn parse_date(value: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|_| AppError::new(ErrorCode::InvalidRequest, "Date must be YYYY-MM-DD"))
}
fn parse_tz(value: &str) -> Result<Tz> {
    value
        .parse()
        .map_err(|_| AppError::new(ErrorCode::InvalidRequest, "Invalid IANA timezone"))
}
fn parse_week(value: &str) -> Result<chrono::IsoWeek> {
    let (year, week) = value
        .split_once("-W")
        .ok_or_else(|| AppError::new(ErrorCode::InvalidRequest, "Week must be YYYY-Www"))?;
    let year: i32 = year
        .parse()
        .map_err(|_| AppError::new(ErrorCode::InvalidRequest, "Invalid week year"))?;
    let week: u32 = week
        .parse()
        .map_err(|_| AppError::new(ErrorCode::InvalidRequest, "Invalid week number"))?;
    NaiveDate::from_isoywd_opt(year, week, chrono::Weekday::Mon)
        .map(|date| date.iso_week())
        .ok_or_else(|| AppError::new(ErrorCode::InvalidRequest, "Invalid ISO week"))
}
fn db_error(error: rusqlite::Error) -> AppError {
    match error {
        rusqlite::Error::QueryReturnedNoRows => {
            AppError::new(ErrorCode::NotFound, "Record not found")
        }
        other => AppError::new(ErrorCode::DbUnavailable, other.to_string()),
    }
}
fn enum_text<T: serde::Serialize>(value: T) -> Result<String> {
    serde_json::to_string(&value)
        .map(|v| v.trim_matches('"').to_string())
        .map_err(|e| AppError::new(ErrorCode::Internal, e.to_string()))
}
fn parse_enum<T: serde::de::DeserializeOwned>(value: String) -> rusqlite::Result<T> {
    serde_json::from_str(&format!("\"{value}\"")).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })
}
fn parse_id(value: String) -> rusqlite::Result<Id> {
    Uuid::parse_str(&value).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })
}
fn parse_optional_id(value: Option<String>) -> rusqlite::Result<Option<Id>> {
    value.map(parse_id).transpose()
}
fn capability(key: &str, state: CapabilityState, reason: Option<&str>) -> Capability {
    Capability {
        key: key.to_string(),
        state,
        reason: reason.map(str::to_string),
        version: Some(env!("CARGO_PKG_VERSION").to_string()),
        checked_at: now(),
        details: CapabilityDetails::default(),
    }
}

async fn load_settings(state: &AppState) -> Result<Settings> {
    state
        .db
        .read(|conn| {
            let read = |key: &str| -> Result<(String, Revision)> {
                conn.query_row(
                    "SELECT value_json,revision FROM settings WHERE key=?1",
                    params![key],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .map_err(db_error)
            };
            let (identity, ir) = read("identity")?;
            let (voice, vr) = read("voice")?;
            let (assistant, ar) = read("assistant")?;
            let (security, sr) = read("security")?;
            let (browser, br) = read("browser")?;
            let revision = ir.max(vr).max(ar).max(sr).max(br);
            Ok(Settings {
                revision,
                identity: decode(&identity)?,
                voice: decode(&voice)?,
                assistant: decode(&assistant)?,
                security: decode(&security)?,
                browser: decode(&browser)?,
            })
        })
        .await
}
async fn save_settings(state: &AppState, settings: &Settings) -> Result<()> {
    validate_settings(settings)?;
    let rows = vec![
        ("identity", serde_json::to_string(&settings.identity)),
        ("voice", serde_json::to_string(&settings.voice)),
        ("assistant", serde_json::to_string(&settings.assistant)),
        ("security", serde_json::to_string(&settings.security)),
        ("browser", serde_json::to_string(&settings.browser)),
    ]
    .into_iter()
    .map(|(k, v)| v.map(|v| (k.to_string(), v)))
    .collect::<std::result::Result<Vec<_>, _>>()
    .map_err(|e| AppError::new(ErrorCode::Internal, e.to_string()))?;
    let expected = settings.revision - 1;
    let revision = settings.revision;
    state.db.write(move|tx|{for(key,value)in rows{let changed=tx.execute("UPDATE settings SET value_json=?1,revision=?2,updated_at=?3 WHERE key=?4 AND revision<=?5",params![value,revision,now(),key,expected]).map_err(db_error)?;if changed!=1{return Err(AppError::new(ErrorCode::StaleRevision,"Settings changed concurrently"));}}Ok(())}).await
}
fn decode<T: serde::de::DeserializeOwned>(value: &str) -> Result<T> {
    serde_json::from_str(value)
        .map_err(|e| AppError::new(ErrorCode::IntegrityFailure, e.to_string()))
}

async fn provider_capability(state: &AppState, id: Id) -> Result<Capability> {
    let provider = state.providers.get_provider(id).await?;
    if !provider.enabled {
        return Ok(capability(
            "provider",
            CapabilityState::Unavailable,
            Some("Provider is disabled"),
        ));
    }
    state.providers.validate_endpoint(id).await?;
    match state.secrets.get(id).await {
        Ok(_) => Ok(capability("provider", CapabilityState::Available, None)),
        Err(error)
            if error.code() == ErrorCode::SecretAccessDenied
                || error.code() == ErrorCode::KeyringUnavailable =>
        {
            Ok(capability(
                "provider",
                CapabilityState::NeedsSetup,
                Some("Provider credential is unavailable"),
            ))
        }
        Err(error) => Err(error),
    }
}

async fn list_tools(state: &AppState, q: ListQuery) -> Result<Page<Tool>> {
    let limit = checked_limit(q.limit)? as usize;
    state.db.read(move|conn|{let mut stmt=conn.prepare("SELECT id,kind,executable,enabled,adapter_version,capability_json,revision FROM tools ORDER BY id LIMIT ?1").map_err(db_error)?;let items=stmt.query_map(params![(limit+1)as i64],parse_tool).map_err(db_error)?.collect::<std::result::Result<Vec<_>,_>>().map_err(db_error)?;Ok(Page{items:items.into_iter().take(limit).collect(),next_cursor:None})}).await
}
fn parse_tool(row: &Row<'_>) -> rusqlite::Result<Tool> {
    let c: String = row.get(5)?;
    Ok(Tool {
        id: parse_id(row.get(0)?)?,
        kind: parse_enum(row.get(1)?)?,
        executable: row.get(2)?,
        enabled: row.get::<_, i64>(3)? != 0,
        adapter_version: row.get(4)?,
        capability: serde_json::from_str(&c).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, Box::new(e))
        })?,
        revision: row.get(6)?,
    })
}
async fn upsert_tool(state: &AppState, id: Id, body: ToolPut) -> Result<Tool> {
    let path = PathBuf::from(&body.executable);
    if !path.is_absolute() {
        return Err(AppError::new(
            ErrorCode::ScopeDenied,
            "Tool executable must be absolute",
        ));
    }
    let metadata = std::fs::metadata(&path).map_err(|_| {
        AppError::new(
            ErrorCode::UnsupportedCapability,
            "Tool executable was not found",
        )
    })?;
    if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
        return Err(AppError::new(
            ErrorCode::UnsupportedCapability,
            "Tool path is not executable",
        ));
    }
    let cap = capability(
        "tool",
        if body.enabled {
            CapabilityState::Available
        } else {
            CapabilityState::Unavailable
        },
        None,
    );
    let cap_json = serde_json::to_string(&cap)
        .map_err(|e| AppError::new(ErrorCode::Internal, e.to_string()))?;
    let kind = enum_text(body.kind)?;
    let exe = body.executable;
    state.db.write(move|tx|{let current:Option<Revision>=tx.query_row("SELECT revision FROM tools WHERE id=?1",params![id.to_string()],|r|r.get(0)).ok();match current{Some(rev)if rev!=body.revision=>return Err(AppError::new(ErrorCode::StaleRevision,"Tool revision is stale")),Some(_)=>{tx.execute("UPDATE tools SET kind=?1,executable=?2,enabled=?3,adapter_version='v0.1',capability_json=?4,revision=revision+1,updated_at=?5 WHERE id=?6",params![kind,exe,body.enabled,cap_json,now(),id.to_string()]).map_err(db_error)?;},None if body.revision==0=>{tx.execute("INSERT INTO tools(id,kind,executable,enabled,adapter_version,executable_sha256,capability_json,revision,updated_at)VALUES(?1,?2,?3,?4,'v0.1',NULL,?5,1,?6)",params![id.to_string(),kind,exe,body.enabled,cap_json,now()]).map_err(db_error)?;},None=>return Err(AppError::new(ErrorCode::NotFound,"Tool not found"))};tx.query_row("SELECT id,kind,executable,enabled,adapter_version,capability_json,revision FROM tools WHERE id=?1",params![id.to_string()],parse_tool).map_err(db_error)}).await
}
async fn tool_capability(state: &AppState, id: Id) -> Result<Capability> {
    let tool = list_tools(
        state,
        ListQuery {
            limit: Some(100),
            cursor: None,
        },
    )
    .await?
    .items
    .into_iter()
    .find(|t| t.id == id)
    .ok_or_else(|| AppError::new(ErrorCode::NotFound, "Tool not found"))?;
    if !tool.enabled {
        return Ok(capability(
            "tool",
            CapabilityState::Unavailable,
            Some("Tool is disabled"),
        ));
    }
    if std::fs::metadata(&tool.executable).is_err() {
        return Ok(capability(
            "tool",
            CapabilityState::Unavailable,
            Some("Configured executable is missing"),
        ));
    }
    Ok(capability("tool", CapabilityState::Available, None))
}

async fn list_commands(state: &AppState, q: CommandQuery) -> Result<Page<CommandSpec>> {
    let limit = checked_limit(q.limit)? as usize;
    let pid = q.project_id.map(|id| id.to_string());
    state.db.read(move|conn|{let mut sql="SELECT id,project_id,label,executable,args_json,timeout_ms,network,enabled,kind,revision FROM command_specs WHERE 1=1".to_string();let mut values=Vec::<Value>::new();if let Some(pid)=pid{sql.push_str(" AND project_id=?");values.push(pid.into());}sql.push_str(" ORDER BY id LIMIT ?");values.push(((limit+1)as i64).into());let refs:Vec<&dyn rusqlite::ToSql>=values.iter().map(|v|v as _).collect();let mut stmt=conn.prepare(&sql).map_err(db_error)?;let items=stmt.query_map(refs.as_slice(),parse_command).map_err(db_error)?.collect::<std::result::Result<Vec<_>,_>>().map_err(db_error)?;Ok(Page{items:items.into_iter().take(limit).collect(),next_cursor:None})}).await
}
fn parse_command(row: &Row<'_>) -> rusqlite::Result<CommandSpec> {
    let args: String = row.get(4)?;
    Ok(CommandSpec {
        id: parse_id(row.get(0)?)?,
        project_id: parse_id(row.get(1)?)?,
        label: row.get(2)?,
        executable: row.get(3)?,
        args: serde_json::from_str(&args).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(e))
        })?,
        timeout_ms: row.get(5)?,
        network: row.get::<_, i64>(6)? != 0,
        enabled: row.get::<_, i64>(7)? != 0,
        kind: parse_enum(row.get(8)?)?,
        revision: row.get(9)?,
    })
}
async fn create_command(state: &AppState, body: CommandInput) -> Result<CommandSpec> {
    require_ack(body.owner_ack)?;
    if body.label.trim().is_empty()
        || body.label.len() > 120
        || body.timeout_ms < 1_000
        || body.timeout_ms > 1_200_000
        || body.network
        || body.args.len() > 64
        || body.args.iter().map(String::len).sum::<usize>() > 8192
    {
        return Err(AppError::new(
            ErrorCode::ActionDenied,
            "Command label, timeout, network policy, or argv is outside the allowed contract",
        ));
    }
    let executable = PathBuf::from(&body.executable);
    let metadata = std::fs::metadata(&executable).map_err(|_| {
        AppError::new(
            ErrorCode::UnsupportedCapability,
            "Command executable was not found",
        )
    })?;
    if !executable.is_absolute()
        || !metadata.is_file()
        || metadata.permissions().mode() & 0o111 == 0
    {
        return Err(AppError::new(
            ErrorCode::ActionDenied,
            "Command executable must be an absolute executable file",
        ));
    }
    let program = executable
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let forbidden_program = matches!(
        program,
        "sh" | "bash"
            | "dash"
            | "zsh"
            | "fish"
            | "sudo"
            | "su"
            | "doas"
            | "rm"
            | "apt"
            | "apt-get"
            | "dpkg"
            | "snap"
            | "flatpak"
    );
    let forbidden_git = program == "git" && body.args.first().is_some_and(|arg| arg == "push");
    if forbidden_program || forbidden_git {
        return Err(AppError::new(
            ErrorCode::ActionDenied,
            "Shells, privilege tools, installers, deletion tools, and Git push are unavailable",
        ));
    }
    state.projects.get(body.project_id).await?;
    let id = Uuid::new_v4();
    let kind = enum_text(body.kind)?;
    let args = serde_json::to_string(&body.args)
        .map_err(|e| AppError::new(ErrorCode::Internal, e.to_string()))?;
    state.db.write(move|tx|{tx.execute("INSERT INTO command_specs(id,project_id,label,executable,args_json,kind,timeout_ms,network,enabled,revision)VALUES(?1,?2,?3,?4,?5,?6,?7,0,?8,1)",params![id.to_string(),body.project_id.to_string(),body.label,body.executable,args,kind,body.timeout_ms,body.enabled]).map_err(db_error)?;tx.query_row("SELECT id,project_id,label,executable,args_json,timeout_ms,network,enabled,kind,revision FROM command_specs WHERE id=?1",params![id.to_string()],parse_command).map_err(db_error)}).await
}
async fn disable_command(state: &AppState, id: Id, revision: Revision) -> Result<()> {
    state.db.write(move|tx|{let changed=tx.execute("UPDATE command_specs SET enabled=0,revision=revision+1 WHERE id=?1 AND revision=?2",params![id.to_string(),revision]).map_err(db_error)?;if changed!=1{return Err(AppError::new(ErrorCode::StaleRevision,"Command revision is stale"));}Ok(())}).await
}

async fn list_task_events(state: &AppState, id: Id, q: ListQuery) -> Result<Page<TaskEvent>> {
    let limit = checked_limit(q.limit)? as usize;
    state.db.read(move|conn|{let mut stmt=conn.prepare("SELECT id,task_id,kind,payload_json,evidence_artifact_id,actor_json,at FROM task_events WHERE task_id=?1 ORDER BY at,id LIMIT ?2").map_err(db_error)?;let items=stmt.query_map(params![id.to_string(),(limit+1)as i64],|row|{let payload:String=row.get(3)?;let actor:String=row.get(5)?;Ok(TaskEvent{id:parse_id(row.get(0)?)?,task_id:parse_id(row.get(1)?)?,kind:parse_enum(row.get(2)?)?,payload:serde_json::from_str(&payload).map_err(|e|rusqlite::Error::FromSqlConversionFailure(3,rusqlite::types::Type::Text,Box::new(e)))?,evidence_artifact_id:parse_optional_id(row.get(4)?)?,actor:serde_json::from_str(&actor).map_err(|e|rusqlite::Error::FromSqlConversionFailure(5,rusqlite::types::Type::Text,Box::new(e)))?,at:row.get(6)?})}).map_err(db_error)?.collect::<std::result::Result<Vec<_>,_>>().map_err(db_error)?;Ok(Page{items:items.into_iter().take(limit).collect(),next_cursor:None})}).await
}
async fn verify_checkin_task(state: &AppState, checkin: Id, task: Id) -> Result<()> {
    state
        .db
        .read(move |conn| {
            let actual: String = conn
                .query_row(
                    "SELECT task_id FROM checkins WHERE id=?1",
                    params![checkin.to_string()],
                    |row| row.get(0),
                )
                .map_err(db_error)?;
            if actual != task.to_string() {
                return Err(AppError::new(
                    ErrorCode::Conflict,
                    "Checkin does not belong to task",
                ));
            }
            Ok(())
        })
        .await
}

async fn list_usage(state: &AppState, q: UsageQuery) -> Result<Page<UsageRecord>> {
    let limit = checked_limit(q.limit)? as usize;
    let pid = q.provider_id.map(|v| v.to_string());
    let jid = q.job_id.map(|v| v.to_string());
    state
        .db
        .read(move |conn| {
            let mut sql = "SELECT id,provider_id,model_id,job_id,request_id,input_tokens,output_tokens,cost_coefficient,cost_scale,currency,cost_source,billing_mode,status,at FROM usage_records WHERE 1=1".to_string();
            let mut values = Vec::<Value>::new();
            if let Some(value) = pid {
                sql.push_str(" AND provider_id=?");
                values.push(value.into());
            }
            if let Some(value) = jid {
                sql.push_str(" AND job_id=?");
                values.push(value.into());
            }
            if let Some(value) = q.since {
                sql.push_str(" AND at>=?");
                values.push(value.into());
            }
            sql.push_str(" ORDER BY at,id LIMIT ?");
            values.push(((limit + 1) as i64).into());
            let refs: Vec<&dyn rusqlite::ToSql> = values.iter().map(|value| value as _).collect();
            let mut statement = conn.prepare(&sql).map_err(db_error)?;
            let items = statement
                .query_map(refs.as_slice(), parse_usage)
                .map_err(db_error)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(db_error)?;
            Ok(Page {
                items: items.into_iter().take(limit).collect(),
                next_cursor: None,
            })
        })
        .await
}
fn parse_usage(row: &Row<'_>) -> rusqlite::Result<UsageRecord> {
    let coefficient: Option<i64> = row.get(7)?;
    let scale: Option<u8> = row.get(8)?;
    let currency: Option<String> = row.get(9)?;
    let source: Option<String> = row.get(10)?;
    let money = match (coefficient, scale, currency, source) {
        (Some(coefficient), Some(scale), Some(currency), Some(source)) => Some(Money {
            coefficient,
            scale,
            currency,
            source: parse_enum(source)?,
        }),
        _ => None,
    };
    Ok(UsageRecord {
        id: parse_id(row.get(0)?)?,
        provider_id: parse_id(row.get(1)?)?,
        model_id: parse_optional_id(row.get(2)?)?,
        job_id: parse_optional_id(row.get(3)?)?,
        request_id: parse_id(row.get(4)?)?,
        input_tokens: row.get(5)?,
        output_tokens: row.get(6)?,
        money,
        billing_mode: parse_enum(row.get(11)?)?,
        status: parse_enum(row.get(12)?)?,
        at: row.get(13)?,
    })
}

async fn execute_effect(
    state: &AppState,
    owner: &SessionContext,
    job: Job,
    revision: Revision,
) -> Result<Job> {
    if job.revision != revision {
        return Err(AppError::new(
            ErrorCode::StaleRevision,
            "Job revision is stale",
        ));
    }
    if job.state != JobState::Approved {
        return Err(AppError::new(
            ErrorCode::ApprovalRequired,
            "Operation is not approved",
        ));
    }
    let jid = job.id;
    let (approval_id,request_digest,scope,policy_version,input):(Id,Digest,ApprovalScope,Revision,String)=state.db.read(move|conn|{let input:String=conn.query_row("SELECT input_json FROM jobs WHERE id=?1",params![jid.to_string()],|row|row.get(0)).map_err(db_error)?;let (aid,digest,scope,policy):(String,String,String,Revision)=conn.query_row("SELECT id,request_digest,scope_json,policy_version FROM approvals WHERE job_id=?1 AND decision='approved' ORDER BY decided_at DESC LIMIT 1",params![jid.to_string()],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).map_err(db_error)?;Ok((parse_id(aid).map_err(db_error)?,Digest::from_hex(&digest)?,decode(&scope)?,policy,input))}).await?;
    let class = *scope
        .action_classes
        .first()
        .ok_or_else(|| AppError::new(ErrorCode::ActionDenied, "Approval has no action scope"))?;
    let permit = state
        .policy
        .authorize(
            &crate::policy::ActionContext {
                actor: owner_actor(owner),
                job_id: job.id,
                step_id: Uuid::new_v4(),
                project_id: job.project_id,
                plan_id: job.plan_id,
                approval_ids: vec![approval_id],
                policy_version,
                now: now(),
            },
            &crate::policy::ActionRequest {
                class,
                scope: scope.clone(),
                request_digest,
                change_digest: scope.operation_digest,
            },
        )
        .await?;
    set_job_state(state, job.id, job.revision, JobState::Executing, None).await?;
    let mut value: serde_json::Value = serde_json::from_str(&input)
        .map_err(|_| AppError::new(ErrorCode::IntegrityFailure, "Stored operation is invalid"))?;
    let effect = value
        .get("effect")
        .and_then(|item| item.as_str())
        .unwrap_or_default()
        .to_string();
    if let Some(object) = value.as_object_mut() {
        object.remove("effect");
    }
    let outcome: Result<OperationResult> = if effect == "delete_history" {
        let selection: DeleteSelection = serde_json::from_value(value).map_err(|_| {
            AppError::new(
                ErrorCode::IntegrityFailure,
                "Stored deletion selection is invalid",
            )
        })?;
        state
            .memory
            .purge(selection, &permit)
            .await
            .map(|receipt| OperationResult {
                kind: "delete_history".to_string(),
                message: format!("Deleted {} selected records", receipt.deleted_count),
                artifact_id: None,
                text: None,
                tab_id: None,
                exit: None,
            })
    } else if effect == "browser" {
        if value.get("prompt").is_some_and(serde_json::Value::is_null)
            && let Some(bytes) = state.workflow.take_transient(job.id).await
        {
            let prompt = String::from_utf8(bytes.to_vec()).map_err(|_| {
                AppError::new(ErrorCode::InvalidRequest, "Transient prompt is not UTF-8")
            })?;
            value
                .as_object_mut()
                .unwrap()
                .insert("prompt".to_string(), serde_json::Value::String(prompt));
        }
        let action: BrowserAction = serde_json::from_value(value).map_err(|_| {
            AppError::new(
                ErrorCode::IntegrityFailure,
                "Stored browser action is invalid",
            )
        })?;
        state
            .browser
            .action(action, &permit, CancellationToken::new())
            .await
    } else {
        let kind = value
            .get("kind")
            .and_then(|item| item.as_str())
            .unwrap_or_default();
        if kind == "clipboard_write" {
            let bytes = state.workflow.take_transient(job.id).await.ok_or_else(|| {
                AppError::new(
                    ErrorCode::NeedsReconciliation,
                    "Sensitive clipboard payload expired; re-enter it",
                )
            })?;
            let text = String::from_utf8(bytes.to_vec()).map_err(|_| {
                AppError::new(ErrorCode::InvalidRequest, "Clipboard payload is not UTF-8")
            })?;
            value
                .as_object_mut()
                .unwrap()
                .insert("text".to_string(), serde_json::Value::String(text));
        }
        let operation: Operation = serde_json::from_value(value).map_err(|_| {
            AppError::new(ErrorCode::IntegrityFailure, "Stored operation is invalid")
        })?;
        match operation {
            Operation::Launch {
                app,
                tool_id,
                project_id,
                folder,
                url,
            } => {
                state
                    .desktop
                    .launch(
                        LaunchInput {
                            app,
                            tool_id,
                            project_id,
                            folder,
                            url,
                        },
                        &permit,
                    )
                    .await
            }
            Operation::ClipboardRead {} => {
                state
                    .desktop
                    .clipboard(ClipboardInput { text: None }, &permit)
                    .await
            }
            Operation::ClipboardWrite { text } => {
                state
                    .desktop
                    .clipboard(ClipboardInput { text: Some(text) }, &permit)
                    .await
            }
            Operation::RestoreDatabase { backup_id } => state
                .backup
                .restore(backup_id, &permit)
                .await
                .map(|receipt| OperationResult {
                    kind: "restore_database".to_string(),
                    message: format!("Restored backup {}", receipt.backup_id),
                    artifact_id: None,
                    text: None,
                    tab_id: None,
                    exit: None,
                }),
            Operation::Rollback {
                job_id: source_job,
                checkpoint_id,
                change_digest,
            } => {
                let source = state.workflow.get(source_job).await?;
                let project_id = source.project_id.ok_or_else(|| {
                    AppError::new(ErrorCode::InvalidRequest, "Checkpoint job has no project")
                })?;
                let project = state.projects.get(project_id).await?;
                state
                    .coding
                    .checkpoint_service()
                    .restore(checkpoint_id, change_digest, &project, &permit)
                    .await
                    .map(|receipt| OperationResult {
                        kind: "rollback".to_string(),
                        message: format!("Restored {} files", receipt.files.len()),
                        artifact_id: None,
                        text: None,
                        tab_id: None,
                        exit: None,
                    })
            }
            Operation::Command { command_id } => {
                if !scope.command_ids.contains(&command_id) {
                    return Err(AppError::new(
                        ErrorCode::ActionDenied,
                        "Command is outside the approved scope",
                    ));
                }
                let project_id = job.project_id.ok_or_else(|| {
                    AppError::new(ErrorCode::IntegrityFailure, "Command job has no project")
                })?;
                let project = state.projects.get(project_id).await?;
                let (_, stage) = state
                    .coding
                    .checkpoint_service()
                    .create_command_stage(&project, job.id, class, &permit)
                    .await?;
                let result = state
                    .files
                    .command(command_id, &stage, &permit, CancellationToken::new())
                    .await?;
                Ok(OperationResult {
                    kind: "command".to_string(),
                    message: match result.exit_code {
                        Some(code) => format!("Command exited with status {code}"),
                        None if result.timed_out => "Command timed out".to_string(),
                        None if result.cancelled => "Command was cancelled".to_string(),
                        None => "Command ended without an exit status".to_string(),
                    },
                    artifact_id: None,
                    text: None,
                    tab_id: None,
                    exit: Some(ProcessResultDto {
                        exit_code: result.exit_code,
                        signal: result.signal,
                        timed_out: result.timed_out,
                        cancelled: result.cancelled,
                        stdout_artifact_id: result.stdout_artifact,
                        stderr_artifact_id: result.stderr_artifact,
                        truncated: result.truncated,
                        children_reaped: result.children_reaped,
                    }),
                })
            }
            Operation::VoiceUpload { .. } => Err(AppError::new(
                ErrorCode::InvalidRequest,
                "Voice consent is consumed by the matching voice endpoint",
            )),
            Operation::Export { .. } => Err(AppError::new(
                ErrorCode::InvalidRequest,
                "Export consent is consumed by the memory export endpoint",
            )),
        }
    };
    match outcome {
        Ok(result) => {
            let bytes = serde_json::to_vec(&result)
                .map_err(|error| AppError::new(ErrorCode::Internal, error.to_string()))?;
            state
                .artifacts
                .put(
                    ArtifactInput {
                        job_id: Some(job.id),
                        task_id: job.task_id,
                        kind: ArtifactKind::Report,
                        mime: "application/json".to_string(),
                        provenance: Provenance::system(),
                        max_bytes: 64 * 1024,
                        expires_at: None,
                    },
                    Box::pin(Cursor::new(bytes)),
                )
                .await?;
            consume_approval(state, approval_id).await?;
            set_job_state(state, job.id, job.revision + 1, JobState::Completed, None).await
        }
        Err(error) => {
            let uncertain = matches!(
                error.code(),
                ErrorCode::Timeout
                    | ErrorCode::ToolFailed
                    | ErrorCode::ProviderError
                    | ErrorCode::ChildStillRunning
            );
            let target = if uncertain {
                JobState::NeedsReconciliation
            } else {
                JobState::Failed
            };
            let _ =
                set_job_state(state, job.id, job.revision + 1, target, Some(error.code())).await;
            Err(error)
        }
    }
}
async fn set_job_state(
    state: &AppState,
    id: Id,
    revision: Revision,
    target: JobState,
    error: Option<ErrorCode>,
) -> Result<Job> {
    let target = enum_text(target)?;
    let error = error.map(enum_text).transpose()?;
    state.db.write(move|tx|{let changed=tx.execute("UPDATE jobs SET state=?1,error_code=?2,revision=revision+1,updated_at=?3 WHERE id=?4 AND revision=?5",params![target,error,now(),id.to_string(),revision]).map_err(db_error)?;if changed!=1{return Err(AppError::new(ErrorCode::StaleRevision,"Job state changed concurrently"));}Ok(())}).await?;
    state.workflow.get(id).await
}
async fn consume_approval(state: &AppState, id: Id) -> Result<()> {
    state.db.write(move|tx|{tx.execute("UPDATE approvals SET decision='consumed',decided_at=COALESCE(decided_at,?1),revision=revision+1 WHERE id=?2 AND decision='approved'",params![now(),id.to_string()]).map_err(db_error)?;Ok(())}).await
}

async fn permit_for_approval(
    state: &AppState,
    owner: &SessionContext,
    approval_id: Id,
    class: ActionClass,
) -> Result<crate::policy::ExecutionPermit> {
    let (job_id,request_digest,scope,policy_version):(Id,Digest,ApprovalScope,Revision)=state.db.read(move|conn|{let (job,digest,scope,version,decision):(String,String,String,Revision,String)=conn.query_row("SELECT job_id,request_digest,scope_json,policy_version,decision FROM approvals WHERE id=?1",params![approval_id.to_string()],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?))).map_err(db_error)?;if decision!="approved"{return Err(AppError::new(ErrorCode::ApprovalRequired,"Approval is not approved"));}Ok((parse_id(job).map_err(db_error)?,Digest::from_hex(&digest)?,decode(&scope)?,version))}).await?;
    if !scope.action_classes.contains(&class) {
        return Err(AppError::new(
            ErrorCode::ActionDenied,
            "Approval does not cover this action",
        ));
    }
    state
        .policy
        .authorize(
            &crate::policy::ActionContext {
                actor: owner_actor(owner),
                job_id,
                step_id: Uuid::new_v4(),
                project_id: scope.project_id,
                plan_id: None,
                approval_ids: vec![approval_id],
                policy_version,
                now: now(),
            },
            &crate::policy::ActionRequest {
                class,
                scope: scope.clone(),
                request_digest,
                change_digest: scope.operation_digest,
            },
        )
        .await
}
async fn verify_voice_binding(
    state: &AppState,
    approval_id: Id,
    model_id: Id,
    digest: Digest,
    size: u64,
    mode: VoiceUploadMode,
) -> Result<()> {
    state.db.read(move|conn|{let input:String=conn.query_row("SELECT j.input_json FROM approvals a JOIN jobs j ON j.id=a.job_id WHERE a.id=?1",params![approval_id.to_string()],|row|row.get(0)).map_err(db_error)?;let value:serde_json::Value=serde_json::from_str(&input).map_err(|_|AppError::new(ErrorCode::IntegrityFailure,"Stored voice consent is invalid"))?;let expected_mode=enum_text(mode)?;if value.get("kind").and_then(|v|v.as_str())!=Some("voice_upload")||value.get("model_id").and_then(|v|v.as_str())!=Some(model_id.to_string().as_str())||value.get("payload_digest").and_then(|v|v.as_str())!=Some(digest.to_hex().as_str())||value.get("size_bytes").and_then(|v|v.as_u64())!=Some(size)||value.get("mode").and_then(|v|v.as_str())!=Some(expected_mode.as_str()){return Err(AppError::new(ErrorCode::ActionDenied,"Voice payload does not match the approved consent"));}Ok(())}).await
}
async fn verify_export_binding(
    state: &AppState,
    approval_id: Id,
    requested: &ExportSelection,
) -> Result<()> {
    let approval = approval_id.to_string();
    let requested = requested.clone();
    state
        .db
        .read(move |conn| {
            let (action, input): (String, String) = conn
                .query_row(
                    "SELECT a.action,j.input_json FROM approvals a JOIN jobs j ON j.id=a.job_id WHERE a.id=?1",
                    params![approval],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .map_err(db_error)?;
            if action != "export_data" {
                return Err(AppError::new(
                    ErrorCode::ActionDenied,
                    "Approval does not authorize export",
                ));
            }
            let mut value: serde_json::Value = serde_json::from_str(&input).map_err(|_| {
                AppError::new(ErrorCode::IntegrityFailure, "Stored export consent is invalid")
            })?;
            value
                .as_object_mut()
                .ok_or_else(|| {
                    AppError::new(ErrorCode::IntegrityFailure, "Stored export is not an object")
                })?
                .remove("effect");
            let operation: Operation = serde_json::from_value(value).map_err(|_| {
                AppError::new(ErrorCode::IntegrityFailure, "Stored export selection is invalid")
            })?;
            match operation {
                Operation::Export { selection } if selection == requested => Ok(()),
                Operation::Export { .. } => Err(AppError::new(
                    ErrorCode::ActionDenied,
                    "Export selection does not match approved consent",
                )),
                _ => Err(AppError::new(
                    ErrorCode::ActionDenied,
                    "Approval is not bound to an export operation",
                )),
            }
        })
        .await
}
async fn complete_approval_job(state: &AppState, approval_id: Id) -> Result<()> {
    let job_id = state
        .db
        .read(move |conn| {
            let job: String = conn
                .query_row(
                    "SELECT job_id FROM approvals WHERE id=?1",
                    params![approval_id.to_string()],
                    |row| row.get(0),
                )
                .map_err(db_error)?;
            parse_id(job).map_err(db_error)
        })
        .await?;
    consume_approval(state, approval_id).await?;
    let job = state.workflow.get(job_id).await?;
    let _ = set_job_state(state, job_id, job.revision, JobState::Completed, None).await?;
    Ok(())
}
fn parse_voice_multipart(headers: &HeaderMap, body: &[u8]) -> Result<(Vec<u8>, String, VoiceMeta)> {
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            AppError::new(
                ErrorCode::UnsupportedMedia,
                "Missing multipart content type",
            )
        })?;
    let boundary = content_type
        .split(';')
        .find_map(|part| part.trim().strip_prefix("boundary="))
        .map(|v| v.trim_matches('"'))
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 70
                && value.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric()
                        || matches!(
                            byte,
                            b'\''
                                | b'('
                                | b')'
                                | b'+'
                                | b'_'
                                | b','
                                | b'-'
                                | b'.'
                                | b'/'
                                | b':'
                                | b'='
                                | b'?'
                        )
                })
        })
        .ok_or_else(|| {
            AppError::new(ErrorCode::UnsupportedMedia, "Multipart boundary is missing")
        })?;
    let marker = format!("--{boundary}").into_bytes();
    let next_marker = [b"\r\n--".as_slice(), boundary.as_bytes()].concat();
    if !body.starts_with(&marker) {
        return Err(AppError::new(
            ErrorCode::InvalidRequest,
            "Multipart body does not begin with its boundary",
        ));
    }
    let mut audio = None;
    let mut mime = None;
    let mut meta = None;
    let mut cursor = marker.len();
    let mut parts = 0u8;
    loop {
        if body.get(cursor..cursor + 2) == Some(b"--") {
            cursor += 2;
            if !matches!(body.get(cursor..), Some(b"") | Some(b"\r\n")) {
                return Err(AppError::new(
                    ErrorCode::InvalidRequest,
                    "Multipart body has data after the closing boundary",
                ));
            }
            break;
        }
        if body.get(cursor..cursor + 2) != Some(b"\r\n") {
            return Err(AppError::new(
                ErrorCode::InvalidRequest,
                "Multipart boundary framing is invalid",
            ));
        }
        cursor += 2;
        let header_offset = find_bytes(&body[cursor..], b"\r\n\r\n").ok_or_else(|| {
            AppError::new(
                ErrorCode::InvalidRequest,
                "Multipart part headers are incomplete",
            )
        })?;
        if header_offset > 8192 {
            return Err(AppError::new(
                ErrorCode::OutputLimit,
                "Multipart part headers are too large",
            ));
        }
        let header_end = cursor + header_offset;
        let header_text = std::str::from_utf8(&body[cursor..header_end]).map_err(|_| {
            AppError::new(ErrorCode::InvalidRequest, "Multipart headers are invalid")
        })?;
        let content_start = header_end + 4;
        let content_offset = find_bytes(&body[content_start..], &next_marker).ok_or_else(|| {
            AppError::new(
                ErrorCode::InvalidRequest,
                "Multipart closing boundary is missing",
            )
        })?;
        let content_end = content_start + content_offset;
        let content = &body[content_start..content_end];
        let disposition = header_text
            .split("\r\n")
            .find(|line| {
                line.to_ascii_lowercase()
                    .starts_with("content-disposition:")
            })
            .ok_or_else(|| {
                AppError::new(
                    ErrorCode::InvalidRequest,
                    "Multipart part has no disposition",
                )
            })?;
        let is_audio = disposition
            .split(';')
            .any(|value| value.trim() == "name=\"audio\"");
        let is_meta = disposition
            .split(';')
            .any(|value| value.trim() == "name=\"meta\"");
        if is_audio {
            if audio.is_some() {
                return Err(AppError::new(
                    ErrorCode::InvalidRequest,
                    "Multipart audio part is duplicated",
                ));
            }
            let part_mime = header_text
                .split("\r\n")
                .find_map(|line| {
                    line.strip_prefix("Content-Type: ")
                        .or_else(|| line.strip_prefix("content-type: "))
                })
                .unwrap_or("application/octet-stream")
                .trim()
                .to_string();
            audio = Some(content.to_vec());
            mime = Some(part_mime);
        } else if is_meta {
            if meta.is_some() {
                return Err(AppError::new(
                    ErrorCode::InvalidRequest,
                    "Multipart metadata part is duplicated",
                ));
            }
            meta = Some(serde_json::from_slice(content).map_err(|_| {
                AppError::new(ErrorCode::InvalidRequest, "Voice metadata is invalid")
            })?);
        } else {
            return Err(AppError::new(
                ErrorCode::InvalidRequest,
                "Unexpected multipart part",
            ));
        }
        parts += 1;
        if parts > 2 {
            return Err(AppError::new(
                ErrorCode::InvalidRequest,
                "Too many multipart parts",
            ));
        }
        cursor = content_end + 2 + marker.len();
    }
    Ok((
        audio.ok_or_else(|| {
            AppError::new(ErrorCode::InvalidRequest, "Multipart audio part is missing")
        })?,
        mime.unwrap_or_else(|| "application/octet-stream".to_string()),
        meta.ok_or_else(|| {
            AppError::new(
                ErrorCode::InvalidRequest,
                "Multipart metadata part is missing",
            )
        })?,
    ))
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
mod voice_multipart_tests {
    use super::*;

    #[test]
    fn binary_voice_multipart_preserves_newlines_and_requires_unique_parts() {
        let boundary = "maya-boundary-012345";
        let audio = b"webm\0\r\nline-two\n\xff\0";
        let meta = VoiceMeta {
            language: Language::English,
            model_id: Uuid::new_v4(),
            approval_id: Uuid::new_v4(),
            payload_digest: Digest::of(audio),
        };
        let mut body = Vec::new();
        body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"audio\"; filename=\"voice.webm\"\r\nContent-Type: audio/webm\r\n\r\n").as_bytes());
        body.extend_from_slice(audio);
        body.extend_from_slice(format!("\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"meta\"\r\nContent-Type: application/json\r\n\r\n").as_bytes());
        body.extend_from_slice(
            &serde_json::to_vec(&serde_json::json!({
                "language": "english",
                "model_id": meta.model_id,
                "approval_id": meta.approval_id,
                "payload_digest": meta.payload_digest
            }))
            .unwrap(),
        );
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={boundary}")
                .parse()
                .unwrap(),
        );
        let (parsed, mime, parsed_meta) = parse_voice_multipart(&headers, &body).unwrap();
        assert_eq!(parsed, audio);
        assert_eq!(mime, "audio/webm");
        assert_eq!(parsed_meta.payload_digest, meta.payload_digest);

        let duplicate = body
            .windows(format!("\r\n--{boundary}").len())
            .position(|window| window == format!("\r\n--{boundary}").as_bytes())
            .unwrap();
        let mut malformed = body[..duplicate].to_vec();
        malformed.extend_from_slice(format!("\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"audio\"\r\n\r\ncopy\r\n--{boundary}--\r\n").as_bytes());
        assert!(parse_voice_multipart(&headers, &malformed).is_err());
    }
}
