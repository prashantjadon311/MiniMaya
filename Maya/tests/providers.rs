#![allow(clippy::result_large_err)]

mod common;

use common::test_db;
use futures_util::future::BoxFuture;
use maya_mini::coding::{apply_manifest_to_stage, validate_manifest};
use maya_mini::error::ErrorCode;
use maya_mini::provider::{ModelProvider, ProviderRegistry};
use maya_mini::providers::anthropic::AnthropicProvider;
use maya_mini::providers::gemini::GeminiProvider;
use maya_mini::providers::openai::{OpenAiChatCompatible, OpenAiResponses};
use maya_mini::secrets::{KeyringSecrets, SecretStore, SecretString};
use maya_mini::types::{
    ActionClass, BillingMode, ChainInput, ChainPurpose, ChainStep, Digest, EditManifest,
    EditManifestChange, EditManifestOperation, ModelCapability, ModelInput, ModelMessage,
    ModelMessageRole, ModelOutputMode, ModelRequest, ModelResult, ModelResultFinish,
    ModelSelection, Plan, ProviderCapabilities, ProviderEvent, ProviderInput, ProviderKind,
    ProviderPatch, ResolvedModelSource, Role, RolesPut, StagedProject, UsageObservation,
};
use std::fs;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

static ATTEMPTS: AtomicU32 = AtomicU32::new(0);

struct MockProvider {
    fail_attempts: u32,
}

impl ModelProvider for MockProvider {
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            text: true,
            structured_edits: true,
            streaming: true,
            usage: true,
            stt: false,
            tts: false,
        }
    }

    fn generate<'a>(
        &'a self,
        _request: ModelRequest,
        _cancel: CancellationToken,
    ) -> BoxFuture<'a, maya_mini::error::Result<ModelResult>> {
        Box::pin(async move {
            let cur = ATTEMPTS.fetch_add(1, Ordering::SeqCst);
            if cur < self.fail_attempts {
                Err(maya_mini::error::AppError::new(
                    ErrorCode::RateLimited,
                    "Simulated rate limit",
                ))
            } else {
                Ok(ModelResult {
                    text: "mock response".to_string(),
                    actual_model_id: Some("mock-v1".to_string()),
                    usage: UsageObservation {
                        input_tokens: Some(10),
                        output_tokens: Some(25),
                        money: None,
                    },
                    finish: ModelResultFinish::Complete,
                })
            }
        })
    }
}

fn mock_factory_success(
    _p: &maya_mini::types::Provider,
    _s: Arc<dyn SecretStore>,
) -> maya_mini::error::Result<Arc<dyn ModelProvider>> {
    Ok(Arc::new(MockProvider { fail_attempts: 0 }))
}

fn mock_factory_retry(
    _p: &maya_mini::types::Provider,
    _s: Arc<dyn SecretStore>,
) -> maya_mini::error::Result<Arc<dyn ModelProvider>> {
    Ok(Arc::new(MockProvider { fail_attempts: 2 }))
}

fn helper_plan() -> Plan {
    Plan {
        id: Uuid::new_v4(),
        job_id: Uuid::new_v4(),
        revision: 1,
        goal: "Refactor core module".to_string(),
        summary: "Summary".to_string(),
        steps: vec![],
        allowed_paths: vec!["src/**".to_string(), "deprecated.txt".to_string()],
        actions: vec![ActionClass::ProjectEdit],
        command_ids: vec![],
        context_artifact_ids: vec![],
        selected_model_id: None,
        selected_tool_id: None,
        project_revision: 1,
        baseline_digest: Digest::of(b"base"),
        timeout_ms: 30000,
        risks: vec![],
        digest: Digest::of(b"plan_digest"),
        created_at: 0,
    }
}

// =========================================================================
// X-21: Registry All Role/Override/Fallback Permutations and Chains
// =========================================================================

