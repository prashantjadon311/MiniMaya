use crate::error::{AppError, ErrorCode, Result};
use serde::{Deserialize, Serialize, de::Deserializer, ser::Serializer};
use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;
use uuid::Uuid;

pub type Id = Uuid;
pub type UtcMs = i64;
pub type Revision = i64;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Digest(pub [u8; 32]);

impl Digest {
    pub const ZERO: Self = Self([0u8; 32]);

    pub fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    pub fn from_hex(s: &str) -> Result<Self> {
        if s.len() != 64 {
            return Err(AppError::new(
                ErrorCode::InvalidRequest,
                "Digest must be exactly 64 hex characters",
            ));
        }
        let mut bytes = [0u8; 32];
        hex::decode_to_slice(s, &mut bytes)
            .map_err(|_| AppError::new(ErrorCode::InvalidRequest, "Invalid hex in digest"))?;
        Ok(Self(bytes))
    }

    pub fn of(data: &[u8]) -> Self {
        use sha2::{Digest as _, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(data);
        let result = hasher.finalize();
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&result);
        Self(bytes)
    }
}

impl std::ops::Deref for Digest {
    type Target = [u8; 32];
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl fmt::Debug for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Digest({})", self.to_hex())
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

impl FromStr for Digest {
    type Err = AppError;
    fn from_str(s: &str) -> Result<Self> {
        Self::from_hex(s)
    }
}

impl From<[u8; 32]> for Digest {
    fn from(b: [u8; 32]) -> Self {
        Self(b)
    }
}

impl Serialize for Digest {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for Digest {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Digest::from_hex(&s).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum PatchField<T> {
    #[default]
    Unchanged,
    Set(T),
    Clear,
}

impl<T> PatchField<T> {
    pub fn is_unchanged(&self) -> bool {
        matches!(self, PatchField::Unchanged)
    }

    pub fn apply_to(self, target: &mut Option<T>) {
        match self {
            PatchField::Unchanged => {}
            PatchField::Set(v) => *target = Some(v),
            PatchField::Clear => *target = None,
        }
    }

