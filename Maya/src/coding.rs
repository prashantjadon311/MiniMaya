use crate::checkpoint::CheckpointService;
use crate::error::{AppError, ErrorCode, Result};
use crate::policy::ExecutionPermit;
use crate::provider::ProviderRegistry;
use crate::types::{
    ActionClass, Capability, CapabilityDetails, CapabilityState, ChangeSet, CodingOutcome,
    CodingRequest, Digest, EditManifest, EditManifestOperation, FileChange, FileChangeKind,
    ModelOutputMode, ModelRequest, Plan, StagedProject, Tool, ToolKind,
};
use futures_util::future::BoxFuture;
use sha2::{Digest as ShaDigest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

pub trait CodingRunner: Send + Sync {
    fn probe<'a>(&'a self, tool: &'a Tool) -> BoxFuture<'a, Result<Capability>>;
    fn execute<'a>(
        &'a self,
        request: CodingRequest,
        permit: &'a ExecutionPermit,
        cancel: CancellationToken,
    ) -> BoxFuture<'a, Result<CodingOutcome>>;
}

pub struct ApiEditRunner {
    provider_registry: Arc<ProviderRegistry>,
}

impl ApiEditRunner {
    pub fn new(provider_registry: Arc<ProviderRegistry>) -> Self {
        Self { provider_registry }
    }
}

impl CodingRunner for ApiEditRunner {
    fn probe<'a>(&'a self, _tool: &'a Tool) -> BoxFuture<'a, Result<Capability>> {
        Box::pin(async move {
            Ok(Capability {
                key: "api_edit".to_string(),
                state: CapabilityState::Available,
                reason: Some("API EditManifest runner available".to_string()),
                version: Some("v0.1".to_string()),
                checked_at: chrono::Utc::now().timestamp_millis(),
                details: CapabilityDetails::default(),
            })
        })
    }

    fn execute<'a>(
        &'a self,
        request: CodingRequest,
        permit: &'a ExecutionPermit,
        cancel: CancellationToken,
    ) -> BoxFuture<'a, Result<CodingOutcome>> {
        Box::pin(async move {
            permit.permits(&ActionClass::ProjectEdit, None)?;

            let model = request.model.clone().ok_or_else(|| {
                AppError::new(
                    ErrorCode::ModelUnavailable,
                    "No model specified for API edit runner",
                )
            })?;

            // Generate EditManifest using Model
            let plan_summary = format!(
                "Goal: {}\nSteps: {}",
                request.plan.goal,
                request.plan.steps.len()
            );
            let model_req = ModelRequest {
                request_id: Uuid::new_v4(),
                model,
                role: crate::types::Role::Coding,
                messages: vec![
                    crate::types::ModelMessage {
                        role: crate::types::ModelMessageRole::System,
                        text: "You are an expert software developer. Given the project plan, output strict JSON conforming to EditManifest schema with keys: schema_version (number), summary (string), changes (array of {path, operation, expected_before_sha256, content_utf8}).".to_string(),
                    },
                    crate::types::ModelMessage {
                        role: crate::types::ModelMessageRole::User,
                        text: plan_summary,
                    },
                ],
                max_output_bytes: 65536,
                timeout_ms: 120000,
                output: ModelOutputMode::EditManifest,
            };

            let model_res = self.provider_registry.generate(model_req, cancel).await?;

            // Parse EditManifest from response text
            let manifest: EditManifest = serde_json::from_str(&model_res.text).map_err(|e| {
                AppError::new(
                    ErrorCode::ProviderError,
                    format!("Failed to parse EditManifest JSON: {e}"),
                )
            })?;

            if request.stage.baseline_digest != request.plan.baseline_digest {
                return Err(AppError::new(
                    ErrorCode::PlanChanged,
                    "Staged project baseline does not match the approved plan",
                ));
            }
            let change_set = validate_manifest(&manifest, &request.plan)?;

            // Apply edits to staged project
            apply_manifest_to_stage(&request.stage, &manifest)?;

            Ok(CodingOutcome {
                process: None,
                changes: change_set,
                summary: manifest.summary,
            })
        })
    }
}

