use crate::auth::SessionContext;
use crate::error::{AppError, ErrorCode, Result};
use crate::protocol::{EventQuery, IdPath};
use crate::state::AppState;
use crate::types::{Id, StreamEvent, WhitelistedEventData};
use axum::Extension;
use axum::extract::{Path, Query, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use futures_util::stream::{self, Stream, StreamExt};
use std::convert::Infallible;
use std::sync::Arc;

pub type EventStream =
    std::pin::Pin<Box<dyn Stream<Item = std::result::Result<Event, Infallible>> + Send>>;

pub async fn stream_global(
    State(state): State<Arc<AppState>>,
    Extension(owner): Extension<SessionContext>,
    Query(query): Query<EventQuery>,
) -> Result<Response> {
    stream(state, owner, query).await
}

pub async fn stream_job(
    State(state): State<Arc<AppState>>,
    Extension(owner): Extension<SessionContext>,
    Path(path): Path<IdPath>,
    Query(mut query): Query<EventQuery>,
) -> Result<Response> {
    query.job_id = Some(path.id);
    stream(state, owner, query).await
}

async fn stream(
    state: Arc<AppState>,
    _owner: SessionContext,
    query: EventQuery,
) -> Result<Response> {
    let stream_permit = state.auth.acquire_stream()?;
    let after = query.after.unwrap_or(0);
    if after < 0 {
        return Err(AppError::new(
            ErrorCode::InvalidCursor,
            "Event cursor must be nonnegative",
        ));
    }
    let replay = replay_events(&state, after, query.job_id).await?;
    let initial = stream::iter(replay.into_iter().map(to_sse));
    let receiver = state.events.subscribe();
    let job_filter = query.job_id;
    let live = stream::unfold(
        (receiver, stream_permit),
        move |(mut receiver, permit)| async move {
            loop {
                match receiver.recv().await {
                    Ok(event) if job_filter.is_none() || event.job_id == job_filter => {
                        return Some((to_sse(event), (receiver, permit)));
                    }
                    Ok(_) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        return None;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
                }
            }
        },
    );
    let output: EventStream = Box::pin(initial.chain(live));
    Ok(Sse::new(output)
        .keep_alive(
            KeepAlive::new()
                .interval(std::time::Duration::from_secs(20))
                .text("heartbeat"),
        )
        .into_response())
}

async fn replay_events(state: &AppState, after: i64, job: Option<Id>) -> Result<Vec<StreamEvent>> {
    let page = state.audit.replay(after, job, 100).await?;
    let mut output = Vec::new();
    for audit in page.items {
        if audit.resource_kind == "job"
            && let Some(job_id) = audit.resource_id.or(audit.job_id)
            && let Ok(current) = state.workflow.get(job_id).await
        {
            output.push(StreamEvent {
                seq: audit.seq,
                kind: "job_state".to_string(),
                job_id: Some(job_id),
                at: audit.at,
                data: WhitelistedEventData::Job {
                    state: current.state,
                    revision: current.revision,
                    step_id: None,
                },
            });
        }
    }
    Ok(output)
}

fn to_sse(event: StreamEvent) -> std::result::Result<Event, Infallible> {
    let mut output = Event::default().event(event.kind.clone());
    if event.seq > 0 {
        output = output.id(event.seq.to_string());
    }
    output = output
        .json_data(event)
        .unwrap_or_else(|_| Event::default().event("serialization_error"));
    Ok(output)
}
