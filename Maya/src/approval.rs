use crate::artifacts::ArtifactStore;
use crate::auth::SessionContext;
use crate::db::Db;
use crate::error::{AppError, ErrorCode, Result};
use crate::types::{
    ActionClass, Approval, ApprovalDecisionKind, ApprovalScope, Digest, Id, Revision, UtcMs,
};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalRequest {
    pub job_revision: Revision,
    pub action: ActionClass,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_id: Option<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub change_digest: Option<Digest>,
    pub ttl_minutes: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalDecision {
    pub revision: Revision,
    pub request_digest: Digest,
    pub decision: ApprovalDecisionKind,
    pub owner_ack: bool,
}

#[derive(Serialize)]
struct DigestPayload<'a> {
    job_id: &'a str,
    action: &'a str,
    scope: &'a ApprovalScope,
    policy_version: i64,
    requested_at: i64,
    expires_at: i64,
    change_digest: Option<String>,
}

#[derive(Clone)]
pub struct ApprovalService {
    db: Db,
    artifacts: Option<Arc<ArtifactStore>>,
}

impl ApprovalService {
    pub fn new(db: Db) -> Self {
        Self {
            db,
            artifacts: None,
        }
    }

    pub fn with_artifacts(db: Db, artifacts: Arc<ArtifactStore>) -> Self {
        Self {
            db,
            artifacts: Some(artifacts),
        }
    }