    pub fn apply_to_val(self, target: &mut T) {
        match self {
            PatchField::Unchanged | PatchField::Clear => {}
            PatchField::Set(v) => *target = v,
        }
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for PatchField<T> {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Option::<Option<T>>::deserialize(deserializer).map(|opt| match opt {
            Some(Some(v)) => PatchField::Set(v),
            Some(None) => PatchField::Clear,
            None => PatchField::Unchanged,
        })
    }
}

impl<T: Serialize> Serialize for PatchField<T> {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            PatchField::Unchanged => serializer.serialize_none(),
            PatchField::Set(v) => serializer.serialize_some(v),
            PatchField::Clear => serializer.serialize_none(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MoneySource {
    Reported,
    UserRateEstimate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Money {
    pub coefficient: i64,
    pub scale: u8,
    pub currency: String,
    pub source: MoneySource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorKind {
    Owner,
    Scheduler,
    Provider,
    CodingCli,
    System,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Actor {
    pub kind: ActorKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Id>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    #[default]
    English,
    Hindi,
    Hinglish,
}

impl std::fmt::Display for Language {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::English => write!(f, "en"),
            Self::Hindi => write!(f, "hi"),
            Self::Hinglish => write!(f, "hi-en"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Tone {
    #[default]
    Concise,
    Standard,
    Detailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    Low,
    #[default]
    Normal,
    High,
    Urgent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    #[default]
    Todo,
    InProgress,
    Blocked,
    Done,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgressSource {
    Manual,
    Observed,
    Verified,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityState {
    Available,
    Unavailable,
    NeedsSetup,
    Unsupported,
    PermissionDenied,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceInfo {
    pub id: String,
    pub label: String,
    pub languages: Vec<Language>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CapabilityDetails {
    pub protocols: Vec<String>,
    pub languages: Vec<Language>,
    pub voices: Vec<VoiceInfo>,
    pub operations: Vec<String>,
    pub limitations: Vec<String>,
    pub sandbox_verified: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capability {
    pub key: String,
    pub state: CapabilityState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub checked_at: UtcMs,
    pub details: CapabilityDetails,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    pub name: String,
    pub greeting: String,
    pub language: Language,
    pub tone: Tone,
}

impl Default for Identity {
    fn default() -> Self {
        Self {
            name: "Maya".to_string(),
            greeting: "Hello, I am Maya Mini. How can I help you today?".to_string(),
            language: Language::English,
            tone: Tone::Concise,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SttMode {
    #[default]
    Disabled,
    OpenaiAudio,
    CompatibleAudio,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TtsMode {
    #[default]
    Disabled,
    Browser,
    EspeakNg,
    OpenaiAudio,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VoiceSettings {
    pub stt: SttMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stt_model_id: Option<Id>,
    pub tts: TtsMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tts_model_id: Option<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voice: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_device: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_device: Option<String>,
    pub rate: f32,
    pub muted: bool,
    pub language: Language,
}

impl Default for VoiceSettings {
    fn default() -> Self {
        Self {
            stt: SttMode::Disabled,
            stt_model_id: None,
            tts: TtsMode::Disabled,
            tts_model_id: None,
            voice: None,
            input_device: None,
            output_device: None,
            rate: 1.0,
            muted: false,
            language: Language::English,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ReportDelivery {
    #[default]
    InApp,
    InAppAndNotification,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssistantSettings {
    pub timezone: String,
    pub work_days: Vec<u8>,
    pub work_start: String,
    pub work_end: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checkin_minutes: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub daily_review_time: Option<String>,
    pub notifications: bool,
    pub report_delivery: ReportDelivery,
}

impl Default for AssistantSettings {
    fn default() -> Self {
        Self {
            timezone: "Asia/Kolkata".to_string(),
            work_days: vec![1, 2, 3, 4, 5],
            work_start: "09:00".to_string(),
            work_end: "18:00".to_string(),
            checkin_minutes: Some(120),
            daily_review_time: Some("17:30".to_string()),
            notifications: true,
            report_delivery: ReportDelivery::InApp,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecuritySettings {
    pub require_plan_approval: bool,
    pub critical_always_confirm: bool,
    pub chat_retention_days: u16,
    pub event_retention_days: u16,
    pub artifact_retention_days: u16,
    pub backup_keep: u8,
    pub storage_cap_bytes: u64,
}

impl Default for SecuritySettings {
    fn default() -> Self {
        Self {
            require_plan_approval: true,
            critical_always_confirm: true,
            chat_retention_days: 90,
            event_retention_days: 365,
            artifact_retention_days: 30,
            backup_keep: 7,
            storage_cap_bytes: 1_000_000_000, // 1 GB
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CdpMode {
    #[default]
    Disabled,
    Owned,
    Attach,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserSettings {
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executable: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_dir: Option<String>,
    pub cdp_mode: CdpMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cdp_url: Option<String>,
}

impl Default for BrowserSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            executable: None,
            profile_dir: None,
            cdp_mode: CdpMode::Disabled,
            cdp_url: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    pub revision: Revision,
    pub identity: Identity,
    pub voice: VoiceSettings,
    pub assistant: AssistantSettings,
    pub security: SecuritySettings,
    pub browser: BrowserSettings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    OpenaiResponses,
    OpenaiCompatibleChat,
    Gemini,
    Anthropic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BillingMode {
    Api,
    SubscriptionCli,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Chat,
    Planning,
    Review,
    Coding,
    Fallback,
    Stt,
    Tts,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialStatus {
    Missing,
    Session,
    Keyring,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretMode {
    Keyring,
    Session,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provider {
    pub id: Id,
    pub label: String,
    pub kind: ProviderKind,
    pub protocol: String,
    pub endpoint: String,
    pub local_endpoint_allowed: bool,
    pub enabled: bool,
    pub billing_mode: BillingMode,
    pub credential_status: CredentialStatus,
    pub timeout_ms: u32,
    pub revision: Revision,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedEndpoint {
    pub url: url::Url,
    pub resolved_addresses: Vec<std::net::IpAddr>,
    pub origin: String,
    pub credential_generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderInput {
    pub label: String,
    pub kind: ProviderKind,
    pub protocol: String,
    pub endpoint: String,
    pub local_endpoint_allowed: bool,
    pub enabled: bool,
    pub billing_mode: BillingMode,
    pub timeout_ms: u32,
    pub owner_ack: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderPatch {
    pub revision: Revision,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_endpoint_allowed: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelInput {
    pub provider_id: Id,
    pub model_id: String,
    pub label: String,
    pub enabled: bool,
    pub capabilities: Vec<ModelCapability>,
    pub context_limit_bytes: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelPatch {
    pub revision: Revision,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<Vec<ModelCapability>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_limit_bytes: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RolesPut {
    pub revision: Revision,
    pub bindings: Vec<RoleBinding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChainInput {
    pub name: String,
    pub enabled: bool,
    pub steps: Vec<ChainStep>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChainPut {
    pub revision: Revision,
    pub name: String,
    pub enabled: bool,
    pub steps: Vec<ChainStep>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolPut {
    pub revision: Revision,
    pub kind: ToolKind,
    pub executable: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelCapability {
    Text,
    StructuredEdits,
    Stt,
    Tts,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Model {
    pub id: Id,
    pub provider_id: Id,
    pub model_id: String,
    pub label: String,
    pub enabled: bool,
    pub capabilities: Vec<ModelCapability>,
    pub context_limit_bytes: u32,
    pub revision: Revision,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleBinding {
    pub role: Role,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_id: Option<Id>,
    pub fallback_model_ids: Vec<Id>,
    pub allow_paid_fallback: bool,
    pub timeout_ms: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RolesView {
    pub revision: Revision,
    pub bindings: Vec<RoleBinding>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChainPurpose {
    Research,
    Review,
    Plan,
    Code,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChainStep {
    pub position: u8,
    pub purpose: ChainPurpose,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_id: Option<Id>,
    pub max_output_bytes: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Chain {
    pub id: Id,
    pub name: String,
    pub enabled: bool,
    pub steps: Vec<ChainStep>,
    pub revision: Revision,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    Codex,
    CompatibleCli,
    Editor,
    Chromium,
    Pdf,
    Clipboard,
    Tts,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tool {
    pub id: Id,
    pub kind: ToolKind,
    pub executable: String,
    pub enabled: bool,
    pub adapter_version: String,
    pub capability: Capability,
    pub revision: Revision,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandKind {
    Test,
    Build,
    Read,
    Custom,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandSpec {
    pub id: Id,
    pub project_id: Id,
    pub label: String,
    pub executable: String,
    pub args: Vec<String>,
    pub timeout_ms: u32,
    pub network: bool,
    pub enabled: bool,
    pub kind: CommandKind,
    pub revision: Revision,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub id: Id,
    pub name: String,
    pub root: String,
    pub root_device: u64,
    pub root_inode: u64,
    pub allowed_paths: Vec<String>,
    pub excluded_paths: Vec<String>,
    pub notes: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preferred_model_id: Option<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preferred_tool_id: Option<Id>,
    pub archived: bool,
    pub revision: Revision,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectInput {
    pub name: String,
    pub root: String,
    pub allowed_paths: Vec<String>,
    pub excluded_paths: Vec<String>,
    pub notes: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preferred_model_id: Option<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preferred_tool_id: Option<Id>,
    pub owner_ack: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectPatch {
    pub revision: Revision,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_paths: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub excluded_paths: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preferred_model_id: Option<Option<Id>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preferred_tool_id: Option<Option<Id>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived: Option<bool>,
    pub owner_ack: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Conversation {
    pub id: Id,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_override_id: Option<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_task_id: Option<Id>,
    pub summary: String,
    pub created_at: UtcMs,
    pub updated_at: UtcMs,
    pub revision: Revision,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ConversationInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_override_id: Option<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_task_id: Option<Id>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationPatch {
    pub revision: Revision,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<Option<Id>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_override_id: Option<Option<Id>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_task_id: Option<Option<Id>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    Owner,
    Assistant,
    System,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Intent {
    Question,
    Task,
    Reminder,
    ProjectWork,
    SystemAction,
    Clarify,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub id: Id,
    pub conversation_id: Id,
    pub role: MessageRole,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intent: Option<Intent>,
    pub created_at: UtcMs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentSource {
    Rule,
    Model,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IntentDraft {
    pub intent: Intent,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<TaskDraft>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation: Option<Operation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clarification: Option<String>,
    pub source: IntentSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChecklistItem {
    pub id: Id,
    pub text: String,
    pub done: bool,
    pub position: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskDraft {
    pub title: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_task_id: Option<Id>,
    pub priority: Priority,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due_at: Option<UtcMs>,
    pub timezone: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated_minutes: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reminder: Option<ScheduleDraft>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    pub id: Id,
    pub title: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_task_id: Option<Id>,
    pub priority: Priority,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due_at: Option<UtcMs>,
    pub timezone: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated_minutes: Option<u32>,
    pub status: TaskStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress_pct: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress_source: Option<ProgressSource>,
    pub checklist: Vec<ChecklistItem>,
    pub revision: Revision,
    pub created_at: UtcMs,
    pub updated_at: UtcMs,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<UtcMs>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskEventKind {
    Note,
    Decision,
    Blocker,
    Status,
    Progress,
    Reschedule,
    Evidence,
    Checkin,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct EventPayload {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub percent: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<ProgressSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_status: Option<TaskStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_status: Option<TaskStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due_at: Option<UtcMs>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskEvent {
    pub id: Id,
    pub task_id: Id,
    pub kind: TaskEventKind,
    pub payload: EventPayload,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence_artifact_id: Option<Id>,
    pub actor: Actor,
    pub at: UtcMs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleKind {
    Reminder,
    Checkin,
    DailyReview,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Recurrence {
    Once,
    Daily,
    Weekdays,
    Weekly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduleDraft {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<Id>,
    pub kind: ScheduleKind,
    pub label: String,
    pub first_at: UtcMs,
    pub timezone: String,
    pub recurrence: Recurrence,
    pub weekdays: Vec<u8>,
    pub local_time: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Schedule {
    pub id: Id,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<Id>,
    pub kind: ScheduleKind,
    pub label: String,
    pub first_at: UtcMs,
    pub timezone: String,
    pub recurrence: Recurrence,
    pub weekdays: Vec<u8>,
    pub local_time: String,
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_at: Option<UtcMs>,
    pub revision: Revision,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OccurrenceState {
    Pending,
    Delivered,
    Missed,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationStatus {
    NotAttempted,
    Accepted,
    Unavailable,
    Failed,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Occurrence {
    pub id: Id,
    pub schedule_id: Id,
    pub scheduled_at: UtcMs,
    pub state: OccurrenceState,
    pub notification_status: NotificationStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checkin_id: Option<Id>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checkin {
    pub id: Id,
    pub task_id: Id,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occurrence_id: Option<Id>,
    pub requested_at: UtcMs,
    pub revision: Revision,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub responded_at: Option<UtcMs>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub percent: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocker: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckinStatus {
    pub checkin: Checkin,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occurrence: Option<Occurrence>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    Preference,
    Decision,
    Note,
    Summary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemorySource {
    Owner,
    ConversationSummary,
    TaskEvent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryItem {
    pub id: Id,
    pub kind: MemoryKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<Id>,
    pub text: String,
    pub source: MemorySource,
    pub revision: Revision,
    pub created_at: UtcMs,
    pub updated_at: UtcMs,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportEntry {
    pub task_id: Id,
    pub event_id: Id,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact_id: Option<Id>,
    pub text: String,
    pub source: ProgressSource,
    pub at: UtcMs,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportProgress {
    pub task_id: Id,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub percent: Option<u8>,
    pub source: ProgressSource,
    pub evidence_event_id: Id,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportSection {
    Completed,
    Pending,
    Delayed,
    Blockers,
    WorkLog,
    Progress,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SectionPage {
    pub total: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ReportSections {
    pub completed: SectionPage,
    pub pending: SectionPage,
    pub delayed: SectionPage,
    pub blockers: SectionPage,
    pub work_log: SectionPage,
    pub progress: SectionPage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    pub period_start: UtcMs,
    pub period_end: UtcMs,
    pub timezone: String,
    pub completed: Vec<Id>,
    pub pending: Vec<Id>,
    pub delayed: Vec<Id>,
    pub blockers: Vec<ReportEntry>,
    pub work_log: Vec<ReportEntry>,
    pub progress: Vec<ReportProgress>,
    pub sections: ReportSections,
    pub generated_at: UtcMs,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserMessage {
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_override_id: Option<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confirmed_task: Option<TaskDraft>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MessageReceipt {
    pub message: Message,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assistant_message: Option<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub draft: Option<IntentDraft>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<Task>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<Id>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskPatch {
    pub revision: Revision,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<Option<Id>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_task_id: Option<Option<Id>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<Priority>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due_at: Option<Option<UtcMs>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated_minutes: Option<Option<u32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<TaskStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress_pct: Option<Option<u8>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress_source: Option<Option<ProgressSource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checklist: Option<Vec<ChecklistItem>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ListQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TaskQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<TaskStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_task_id: Option<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
    pub limit: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Planner {
    pub date: String,
    pub timezone: String,
    pub overdue: Vec<Task>,
    pub today: Vec<Task>,
    pub upcoming: Vec<Task>,
    pub recommended: Vec<Id>,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckinReply {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checkin_id: Option<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<Revision>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub percent: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocker: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchedulePatch {
    pub revision: Revision,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<Option<Id>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_at: Option<UtcMs>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recurrence: Option<Recurrence>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weekdays: Option<Vec<u8>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CheckinQuery {
    pub limit: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    pub pending_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryInput {
    pub kind: MemoryKind,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<Id>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryPatch {
    pub revision: Revision,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MemoryQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub q: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<MemoryKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<Id>,
    pub limit: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ReportPageQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub section: Option<ReportSection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    pub limit: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    Coding,
    Chain,
    Chat,
    Operation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    #[default]
    Created,
    Inspecting,
    Reviewing,
    Planned,
    AwaitingApproval,
    Approved,
    Checkpointing,
    Executing,
    Verifying,
    Promoting,
    Reporting,
    Completed,
    CompletedWithIssues,
    Blocked,
    Paused,
    Failed,
    Cancelling,
    Cancelled,
    NeedsReconciliation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum StepState {
    #[default]
    Pending,
    Running,
    Succeeded,
    Failed,
    Cancelled,
    Uncertain,
    Skipped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Verification {
    #[default]
    NotRun,
    Passed,
    Failed,
    NotConfigured,
    NotApplicable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Continuation {
    Review,
    Checkpoint,
    Promote,
    Operation,
    VoiceReceive,
    Report,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalPurpose {
    Context,
    Implementation,
    Destructive,
    VerificationWaiver,
    Operation,
    Voice,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Job {
    pub id: Id,
    pub kind: JobKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<Id>,
    pub goal: String,
    pub state: JobState,
    pub revision: Revision,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_id: Option<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checkpoint_id: Option<Id>,
    pub verification: Verification,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paused_from: Option<JobState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub continuation: Option<Continuation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approval_purpose: Option<ApprovalPurpose>,
    pub pause_requested: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<ErrorCode>,
    pub created_at: UtcMs,
    pub updated_at: UtcMs,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobStep {
    pub id: Id,
    pub job_id: Id,
    pub position: u16,
    pub kind: String,
    pub state: StepState,
    pub attempt: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<UtcMs>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<UtcMs>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact_id: Option<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<ErrorCode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionClass {
    ReadProject,
    ReadSystem,
    ReadArtifact,
    ExportData,
    ProjectEdit,
    RunTest,
    RunCommand,
    DeleteProjectFiles,
    RestoreCheckpoint,
    DeleteHistory,
    RestoreDatabase,
    AcceptUnverifiedChanges,
    ClipboardRead,
    ClipboardWrite,
    LaunchApp,
    OpenUrl,
    UploadContext,
    BrowserRead,
    BrowserSubmit,
    VoiceUpload,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanStep {
    pub position: u16,
    pub description: String,
    pub expected_paths: Vec<String>,
    pub verification_command_ids: Vec<Id>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanDraft {
    pub summary: String,
    pub steps: Vec<PlanStep>,
    pub allowed_paths: Vec<String>,
    pub actions: Vec<ActionClass>,
    pub command_ids: Vec<Id>,
    pub context_artifact_ids: Vec<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected_model_id: Option<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected_tool_id: Option<Id>,
    pub timeout_ms: u32,
    pub risks: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    pub id: Id,
    pub job_id: Id,
    pub revision: Revision,
    pub goal: String,
    pub summary: String,
    pub steps: Vec<PlanStep>,
    pub allowed_paths: Vec<String>,
    pub actions: Vec<ActionClass>,
    pub command_ids: Vec<Id>,
    pub context_artifact_ids: Vec<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected_model_id: Option<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected_tool_id: Option<Id>,
    pub project_revision: Revision,
    pub baseline_digest: Digest,
    pub timeout_ms: u32,
    pub risks: Vec<String>,
    pub digest: Digest,
    pub created_at: UtcMs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecisionKind {
    Pending,
    Approved,
    Denied,
    Revoked,
    Consumed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ApprovalScope {
    pub action_classes: Vec<ActionClass>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_revision: Option<Revision>,
    pub paths: Vec<String>,
    pub command_ids: Vec<Id>,
    pub model_ids: Vec<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_id: Option<Id>,
    pub artifact_ids: Vec<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_digest: Option<Digest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Approval {
    pub id: Id,
    pub job_id: Id,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_id: Option<Id>,
    pub action: ActionClass,
    pub request_digest: Digest,
    pub scope: ApprovalScope,
    pub decision: ApprovalDecisionKind,
    pub requested_at: UtcMs,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decided_at: Option<UtcMs>,
    pub expires_at: UtcMs,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deciding_session_id: Option<Id>,
    pub revision: Revision,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    Scan,
    Context,
    Plan,
    Review,
    Handoff,
    Checkpoint,
    Changes,
    Diff,
    Test,
    Log,
    Report,
    Export,
    Audio,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProvenanceSource {
    Owner,
    Model,
    Cli,
    Browser,
    ManualHandoff,
    System,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    pub source: ProvenanceSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub observed_at: UtcMs,
}

impl Provenance {
    pub fn system() -> Self {
        Self {
            source: ProvenanceSource::System,
            provider_id: None,
            model_id: None,
            url: None,
            observed_at: chrono::Utc::now().timestamp_millis(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Artifact {
    pub id: Id,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<Id>,
    pub kind: ArtifactKind,
    pub storage_name: String,
    pub sha256: Digest,
    pub size_bytes: u64,
    pub mime: String,
    pub provenance: Provenance,
    pub created_at: UtcMs,
    pub redacted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<UtcMs>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileChangeKind {
    Create,
    Modify,
    Delete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileChange {
    pub path: String,
    pub kind: FileChangeKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<Digest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<Digest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact_id: Option<Id>,
    pub mode: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeSet {
    pub baseline_digest: Digest,
    pub changes: Vec<FileChange>,
    pub digest: Digest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageRecordStatus {
    Succeeded,
    Failed,
    Cancelled,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageRecord {
    pub id: Id,
    pub provider_id: Id,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<Id>,
    pub request_id: Id,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub money: Option<Money>,
    pub billing_mode: BillingMode,
    pub status: UsageRecordStatus,
    pub at: UtcMs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessCategory {
    Core,
    Coding,
    Browser,
    Editor,
    Voice,
    Helper,
    Other,
}

impl ProcessCategory {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Core => "core",
            Self::Coding => "coding",
            Self::Browser => "browser",
            Self::Editor => "editor",
            Self::Voice => "voice",
            Self::Helper | Self::Other => "helper",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessLeaseState {
    Starting,
    Running,
    Exited,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessLease {
    pub id: Id,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<Id>,
    pub category: ProcessCategory,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_ticks: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pgid: Option<u32>,
    pub boot_id: String,
    pub owned: bool,
    pub state: ProcessLeaseState,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rss_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_percent_one_core: Option<f64>,
    pub category: ProcessCategory,
    pub owned: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceMeasurementStatus {
    Ok,
    Partial,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResourceGroup {
    pub category: ProcessCategory,
    pub pids: Vec<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rss_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_percent_one_core: Option<f64>,
    pub measurement_status: ResourceMeasurementStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemResources {
    pub total_ram_bytes: u64,
    pub available_ram_bytes: u64,
    pub storage_total_bytes: u64,
    pub storage_available_bytes: u64,
    pub process_count: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResourceSnapshot {
    pub at: UtcMs,
    pub core: ResourceGroup,
    pub external: Vec<ResourceGroup>,
    pub system: SystemResources,
    pub warnings: Vec<String>,
    pub processes: Vec<ProcessInfo>,
    pub processes_truncated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LaunchAppTarget {
    Editor,
    Browser,
    InstalledTool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoiceUploadMode {
    Stt,
    Tts,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Operation {
    Export {
        selection: ExportSelection,
    },
    Launch {
        app: LaunchAppTarget,
        #[serde(skip_serializing_if = "Option::is_none")]
        tool_id: Option<Id>,
        #[serde(skip_serializing_if = "Option::is_none")]
        project_id: Option<Id>,
        #[serde(skip_serializing_if = "Option::is_none")]
        folder: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        url: Option<String>,
    },
    Command {
        command_id: Id,
    },
    ClipboardRead {},
    ClipboardWrite {
        text: String,
    },
    Rollback {
        job_id: Id,
        checkpoint_id: Id,
        change_digest: Digest,
    },
    RestoreDatabase {
        backup_id: Id,
    },
    VoiceUpload {
        payload_digest: Digest,
        size_bytes: u64,
        model_id: Id,
        mode: VoiceUploadMode,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaunchInput {
    pub app: LaunchAppTarget,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_id: Option<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ClipboardInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessResultDto {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signal: Option<i32>,
    pub timed_out: bool,
    pub cancelled: bool,
    pub stdout_artifact_id: Id,
    pub stderr_artifact_id: Id,
    pub truncated: bool,
    pub children_reaped: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OperationResult {
    pub kind: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact_id: Option<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tab_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit: Option<ProcessResultDto>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OperationReceipt {
    pub job_id: Id,
    pub state: JobState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<OperationResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approval: Option<Approval>,
    pub preview: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserActionKind {
    Open,
    Shortcut,
    Search,
    Read,
    Submit,
    Close,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserSite {
    Chatgpt,
    Gemini,
    Claude,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserAction {
    pub kind: BrowserActionKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tab_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub site: Option<BrowserSite>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<Id>,
    pub owner_ack: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserTabCapability {
    Open,
    Read,
    Submit,
    Close,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserTab {
    pub id: String,
    pub url: String,
    pub title: String,
    pub owned: bool,
    pub capabilities: Vec<BrowserTabCapability>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct FileQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub q: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileEntryKind {
    File,
    Directory,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    pub path: String,
    pub kind: FileEntryKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilePreview {
    pub project_id: Id,
    pub path: String,
    pub mime: String,
    pub text: String,
    pub truncated: bool,
    pub sha256: Digest,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AuditMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<ErrorCode>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ids: Vec<Id>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command_id: Option<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub counts: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditEvent {
    pub seq: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<Id>,
    pub actor: Actor,
    pub action: String,
    pub resource_kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_id: Option<Id>,
    pub decision: String,
    pub metadata: AuditMetadata,
    pub at: UtcMs,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<Id>,
    pub actor: Actor,
    pub action: String,
    pub resource_kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_id: Option<Id>,
    pub decision: String,
    pub metadata: AuditMetadata,
    pub at: UtcMs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackupRecordState {
    Creating,
    Verified,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupRecord {
    pub id: Id,
    pub storage_name: String,
    pub schema_version: i64,
    pub manifest_sha256: Digest,
    pub size_bytes: u64,
    pub state: BackupRecordState,
    pub created_at: UtcMs,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verified_at: Option<UtcMs>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestoreReceipt {
    pub backup_id: Id,
    pub restored_at: UtcMs,
    pub reconciliation_job_ids: Vec<Id>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReceiptResult {
    Archived,
    Disabled,
    Removed,
    Revoked,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Receipt {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Id>,
    pub result: ReceiptResult,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    Json,
    Markdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportKind {
    Daily,
    Weekly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportExportSpec {
    pub kind: ReportKind,
    pub date_or_week: String,
    pub timezone: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportSelection {
    pub format: ExportFormat,
    pub settings: bool,
    pub tasks: Vec<Id>,
    pub memory: Vec<Id>,
    pub reports: Vec<ReportExportSpec>,
    pub include_project_paths: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub all_tasks: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeleteSelectionKind {
    MemoryItems,
    ChatHistory,
    TaskHistory,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteSelection {
    pub kind: DeleteSelectionKind,
    pub ids: Vec<Id>,
    pub include_subtasks: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeletionReceipt {
    pub selection_digest: Digest,
    pub deleted_count: u64,
    pub retained_audit_count: u64,
    pub backup_copies_remain: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoverySummary {
    pub reconciled: Vec<Id>,
    pub needs_owner: Vec<Id>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobControl {
    Start,
    Pause,
    PauseAtBoundary,
    Resume,
    Cancel,
    Retry,
    Reconcile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReconciliationDecision {
    Inspect,
    ContinueVerification,
    Replan,
    Restore,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum JobEvent {
    Dispatch,
    ScanReady {
        artifact_id: Id,
    },
    ContextNeeded,
    ReviewReady {
        plan_id: Id,
    },
    PlanFrozen {
        plan_id: Id,
    },
    ApprovalGranted {
        approval_id: Id,
        purpose: ApprovalPurpose,
    },
    ApprovalDenied {
        approval_id: Id,
    },
    Start,
    CheckpointReady {
        id: Id,
    },
    ExecutionFinished {
        exit: Option<i32>,
        has_changes: bool,
    },
    ChangesReady {
        digest: Digest,
    },
    VerificationFinished {
        status: Verification,
    },
    PromotionFinished {
        complete: bool,
    },
    ReportSaved {
        id: Id,
    },
    PauseRequested,
    PauseBoundary,
    Resume,
    CancelRequested,
    ChildrenStopped {
        effects_uncertain: bool,
    },
    Failed {
        code: ErrorCode,
        effects_uncertain: bool,
    },
    RestartUncertain,
    Reconcile {
        decision: ReconciliationDecision,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notice {
    pub occurrence_id: Id,
    pub title: String,
    pub body: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationReceiptStatus {
    Accepted,
    Unavailable,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationReceipt {
    pub status: NotificationReceiptStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub desktop_id: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScheduleWake {
    Changed(Id),
    ClockChanged,
    Resumed,
    Shutdown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelSelection {
    pub role: Role,
    pub override_id: Option<Id>,
    pub project_id: Option<Id>,
    pub fallback_consent: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelMessageRole {
    System,
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelMessage {
    pub role: ModelMessageRole,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelOutputMode {
    Text,
    EditManifest,
    PlanDraft,
}

#[derive(Debug, Clone)]
pub struct ModelRequest {
    pub request_id: Id,
    pub model: Model,
    pub role: Role,
    pub messages: Vec<ModelMessage>,
    pub max_output_bytes: u32,
    pub timeout_ms: u32,
    pub output: ModelOutputMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UsageObservation {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub money: Option<Money>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelResultFinish {
    Complete,
    Truncated,
}

#[derive(Debug, Clone)]
pub struct ModelResult {
    pub text: String,
    pub actual_model_id: Option<String>,
    pub usage: UsageObservation,
    pub finish: ModelResultFinish,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ProviderCapabilities {
    pub text: bool,
    pub structured_edits: bool,
    pub streaming: bool,
    pub usage: bool,
    pub stt: bool,
    pub tts: bool,
}

#[derive(Debug, Clone)]
pub enum ProviderEvent {
    TextDelta(String),
    Usage(UsageObservation),
    Completed { actual_model_id: Option<String> },
    Refused(String),
    Failed(ErrorCode),
    Ignored,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedModelSource {
    Override,
    Conversation,
    Project,
    Role,
    Fallback,
}

#[derive(Debug, Clone)]
pub struct ResolvedModel {
    pub model: Model,
    pub provider: Provider,
    pub source: ResolvedModelSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checkpoint {
    pub id: Id,
    pub baseline_digest: Digest,
    pub project_revision: Revision,
    pub git_head: Option<String>,
    pub dirty_paths: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct StagedProject {
    pub path: std::path::PathBuf,
    pub checkpoint_id: Id,
    pub baseline_digest: Digest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessLimits {
    pub max_output_bytes: u64,
    pub max_line_bytes: usize,
    pub max_open_files: u64,
    pub max_processes: u64,
    pub max_file_bytes: u64,
    pub address_space_bytes: Option<u64>,
}

impl Default for ProcessLimits {
    fn default() -> Self {
        Self {
            max_output_bytes: 5 * 1024 * 1024, // 5 MiB
            max_line_bytes: 8192,
            max_open_files: 256,
            max_processes: 64,
            max_file_bytes: 64 * 1024 * 1024, // 64 MiB
            address_space_bytes: Some(2 * 1024 * 1024 * 1024), // 2 GiB
        }
    }
}

#[derive(Debug, Clone)]
pub struct SpawnSpec {
    pub executable: std::path::PathBuf,
    pub args: Vec<std::ffi::OsString>,
    pub cwd: std::path::PathBuf,
    pub env: Vec<(std::ffi::OsString, std::ffi::OsString)>,
    pub limits: ProcessLimits,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkMode {
    Denied,
    CliParentOnly,
}

#[derive(Debug, Clone)]
pub struct SandboxSpec {
    pub stage: std::path::PathBuf,
    pub read_roots: Vec<std::path::PathBuf>,
    pub writable_roots: Vec<std::path::PathBuf>,
    pub network: NetworkMode,
    pub deadline: UtcMs,
}

#[derive(Debug)]
pub struct ProcessSpec {
    pub executable: std::path::PathBuf,
    pub args: Vec<std::ffi::OsString>,
    pub cwd: std::path::PathBuf,
    pub env: Vec<(std::ffi::OsString, std::ffi::OsString)>,
    pub stdin: Option<Vec<u8>>,
    pub timeout: std::time::Duration,
    pub limits: ProcessLimits,
    pub sandbox: SandboxSpec,
    pub category: ProcessCategory,
    pub job_id: Option<Id>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessResult {
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
    pub timed_out: bool,
    pub cancelled: bool,
    pub stdout_artifact: Id,
    pub stderr_artifact: Id,
    pub truncated: bool,
    pub children_reaped: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EditManifestOperation {
    Create,
    Replace,
    Delete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditManifestChange {
    pub path: String,
    pub operation: EditManifestOperation,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_before_sha256: Option<Digest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_utf8: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditManifest {
    pub schema_version: u32,
    pub summary: String,
    pub changes: Vec<EditManifestChange>,
}

#[derive(Debug, Clone)]
pub struct CodingRequest {
    pub job_id: Id,
    pub plan: Plan,
    pub stage: StagedProject,
    pub tool: Option<Tool>,
    pub model: Option<Model>,
}

#[derive(Debug, Clone)]
pub struct CodingOutcome {
    pub process: Option<ProcessResult>,
    pub changes: ChangeSet,
    pub summary: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScanLimits {
    pub entries: u32,
    pub per_file_bytes: u64,
    pub total_bytes: u64,
}

impl Default for ScanLimits {
    fn default() -> Self {
        Self {
            entries: 10_000,
            per_file_bytes: 2 * 1024 * 1024, // 2 MiB
            total_bytes: 32 * 1024 * 1024,   // 32 MiB
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotEntry {
    pub path: String,
    pub size_bytes: u64,
    pub sha256: Digest,
    pub mode: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkippedPath {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub root_device: u64,
    pub root_inode: u64,
    pub entries: Vec<SnapshotEntry>,
    pub digest: Digest,
    pub skipped: Vec<SkippedPath>,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileReceipt {
    pub path: String,
    pub before: Option<Digest>,
    pub after: Option<Digest>,
    pub applied: bool,
    pub recovery_path: Option<String>,
    pub conflict: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromotionReceipt {
    pub job_id: Id,
    pub change_digest: Digest,
    pub files: Vec<FileReceipt>,
    pub complete: bool,
}

#[derive(Debug, Clone)]
pub struct AudioInput {
    pub data: Vec<u8>,
    pub mime: String,
    pub language: Language,
    pub model_id: Id,
    pub approval_id: Option<Id>,
    pub payload_digest: Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transcript {
    pub text: String,
    pub language: Language,
    pub model_id: Id,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage_record_id: Option<Id>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpeechInput {
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<Id>,
    pub voice: String,
    pub rate: f32,
    pub language: Language,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approval_id: Option<Id>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioOutputMode {
    Audio,
    Browser,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioOutput {
    pub mode: AudioOutputMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio_artifact_id: Option<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandoffInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<Id>,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    pub source_label: String,
    pub owner_ack: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum WhitelistedEventData {
    Job {
        state: JobState,
        revision: Revision,
        #[serde(skip_serializing_if = "Option::is_none")]
        step_id: Option<Id>,
    },
    Task {
        id: Id,
        revision: Revision,
    },
    Approval {
        id: Id,
        decision: ApprovalDecisionKind,
    },
    Artifact {
        id: Id,
        kind: ArtifactKind,
    },
    Occurrence {
        id: Id,
        state: OccurrenceState,
    },
    Resources(ResourceSnapshot),
    RetentionGap {
        minimum_seq: i64,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StreamEvent {
    pub seq: i64,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<Id>,
    pub at: UtcMs,
    pub data: WhitelistedEventData,
}
