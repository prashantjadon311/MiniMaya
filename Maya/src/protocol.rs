use crate::approval::{ApprovalDecision, ApprovalRequest};
use crate::error::{AppError, ErrorCode, PublicError, Result};
use crate::types::*;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize)]
pub struct ApiMeta {
    pub request_id: Uuid,
    pub version: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct ApiEnvelope<T> {
    pub data: T,
    pub meta: ApiMeta,
}

#[derive(Debug, Clone, Serialize)]
pub struct ApiErrorEnvelope {
    pub error: PublicError,
    pub meta: ApiMeta,
}

impl ApiMeta {
    pub fn new(request_id: Uuid) -> Self {
        Self {
            request_id,
            version: "v1",
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Empty {}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct IdPath {
    pub id: Id,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionExchange {
    pub bootstrap_token: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionInfo {
    pub session_id: Id,
    pub session_token: String,
    pub csrf_token: String,
    pub expires_at: UtcMs,
}

#[derive(Debug, Clone, Serialize)]
pub struct Health {
    pub ready: bool,
    pub version: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct Capabilities {
    pub items: Vec<Capability>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProbeKind {
    All,
    Provider,
    Tool,
    Browser,
    Voice,
    Desktop,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProbeInput {
    pub kind: ProbeKind,
    pub id: Option<Id>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsPatch {
    pub revision: Revision,
    pub identity: Option<Identity>,
    pub voice: Option<VoiceSettings>,
    pub assistant: Option<AssistantSettings>,
    pub security: Option<SecuritySettings>,
    pub browser: Option<BrowserSettings>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecretWrite {
    pub secret: String,
    pub mode: SecretMode,
}

#[derive(Debug, Clone, Serialize)]
pub struct SecretStatus {
    pub credential_status: CredentialStatus,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfirmRevision {
    pub revision: Revision,
    pub owner_ack: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelQuery {
    #[serde(flatten)]
    pub page: ListQuery,
    pub provider_id: Option<Id>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandInput {
    pub project_id: Id,
    pub label: String,
    pub executable: String,
    pub args: Vec<String>,
    pub timeout_ms: u32,
    pub network: bool,
    pub enabled: bool,
    pub kind: CommandKind,
    pub owner_ack: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandQuery {
    #[serde(flatten)]
    pub page: ListQuery,
    pub project_id: Option<Id>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventInput {
    pub kind: TaskEventKind,
    pub text: Option<String>,
    pub percent: Option<u8>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduleQuery {
    #[serde(flatten)]
    pub page: ListQuery,
    pub task_id: Option<Id>,
    pub kind: Option<ScheduleKind>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlannerQuery {
    pub date: String,
    pub timezone: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DailyReportQuery {
    pub date: String,
    pub timezone: String,
    pub section: Option<ReportSection>,
    pub cursor: Option<String>,
    pub limit: Option<u16>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeeklyReportQuery {
    pub week: String,
    pub timezone: String,
    pub section: Option<ReportSection>,
    pub cursor: Option<String>,
    pub limit: Option<u16>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobQuery {
    #[serde(flatten)]
    pub page: ListQuery,
    pub state: Option<JobState>,
    pub project_id: Option<Id>,
    pub task_id: Option<Id>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct JobInput {
    pub kind: JobKind,
    pub project_id: Option<Id>,
    pub task_id: Option<Id>,
    pub conversation_id: Option<Id>,
    pub goal: String,
    pub mode: JobMode,
    pub chain_id: Option<Id>,
    pub model_override_id: Option<Id>,
    pub tool_override_id: Option<Id>,
    pub input_artifact_ids: Vec<Id>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobMode {
    PlanOnly,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanPut {
    pub revision: Revision,
    pub plan: PlanDraft,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobAction {
    pub revision: Revision,
    pub action: JobControl,
    pub step_id: Option<Id>,
    pub reconciliation: Option<ReconciliationDecision>,
    pub checkpoint_id: Option<Id>,
}

#[derive(Debug, Clone, Serialize)]
pub struct JobDetail {
    pub job: Job,
    pub steps: Vec<JobStep>,
    pub plans: Vec<Plan>,
    pub approvals: Vec<Approval>,
    pub artifacts: Vec<Artifact>,
    pub changes: Option<ChangeSet>,
    pub allowed_actions: Vec<JobControl>,
    pub usage: Vec<UsageRecord>,
    pub cursor: i64,
    pub truncated: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventQuery {
    pub after: Option<i64>,
    pub job_id: Option<Id>,
    #[serde(default)]
    pub resources: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileReadQuery {
    pub path: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceQuery {
    #[serde(default)]
    pub include_processes: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoiceMeta {
    pub language: Language,
    pub model_id: Id,
    pub approval_id: Id,
    pub payload_digest: Digest,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsageQuery {
    #[serde(flatten)]
    pub page: ListQuery,
    pub provider_id: Option<Id>,
    pub job_id: Option<Id>,
    pub since: Option<UtcMs>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovalRequestBody {
    pub job_revision: Revision,
    pub action: ActionClass,
    pub plan_id: Option<Id>,
    pub change_digest: Option<Digest>,
    pub ttl_minutes: u8,
}

impl From<ApprovalRequestBody> for ApprovalRequest {
    fn from(value: ApprovalRequestBody) -> Self {
        Self {
            job_revision: value.job_revision,
            action: value.action,
            plan_id: value.plan_id,
            change_digest: value.change_digest,
            ttl_minutes: value.ttl_minutes,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovalDecisionBody {
    pub revision: Revision,
    pub request_digest: Digest,
    pub decision: ApprovalDecisionWire,
    pub owner_ack: bool,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecisionWire {
    Approve,
    Deny,
}

impl TryFrom<ApprovalDecisionBody> for ApprovalDecision {
    type Error = AppError;

    fn try_from(value: ApprovalDecisionBody) -> Result<Self> {
        if !value.owner_ack {
            return Err(AppError::new(
                ErrorCode::ActionDenied,
                "Explicit owner acknowledgement is required",
            ));
        }
        Ok(Self {
            revision: value.revision,
            request_digest: value.request_digest,
            decision: match value.decision {
                ApprovalDecisionWire::Approve => ApprovalDecisionKind::Approved,
                ApprovalDecisionWire::Deny => ApprovalDecisionKind::Denied,
            },
            owner_ack: true,
        })
    }
}

impl SettingsPatch {
    pub fn apply(self, mut settings: Settings) -> Result<Settings> {
        if self.revision != settings.revision {
            return Err(AppError::new(
                ErrorCode::StaleRevision,
                "Settings revision is stale",
            ));
        }
        if let Some(value) = self.identity {
            settings.identity = value;
        }
        if let Some(value) = self.voice {
            settings.voice = value;
        }
        if let Some(value) = self.assistant {
            settings.assistant = value;
        }
        if let Some(value) = self.security {
            settings.security = value;
        }
        if let Some(value) = self.browser {
            settings.browser = value;
        }
        settings.revision += 1;
        crate::config::validate_settings(&settings)?;
        Ok(settings)
    }
}

pub fn checked_limit(limit: Option<u16>) -> Result<u16> {
    let value = limit.unwrap_or(50);
    if !(1..=100).contains(&value) {
        return Err(AppError::new(
            ErrorCode::InvalidRequest,
            "limit must be between 1 and 100",
        ));
    }
    Ok(value)
}

pub fn require_ack(owner_ack: bool) -> Result<()> {
    if owner_ack {
        Ok(())
    } else {
        Err(AppError::new(
            ErrorCode::ActionDenied,
            "Explicit owner acknowledgement is required",
        ))
    }
}