pub struct CodingService {
    checkpoint_service: Arc<CheckpointService>,
    api_runner: Arc<ApiEditRunner>,
    runners: HashMap<ToolKind, Arc<dyn CodingRunner>>,
}

impl CodingService {
    pub fn new(
        checkpoint_service: Arc<CheckpointService>,
        provider_registry: Arc<ProviderRegistry>,
    ) -> Self {
        let api_runner = Arc::new(ApiEditRunner::new(provider_registry));
        Self {
            checkpoint_service,
            api_runner,
            runners: HashMap::new(),
        }
    }

    pub fn checkpoint_service(&self) -> &Arc<CheckpointService> {
        &self.checkpoint_service
    }

    pub fn register_runner(&mut self, kind: ToolKind, runner: Arc<dyn CodingRunner>) {
        self.runners.insert(kind, runner);
    }

    pub async fn execute(
        &self,
        request: CodingRequest,
        permit: &ExecutionPermit,
        cancel: CancellationToken,
    ) -> Result<CodingOutcome> {
        if let Some(tool) = &request.tool
            && let Some(runner) = self.runners.get(&tool.kind)
        {
            return runner.execute(request, permit, cancel).await;
        }

        // Default to API edit runner
        self.api_runner.execute(request, permit, cancel).await
    }
}

pub fn validate_manifest(manifest: &EditManifest, plan: &Plan) -> Result<ChangeSet> {
    if manifest.schema_version != 1 {
        return Err(AppError::new(
            ErrorCode::InvalidPlan,
            format!(
                "Unsupported manifest schema_version: {}",
                manifest.schema_version
            ),
        ));
    }

    if !plan.actions.contains(&ActionClass::ProjectEdit) {
        return Err(AppError::new(
            ErrorCode::ActionDenied,
            "The approved plan does not authorize project edits",
        ));
    }
    if manifest.changes.is_empty() || manifest.changes.len() > 100 {
        return Err(AppError::new(
            ErrorCode::InvalidPlan,
            "Edit manifest must contain between 1 and 100 changes",
        ));
    }

    let mut seen_paths = HashSet::new();
    let mut changes = Vec::new();
    let mut content_bytes = 0usize;

    for change in &manifest.changes {
        let clean_path = normalize_manifest_path(&change.path)?;
        if !plan_path_allows(plan, &clean_path) {
            return Err(AppError::new(
                ErrorCode::ScopeDenied,
                format!("Manifest path is outside the approved plan: {clean_path}"),
            ));
        }

        if !seen_paths.insert(clean_path.clone()) {
            return Err(AppError::new(
                ErrorCode::InvalidPlan,
                format!("Duplicate path in manifest: {clean_path}"),
            ));
        }

        match change.operation {
            EditManifestOperation::Create => {
                if change.expected_before_sha256.is_some() {
                    return Err(AppError::new(
                        ErrorCode::InvalidPlan,
                        "Create change cannot declare a preimage hash",
                    ));
                }
                let content = change.content_utf8.as_ref().ok_or_else(|| {
                    AppError::new(
                        ErrorCode::InvalidPlan,
                        "Create change must have content_utf8",
                    )
                })?;
                content_bytes = content_bytes.saturating_add(content.len());

                let after_digest = compute_sha256(content.as_bytes());
                changes.push(FileChange {
                    path: clean_path,
                    kind: FileChangeKind::Create,
                    before: None,
                    after: Some(after_digest),
                    artifact_id: None,
                    mode: 0o644,
                });
            }
            EditManifestOperation::Replace => {
                let before = change.expected_before_sha256.ok_or_else(|| {
                    AppError::new(
                        ErrorCode::InvalidPlan,
                        "Replace change requires an exact preimage hash",
                    )
                })?;
                let content = change.content_utf8.as_ref().ok_or_else(|| {
                    AppError::new(
                        ErrorCode::InvalidPlan,
                        "Replace change must have content_utf8",
                    )
                })?;
                content_bytes = content_bytes.saturating_add(content.len());

                let after_digest = compute_sha256(content.as_bytes());
                changes.push(FileChange {
                    path: clean_path,
                    kind: FileChangeKind::Modify,
                    before: Some(before),
                    after: Some(after_digest),
                    artifact_id: None,
                    mode: 0o644,
                });
            }
            EditManifestOperation::Delete => {
                if change.content_utf8.is_some() {
                    return Err(AppError::new(
                        ErrorCode::InvalidPlan,
                        "Delete change cannot include replacement content",
                    ));
                }
                let before = change.expected_before_sha256.ok_or_else(|| {
                    AppError::new(
                        ErrorCode::InvalidPlan,
                        "Delete change requires an exact preimage hash",
                    )
                })?;
                changes.push(FileChange {
                    path: clean_path,
                    kind: FileChangeKind::Delete,
                    before: Some(before),
                    after: None,
                    artifact_id: None,
                    mode: 0o644,
                });
            }
        }
    }
    if content_bytes > 2 * 1024 * 1024 {
        return Err(AppError::new(
            ErrorCode::OutputLimit,
            "Edit manifest content exceeds 2 MiB",
        ));
    }

    let cs_digest = compute_changeset_digest(plan.baseline_digest, &changes);

    Ok(ChangeSet {
        baseline_digest: plan.baseline_digest,
        changes,
        digest: cs_digest,
    })
}