#[tokio::test]
async fn test_x21_registry_roles_overrides_fallbacks_chains() {
    let (_dir, db) = test_db().await;
    let secrets: Arc<dyn SecretStore> = Arc::new(KeyringSecrets::new());

    let registry = ProviderRegistry::new(db.clone(), secrets.clone(), mock_factory_success);

    let unacknowledged = registry
        .create_provider(ProviderInput {
            label: "Unacknowledged".to_string(),
            kind: ProviderKind::OpenaiResponses,
            protocol: "responses".to_string(),
            endpoint: "https://api.openai.com/v1".to_string(),
            local_endpoint_allowed: false,
            enabled: true,
            billing_mode: BillingMode::Api,
            timeout_ms: 30000,
            owner_ack: false,
        })
        .await
        .unwrap_err();
    assert_eq!(unacknowledged.code(), ErrorCode::ActionDenied);
    let wrong_protocol = registry
        .create_provider(ProviderInput {
            label: "Wrong protocol".to_string(),
            kind: ProviderKind::OpenaiResponses,
            protocol: "chat_completions".to_string(),
            endpoint: "https://api.openai.com/v1".to_string(),
            local_endpoint_allowed: false,
            enabled: true,
            billing_mode: BillingMode::Api,
            timeout_ms: 30000,
            owner_ack: true,
        })
        .await
        .unwrap_err();
    assert_eq!(wrong_protocol.code(), ErrorCode::InvalidRequest);
    let non_loopback_http = registry
        .create_provider(ProviderInput {
            label: "Not local".to_string(),
            kind: ProviderKind::OpenaiCompatibleChat,
            protocol: "chat_completions".to_string(),
            endpoint: "http://example.com/v1".to_string(),
            local_endpoint_allowed: true,
            enabled: true,
            billing_mode: BillingMode::Api,
            timeout_ms: 30000,
            owner_ack: true,
        })
        .await
        .unwrap_err();
    assert_eq!(non_loopback_http.code(), ErrorCode::ScopeDenied);

    // 1. Create provider
    let p_in = ProviderInput {
        label: "OpenAI Main".to_string(),
        kind: ProviderKind::OpenaiResponses,
        protocol: "responses".to_string(),
        endpoint: "https://api.openai.com/v1".to_string(),
        local_endpoint_allowed: false,
        enabled: true,
        billing_mode: BillingMode::Api,
        timeout_ms: 30000,
        owner_ack: true,
    };
    let prov = registry.create_provider(p_in).await.unwrap();
    let insecure_patch = registry
        .patch_provider(
            prov.id,
            ProviderPatch {
                revision: prov.revision,
                label: None,
                endpoint: Some("http://example.com/v1".to_string()),
                local_endpoint_allowed: Some(false),
                enabled: None,
                timeout_ms: None,
            },
        )
        .await
        .unwrap_err();
    assert_eq!(insecure_patch.code(), ErrorCode::ScopeDenied);

    // 2. Create models: primary model A, and fallback model B
    let m_a = registry
        .create_model(ModelInput {
            provider_id: prov.id,
            model_id: "gpt-4o".to_string(),
            label: "GPT-4o Primary".to_string(),
            enabled: true,
            capabilities: vec![ModelCapability::Text, ModelCapability::StructuredEdits],
            context_limit_bytes: 32768,
        })
        .await
        .unwrap();

    let m_b = registry
        .create_model(ModelInput {
            provider_id: prov.id,
            model_id: "gpt-4o-mini".to_string(),
            label: "GPT-4o Mini Fallback".to_string(),
            enabled: true,
            capabilities: vec![ModelCapability::Text],
            context_limit_bytes: 16384,
        })
        .await
        .unwrap();

    // 3. Unassigned initial role resolution -> fails with ModelUnavailable
    let unassigned_res = registry
        .resolve(ModelSelection {
            role: Role::Planning,
            override_id: None,
            project_id: None,
            fallback_consent: false,
        })
        .await;
    assert!(unassigned_res.is_err());
    assert_eq!(
        unassigned_res.unwrap_err().code,
        ErrorCode::ModelUnavailable
    );

    // 4. Configure role binding for Planning
    let roles_view = registry.get_roles().await.unwrap();
    let mut bindings = roles_view.bindings.clone();
    for b in &mut bindings {
        if b.role == Role::Planning {
            b.model_id = Some(m_a.id);
            b.fallback_model_ids = vec![m_b.id];
            b.allow_paid_fallback = true;
        }
    }
    registry
        .put_roles(RolesPut {
            revision: roles_view.revision,
            bindings,
        })
        .await
        .unwrap();

    // 5. Default role resolution -> resolves model A with Role source
    let resolved = registry
        .resolve(ModelSelection {
            role: Role::Planning,
            override_id: None,
            project_id: None,
            fallback_consent: false,
        })
        .await
        .unwrap();
    assert_eq!(resolved.model.id, m_a.id);
    assert_eq!(resolved.source, ResolvedModelSource::Role);

    // 6. Explicit override resolution -> resolves model B with Override source
    let resolved_override = registry
        .resolve(ModelSelection {
            role: Role::Planning,
            override_id: Some(m_b.id),
            project_id: None,
            fallback_consent: false,
        })
        .await
        .unwrap();
    assert_eq!(resolved_override.model.id, m_b.id);
    assert_eq!(resolved_override.source, ResolvedModelSource::Override);

    // 7. Disable model A -> resolution without fallback consent fails
    registry
        .patch_model(
            m_a.id,
            maya_mini::types::ModelPatch {
                revision: m_a.revision,
                label: None,
                enabled: Some(false),
                capabilities: None,
                context_limit_bytes: None,
            },
        )
        .await
        .unwrap();

    let fail_no_consent = registry
        .resolve(ModelSelection {
            role: Role::Planning,
            override_id: None,
            project_id: None,
            fallback_consent: false,
        })
        .await;
    assert!(fail_no_consent.is_err());
    assert_eq!(
        fail_no_consent.unwrap_err().code,
        ErrorCode::ModelUnavailable
    );

    // 8. Resolution with fallback consent -> resolves fallback model B
    let resolved_fallback = registry
        .resolve(ModelSelection {
            role: Role::Planning,
            override_id: None,
            project_id: None,
            fallback_consent: true,
        })
        .await
        .unwrap();
    assert_eq!(resolved_fallback.model.id, m_b.id);
    assert_eq!(resolved_fallback.source, ResolvedModelSource::Fallback);

    // 9. Chains: 1-step (valid), 8-step (valid), 9-step (rejected)
    let c1 = registry
        .create_chain(ChainInput {
            name: "Single Step".to_string(),
            enabled: true,
            steps: vec![ChainStep {
                position: 1,
                purpose: ChainPurpose::Plan,
                model_id: Some(m_b.id),
                tool_id: None,
                max_output_bytes: 4096,
            }],
        })
        .await
        .unwrap();
    assert_eq!(c1.steps.len(), 1);

    // 8 steps
    let mut eight_steps = Vec::new();
    for i in 1..=8 {
        eight_steps.push(ChainStep {
            position: i as u8,
            purpose: ChainPurpose::Research,
            model_id: Some(m_b.id),
            tool_id: None,
            max_output_bytes: 2048,
        });
    }
    let c8 = registry
        .create_chain(ChainInput {
            name: "Eight Steps".to_string(),
            enabled: true,
            steps: eight_steps.clone(),
        })
        .await
        .unwrap();
    assert_eq!(c8.steps.len(), 8);

    // 9 steps -> must be rejected with InvalidPlan
    let mut nine_steps = eight_steps;
    nine_steps.push(ChainStep {
        position: 9,
        purpose: ChainPurpose::Review,
        model_id: Some(m_b.id),
        tool_id: None,
        max_output_bytes: 2048,
    });
    let c9_err = registry
        .create_chain(ChainInput {
            name: "Nine Steps Invalid".to_string(),
            enabled: true,
            steps: nine_steps,
        })
        .await;
    assert!(c9_err.is_err());
    assert_eq!(c9_err.unwrap_err().code, ErrorCode::InvalidPlan);
}

