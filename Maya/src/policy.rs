use crate::db::Db;
use crate::error::{AppError, ErrorCode, Result};
use crate::types::{ActionClass, Actor, ApprovalScope, Digest, Id, Operation, Revision, UtcMs};
use rusqlite::params;
use std::time::Instant;

#[derive(Debug, Clone)]
pub struct ActionContext {
    pub actor: Actor,
    pub job_id: Id,
    pub step_id: Id,
    pub project_id: Option<Id>,
    pub plan_id: Option<Id>,
    pub approval_ids: Vec<Id>,
    pub policy_version: Revision,
    pub now: UtcMs,
}

#[derive(Debug, Clone)]
pub struct ActionRequest {
    pub class: ActionClass,
    pub scope: ApprovalScope,
    pub request_digest: Digest,
    pub change_digest: Option<Digest>,
}

#[derive(Debug)]
pub struct ExecutionPermit {
    job_id: Id,
    step_id: Id,
    action_classes: Vec<ActionClass>,
    scope: ApprovalScope,
    request_digest: Digest,
    approval_ids: Vec<Id>,
    expires_at: UtcMs,
    deadline: Instant,
}

impl ExecutionPermit {
    pub(crate) fn new(
        job_id: Id,
        step_id: Id,
        action_classes: Vec<ActionClass>,
        scope: ApprovalScope,
        request_digest: Digest,
        approval_ids: Vec<Id>,
        validity: (UtcMs, Instant),
    ) -> Self {
        Self {
            job_id,
            step_id,
            action_classes,
            scope,
            request_digest,
            approval_ids,
            expires_at: validity.0,
            deadline: validity.1,
        }
    }

    pub fn new_test_permit(
        job_id: Id,
        step_id: Id,
        action_classes: Vec<ActionClass>,
        scope: ApprovalScope,
        request_digest: Digest,
        approval_ids: Vec<Id>,
        validity: (UtcMs, Instant),
    ) -> Self {
        Self::new(
            job_id,
            step_id,
            action_classes,
            scope,
            request_digest,
            approval_ids,
            validity,
        )
    }

    pub fn permits(&self, class: &ActionClass, path: Option<&str>) -> Result<()> {
        if Instant::now() > self.deadline {
            return Err(AppError::new(
                ErrorCode::ApprovalExpired,
                "Execution permit deadline expired",
            ));
        }

        if !self.action_classes.contains(class) {
            return Err(AppError::new(
                ErrorCode::ActionDenied,
                format!("Permit does not authorize action {:?}", class),
            ));
        }

        if let Some(p) = path
            && !self.scope.paths.is_empty()
        {
            let norm = p.trim_start_matches('/');
            let matched = self.scope.paths.iter().any(|allowed| {
                let a = allowed.trim_start_matches('/');
                norm == a || norm.starts_with(&format!("{}/", a))
            });
            if !matched {
                return Err(AppError::new(
                    ErrorCode::ScopeDenied,
                    format!("Path '{}' is outside approved scope", p),
                ));
            }
        }

        Ok(())
    }

    pub fn job_id(&self) -> Id {
        self.job_id
    }

    pub fn request_digest(&self) -> Digest {
        self.request_digest
    }

    pub fn approval_ids(&self) -> &[Id] {
        &self.approval_ids
    }

    pub fn step_id(&self) -> Id {
        self.step_id
    }

    pub fn expires_at(&self) -> UtcMs {
        self.expires_at
    }

    pub fn deadline(&self) -> Instant {
        self.deadline
    }

    pub fn scope(&self) -> &ApprovalScope {
        &self.scope
    }
}

#[derive(Clone)]
pub struct Policy {
    db: Db,
}