    pub fn compute_digest(
        job_id: Id,
        action: &ActionClass,
        scope: &ApprovalScope,
        policy_version: Revision,
        requested_at: UtcMs,
        expires_at: UtcMs,
        change_digest: Option<Digest>,
    ) -> Result<Digest> {
        let action_str = serde_json::to_string(action).unwrap_or_default();
        let payload = DigestPayload {
            job_id: &job_id.to_string(),
            action: action_str.trim_matches('"'),
            scope,
            policy_version,
            requested_at,
            expires_at,
            change_digest: change_digest.map(|d| d.to_hex()),
        };
        let canonical_json = serde_json::to_string(&payload).map_err(|e| {
            AppError::new(
                ErrorCode::Internal,
                format!("Digest serialization error: {e}"),
            )
        })?;
        let mut hasher = Sha256::new();
        hasher.update(canonical_json.as_bytes());
        let res = hasher.finalize();
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&res);
        Ok(Digest(bytes))
    }

    pub async fn request(&self, job_id: Id, req: ApprovalRequest) -> Result<Approval> {
        let ttl_mins = req.ttl_minutes.clamp(1, 60) as i64;
        let now = chrono::Utc::now().timestamp_millis();
        let expires_at = now + ttl_mins * 60 * 1000;
        let approval_id = Uuid::new_v4();

        let actual_change_scope = if matches!(
            req.action,
            ActionClass::DeleteProjectFiles | ActionClass::AcceptUnverifiedChanges
        ) {
            let artifacts = self.artifacts.as_ref().ok_or_else(|| {
                AppError::new(
                    ErrorCode::UnsupportedCapability,
                    "Change-set approvals require the artifact store",
                )
            })?;
            let artifact_id = self
                .db
                .read(move |conn| {
                    let value: String = conn
                        .query_row(
                            "SELECT id FROM artifacts WHERE job_id=?1 AND kind='changes' AND mime='application/json' ORDER BY created_at DESC,id DESC LIMIT 1",
                            params![job_id.to_string()],
                            |row| row.get(0),
                        )
                        .map_err(|_| {
                            AppError::new(ErrorCode::InvalidPlan, "Actual change set is unavailable")
                        })?;
                    value.parse::<Id>().map_err(|_| {
                        AppError::new(ErrorCode::IntegrityFailure, "Change artifact ID is invalid")
                    })
                })
                .await?;
            let changes: crate::types::ChangeSet =
                serde_json::from_slice(&artifacts.read_bytes(artifact_id).await?).map_err(
                    |_| AppError::new(ErrorCode::IntegrityFailure, "Stored change set is invalid"),
                )?;
            if req.change_digest != Some(changes.digest) {
                return Err(AppError::new(
                    ErrorCode::PlanChanged,
                    "Approval must bind the actual change-set digest",
                ));
            }
            let paths = changes
                .changes
                .iter()
                .filter(|change| {
                    req.action == ActionClass::AcceptUnverifiedChanges
                        || change.kind == crate::types::FileChangeKind::Delete
                })
                .map(|change| change.path.clone())
                .collect::<Vec<_>>();
            if paths.is_empty() {
                return Err(AppError::new(
                    ErrorCode::InvalidPlan,
                    "No matching actual changes require this approval",
                ));
            }
            Some((artifact_id, paths, changes.digest))
        } else {
            None
        };

        // 1. Fetch job, project, and current policy_version
        let requested_plan_id = req.plan_id;
        let requested_action = req.action;
        let requested_change_digest = req.change_digest;
        let (job_rev, project_id, plan_scope_opt) = self
            .db
            .read(move |conn| {
                let mut stmt = conn
                    .prepare("SELECT revision, project_id, plan_id, state FROM jobs WHERE id = ?1")
                    .map_err(|e| AppError::new(ErrorCode::DbUnavailable, e.to_string()))?;
                let (rev, proj_id_str, active_plan, job_state): (
                    i64,
                    Option<String>,
                    Option<String>,
                    String,
                ) = stmt
                    .query_row(params![job_id.to_string()], |r| {
                        Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
                    })
                    .map_err(|_| AppError::new(ErrorCode::NotFound, "Job not found"))?;

                let proj_id = proj_id_str.and_then(|s| s.parse::<Id>().ok());
                let scope = if let Some((artifact_id, paths, digest)) = actual_change_scope {
                    Some(ApprovalScope {
                        action_classes: vec![requested_action],
                        project_id: proj_id,
                        paths,
                        artifact_ids: vec![artifact_id],
                        operation_digest: Some(digest),
                        ..ApprovalScope::default()
                    })
                } else if requested_action == ActionClass::DeleteHistory {
                    let input: String = conn
                        .query_row(
                            "SELECT input_json FROM jobs WHERE id=?1",
                            params![job_id.to_string()],
                            |row| row.get(0),
                        )
                        .map_err(|_| AppError::new(ErrorCode::NotFound, "Job not found"))?;
                    let value: serde_json::Value = serde_json::from_str(&input).map_err(|_| {
                        AppError::new(ErrorCode::IntegrityFailure, "Stored deletion is invalid")
                    })?;
                    let kind = value
                        .get("kind")
                        .and_then(|item| item.as_str())
                        .unwrap_or("history");
                    let paths = value
                        .get("ids")
                        .and_then(|item| item.as_array())
                        .into_iter()
                        .flatten()
                        .filter_map(|item| item.as_str())
                        .map(|id| format!("{kind}:{id}"))
                        .collect::<Vec<_>>();
                    if paths.is_empty() || requested_change_digest.is_none() {
                        return Err(AppError::new(
                            ErrorCode::InvalidRequest,
                            "Deletion approval has no exact persisted selection",
                        ));
                    }
                    Some(ApprovalScope {
                        action_classes: vec![ActionClass::DeleteHistory],
                        paths,
                        operation_digest: requested_change_digest,
                        ..ApprovalScope::default()
                    })
                } else if matches!(
                    requested_action,
                    ActionClass::RunCommand | ActionClass::RunTest
                ) {
                    let input: String = conn
                        .query_row(
                            "SELECT input_json FROM jobs WHERE id=?1",
                            params![job_id.to_string()],
                            |row| row.get(0),
                        )
                        .map_err(|_| AppError::new(ErrorCode::NotFound, "Job not found"))?;
                    let value: serde_json::Value = serde_json::from_str(&input).map_err(|_| {
                        AppError::new(ErrorCode::IntegrityFailure, "Stored operation is invalid")
                    })?;
                    let command_id = value
                        .get("command_id")
                        .and_then(|value| value.as_str())
                        .and_then(|value| value.parse::<Id>().ok())
                        .ok_or_else(|| {
                            AppError::new(
                                ErrorCode::IntegrityFailure,
                                "Stored command operation has no command ID",
                            )
                        })?;
                    let (command_project, command_kind, enabled): (String, String, bool) = conn
                        .query_row(
                            "SELECT project_id,kind,enabled FROM command_specs WHERE id=?1",
                            params![command_id.to_string()],
                            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                        )
                        .map_err(|_| AppError::new(ErrorCode::NotFound, "Command not found"))?;
                    let expected_action = if matches!(command_kind.as_str(), "test" | "build") {
                        ActionClass::RunTest
                    } else {
                        ActionClass::RunCommand
                    };
                    if !enabled
                        || expected_action != requested_action
                        || Some(command_project) != proj_id.map(|id| id.to_string())
                    {
                        return Err(AppError::new(
                            ErrorCode::ActionDenied,
                            "Command binding changed before approval",
                        ));
                    }
                    Some(ApprovalScope {
                        action_classes: vec![requested_action],
                        project_id: proj_id,
                        command_ids: vec![command_id],
                        operation_digest: requested_change_digest,
                        ..ApprovalScope::default()
                    })
                } else if requested_action == ActionClass::ProjectEdit {
                    let plan_id = requested_plan_id.ok_or_else(|| {
                        AppError::new(
                            ErrorCode::InvalidPlan,
                            "Implementation approval requires a plan",
                        )
                    })?;
                    if active_plan != Some(plan_id.to_string()) {
                        return Err(AppError::new(
                            ErrorCode::PlanChanged,
                            "Requested plan is not the job's active immutable plan",
                        ));
                    }
                    if !matches!(job_state.as_str(), "planned" | "blocked") {
                        return Err(AppError::new(
                            ErrorCode::ResourceInUse,
                            "Implementation approval is not valid in the current job state",
                        ));
                    }
                    let (body, stored_digest, stored_project_revision): (String, String, Revision) = conn
                        .query_row(
                            "SELECT body_json,digest,project_revision FROM plans WHERE id=?1 AND job_id=?2",
                            params![plan_id.to_string(), job_id.to_string()],
                            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                        )
                        .map_err(|_| {
                            AppError::new(ErrorCode::PlanChanged, "Plan was not found for this job")
                        })?;
                    let plan: crate::types::Plan = serde_json::from_str(&body).map_err(|_| {
                        AppError::new(ErrorCode::IntegrityFailure, "Stored plan is invalid")
                    })?;
                    if plan.id != plan_id
                        || plan.job_id != job_id
                        || plan.digest.to_hex() != stored_digest
                        || plan.project_revision != stored_project_revision
                    {
                        return Err(AppError::new(
                            ErrorCode::IntegrityFailure,
                            "Stored plan binding is inconsistent",
                        ));
                    }
                    if !plan.actions.contains(&ActionClass::ProjectEdit) {
                        return Err(AppError::new(
                            ErrorCode::InvalidPlan,
                            "Implementation plan does not authorize project edits",
                        ));
                    }
                    if requested_change_digest != Some(plan.digest) {
                        return Err(AppError::new(
                            ErrorCode::PlanChanged,
                            "Approval must bind the active plan digest",
                        ));
                    }
                    let current_project_revision: Revision = conn
                        .query_row(
                            "SELECT revision FROM projects WHERE id=?1 AND archived=0",
                            params![proj_id.map(|id| id.to_string())],
                            |row| row.get(0),
                        )
                        .map_err(|_| {
                            AppError::new(ErrorCode::PlanChanged, "Plan project is unavailable")
                        })?;
                    if current_project_revision != plan.project_revision {
                        return Err(AppError::new(
                            ErrorCode::PlanChanged,
                            "Project configuration changed after the plan was created",
                        ));
                    }
                    Some(ApprovalScope {
                        action_classes: plan.actions.clone(),
                        project_id: proj_id,
                        project_revision: Some(plan.project_revision),
                        paths: plan.allowed_paths.clone(),
                        command_ids: plan.command_ids.clone(),
                        model_ids: plan.selected_model_id.into_iter().collect(),
                        tool_id: plan.selected_tool_id,
                        artifact_ids: plan.context_artifact_ids.clone(),
                        operation_digest: Some(plan.digest),
                    })
                } else if requested_action == ActionClass::UploadContext {
                    let input: String = conn
                        .query_row(
                            "SELECT input_json FROM jobs WHERE id=?1",
                            params![job_id.to_string()],
                            |row| row.get(0),
                        )
                        .map_err(|_| AppError::new(ErrorCode::NotFound, "Job not found"))?;
                    let value: serde_json::Value = serde_json::from_str(&input).map_err(|_| {
                        AppError::new(ErrorCode::IntegrityFailure, "Stored job input is invalid")
                    })?;
                    let mut artifact_ids: Vec<Id> = value
                        .get("input_artifact_ids")
                        .and_then(|items| items.as_array())
                        .into_iter()
                        .flatten()
                        .filter_map(|item| item.as_str())
                        .filter_map(|item| item.parse().ok())
                        .collect();
                    let mut statement = conn
                        .prepare(
                            "SELECT id FROM artifacts WHERE job_id=?1 AND kind IN ('scan','context') ORDER BY created_at,id",
                        )
                        .map_err(|error| AppError::new(ErrorCode::DbUnavailable, error.to_string()))?;
                    let generated = statement
                        .query_map(params![job_id.to_string()], |row| row.get::<_, String>(0))
                        .map_err(|error| AppError::new(ErrorCode::DbUnavailable, error.to_string()))?
                        .collect::<std::result::Result<Vec<_>, _>>()
                        .map_err(|error| AppError::new(ErrorCode::DbUnavailable, error.to_string()))?;
                    artifact_ids.extend(
                        generated
                            .into_iter()
                            .filter_map(|id| id.parse::<Id>().ok()),
                    );
                    artifact_ids.sort();
                    artifact_ids.dedup();
                    Some(ApprovalScope {
                        action_classes: vec![ActionClass::UploadContext],
                        project_id: proj_id,
                        artifact_ids,
                        ..ApprovalScope::default()
                    })
                } else {
                    None
                };
                Ok((rev, proj_id, scope))
            })
            .await?;

        if job_rev != req.job_revision {
            return Err(AppError::new(
                ErrorCode::StaleRevision,
                "Job revision mismatch",
            ));
        }

        // Fetch policy version
        let policy_version: i64 = self
            .db
            .read(|conn| {
                let mut stmt = conn
                    .prepare("SELECT value_json FROM settings WHERE key = 'policy_version'")
                    .map_err(|e| AppError::new(ErrorCode::DbUnavailable, e.to_string()))?;
                let val: String = stmt.query_row([], |r| r.get(0)).unwrap_or_default();
                let v: serde_json::Value = serde_json::from_str(&val).unwrap_or_default();
                Ok(v.get("policy_version")
                    .and_then(|x| x.as_i64())
                    .unwrap_or(1))
            })
            .await?;

        // Construct scope
        let scope = plan_scope_opt.unwrap_or_else(|| ApprovalScope {
            action_classes: vec![req.action],
            project_id,
            project_revision: None,
            paths: Vec::new(),
            command_ids: Vec::new(),
            model_ids: Vec::new(),
            tool_id: None,
            artifact_ids: Vec::new(),
            operation_digest: req.change_digest,
        });

        let request_digest = Self::compute_digest(
            job_id,
            &req.action,
            &scope,
            policy_version,
            now,
            expires_at,
            req.change_digest,
        )?;

        let scope_json = serde_json::to_string(&scope).unwrap_or_default();
        let action_clean = serde_json::to_string(&req.action)
            .unwrap_or_default()
            .trim_matches('"')
            .to_string();

        let approval = Approval {
            id: approval_id,
            job_id,
            plan_id: req.plan_id,
            action: req.action,
            request_digest,
            scope,
            decision: ApprovalDecisionKind::Pending,
            requested_at: now,
            decided_at: None,
            expires_at,
            deciding_session_id: None,
            revision: 1,
        };

        let app_clone = approval.clone();
        self.db
            .write(move |tx| {
                tx.execute(
                    r#"INSERT INTO approvals (id, job_id, plan_id, action, request_digest, scope_json, policy_version, decision, requested_at, expires_at, revision)
                       VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'pending', ?8, ?9, 1)"#,
                    params![
                        app_clone.id.to_string(),
                        app_clone.job_id.to_string(),
                        app_clone.plan_id.map(|id| id.to_string()),
                        action_clean,
                        app_clone.request_digest.to_hex(),
                        scope_json,
                        policy_version,
                        app_clone.requested_at,
                        app_clone.expires_at,
                    ],
                )
                .map_err(|e| AppError::new(ErrorCode::DbUnavailable, e.to_string()))?;
                tx.execute(
                    "UPDATE jobs SET state='awaiting_approval', approval_purpose=CASE WHEN ?1='project_edit' THEN 'implementation' WHEN ?1='voice_upload' THEN 'voice' ELSE 'operation' END, revision=revision+1, updated_at=?2 WHERE id=?3 AND state<>'awaiting_approval'",
                    params![action_clean, now, app_clone.job_id.to_string()],
                )
                .map_err(|e| AppError::new(ErrorCode::DbUnavailable, e.to_string()))?;
                Ok(())
            })
            .await?;

        Ok(approval)
    }

    pub async fn decide(
        &self,
        id: Id,
        decision: ApprovalDecision,
        session: &SessionContext,
    ) -> Result<Approval> {
        if !decision.owner_ack {
            return Err(AppError::new(
                ErrorCode::ActionDenied,
                "Explicit owner acknowledgement is required",
            ));
        }
        let now = chrono::Utc::now().timestamp_millis();
        let session_id = session.id;
        let session_id_str = session_id.to_string();

        self.db
            .write(move |tx| {
                let mut stmt = tx
                    .prepare(
                        r#"SELECT job_id, plan_id, action, request_digest, scope_json, decision, requested_at, expires_at, revision
                           FROM approvals WHERE id = ?1"#,
                    )
                    .map_err(|e| AppError::new(ErrorCode::DbUnavailable, e.to_string()))?;

                let row = stmt
                    .query_row(params![id.to_string()], |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, Option<String>>(1)?,
                            r.get::<_, String>(2)?,
                            r.get::<_, String>(3)?,
                            r.get::<_, String>(4)?,
                            r.get::<_, String>(5)?,
                            r.get::<_, i64>(6)?,
                            r.get::<_, i64>(7)?,
                            r.get::<_, i64>(8)?,
                        ))
                    })
                    .map_err(|_| AppError::new(ErrorCode::NotFound, "Approval not found"))?;

                let (job_id_str, plan_id_str, action_str, req_dig_str, scope_json, cur_decision, requested_at, expires_at, rev) = row;

                if rev != decision.revision {
                    return Err(AppError::new(
                        ErrorCode::StaleRevision,
                        "Approval revision mismatch",
                    ));
                }

                if cur_decision != "pending" {
                    return Err(AppError::new(
                        ErrorCode::Conflict,
                        format!("Approval already decided: {cur_decision}"),
                    ));
                }

                if req_dig_str != decision.request_digest.to_hex() {
                    return Err(AppError::new(
                        ErrorCode::ActionDenied,
                        "Request digest does not match approval record",
                    ));
                }

                let (job_state, active_plan_id, project_id): (
                    String,
                    Option<String>,
                    Option<String>,
                ) = tx
                    .query_row(
                        "SELECT state,plan_id,project_id FROM jobs WHERE id=?1",
                        params![job_id_str],
                        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                    )
                    .map_err(|_| AppError::new(ErrorCode::NotFound, "Approval job not found"))?;
                if job_state != "awaiting_approval" {
                    return Err(AppError::new(
                        ErrorCode::Conflict,
                        "Job is no longer awaiting this approval",
                    ));
                }

                if decision.decision == ApprovalDecisionKind::Approved
                    && action_str == "project_edit"
                {
                    if active_plan_id != plan_id_str {
                        return Err(AppError::new(
                            ErrorCode::PlanChanged,
                            "The active plan changed after approval was requested",
                        ));
                    }
                    let plan_id = plan_id_str.as_ref().ok_or_else(|| {
                        AppError::new(ErrorCode::InvalidPlan, "Approval has no plan")
                    })?;
                    let (body, stored_digest): (String, String) = tx
                        .query_row(
                            "SELECT body_json,digest FROM plans WHERE id=?1 AND job_id=?2",
                            params![plan_id, job_id_str],
                            |row| Ok((row.get(0)?, row.get(1)?)),
                        )
                        .map_err(|_| {
                            AppError::new(ErrorCode::PlanChanged, "Approved plan is unavailable")
                        })?;
                    let plan: crate::types::Plan = serde_json::from_str(&body).map_err(|_| {
                        AppError::new(ErrorCode::IntegrityFailure, "Stored plan is invalid")
                    })?;
                    if plan.digest.to_hex() != stored_digest {
                        return Err(AppError::new(
                            ErrorCode::IntegrityFailure,
                            "Stored plan digest is inconsistent",
                        ));
                    }
                    let current_project_revision: Revision = tx
                        .query_row(
                            "SELECT revision FROM projects WHERE id=?1 AND archived=0",
                            params![project_id],
                            |row| row.get(0),
                        )
                        .map_err(|_| {
                            AppError::new(ErrorCode::PlanChanged, "Plan project is unavailable")
                        })?;
                    if current_project_revision != plan.project_revision {
                        return Err(AppError::new(
                            ErrorCode::PlanChanged,
                            "Project configuration changed after approval was requested",
                        ));
                    }
                }

                if now >= expires_at {
                    tx.execute(
                        "UPDATE approvals SET decision = 'denied', decided_at = ?1, revision = revision + 1 WHERE id = ?2",
                        params![now, id.to_string()],
                    ).map_err(|e| AppError::new(ErrorCode::DbUnavailable, e.to_string()))?;
                    return Err(AppError::new(
                        ErrorCode::ApprovalExpired,
                        "Approval has expired",
                    ));
                }

                let new_dec_str = match decision.decision {
                    ApprovalDecisionKind::Approved => "approved",
                    ApprovalDecisionKind::Denied => "denied",
                    ApprovalDecisionKind::Revoked => "revoked",
                    _ => {
                        return Err(AppError::new(
                            ErrorCode::InvalidRequest,
                            "Invalid decision for owner approval",
                        ))
                    }
                };

                tx.execute(
                    r#"UPDATE approvals
                       SET decision = ?1, decided_at = ?2, deciding_session_id = ?3, revision = revision + 1
                       WHERE id = ?4"#,
                    params![new_dec_str, now, session_id_str, id.to_string()],
                )
                .map_err(|e| AppError::new(ErrorCode::DbUnavailable, e.to_string()))?;

                let (next_job_state, continuation) = if decision.decision == ApprovalDecisionKind::Approved {
                    match action_str.as_str() {
                        "upload_context" => ("reviewing", None),
                        "project_edit" => ("approved", Some("checkpoint")),
                        "voice_upload" => ("approved", Some("voice_receive")),
                        "delete_project_files" | "restore_checkpoint" | "accept_unverified_changes" => ("approved", Some("promote")),
                        _ => ("approved", Some("operation")),
                    }
                } else {
                    ("blocked", None)
                };
                tx.execute(
                    "UPDATE jobs SET state=?1,continuation=?2,revision=revision+1,updated_at=?3 WHERE id=?4 AND state='awaiting_approval'",
                    params![next_job_state, continuation, now, job_id_str],
                )
                .map_err(|e| AppError::new(ErrorCode::DbUnavailable, e.to_string()))?;

                let job_id = job_id_str.parse::<Id>().unwrap();
                let plan_id = plan_id_str.and_then(|s| s.parse::<Id>().ok());
                let action: ActionClass = serde_json::from_str(&format!("\"{}\"", action_str))
                    .unwrap_or(ActionClass::ProjectEdit);
                let scope: ApprovalScope = serde_json::from_str(&scope_json).unwrap_or_default();

                Ok(Approval {
                    id,
                    job_id,
                    plan_id,
                    action,
                    request_digest: decision.request_digest,
                    scope,
                    decision: decision.decision,
                    requested_at,
                    decided_at: Some(now),
                    expires_at,
                    deciding_session_id: Some(session_id),
                    revision: rev + 1,
                })
            })
            .await
    }

    pub async fn invalidate_project(&self, project_id: Id, _reason: &str) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        let proj_str = project_id.to_string();

        self.db
            .write(move |tx| {
                tx.execute(
                    r#"UPDATE approvals
                       SET decision = 'revoked', decided_at = ?1, revision = revision + 1
                       WHERE decision = 'pending' AND job_id IN (
                           SELECT id FROM jobs WHERE project_id = ?2
                       )"#,
                    params![now, proj_str],
                )
                .map_err(|e| AppError::new(ErrorCode::DbUnavailable, e.to_string()))?;
                Ok(())
            })
            .await
    }
}