// =========================================================================
// X-22: Timeout, Pre-Send Retry, Rate Limit & Usage Tracking
// =========================================================================

#[tokio::test]
async fn test_x22_timeout_retry_rate_limit_usage_tracking() {
    let (_dir, db) = test_db().await;
    let secrets: Arc<dyn SecretStore> = Arc::new(KeyringSecrets::new());

    ATTEMPTS.store(0, Ordering::SeqCst);

    let registry = ProviderRegistry::new(db.clone(), secrets.clone(), mock_factory_retry);

    let prov = registry
        .create_provider(ProviderInput {
            label: "OpenAI Retry Test".to_string(),
            kind: ProviderKind::OpenaiResponses,
            protocol: "responses".to_string(),
            endpoint: "http://127.0.0.1:8080".to_string(),
            local_endpoint_allowed: true,
            enabled: true,
            billing_mode: BillingMode::Api,
            timeout_ms: 30000,
            owner_ack: true,
        })
        .await
        .unwrap();

    let model = registry
        .create_model(ModelInput {
            provider_id: prov.id,
            model_id: "gpt-4o".to_string(),
            label: "GPT-4o".to_string(),
            enabled: true,
            capabilities: vec![ModelCapability::Text],
            context_limit_bytes: 32768,
        })
        .await
        .unwrap();

    let cancel = CancellationToken::new();

    let req = ModelRequest {
        request_id: Uuid::new_v4(),
        model: model.clone(),
        role: Role::Planning,
        messages: vec![ModelMessage {
            role: ModelMessageRole::User,
            text: "Hello".to_string(),
        }],
        max_output_bytes: 4096,
        timeout_ms: 10000,
        output: ModelOutputMode::Text,
    };

    let res = registry.generate(req, cancel).await.unwrap();
    assert_eq!(res.text, "mock response");
    assert_eq!(ATTEMPTS.load(Ordering::SeqCst), 3);

    // Inspect usage_records table: exactly 3 attempt records
    let usage_count: i64 = db
        .read(|conn| {
            conn.query_row("SELECT COUNT(*) FROM usage_records", [], |r| r.get(0))
                .map_err(|e| {
                    maya_mini::error::AppError::new(ErrorCode::DbUnavailable, e.to_string())
                })
        })
        .await
        .unwrap();
    assert_eq!(usage_count, 3);

    // Check cancellation
    let cancel_token = CancellationToken::new();
    cancel_token.cancel();

    let cancel_req = ModelRequest {
        request_id: Uuid::new_v4(),
        model,
        role: Role::Planning,
        messages: vec![ModelMessage {
            role: ModelMessageRole::User,
            text: "Hello".to_string(),
        }],
        max_output_bytes: 4096,
        timeout_ms: 10000,
        output: ModelOutputMode::Text,
    };

    let cancel_res = registry.generate(cancel_req, cancel_token).await;
    assert!(cancel_res.is_err());
    assert_eq!(cancel_res.unwrap_err().code, ErrorCode::Cancelled);
}

