use crate::approval::{ApprovalDecision, ApprovalRequest};
use crate::error::{AppError, ErrorCode, PublicError, Result};
use crate::types::*;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

mod double_option {
    use serde::{Deserialize, Deserializer};

    pub fn deserialize<'de, D, T>(
        deserializer: D,
    ) -> std::result::Result<Option<Option<T>>, D::Error>
    where
        D: Deserializer<'de>,
        T: Deserialize<'de>,
    {
        Option::<T>::deserialize(deserializer).map(Some)
    }
}

fn parse_rfc3339(value: &str, field: &'static str) -> Result<UtcMs> {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|parsed| parsed.timestamp_millis())
        .map_err(|_| {
            AppError::new(
                ErrorCode::InvalidRequest,
                format!("{field} must be RFC3339 with an explicit UTC offset"),
            )
        })
}

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

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduleInput {
    pub task_id: Option<Id>,
    pub kind: ScheduleKind,
    pub label: String,
    pub first_at: String,
    pub timezone: String,
    pub recurrence: Recurrence,
    pub weekdays: Vec<u8>,
    pub local_time: String,
    pub enabled: bool,
}

impl TryFrom<ScheduleInput> for ScheduleDraft {
    type Error = AppError;

    fn try_from(value: ScheduleInput) -> Result<Self> {
        Ok(Self {
            task_id: value.task_id,
            kind: value.kind,
            label: value.label,
            first_at: parse_rfc3339(&value.first_at, "first_at")?,
            timezone: value.timezone,
            recurrence: value.recurrence,
            weekdays: value.weekdays,
            local_time: value.local_time,
            enabled: value.enabled,
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskInput {
    pub title: String,
    pub description: String,
    pub project_id: Option<Id>,
    pub parent_task_id: Option<Id>,
    pub priority: Priority,
    pub due_at: Option<String>,
    pub timezone: String,
    pub estimated_minutes: Option<u32>,
    pub reminder: Option<ScheduleInput>,
}

impl TryFrom<TaskInput> for TaskDraft {
    type Error = AppError;

    fn try_from(value: TaskInput) -> Result<Self> {
        Ok(Self {
            title: value.title,
            description: value.description,
            project_id: value.project_id,
            parent_task_id: value.parent_task_id,
            priority: value.priority,
            due_at: value
                .due_at
                .as_deref()
                .map(|due_at| parse_rfc3339(due_at, "due_at"))
                .transpose()?,
            timezone: value.timezone,
            estimated_minutes: value.estimated_minutes,
            reminder: value.reminder.map(TryInto::try_into).transpose()?,
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskPatchInput {
    pub revision: Revision,
    pub title: Option<String>,
    pub description: Option<String>,
    #[serde(default, deserialize_with = "double_option::deserialize")]
    pub project_id: Option<Option<Id>>,
    #[serde(default, deserialize_with = "double_option::deserialize")]
    pub parent_task_id: Option<Option<Id>>,
    pub priority: Option<Priority>,
    #[serde(default, deserialize_with = "double_option::deserialize")]
    pub due_at: Option<Option<String>>,
    pub timezone: Option<String>,
    #[serde(default, deserialize_with = "double_option::deserialize")]
    pub estimated_minutes: Option<Option<u32>>,
    pub status: Option<TaskStatus>,
    #[serde(default, deserialize_with = "double_option::deserialize")]
    pub progress_pct: Option<Option<u8>>,
    #[serde(default, deserialize_with = "double_option::deserialize")]
    pub progress_source: Option<Option<ProgressSource>>,
    pub checklist: Option<Vec<ChecklistItem>>,
}

impl TryFrom<TaskPatchInput> for TaskPatch {
    type Error = AppError;

    fn try_from(value: TaskPatchInput) -> Result<Self> {
        let due_at = value
            .due_at
            .map(|timestamp| {
                timestamp
                    .as_deref()
                    .map(|timestamp| parse_rfc3339(timestamp, "due_at"))
                    .transpose()
            })
            .transpose()?;
        Ok(Self {
            revision: value.revision,
            title: value.title,
            description: value.description,
            project_id: value.project_id,
            parent_task_id: value.parent_task_id,
            priority: value.priority,
            due_at,
            timezone: value.timezone,
            estimated_minutes: value.estimated_minutes,
            status: value.status,
            progress_pct: value.progress_pct,
            progress_source: value.progress_source,
            checklist: value.checklist,
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SchedulePatchInput {
    pub revision: Revision,
    #[serde(default, deserialize_with = "double_option::deserialize")]
    pub task_id: Option<Option<Id>>,
    pub label: Option<String>,
    pub first_at: Option<String>,
    pub timezone: Option<String>,
    pub recurrence: Option<Recurrence>,
    pub weekdays: Option<Vec<u8>>,
    pub local_time: Option<String>,
    pub enabled: Option<bool>,
}

impl TryFrom<SchedulePatchInput> for SchedulePatch {
    type Error = AppError;

    fn try_from(value: SchedulePatchInput) -> Result<Self> {
        Ok(Self {
            revision: value.revision,
            task_id: value.task_id,
            label: value.label,
            first_at: value
                .first_at
                .as_deref()
                .map(|first_at| parse_rfc3339(first_at, "first_at"))
                .transpose()?,
            timezone: value.timezone,
            recurrence: value.recurrence,
            weekdays: value.weekdays,
            local_time: value.local_time,
            enabled: value.enabled,
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserMessageInput {
    pub text: String,
    pub model_override_id: Option<Id>,
    pub confirmed_task: Option<TaskInput>,
}

impl TryFrom<UserMessageInput> for UserMessage {
    type Error = AppError;

    fn try_from(value: UserMessageInput) -> Result<Self> {
        Ok(Self {
            text: value.text,
            model_override_id: value.model_override_id,
            confirmed_task: value.confirmed_task.map(TryInto::try_into).transpose()?,
        })
    }
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
    pub limit: Option<u16>,
    pub cursor: Option<String>,
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
    pub limit: Option<u16>,
    pub cursor: Option<String>,
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
    pub limit: Option<u16>,
    pub cursor: Option<String>,
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
    pub limit: Option<u16>,
    pub cursor: Option<String>,
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
    pub limit: Option<u16>,
    pub cursor: Option<String>,
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