impl Policy {
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    pub fn classify(operation: &Operation) -> ActionClass {
        match operation {
            Operation::Export { .. } => ActionClass::ExportData,
            Operation::Launch { .. } => ActionClass::LaunchApp,
            Operation::Command { .. } => ActionClass::RunCommand,
            Operation::ClipboardRead {} => ActionClass::ClipboardRead,
            Operation::ClipboardWrite { .. } => ActionClass::ClipboardWrite,
            Operation::Rollback { .. } => ActionClass::RestoreCheckpoint,
            Operation::RestoreDatabase { .. } => ActionClass::RestoreDatabase,
            Operation::VoiceUpload { .. } => ActionClass::VoiceUpload,
        }
    }

    pub async fn authorize(
        &self,
        ctx: &ActionContext,
        request: &ActionRequest,
    ) -> Result<ExecutionPermit> {
        let now = chrono::Utc::now().timestamp_millis();

        // 1. Local read-only actions do not require an approval. Export is deliberately
        // excluded because it releases a selected data set into a downloadable artifact.
        let is_read_only = matches!(
            request.class,
            ActionClass::ReadProject | ActionClass::ReadSystem | ActionClass::ReadArtifact
        );

        if is_read_only && ctx.approval_ids.is_empty() {
            let deadline = Instant::now() + std::time::Duration::from_secs(60);
            return Ok(ExecutionPermit::new(
                ctx.job_id,
                ctx.step_id,
                vec![request.class],
                request.scope.clone(),
                request.request_digest,
                vec![],
                (now + 60_000, deadline),
            ));
        }

        // 2. Validate approval IDs from DB
        if ctx.approval_ids.is_empty() {
            return Err(AppError::new(
                ErrorCode::ApprovalRequired,
                "Action requires explicit owner approval",
            ));
        }

        let approval_id = ctx.approval_ids[0];

        let (
            approval_job_id,
            approval_plan_id,
            decision,
            expires_at,
            db_req_digest,
            scope_json,
            pol_ver,
        ) = self
            .db
            .read(move |conn| {
                let mut stmt = conn
                    .prepare(
                        r#"SELECT job_id, plan_id, decision, expires_at, request_digest, scope_json, policy_version
                           FROM approvals WHERE id = ?1"#,
                    )
                    .map_err(|e| AppError::new(ErrorCode::DbUnavailable, e.to_string()))?;

                stmt.query_row(params![approval_id.to_string()], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, Option<String>>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, i64>(3)?,
                        r.get::<_, String>(4)?,
                        r.get::<_, String>(5)?,
                        r.get::<_, i64>(6)?,
                    ))
                })
                .map_err(|_| AppError::new(ErrorCode::NotFound, "Approval not found"))
            })
            .await?;

        if approval_job_id != ctx.job_id.to_string() {
            return Err(AppError::new(
                ErrorCode::ActionDenied,
                "Approval belongs to a different job",
            ));
        }
        let expected_plan = ctx.plan_id.map(|id| id.to_string());
        if approval_plan_id != expected_plan {
            return Err(AppError::new(
                ErrorCode::PlanChanged,
                "Approval plan binding does not match the active job plan",
            ));
        }

        if decision != "approved" {
            return Err(AppError::new(
                ErrorCode::ActionDenied,
                format!("Approval is not in approved state (status: {decision})"),
            ));
        }

        if now >= expires_at {
            return Err(AppError::new(
                ErrorCode::ApprovalExpired,
                "Approval has expired",
            ));
        }

        // Verify policy version
        if pol_ver != ctx.policy_version {
            return Err(AppError::new(
                ErrorCode::StaleRevision,
                "Policy version changed since approval was granted",
            ));
        }

        // Verify request digest matches
        if db_req_digest != request.request_digest.to_hex() {
            return Err(AppError::new(
                ErrorCode::ActionDenied,
                "Request digest mismatch with approval record",
            ));
        }

        let approved_scope: ApprovalScope = serde_json::from_str(&scope_json).map_err(|_| {
            AppError::new(
                ErrorCode::IntegrityFailure,
                "Stored approval scope is invalid",
            )
        })?;
        if approved_scope != request.scope {
            return Err(AppError::new(
                ErrorCode::ActionDenied,
                "Requested scope differs from the approved scope",
            ));
        }
        if !approved_scope.action_classes.contains(&request.class) {
            return Err(AppError::new(
                ErrorCode::ActionDenied,
                "Requested action is outside the approved scope",
            ));
        }
        if approved_scope.project_id != ctx.project_id {
            return Err(AppError::new(
                ErrorCode::ActionDenied,
                "Approval project binding does not match the active job",
            ));
        }
        if approved_scope.operation_digest != request.change_digest {
            return Err(AppError::new(
                ErrorCode::ActionDenied,
                "Effect digest differs from the approved effect",
            ));
        }

        let remaining_ms = (expires_at - now).max(0) as u64;
        let deadline = Instant::now() + std::time::Duration::from_millis(remaining_ms);

        Ok(ExecutionPermit::new(
            ctx.job_id,
            ctx.step_id,
            approved_scope.action_classes.clone(),
            approved_scope,
            request.request_digest,
            ctx.approval_ids.clone(),
            (expires_at, deadline),
        ))
    }

    pub fn redact(&self, text: &str) -> String {
        Self::redact_static(text)
    }

    pub fn redact_static(text: &str) -> String {
        let stripped = strip_ansi(text);
        redact_tokens(&stripped)
    }
}

fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1B'
            && let Some(&next_ch) = chars.peek()
        {
            if next_ch == '[' {
                chars.next(); // consume '['
                // consume parameters and intermediates
                while let Some(&c) = chars.peek() {
                    if ('\x20'..='\x3F').contains(&c) {
                        chars.next();
                    } else {
                        break;
                    }
                }
                // consume final byte
                if let Some(&c) = chars.peek()
                    && ('\x40'..='\x7E').contains(&c)
                {
                    chars.next();
                }
                continue;
            } else if next_ch == ']' {
                chars.next(); // consume ']'
                while let Some(c) = chars.next() {
                    if c == '\x07' {
                        break;
                    }
                    if c == '\x1B' && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
                continue;
            } else {
                chars.next();
                continue;
            }
        }
        out.push(ch);
    }
    out
}

fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.'
}

fn redact_tokens(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut idx = 0;
    let bytes = text.as_bytes();
    let len = bytes.len();

    while idx < len {
        if text[idx..].starts_with("Bearer ") {
            let start_tok = idx + 7;
            let mut end_tok = start_tok;
            while end_tok < len && is_token_char(bytes[end_tok] as char) {
                end_tok += 1;
            }
            if end_tok - start_tok >= 20 {
                result.push_str("Bearer [REDACTED SECRET]");
                idx = end_tok;
                continue;
            }
        }
        if text[idx..].starts_with("sk-") {
            let start_tok = idx;
            let mut end_tok = idx + 3;
            while end_tok < len && is_token_char(bytes[end_tok] as char) {
                end_tok += 1;
            }
            if end_tok - start_tok >= 23 {
                result.push_str("[REDACTED SECRET]");
                idx = end_tok;
                continue;
            }
        }
        if text[idx..].starts_with("AIza") {
            let start_tok = idx;
            let mut end_tok = idx + 4;
            while end_tok < len && is_token_char(bytes[end_tok] as char) {
                end_tok += 1;
            }
            if end_tok - start_tok >= 35 {
                result.push_str("[REDACTED SECRET]");
                idx = end_tok;
                continue;
            }
        }
        if text[idx..].starts_with("ghp_") {
            let start_tok = idx;
            let mut end_tok = idx + 4;
            while end_tok < len && is_token_char(bytes[end_tok] as char) {
                end_tok += 1;
            }
            if end_tok - start_tok >= 36 {
                result.push_str("[REDACTED SECRET]");
                idx = end_tok;
                continue;
            }
        }

        let ch = text[idx..].chars().next().unwrap();
        result.push(ch);
        idx += ch.len_utf8();
    }
    result
}