// =========================================================================
// X-23: Wire Fixtures All Five Protocols
// =========================================================================

#[test]
fn test_x23_all_five_wire_protocols() {
    // 1. OpenAI Responses wire decoder
    let ev1 = OpenAiResponses::decode_event(b"data: {\"delta\": \"hello responses\"}").unwrap();
    match ev1 {
        ProviderEvent::TextDelta(d) => assert_eq!(d, "hello responses"),
        _ => panic!("Expected TextDelta"),
    }

    let ev2 = OpenAiResponses::decode_event(
        b"data: {\"usage\": {\"input_tokens\": 12, \"output_tokens\": 34}}",
    )
    .unwrap();
    match ev2 {
        ProviderEvent::Usage(u) => {
            assert_eq!(u.input_tokens, Some(12));
            assert_eq!(u.output_tokens, Some(34));
        }
        _ => panic!("Expected Usage"),
    }

    let ev_done = OpenAiResponses::decode_event(b"data: [DONE]").unwrap();
    assert!(matches!(ev_done, ProviderEvent::Completed { .. }));

    let ev_refusal =
        OpenAiResponses::decode_event(b"data: {\"refusal\": \"Request violates policy\"}").unwrap();
    match ev_refusal {
        ProviderEvent::Refused(r) => assert_eq!(r, "Request violates policy"),
        _ => panic!("Expected Refused"),
    }

    // 2. OpenAI Compatible Chat wire decoder
    let ev_chat = OpenAiChatCompatible::decode_event(
        b"data: {\"choices\": [{\"delta\": {\"content\": \"hello chat\"}}]}",
    )
    .unwrap();
    match ev_chat {
        ProviderEvent::TextDelta(d) => assert_eq!(d, "hello chat"),
        _ => panic!("Expected TextDelta"),
    }

    let ev_chat_stop = OpenAiChatCompatible::decode_event(
        b"data: {\"choices\": [{\"delta\": {}, \"finish_reason\": \"stop\"}], \"model\": \"gpt-3.5-turbo\"}",
    )
    .unwrap();
    match ev_chat_stop {
        ProviderEvent::Completed { actual_model_id } => {
            assert_eq!(actual_model_id, Some("gpt-3.5-turbo".to_string()))
        }
        _ => panic!("Expected Completed"),
    }

    // 3. Gemini Interactions wire decoder
    let ev_gem_inter =
        GeminiProvider::decode_event(b"data: {\"output\": \"gemini interaction text\"}").unwrap();
    match ev_gem_inter {
        ProviderEvent::TextDelta(d) => assert_eq!(d, "gemini interaction text"),
        _ => panic!("Expected TextDelta"),
    }

    let ev_gem_usage = GeminiProvider::decode_event(
        b"data: {\"usage\": {\"prompt_tokens\": 8, \"completion_tokens\": 16}}",
    )
    .unwrap();
    match ev_gem_usage {
        ProviderEvent::Usage(u) => {
            assert_eq!(u.input_tokens, Some(8));
            assert_eq!(u.output_tokens, Some(16));
        }
        _ => panic!("Expected Usage"),
    }

    // 4. Gemini generateContent wire decoder
    let ev_gem_gen = GeminiProvider::decode_event(
        b"data: {\"candidates\": [{\"content\": {\"parts\": [{\"text\": \"gen content part\"}]}}]}",
    )
    .unwrap();
    match ev_gem_gen {
        ProviderEvent::TextDelta(d) => assert_eq!(d, "gen content part"),
        _ => panic!("Expected TextDelta"),
    }

    let ev_gem_blocked =
        GeminiProvider::decode_event(b"data: {\"promptFeedback\": {\"blockReason\": \"SAFETY\"}}")
            .unwrap();
    match ev_gem_blocked {
        ProviderEvent::Refused(r) => assert!(r.contains("SAFETY")),
        _ => panic!("Expected Refused"),
    }

    // 5. Anthropic Messages wire decoder
    let ev_ant_delta = AnthropicProvider::decode_event(
        b"data: {\"type\": \"content_block_delta\", \"delta\": {\"type\": \"text_delta\", \"text\": \"claude delta\"}}",
    )
    .unwrap();
    match ev_ant_delta {
        ProviderEvent::TextDelta(d) => assert_eq!(d, "claude delta"),
        _ => panic!("Expected TextDelta"),
    }

    let ev_ant_usage = AnthropicProvider::decode_event(
        b"data: {\"type\": \"message_delta\", \"usage\": {\"output_tokens\": 42}}",
    )
    .unwrap();
    match ev_ant_usage {
        ProviderEvent::Usage(u) => assert_eq!(u.output_tokens, Some(42)),
        _ => panic!("Expected Usage"),
    }

    let ev_ant_ping = AnthropicProvider::decode_event(b"data: {\"type\": \"ping\"}").unwrap();
    assert!(matches!(ev_ant_ping, ProviderEvent::Ignored));

    let ev_ant_err = AnthropicProvider::decode_event(
        b"data: {\"type\": \"error\", \"error\": {\"type\": \"rate_limit_error\", \"message\": \"Rate limited\"}}",
    )
    .unwrap();
    match ev_ant_err {
        ProviderEvent::Failed(code) => assert_eq!(code, ErrorCode::RateLimited),
        _ => panic!("Expected Failed RateLimited"),
    }
}