pub fn apply_manifest_to_stage(stage: &StagedProject, manifest: &EditManifest) -> Result<()> {
    for change in &manifest.changes {
        let rel_path = normalize_manifest_path(&change.path)?;
        let full_path = checked_stage_path(stage, &rel_path)?;
        match change.operation {
            EditManifestOperation::Create => {
                if fs::symlink_metadata(&full_path).is_ok() {
                    return Err(AppError::new(
                        ErrorCode::PlanChanged,
                        format!("Create target already exists: {rel_path}"),
                    ));
                }
            }
            EditManifestOperation::Replace | EditManifestOperation::Delete => {
                let metadata = fs::symlink_metadata(&full_path).map_err(|_| {
                    AppError::new(
                        ErrorCode::PlanChanged,
                        format!("Expected preimage is missing: {rel_path}"),
                    )
                })?;
                if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
                    return Err(AppError::new(
                        ErrorCode::ScopeDenied,
                        format!("Manifest target is not a regular file: {rel_path}"),
                    ));
                }
                let expected = change.expected_before_sha256.ok_or_else(|| {
                    AppError::new(ErrorCode::InvalidPlan, "Preimage hash is required")
                })?;
                let actual = compute_sha256(&fs::read(&full_path).map_err(|error| {
                    AppError::new(
                        ErrorCode::Internal,
                        format!("Failed to read stage file: {error}"),
                    )
                })?);
                if actual != expected {
                    return Err(AppError::new(
                        ErrorCode::PlanChanged,
                        format!("Staged preimage changed for {rel_path}"),
                    ));
                }
            }
        }
    }
    for change in &manifest.changes {
        let rel_path = normalize_manifest_path(&change.path)?;
        let full_path = checked_stage_path(stage, &rel_path)?;

        match change.operation {
            EditManifestOperation::Create | EditManifestOperation::Replace => {
                let content = change.content_utf8.as_ref().ok_or_else(|| {
                    AppError::new(ErrorCode::InvalidPlan, "Missing content for edit")
                })?;

                if let Some(parent) = full_path.parent() {
                    fs::create_dir_all(parent).map_err(|e| {
                        AppError::new(
                            ErrorCode::Internal,
                            format!("Failed to create parent dirs: {e}"),
                        )
                    })?;
                }

                fs::write(&full_path, content.as_bytes()).map_err(|e| {
                    AppError::new(
                        ErrorCode::Internal,
                        format!("Failed to write stage file: {e}"),
                    )
                })?;
            }
            EditManifestOperation::Delete => {
                if full_path.exists() {
                    fs::remove_file(&full_path).map_err(|e| {
                        AppError::new(
                            ErrorCode::Internal,
                            format!("Failed to delete stage file: {e}"),
                        )
                    })?;
                }
            }
        }
    }
    Ok(())
}

