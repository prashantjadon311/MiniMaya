use crate::approval::ApprovalService;
use crate::artifacts::ArtifactStore;
use crate::assistant::AssistantService;
use crate::audit::{AuditLog, EventHub};
use crate::auth::AuthService;
use crate::backup::BackupService;
use crate::browser::BrowserService;
use crate::coding::CodingService;
use crate::conversation::ConversationService;
use crate::db::Db;
use crate::desktop::DesktopService;
use crate::fileops::FileService;
use crate::memory::MemoryService;
use crate::policy::Policy;
use crate::process::ProcessSupervisor;
use crate::project::ProjectService;
use crate::provider::ProviderRegistry;
use crate::reports::ReportService;
use crate::sandbox::SandboxBuilder;
use crate::schedule::{ScheduleWake, Scheduler};
use crate::secrets::SecretStore;
use crate::tasks::TaskService;
use crate::telemetry::ResourceMonitor;
use crate::voice::VoiceService;
use crate::workflow::WorkflowEngine;
use std::sync::Arc;
use tokio::sync::mpsc;

#[derive(Clone)]
pub struct ServiceHandles {
    pub db: Db,
    pub auth: Arc<AuthService>,
    pub policy: Arc<Policy>,
    pub approval: Arc<ApprovalService>,
    pub providers: Arc<ProviderRegistry>,
    pub projects: Arc<ProjectService>,
    pub tasks: Arc<TaskService>,
    pub scheduler: Arc<Scheduler>,
    pub conversations: Arc<ConversationService>,
    pub assistant: Arc<AssistantService>,
    pub workflow: Arc<WorkflowEngine>,
    pub memory: Arc<MemoryService>,
    pub reports: Arc<ReportService>,
    pub artifacts: Arc<ArtifactStore>,
    pub audit: Arc<AuditLog>,
    pub desktop: Arc<DesktopService>,
    pub browser: Arc<BrowserService>,
    pub voice: Arc<VoiceService>,
    pub telemetry: Arc<ResourceMonitor>,
    pub backup: Arc<BackupService>,
    pub files: Arc<FileService>,
    pub coding: Arc<CodingService>,
    pub process: Arc<ProcessSupervisor>,
    pub sandbox: Arc<SandboxBuilder>,
    pub secrets: Arc<dyn SecretStore>,
    pub events: Arc<EventHub>,
    pub schedule_wake: mpsc::Sender<ScheduleWake>,
}

#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub auth: Arc<AuthService>,
    pub policy: Arc<Policy>,
    pub approval: Arc<ApprovalService>,
    pub providers: Arc<ProviderRegistry>,
    pub projects: Arc<ProjectService>,
    pub tasks: Arc<TaskService>,
    pub scheduler: Arc<Scheduler>,
    pub conversations: Arc<ConversationService>,
    pub assistant: Arc<AssistantService>,
    pub workflow: Arc<WorkflowEngine>,
    pub memory: Arc<MemoryService>,
    pub reports: Arc<ReportService>,
    pub artifacts: Arc<ArtifactStore>,
    pub audit: Arc<AuditLog>,
    pub desktop: Arc<DesktopService>,
    pub browser: Arc<BrowserService>,
    pub voice: Arc<VoiceService>,
    pub telemetry: Arc<ResourceMonitor>,
    pub backup: Arc<BackupService>,
    pub files: Arc<FileService>,
    pub coding: Arc<CodingService>,
    pub process: Arc<ProcessSupervisor>,
    pub sandbox: Arc<SandboxBuilder>,
    pub secrets: Arc<dyn SecretStore>,
    pub events: Arc<EventHub>,
    pub schedule_wake: mpsc::Sender<ScheduleWake>,
}

impl AppState {
    pub fn new(services: ServiceHandles) -> Self {
        Self {
            db: services.db,
            auth: services.auth,
            policy: services.policy,
            approval: services.approval,
            providers: services.providers,
            projects: services.projects,
            tasks: services.tasks,
            scheduler: services.scheduler,
            conversations: services.conversations,
            assistant: services.assistant,
            workflow: services.workflow,
            memory: services.memory,
            reports: services.reports,
            artifacts: services.artifacts,
            audit: services.audit,
            desktop: services.desktop,
            browser: services.browser,
            voice: services.voice,
            telemetry: services.telemetry,
            backup: services.backup,
            files: services.files,
            coding: services.coding,
            process: services.process,
            sandbox: services.sandbox,
            secrets: services.secrets,
            events: services.events,
            schedule_wake: services.schedule_wake,
        }
    }
}