// =========================================================================
// X-30: Valid API Edit Manifest Applied to Stage, Paths & Security
// =========================================================================

#[test]
fn test_x30_api_edit_manifest_validation_and_staging() {
    let stage_dir = tempfile::tempdir().unwrap();
    let plan = helper_plan();
    let stage = StagedProject {
        path: stage_dir.path().to_path_buf(),
        checkpoint_id: Uuid::new_v4(),
        baseline_digest: plan.baseline_digest,
    };

    fs::create_dir_all(stage.path.join("src")).unwrap();
    fs::write(stage.path.join("src/existing.rs"), b"fn old_func() {}").unwrap();
    fs::write(stage.path.join("deprecated.txt"), b"old content").unwrap();

    // 1. Valid manifest
    let manifest = EditManifest {
        schema_version: 1,
        summary: "Create file1, replace file2, delete file3".to_string(),
        changes: vec![
            EditManifestChange {
                path: "src/new_file.rs".to_string(),
                operation: EditManifestOperation::Create,
                expected_before_sha256: None,
                content_utf8: Some("fn new_func() {}".to_string()),
            },
            EditManifestChange {
                path: "src/existing.rs".to_string(),
                operation: EditManifestOperation::Replace,
                expected_before_sha256: Some(Digest::of(b"fn old_func() {}")),
                content_utf8: Some("fn updated_func() {}".to_string()),
            },
            EditManifestChange {
                path: "deprecated.txt".to_string(),
                operation: EditManifestOperation::Delete,
                expected_before_sha256: Some(Digest::of(b"old content")),
                content_utf8: None,
            },
        ],
    };

    let cs = validate_manifest(&manifest, &plan).unwrap();
    assert_eq!(cs.changes.len(), 3);
    assert_eq!(cs.baseline_digest, plan.baseline_digest);

    apply_manifest_to_stage(&stage, &manifest).unwrap();

    // Verify stage contents
    assert!(stage.path.join("src/new_file.rs").exists());
    assert_eq!(
        fs::read_to_string(stage.path.join("src/new_file.rs")).unwrap(),
        "fn new_func() {}"
    );
    assert_eq!(
        fs::read_to_string(stage.path.join("src/existing.rs")).unwrap(),
        "fn updated_func() {}"
    );
    assert!(!stage.path.join("deprecated.txt").exists());

    // 2. Reject duplicate paths
    let dup_manifest = EditManifest {
        schema_version: 1,
        summary: "Duplicate path".to_string(),
        changes: vec![
            EditManifestChange {
                path: "src/dup.txt".to_string(),
                operation: EditManifestOperation::Create,
                expected_before_sha256: None,
                content_utf8: Some("1".to_string()),
            },
            EditManifestChange {
                path: "src/dup.txt".to_string(),
                operation: EditManifestOperation::Create,
                expected_before_sha256: None,
                content_utf8: Some("2".to_string()),
            },
        ],
    };
    let err_dup = validate_manifest(&dup_manifest, &plan);
    assert!(err_dup.is_err());
    assert_eq!(err_dup.unwrap_err().code, ErrorCode::InvalidPlan);

    // 3. Reject path traversal
    let traversal_manifest = EditManifest {
        schema_version: 1,
        summary: "Traversal attack".to_string(),
        changes: vec![EditManifestChange {
            path: "../secret.txt".to_string(),
            operation: EditManifestOperation::Create,
            expected_before_sha256: None,
            content_utf8: Some("hacked".to_string()),
        }],
    };
    let err_trav = validate_manifest(&traversal_manifest, &plan);
    assert!(err_trav.is_err());
    assert_eq!(err_trav.unwrap_err().code, ErrorCode::ScopeDenied);

    // 4. Reject absolute path
    let abs_manifest = EditManifest {
        schema_version: 1,
        summary: "Absolute path".to_string(),
        changes: vec![EditManifestChange {
            path: "/etc/passwd".to_string(),
            operation: EditManifestOperation::Create,
            expected_before_sha256: None,
            content_utf8: Some("root:x:0:0:".to_string()),
        }],
    };
    let err_abs = validate_manifest(&abs_manifest, &plan);
    assert!(err_abs.is_err());
    assert_eq!(err_abs.unwrap_err().code, ErrorCode::ScopeDenied);

    // 5. A normalized but unapproved path is rejected by the immutable plan.
    let outside = EditManifest {
        schema_version: 1,
        summary: "Outside plan".to_string(),
        changes: vec![EditManifestChange {
            path: "README.md".to_string(),
            operation: EditManifestOperation::Create,
            expected_before_sha256: None,
            content_utf8: Some("unexpected".to_string()),
        }],
    };
    let err_outside = validate_manifest(&outside, &plan).unwrap_err();
    assert_eq!(err_outside.code(), ErrorCode::ScopeDenied);

    // 6. Replace/delete operations must bind the exact staged preimage.
    let missing_preimage = EditManifest {
        schema_version: 1,
        summary: "Unbound replace".to_string(),
        changes: vec![EditManifestChange {
            path: "src/existing.rs".to_string(),
            operation: EditManifestOperation::Replace,
            expected_before_sha256: None,
            content_utf8: Some("changed".to_string()),
        }],
    };
    assert_eq!(
        validate_manifest(&missing_preimage, &plan)
            .unwrap_err()
            .code(),
        ErrorCode::InvalidPlan
    );

    let mismatched = EditManifest {
        schema_version: 1,
        summary: "Stale replace".to_string(),
        changes: vec![EditManifestChange {
            path: "src/existing.rs".to_string(),
            operation: EditManifestOperation::Replace,
            expected_before_sha256: Some(Digest::of(b"stale bytes")),
            content_utf8: Some("changed".to_string()),
        }],
    };
    let mismatch_error = apply_manifest_to_stage(&stage, &mismatched).unwrap_err();
    assert_eq!(mismatch_error.code(), ErrorCode::PlanChanged);
}