fn normalize_manifest_path(p: &str) -> Result<String> {
    let trimmed = p.trim();
    if trimmed.is_empty() || trimmed.contains('\0') {
        return Err(AppError::new(
            ErrorCode::InvalidPlan,
            "Empty or invalid file path",
        ));
    }
    if trimmed.starts_with('/') || trimmed.starts_with('\\') {
        return Err(AppError::new(
            ErrorCode::ScopeDenied,
            "Absolute paths forbidden",
        ));
    }

    let path = Path::new(trimmed);
    let mut normalized = std::path::PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                return Err(AppError::new(
                    ErrorCode::ScopeDenied,
                    "Path traversal forbidden",
                ));
            }
            std::path::Component::RootDir | std::path::Component::Prefix(_) => {
                return Err(AppError::new(
                    ErrorCode::ScopeDenied,
                    "Absolute paths forbidden",
                ));
            }
            std::path::Component::Normal(value) => normalized.push(value),
            std::path::Component::CurDir => {}
        }
    }
    let normalized = normalized.to_string_lossy().into_owned();
    if normalized.is_empty()
        || normalized == ".git"
        || normalized.starts_with(".git/")
        || normalized == ".env"
        || normalized.starts_with(".env.")
        || normalized == ".codex/hooks"
        || normalized.starts_with(".codex/hooks/")
    {
        return Err(AppError::new(
            ErrorCode::ScopeDenied,
            "Protected project metadata cannot be edited",
        ));
    }
    Ok(normalized)
}

fn plan_path_allows(plan: &Plan, path: &str) -> bool {
    plan.allowed_paths.iter().any(|allowed| {
        let allowed = allowed.trim_matches('/');
        if let Some(prefix) = allowed.strip_suffix("/**") {
            path == prefix || path.starts_with(&format!("{prefix}/"))
        } else {
            path == allowed || path.starts_with(&format!("{allowed}/"))
        }
    })
}

fn checked_stage_path(stage: &StagedProject, relative: &str) -> Result<std::path::PathBuf> {
    let mut current = stage.path.clone();
    for component in Path::new(relative).components() {
        let std::path::Component::Normal(part) = component else {
            return Err(AppError::new(ErrorCode::ScopeDenied, "Invalid staged path"));
        };
        current.push(part);
        if let Ok(metadata) = fs::symlink_metadata(&current)
            && metadata.file_type().is_symlink()
        {
            return Err(AppError::new(
                ErrorCode::ScopeDenied,
                format!("Symbolic links are forbidden in staged path: {relative}"),
            ));
        }
    }
    Ok(current)
}

fn compute_sha256(bytes: &[u8]) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    Digest::new(hasher.finalize().into())
}

fn compute_changeset_digest(baseline: Digest, files: &[FileChange]) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(baseline.as_bytes());
    for f in files {
        hasher.update(f.path.as_bytes());
        match f.kind {
            FileChangeKind::Create => hasher.update(b"create"),
            FileChangeKind::Modify => hasher.update(b"modify"),
            FileChangeKind::Delete => hasher.update(b"delete"),
        }
        if let Some(b) = &f.before {
            hasher.update(b.as_bytes());
        }
        if let Some(a) = &f.after {
            hasher.update(a.as_bytes());
        }
    }
    Digest::new(hasher.finalize().into())
}