// =========================================================================
// X-29: Codex Relay Security, Method, Path, Auth & Quota
// =========================================================================

#[tokio::test]
async fn test_x29_codex_relay_security_and_quota() {
    let (_dir, _db) = test_db().await;
    let secrets: Arc<dyn SecretStore> = Arc::new(KeyringSecrets::new());

    let fake_token = SecretString::new("test_relay_token_123");

    // Bind listener
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    let prov = maya_mini::types::Provider {
        id: Uuid::new_v4(),
        label: "OpenAI Relay".to_string(),
        kind: ProviderKind::OpenaiResponses,
        protocol: "responses".to_string(),
        endpoint: "http://127.0.0.1:9".to_string(), // Dead port, testing relay rejection layer
        local_endpoint_allowed: true,
        enabled: true,
        billing_mode: BillingMode::Api,
        credential_status: maya_mini::types::CredentialStatus::Session,
        timeout_ms: 10000,
        revision: 1,
    };

    let model = maya_mini::types::Model {
        id: Uuid::new_v4(),
        provider_id: prov.id,
        model_id: "gpt-4o".to_string(),
        label: "GPT-4o".to_string(),
        enabled: true,
        capabilities: vec![ModelCapability::Text],
        context_limit_bytes: 32768,
        revision: 1,
    };

    let session = maya_mini::codex::RelaySession {
        token: fake_token,
        port,
        max_requests: 2,
        deadline: std::time::Instant::now() + std::time::Duration::from_secs(60),
        provider: prov,
        model,
        secrets,
    };

    let cancel = CancellationToken::new();
    let cancel_clone = cancel.clone();

    let relay_task = tokio::spawn(async move {
        let _ = maya_mini::codex::serve_relay_listener(listener, session, cancel_clone).await;
    });

    // Wait for listener ready
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // 1. Send GET request -> Expect 405 Method Not Allowed
    let mut stream = tokio::net::TcpStream::connect(format!("127.0.0.1:{port}"))
        .await
        .unwrap();
    stream
        .write_all(b"GET /responses HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
        .await
        .unwrap();
    let mut resp = vec![0u8; 1024];
    let n = stream.read(&mut resp).await.unwrap();
    let resp_str = String::from_utf8_lossy(&resp[..n]);
    assert!(resp_str.contains("405 Method Not Allowed"));

    // 2. Send POST to wrong path -> Expect 404 Not Found
    let mut stream = tokio::net::TcpStream::connect(format!("127.0.0.1:{port}"))
        .await
        .unwrap();
    stream
        .write_all(b"POST /chat HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
        .await
        .unwrap();
    let mut resp = vec![0u8; 1024];
    let n = stream.read(&mut resp).await.unwrap();
    let resp_str = String::from_utf8_lossy(&resp[..n]);
    assert!(resp_str.contains("404 Not Found"));

    // 3. Send POST with missing or wrong token -> Expect 401 Unauthorized
    let mut stream = tokio::net::TcpStream::connect(format!("127.0.0.1:{port}"))
        .await
        .unwrap();
    stream
        .write_all(b"POST /responses HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer wrong_token\r\n\r\n")
        .await
        .unwrap();
    let mut resp = vec![0u8; 1024];
    let n = stream.read(&mut resp).await.unwrap();
    let resp_str = String::from_utf8_lossy(&resp[..n]);
    assert!(resp_str.contains("401 Unauthorized"));

    // Clean up
    cancel.cancel();
    let _ = relay_task.await;
}
