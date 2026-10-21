use crate::db::Db;
use crate::error::{AppError, ErrorCode, Result};
use crate::secrets::SecretStore;
use crate::types::{
    BillingMode, Chain, ChainInput, ChainPut, ChainStep, CredentialStatus, Id, Model,
    ModelCapability, ModelInput, ModelPatch, ModelRequest, ModelResult, ModelSelection, Provider,
    ProviderCapabilities, ProviderInput, ProviderKind, ProviderPatch, ResolvedModel,
    ResolvedModelSource, Revision, Role, RoleBinding, RolesPut, RolesView, UsageObservation,
    UsageRecordStatus, UtcMs, ValidatedEndpoint,
};
use futures_util::future::BoxFuture;
use rusqlite::params;
use std::net::IpAddr;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use url::Url;
use uuid::Uuid;

pub trait ModelProvider: Send + Sync {
    fn capabilities(&self) -> ProviderCapabilities;
    fn generate<'a>(
        &'a self,
        request: ModelRequest,
        cancel: CancellationToken,
    ) -> BoxFuture<'a, Result<ModelResult>>;
}

pub type ProviderFactory = fn(&Provider, Arc<dyn SecretStore>) -> Result<Arc<dyn ModelProvider>>;

pub struct ProviderRegistry {
    db: Db,
    secrets: Arc<dyn SecretStore>,
    factory: ProviderFactory,
}

impl ProviderRegistry {
    pub fn new(db: Db, secrets: Arc<dyn SecretStore>, factory: ProviderFactory) -> Self {
        Self {
            db,
            secrets,
            factory,
        }
    }

    pub fn db(&self) -> &Db {
        &self.db
    }

    pub fn secrets(&self) -> Arc<dyn SecretStore> {
        self.secrets.clone()
    }

    pub fn factory(&self) -> ProviderFactory {
        self.factory
    }

    // --- Providers CRUD ---

    pub async fn list_providers(&self) -> Result<Vec<Provider>> {
        self.db
            .read(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, label, kind, protocol, endpoint, local_endpoint_allowed, enabled,
                            billing_mode, secret_ref, credential_generation, timeout_ms, revision
                     FROM providers ORDER BY created_at ASC",
                )?;
                let rows = stmt.query_map([], |row| {
                    let id_str: String = row.get(0)?;
                    let label: String = row.get(1)?;
                    let kind_str: String = row.get(2)?;
                    let protocol: String = row.get(3)?;
                    let endpoint: String = row.get(4)?;
                    let local_allowed: i64 = row.get(5)?;
                    let enabled: i64 = row.get(6)?;
                    let billing_str: String = row.get(7)?;
                    let secret_ref: Option<String> = row.get(8)?;
                    let _cred_gen: i64 = row.get(9)?;
                    let timeout_ms: i64 = row.get(10)?;
                    let revision: i64 = row.get(11)?;

                    let kind = match kind_str.as_str() {
                        "openai_responses" => ProviderKind::OpenaiResponses,
                        "openai_compatible_chat" => ProviderKind::OpenaiCompatibleChat,
                        "gemini" => ProviderKind::Gemini,
                        "anthropic" => ProviderKind::Anthropic,
                        _ => ProviderKind::OpenaiResponses,
                    };

                    let billing_mode = match billing_str.as_str() {
                        "api" => BillingMode::Api,
                        "subscription_cli" => BillingMode::SubscriptionCli,
                        _ => BillingMode::Unknown,
                    };

                    let credential_status = if secret_ref.is_some() {
                        CredentialStatus::Session
                    } else {
                        CredentialStatus::Missing
                    };

                    let id = Uuid::parse_str(&id_str).map_err(|e| {
                        rusqlite::Error::FromSqlConversionFailure(
                            0,
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    })?;

                    Ok(Provider {
                        id,
                        label,
                        kind,
                        protocol,
                        endpoint,
                        local_endpoint_allowed: local_allowed != 0,
                        enabled: enabled != 0,
                        billing_mode,
                        credential_status,
                        timeout_ms: timeout_ms as u32,
                        revision,
                    })
                })?;

                let mut out = Vec::new();
                for r in rows {
                    out.push(r?);
                }
                Ok(out)
            })
            .await
    }

    pub async fn get_provider(&self, id: Id) -> Result<Provider> {
        let id_str = id.to_string();
        self.db
            .read(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, label, kind, protocol, endpoint, local_endpoint_allowed, enabled,
                            billing_mode, secret_ref, credential_generation, timeout_ms, revision
                     FROM providers WHERE id = ?1",
                )?;
                let mut rows = stmt.query(params![id_str])?;
                if let Some(row) = rows.next()? {
                    let id_str: String = row.get(0)?;
                    let label: String = row.get(1)?;
                    let kind_str: String = row.get(2)?;
                    let protocol: String = row.get(3)?;
                    let endpoint: String = row.get(4)?;
                    let local_allowed: i64 = row.get(5)?;
                    let enabled: i64 = row.get(6)?;
                    let billing_str: String = row.get(7)?;
                    let secret_ref: Option<String> = row.get(8)?;
                    let _cred_gen: i64 = row.get(9)?;
                    let timeout_ms: i64 = row.get(10)?;
                    let revision: i64 = row.get(11)?;

                    let kind = match kind_str.as_str() {
                        "openai_responses" => ProviderKind::OpenaiResponses,
                        "openai_compatible_chat" => ProviderKind::OpenaiCompatibleChat,
                        "gemini" => ProviderKind::Gemini,
                        "anthropic" => ProviderKind::Anthropic,
                        _ => ProviderKind::OpenaiResponses,
                    };

                    let billing_mode = match billing_str.as_str() {
                        "api" => BillingMode::Api,
                        "subscription_cli" => BillingMode::SubscriptionCli,
                        _ => BillingMode::Unknown,
                    };

                    let credential_status = if secret_ref.is_some() {
                        CredentialStatus::Session
                    } else {
                        CredentialStatus::Missing
                    };

                    let id = Uuid::parse_str(&id_str).map_err(|e| {
                        rusqlite::Error::FromSqlConversionFailure(
                            0,
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    })?;

                    Ok(Provider {
                        id,
                        label,
                        kind,
                        protocol,
                        endpoint,
                        local_endpoint_allowed: local_allowed != 0,
                        enabled: enabled != 0,
                        billing_mode,
                        credential_status,
                        timeout_ms: timeout_ms as u32,
                        revision,
                    })
                } else {
                    Err(AppError::new(ErrorCode::NotFound, "Provider not found"))
                }
            })
            .await
    }

    pub async fn create_provider(&self, input: ProviderInput) -> Result<Provider> {
        validate_provider_input(&input)?;
        let id = Uuid::new_v4();
        let id_str = id.to_string();
        let now_ms = chrono::Utc::now().timestamp_millis();
        let kind_str = match input.kind {
            ProviderKind::OpenaiResponses => "openai_responses",
            ProviderKind::OpenaiCompatibleChat => "openai_compatible_chat",
            ProviderKind::Gemini => "gemini",
            ProviderKind::Anthropic => "anthropic",
        };
        let billing_str = match input.billing_mode {
            BillingMode::Api => "api",
            BillingMode::SubscriptionCli => "subscription_cli",
            BillingMode::Unknown => "unknown",
        };

        if input.timeout_ms < 10000 || input.timeout_ms > 300000 {
            return Err(AppError::new(
                ErrorCode::InvalidRequest,
                "timeout_ms must be between 10000 and 300000",
            ));
        }

        let cap_json = serde_json::to_string(&ProviderCapabilities {
            text: true,
            structured_edits: true,
            streaming: true,
            usage: true,
            stt: false,
            tts: false,
        })
        .unwrap_or_else(|_| "{}".to_string());

        let label = input.label.clone();
        let protocol = input.protocol.clone();
        let endpoint = input.endpoint.clone();
        let local_allowed = if input.local_endpoint_allowed { 1 } else { 0 };
        let enabled_val = if input.enabled { 1 } else { 0 };
        let timeout = input.timeout_ms as i64;

        self.db
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO providers (
                        id, label, kind, protocol, endpoint, local_endpoint_allowed, enabled,
                        billing_mode, secret_ref, credential_generation, timeout_ms, capability_json,
                        revision, created_at, updated_at
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, 0, ?9, ?10, 1, ?11, ?11)",
                    params![
                        id_str,
                        label,
                        kind_str,
                        protocol,
                        endpoint,
                        local_allowed,
                        enabled_val,
                        billing_str,
                        timeout,
                        cap_json,
                        now_ms
                    ],
                )?;
                Ok(())
            })
            .await?;

        self.get_provider(id).await
    }

    pub async fn patch_provider(&self, id: Id, patch: ProviderPatch) -> Result<Provider> {
        let existing = self.get_provider(id).await?;
        if existing.revision != patch.revision {
            return Err(AppError::new(
                ErrorCode::StaleRevision,
                format!(
                    "Provider revision mismatch: expected {}, got {}",
                    existing.revision, patch.revision
                ),
            ));
        }

        let new_label = patch.label.unwrap_or(existing.label);
        let endpoint_changed = patch
            .endpoint
            .as_ref()
            .map(|e| e != &existing.endpoint)
            .unwrap_or(false);
        let new_endpoint = patch.endpoint.unwrap_or(existing.endpoint);
        let new_local_allowed = patch
            .local_endpoint_allowed
            .unwrap_or(existing.local_endpoint_allowed);
        let new_enabled = patch.enabled.unwrap_or(existing.enabled);
        let new_timeout = patch.timeout_ms.unwrap_or(existing.timeout_ms);
        validate_provider_input(&ProviderInput {
            label: new_label.clone(),
            kind: existing.kind,
            protocol: existing.protocol.clone(),
            endpoint: new_endpoint.clone(),
            local_endpoint_allowed: new_local_allowed,
            enabled: new_enabled,
            billing_mode: existing.billing_mode,
            timeout_ms: new_timeout,
            owner_ack: true,
        })?;
        if !(10_000..=300_000).contains(&new_timeout) {
            return Err(AppError::new(
                ErrorCode::InvalidRequest,
                "timeout_ms must be between 10000 and 300000",
            ));
        }
        let new_rev = existing.revision + 1;
        let now_ms = chrono::Utc::now().timestamp_millis();
        let id_str = id.to_string();

        self.db
            .write(move |conn| {
                if endpoint_changed {
                    conn.execute(
                        "UPDATE providers SET
                            label = ?1, endpoint = ?2, local_endpoint_allowed = ?3,
                            enabled = ?4, timeout_ms = ?5, secret_ref = NULL,
                            credential_generation = credential_generation + 1,
                            revision = ?6, updated_at = ?7
                         WHERE id = ?8",
                        params![
                            new_label,
                            new_endpoint,
                            if new_local_allowed { 1 } else { 0 },
                            if new_enabled { 1 } else { 0 },
                            new_timeout as i64,
                            new_rev,
                            now_ms,
                            id_str
                        ],
                    )?;
                } else {
                    conn.execute(
                        "UPDATE providers SET
                            label = ?1, endpoint = ?2, local_endpoint_allowed = ?3,
                            enabled = ?4, timeout_ms = ?5, revision = ?6, updated_at = ?7
                         WHERE id = ?8",
                        params![
                            new_label,
                            new_endpoint,
                            if new_local_allowed { 1 } else { 0 },
                            if new_enabled { 1 } else { 0 },
                            new_timeout as i64,
                            new_rev,
                            now_ms,
                            id_str
                        ],
                    )?;
                }
                Ok(())
            })
            .await?;

        self.get_provider(id).await
    }

    pub async fn disable_or_delete_provider(
        &self,
        id: Id,
        expected_revision: Revision,
    ) -> Result<()> {
        let existing = self.get_provider(id).await?;
        if existing.revision != expected_revision {
            return Err(AppError::new(
                ErrorCode::StaleRevision,
                "Provider revision mismatch",
            ));
        }

        let id_str = id.to_string();
        let new_rev = existing.revision + 1;
        let now_ms = chrono::Utc::now().timestamp_millis();

        self.db
            .write(move |conn| {
                let count: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM usage_records WHERE provider_id = ?1",
                    params![id_str],
                    |r| r.get(0),
                )?;
                if count > 0 {
                    conn.execute(
                        "UPDATE providers SET enabled = 0, revision = ?1, updated_at = ?2 WHERE id = ?3",
                        params![new_rev, now_ms, id_str],
                    )?;
                } else {
                    let model_count: i64 = conn.query_row(
                        "SELECT COUNT(*) FROM models WHERE provider_id = ?1",
                        params![id_str],
                        |r| r.get(0),
                    )?;
                    if model_count > 0 {
                        conn.execute(
                            "UPDATE providers SET enabled = 0, revision = ?1, updated_at = ?2 WHERE id = ?3",
                            params![new_rev, now_ms, id_str],
                        )?;
                    } else {
                        conn.execute("DELETE FROM providers WHERE id = ?1", params![id_str])?;
                    }
                }
                Ok(())
            })
            .await
    }

    // --- Models CRUD ---

    pub async fn list_models(&self, provider_id: Option<Id>) -> Result<Vec<Model>> {
        let pid_str = provider_id.map(|p| p.to_string());
        self.db
            .read(move |conn| {
                let mut out = Vec::new();
                if let Some(pid) = pid_str {
                    let mut stmt = conn.prepare(
                        "SELECT id, provider_id, model_id, label, enabled, capabilities_json, context_limit_bytes, revision
                         FROM models WHERE provider_id = ?1 ORDER BY model_id ASC",
                    )?;
                    let rows = stmt.query_map(params![pid], parse_model_row)?;
                    for r in rows {
                        out.push(r?);
                    }
                } else {
                    let mut stmt = conn.prepare(
                        "SELECT id, provider_id, model_id, label, enabled, capabilities_json, context_limit_bytes, revision
                         FROM models ORDER BY model_id ASC",
                    )?;
                    let rows = stmt.query_map([], parse_model_row)?;
                    for r in rows {
                        out.push(r?);
                    }
                }
                Ok(out)
            })
            .await
    }

    pub async fn get_model(&self, id: Id) -> Result<Model> {
        let id_str = id.to_string();
        self.db
            .read(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, provider_id, model_id, label, enabled, capabilities_json, context_limit_bytes, revision
                     FROM models WHERE id = ?1",
                )?;
                let mut rows = stmt.query(params![id_str])?;
                if let Some(row) = rows.next()? {
                    parse_model_row(row).map_err(|e| AppError::new(ErrorCode::DbUnavailable, e.to_string()))
                } else {
                    Err(AppError::new(ErrorCode::NotFound, "Model not found"))
                }
            })
            .await
    }

    pub async fn create_model(&self, input: ModelInput) -> Result<Model> {
        let provider = self.get_provider(input.provider_id).await?;
        if !provider.enabled {
            return Err(AppError::new(
                ErrorCode::ModelUnavailable,
                "Provider is disabled",
            ));
        }

        let id = Uuid::new_v4();
        let id_str = id.to_string();
        let pid_str = input.provider_id.to_string();
        let caps_json = serde_json::to_string(&input.capabilities).map_err(|e| {
            AppError::new(
                ErrorCode::InvalidRequest,
                format!("Invalid capabilities: {e}"),
            )
        })?;

        if input.context_limit_bytes < 1024 || input.context_limit_bytes > 131072 {
            return Err(AppError::new(
                ErrorCode::InvalidRequest,
                "context_limit_bytes must be between 1024 and 131072",
            ));
        }

        let model_id = input.model_id.clone();
        let label = input.label.clone();
        let enabled_val = if input.enabled { 1 } else { 0 };
        let ctx_limit = input.context_limit_bytes as i64;

        self.db
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO models (
                        id, provider_id, model_id, label, enabled, capabilities_json,
                        context_limit_bytes, revision
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1)",
                    params![
                        id_str,
                        pid_str,
                        model_id,
                        label,
                        enabled_val,
                        caps_json,
                        ctx_limit
                    ],
                )?;
                Ok(())
            })
            .await?;

        self.get_model(id).await
    }

    pub async fn patch_model(&self, id: Id, patch: ModelPatch) -> Result<Model> {
        let existing = self.get_model(id).await?;
        if existing.revision != patch.revision {
            return Err(AppError::new(
                ErrorCode::StaleRevision,
                "Model revision mismatch",
            ));
        }

        let new_label = patch.label.unwrap_or(existing.label);
        let new_enabled = patch.enabled.unwrap_or(existing.enabled);
        let new_caps = patch.capabilities.unwrap_or(existing.capabilities);
        let new_caps_json = serde_json::to_string(&new_caps).unwrap_or_else(|_| "[]".to_string());
        let new_ctx_limit = patch
            .context_limit_bytes
            .unwrap_or(existing.context_limit_bytes);
        let new_rev = existing.revision + 1;
        let id_str = id.to_string();

        self.db
            .write(move |conn| {
                conn.execute(
                    "UPDATE models SET
                        label = ?1, enabled = ?2, capabilities_json = ?3,
                        context_limit_bytes = ?4, revision = ?5
                     WHERE id = ?6",
                    params![
                        new_label,
                        if new_enabled { 1 } else { 0 },
                        new_caps_json,
                        new_ctx_limit as i64,
                        new_rev,
                        id_str
                    ],
                )?;
                Ok(())
            })
            .await?;

        self.get_model(id).await
    }

    pub async fn disable_or_delete_model(&self, id: Id, expected_revision: Revision) -> Result<()> {
        let existing = self.get_model(id).await?;
        if existing.revision != expected_revision {
            return Err(AppError::new(
                ErrorCode::StaleRevision,
                "Model revision mismatch",
            ));
        }

        let id_str = id.to_string();
        let new_rev = existing.revision + 1;

        self.db
            .write(move |conn| {
                let role_count: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM model_roles WHERE model_id = ?1",
                    params![id_str],
                    |r| r.get(0),
                )?;
                if role_count > 0 {
                    return Err(AppError::new(
                        ErrorCode::ResourceInUse,
                        "Model is actively bound to a role",
                    ));
                }

                let usage_count: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM usage_records WHERE model_id = ?1",
                    params![id_str],
                    |r| r.get(0),
                )?;
                if usage_count > 0 {
                    conn.execute(
                        "UPDATE models SET enabled = 0, revision = ?1 WHERE id = ?2",
                        params![new_rev, id_str],
                    )?;
                } else {
                    conn.execute("DELETE FROM models WHERE id = ?1", params![id_str])?;
                }
                Ok(())
            })
            .await
    }

    // --- Roles View & Put ---

    pub async fn get_roles(&self) -> Result<RolesView> {
        self.db
            .read(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT role, model_id, tool_id, fallback_json, allow_paid_fallback, timeout_ms, revision
                     FROM model_roles ORDER BY role ASC",
                )?;
                let rows = stmt.query_map([], |row| {
                    let role_str: String = row.get(0)?;
                    let model_id_str: Option<String> = row.get(1)?;
                    let tool_id_str: Option<String> = row.get(2)?;
                    let fallback_json: String = row.get(3)?;
                    let allow_paid: i64 = row.get(4)?;
                    let timeout_ms: i64 = row.get(5)?;
                    let _rev: i64 = row.get(6)?;

                    let role = parse_role(&role_str);
                    let model_id = model_id_str.and_then(|s| Uuid::parse_str(&s).ok());
                    let tool_id = tool_id_str.and_then(|s| Uuid::parse_str(&s).ok());
                    let fallback_ids: Vec<String> =
                        serde_json::from_str(&fallback_json).unwrap_or_default();
                    let fallback_model_ids = fallback_ids
                        .into_iter()
                        .filter_map(|s| Uuid::parse_str(&s).ok())
                        .collect();

                    Ok(RoleBinding {
                        role,
                        model_id,
                        tool_id,
                        fallback_model_ids,
                        allow_paid_fallback: allow_paid != 0,
                        timeout_ms: timeout_ms as u32,
                    })
                })?;

                let mut bindings = Vec::new();
                for r in rows {
                    bindings.push(r?);
                }

                let rev_val: Option<String> = conn
                    .query_row(
                        "SELECT value_json FROM settings WHERE key = 'policy_version'",
                        [],
                        |r| r.get(0),
                    )
                    .ok();
                let config_rev = if let Some(v) = rev_val {
                    let obj: serde_json::Value =
                        serde_json::from_str(&v).unwrap_or(serde_json::Value::Null);
                    obj.get("config_revision")
                        .and_then(|c| c.as_i64())
                        .unwrap_or(1)
                } else {
                    1
                };

                Ok(RolesView {
                    revision: config_rev,
                    bindings,
                })
            })
            .await
    }

    pub async fn put_roles(&self, put: RolesPut) -> Result<RolesView> {
        let current = self.get_roles().await?;
        if current.revision != put.revision {
            return Err(AppError::new(
                ErrorCode::StaleRevision,
                format!(
                    "Roles config_revision mismatch: expected {}, got {}",
                    current.revision, put.revision
                ),
            ));
        }

        for b in &put.bindings {
            if b.role != Role::Coding && b.model_id.is_some() && b.tool_id.is_some() {
                return Err(AppError::new(
                    ErrorCode::InvalidRequest,
                    format!("Role {:?} cannot specify both model and tool", b.role),
                ));
            }
            if let Some(mid) = b.model_id {
                let model = self.get_model(mid).await?;
                match b.role {
                    Role::Stt => {
                        if !model.capabilities.contains(&ModelCapability::Stt) {
                            return Err(AppError::new(
                                ErrorCode::UnsupportedCapability,
                                "Model lacks STT capability",
                            ));
                        }
                    }
                    Role::Tts if !model.capabilities.contains(&ModelCapability::Tts) => {
                        return Err(AppError::new(
                            ErrorCode::UnsupportedCapability,
                            "Model lacks TTS capability",
                        ));
                    }
                    _ => {}
                }
            }
        }

        let bindings = put.bindings.clone();
        let new_config_rev = current.revision + 1;

        self.db
            .write(move |conn| {
                for b in bindings {
                    let role_str = role_to_str(b.role);
                    let model_str = b.model_id.map(|id| id.to_string());
                    let tool_str = b.tool_id.map(|id| id.to_string());
                    let fallback_strs: Vec<String> =
                        b.fallback_model_ids.iter().map(|id| id.to_string()).collect();
                    let fallback_json =
                        serde_json::to_string(&fallback_strs).unwrap_or_else(|_| "[]".to_string());
                    let allow_paid = if b.allow_paid_fallback { 1 } else { 0 };
                    let timeout = b.timeout_ms as i64;

                    conn.execute(
                        "INSERT INTO model_roles (
                            role, model_id, tool_id, fallback_json, allow_paid_fallback, timeout_ms, revision
                         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1)
                         ON CONFLICT(role) DO UPDATE SET
                            model_id = excluded.model_id,
                            tool_id = excluded.tool_id,
                            fallback_json = excluded.fallback_json,
                            allow_paid_fallback = excluded.allow_paid_fallback,
                            timeout_ms = excluded.timeout_ms,
                            revision = model_roles.revision + 1",
                        params![role_str, model_str, tool_str, fallback_json, allow_paid, timeout],
                    )?;
                }

                let val_json = format!(
                    "{{\"config_revision\":{},\"policy_version\":{}}}",
                    new_config_rev, new_config_rev
                );
                conn.execute(
                    "INSERT INTO settings (key, value_json, revision, updated_at)
                     VALUES ('policy_version', ?1, 1, ?2)
                     ON CONFLICT(key) DO UPDATE SET
                        value_json = excluded.value_json,
                        revision = settings.revision + 1,
                        updated_at = excluded.updated_at",
                    params![val_json, chrono::Utc::now().timestamp_millis()],
                )?;

                Ok(())
            })
            .await?;

        self.get_roles().await
    }

    // --- Chains CRUD ---

    pub async fn list_chains(&self) -> Result<Vec<Chain>> {
        self.db
            .read(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, name, enabled, revision FROM chains ORDER BY created_at ASC",
                )?;
                let rows = stmt.query_map([], |row| {
                    let id_str: String = row.get(0)?;
                    let name: String = row.get(1)?;
                    let enabled: i64 = row.get(2)?;
                    let revision: i64 = row.get(3)?;
                    let id = Uuid::parse_str(&id_str).map_err(|e| {
                        rusqlite::Error::FromSqlConversionFailure(
                            0,
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    })?;
                    Ok((id, name, enabled != 0, revision))
                })?;

                let mut chains = Vec::new();
                for r in rows {
                    let (id, name, enabled, revision) = r?;
                    let mut step_stmt = conn.prepare(
                        "SELECT position, purpose, model_id, tool_id, max_output_bytes
                         FROM chain_steps WHERE chain_id = ?1 ORDER BY position ASC",
                    )?;
                    let step_rows = step_stmt.query_map(params![id.to_string()], |srow| {
                        let pos: i64 = srow.get(0)?;
                        let purp_str: String = srow.get(1)?;
                        let mid_str: Option<String> = srow.get(2)?;
                        let tid_str: Option<String> = srow.get(3)?;
                        let max_bytes: i64 = srow.get(4)?;

                        let purpose = match purp_str.as_str() {
                            "research" => crate::types::ChainPurpose::Research,
                            "review" => crate::types::ChainPurpose::Review,
                            "plan" => crate::types::ChainPurpose::Plan,
                            "code" => crate::types::ChainPurpose::Code,
                            _ => crate::types::ChainPurpose::Research,
                        };

                        let model_id = mid_str.and_then(|s| Uuid::parse_str(&s).ok());
                        let tool_id = tid_str.and_then(|s| Uuid::parse_str(&s).ok());

                        Ok(ChainStep {
                            position: pos as u8,
                            purpose,
                            model_id,
                            tool_id,
                            max_output_bytes: max_bytes as u32,
                        })
                    })?;

                    let mut steps = Vec::new();
                    for sr in step_rows {
                        steps.push(sr?);
                    }

                    chains.push(Chain {
                        id,
                        name,
                        enabled,
                        steps,
                        revision,
                    });
                }

                Ok(chains)
            })
            .await
    }

    pub async fn create_chain(&self, input: ChainInput) -> Result<Chain> {
        if input.steps.is_empty() || input.steps.len() > 8 {
            return Err(AppError::new(
                ErrorCode::InvalidPlan,
                "Chain must have between 1 and 8 steps",
            ));
        }

        for (i, step) in input.steps.iter().enumerate() {
            if step.position != (i as u8 + 1) {
                return Err(AppError::new(
                    ErrorCode::InvalidPlan,
                    "Chain steps must be numbered sequentially starting from 1",
                ));
            }
            if step.model_id.is_none() && step.tool_id.is_none() {
                return Err(AppError::new(
                    ErrorCode::InvalidPlan,
                    "Chain step must specify model or tool",
                ));
            }
            if step.max_output_bytes < 1024 || step.max_output_bytes > 65536 {
                return Err(AppError::new(
                    ErrorCode::InvalidPlan,
                    "Chain step max_output_bytes must be between 1024 and 65536",
                ));
            }
        }

        let id = Uuid::new_v4();
        let id_str = id.to_string();
        let name = input.name.clone();
        let enabled_val = if input.enabled { 1 } else { 0 };
        let steps = input.steps.clone();
        let now_ms = chrono::Utc::now().timestamp_millis();

        self.db
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO chains (id, name, enabled, revision, created_at, updated_at)
                     VALUES (?1, ?2, ?3, 1, ?4, ?4)",
                    params![id_str, name, enabled_val, now_ms],
                )?;

                for s in steps {
                    let purp_str = match s.purpose {
                        crate::types::ChainPurpose::Research => "research",
                        crate::types::ChainPurpose::Review => "review",
                        crate::types::ChainPurpose::Plan => "plan",
                        crate::types::ChainPurpose::Code => "code",
                    };
                    conn.execute(
                        "INSERT INTO chain_steps (chain_id, position, purpose, model_id, tool_id, max_output_bytes)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                        params![
                            id_str,
                            s.position as i64,
                            purp_str,
                            s.model_id.map(|m| m.to_string()),
                            s.tool_id.map(|t| t.to_string()),
                            s.max_output_bytes as i64
                        ],
                    )?;
                }

                Ok(())
            })
            .await?;

        let chains = self.list_chains().await?;
        chains
            .into_iter()
            .find(|c| c.id == id)
            .ok_or_else(|| AppError::new(ErrorCode::NotFound, "Failed to retrieve created chain"))
    }

    pub async fn put_chain(&self, id: Id, put: ChainPut) -> Result<Chain> {
        let existing = self
            .list_chains()
            .await?
            .into_iter()
            .find(|c| c.id == id)
            .ok_or_else(|| AppError::new(ErrorCode::NotFound, "Chain not found"))?;

        if existing.revision != put.revision {
            return Err(AppError::new(
                ErrorCode::StaleRevision,
                "Chain revision mismatch",
            ));
        }

        if put.steps.is_empty() || put.steps.len() > 8 {
            return Err(AppError::new(
                ErrorCode::InvalidPlan,
                "Chain must have between 1 and 8 steps",
            ));
        }

        for (i, step) in put.steps.iter().enumerate() {
            if step.position != (i as u8 + 1) {
                return Err(AppError::new(
                    ErrorCode::InvalidPlan,
                    "Chain steps must be numbered sequentially starting from 1",
                ));
            }
            if step.model_id.is_none() && step.tool_id.is_none() {
                return Err(AppError::new(
                    ErrorCode::InvalidPlan,
                    "Chain step must specify model or tool",
                ));
            }
        }

        let id_str = id.to_string();
        let name = put.name.clone();
        let enabled_val = if put.enabled { 1 } else { 0 };
        let steps = put.steps.clone();
        let new_rev = existing.revision + 1;
        let now_ms = chrono::Utc::now().timestamp_millis();

        self.db
            .write(move |conn| {
                conn.execute(
                    "UPDATE chains SET name = ?1, enabled = ?2, revision = ?3, updated_at = ?4 WHERE id = ?5",
                    params![name, enabled_val, new_rev, now_ms, id_str],
                )?;

                conn.execute("DELETE FROM chain_steps WHERE chain_id = ?1", params![id_str])?;

                for s in steps {
                    let purp_str = match s.purpose {
                        crate::types::ChainPurpose::Research => "research",
                        crate::types::ChainPurpose::Review => "review",
                        crate::types::ChainPurpose::Plan => "plan",
                        crate::types::ChainPurpose::Code => "code",
                    };
                    conn.execute(
                        "INSERT INTO chain_steps (chain_id, position, purpose, model_id, tool_id, max_output_bytes)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                        params![
                            id_str,
                            s.position as i64,
                            purp_str,
                            s.model_id.map(|m| m.to_string()),
                            s.tool_id.map(|t| t.to_string()),
                            s.max_output_bytes as i64
                        ],
                    )?;
                }

                Ok(())
            })
            .await?;

        let chains = self.list_chains().await?;
        chains
            .into_iter()
            .find(|c| c.id == id)
            .ok_or_else(|| AppError::new(ErrorCode::NotFound, "Failed to retrieve updated chain"))
    }

    // --- Resolution ---

    pub async fn resolve(&self, selection: ModelSelection) -> Result<ResolvedModel> {
        // 1. Explicit override_id
        if let Some(oid) = selection.override_id {
            let model = self.get_model(oid).await?;
            if !model.enabled {
                return Err(AppError::new(
                    ErrorCode::ModelUnavailable,
                    "Explicit override model is disabled",
                ));
            }
            let provider = self.get_provider(model.provider_id).await?;
            if !provider.enabled {
                return Err(AppError::new(
                    ErrorCode::ModelUnavailable,
                    "Provider for override model is disabled",
                ));
            }
            return Ok(ResolvedModel {
                model,
                provider,
                source: ResolvedModelSource::Override,
            });
        }

        // 2. Project preferred model (if role is Coding and project_id provided)
        if selection.role == Role::Coding
            && let Some(pid) = selection.project_id
        {
            let pid_str = pid.to_string();
            let preferred_mid: Option<String> = self
                .db
                .read(move |conn| {
                    let res: rusqlite::Result<Option<String>> = conn.query_row(
                        "SELECT preferred_model_id FROM projects WHERE id = ?1",
                        params![pid_str],
                        |r| r.get(0),
                    );
                    match res {
                        Ok(v) => Ok(v),
                        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                        Err(e) => Err(AppError::new(ErrorCode::DbUnavailable, e.to_string())),
                    }
                })
                .await
                .ok()
                .flatten();

            if let Some(mid_str) = preferred_mid
                && let Ok(uuid) = Uuid::parse_str(&mid_str)
                && let Ok(model) = self.get_model(uuid).await
                && model.enabled
                && let Ok(provider) = self.get_provider(model.provider_id).await
                && provider.enabled
            {
                return Ok(ResolvedModel {
                    model,
                    provider,
                    source: ResolvedModelSource::Project,
                });
            }
        }

        // 3. Role default binding
        let roles = self.get_roles().await?;
        let binding = roles
            .bindings
            .into_iter()
            .find(|b| b.role == selection.role);

        if let Some(b) = binding {
            if let Some(mid) = b.model_id {
                let model = self.get_model(mid).await?;
                if model.enabled {
                    let provider = self.get_provider(model.provider_id).await?;
                    if provider.enabled {
                        return Ok(ResolvedModel {
                            model,
                            provider,
                            source: ResolvedModelSource::Role,
                        });
                    }
                }
            }

            // 4. Fallback
            if selection.fallback_consent && b.allow_paid_fallback {
                for fid in b.fallback_model_ids {
                    if let Ok(model) = self.get_model(fid).await
                        && model.enabled
                        && let Ok(provider) = self.get_provider(model.provider_id).await
                        && provider.enabled
                    {
                        return Ok(ResolvedModel {
                            model,
                            provider,
                            source: ResolvedModelSource::Fallback,
                        });
                    }
                }
            }
        }

        Err(AppError::new(
            ErrorCode::ModelUnavailable,
            format!("No available model resolved for role {:?}", selection.role),
        ))
    }

    // --- Endpoint Validation ---

    pub async fn validate_endpoint(&self, provider_id: Id) -> Result<ValidatedEndpoint> {
        let provider = self.get_provider(provider_id).await?;
        let parsed_url = Url::parse(&provider.endpoint).map_err(|e| {
            AppError::new(
                ErrorCode::InvalidRequest,
                format!("Invalid provider endpoint URL: {e}"),
            )
        })?;

        match parsed_url.scheme() {
            "http" | "https" => {}
            _ => {
                return Err(AppError::new(
                    ErrorCode::InvalidRequest,
                    "Provider endpoint must use http or https scheme",
                ));
            }
        }

        let host = parsed_url
            .host_str()
            .ok_or_else(|| AppError::new(ErrorCode::InvalidRequest, "Endpoint URL has no host"))?;

        let port = parsed_url.port_or_known_default().unwrap_or(80);

        let addrs: Vec<IpAddr> = tokio::net::lookup_host(format!("{}:{}", host, port))
            .await
            .map_err(|e| {
                AppError::new(
                    ErrorCode::Offline,
                    format!("DNS lookup failed for endpoint host {host}: {e}"),
                )
            })?
            .map(|s| s.ip())
            .collect();

        if addrs.is_empty() {
            return Err(AppError::new(
                ErrorCode::Offline,
                format!("DNS resolved no IP addresses for endpoint host {host}"),
            ));
        }

        if !provider.local_endpoint_allowed {
            for ip in &addrs {
                if is_local_or_private(ip) {
                    return Err(AppError::new(
                        ErrorCode::ActionDenied,
                        format!(
                            "Destination IP {} is private or loopback; local endpoints require local_endpoint_allowed",
                            ip
                        ),
                    ));
                }
            }
        }

        let origin = format!(
            "{}://{}{}",
            parsed_url.scheme(),
            host,
            if let Some(p) = parsed_url.port() {
                format!(":{}", p)
            } else {
                String::new()
            }
        );

        Ok(ValidatedEndpoint {
            url: parsed_url,
            resolved_addresses: addrs,
            origin,
            credential_generation: 0,
        })
    }

    // --- Generate with Retry & Usage ---

    pub async fn generate(
        &self,
        request: ModelRequest,
        cancel: CancellationToken,
    ) -> Result<ModelResult> {
        if cancel.is_cancelled() {
            return Err(AppError::new(
                ErrorCode::Cancelled,
                "Model request cancelled before start",
            ));
        }

        let provider = self.get_provider(request.model.provider_id).await?;
        if !provider.enabled {
            return Err(AppError::new(
                ErrorCode::ModelUnavailable,
                "Provider is disabled",
            ));
        }

        self.validate_endpoint(provider.id).await?;

        let adapter = (self.factory)(&provider, self.secrets.clone())?;

        let mut last_err = AppError::new(ErrorCode::ProviderError, "No attempts made");
        let max_attempts = 3;

        for attempt in 1..=max_attempts {
            if cancel.is_cancelled() {
                return Err(AppError::new(
                    ErrorCode::Cancelled,
                    "Model request cancelled",
                ));
            }

            let attempt_req_id = Uuid::new_v4();
            let now_ms = chrono::Utc::now().timestamp_millis();

            let res = adapter.generate(request.clone(), cancel.clone()).await;

            match res {
                Ok(result) => {
                    let _ = self
                        .record_usage(
                            provider.id,
                            Some(request.model.id),
                            None,
                            attempt_req_id,
                            &result.usage,
                            provider.billing_mode,
                            UsageRecordStatus::Succeeded,
                            now_ms,
                        )
                        .await;
                    return Ok(result);
                }
                Err(err) => {
                    let status = if err.code() == ErrorCode::Cancelled {
                        UsageRecordStatus::Cancelled
                    } else {
                        UsageRecordStatus::Failed
                    };

                    let _ = self
                        .record_usage(
                            provider.id,
                            Some(request.model.id),
                            None,
                            attempt_req_id,
                            &UsageObservation::default(),
                            provider.billing_mode,
                            status,
                            now_ms,
                        )
                        .await;

                    if err.code() == ErrorCode::Cancelled {
                        return Err(err);
                    }

                    let retryable = matches!(
                        err.code(),
                        ErrorCode::RateLimited
                            | ErrorCode::Busy
                            | ErrorCode::ProviderError
                            | ErrorCode::Timeout
                    );

                    last_err = err;

                    if !retryable || attempt == max_attempts {
                        break;
                    }

                    tokio::time::sleep(std::time::Duration::from_millis(50 * (1 << attempt))).await;
                }
            }
        }

        Err(last_err)
    }

    #[allow(clippy::too_many_arguments)]
    async fn record_usage(
        &self,
        provider_id: Id,
        model_id: Option<Id>,
        job_id: Option<Id>,
        request_id: Id,
        usage: &UsageObservation,
        billing_mode: BillingMode,
        status: UsageRecordStatus,
        at: UtcMs,
    ) -> Result<()> {
        let id_str = Uuid::new_v4().to_string();
        let pid_str = provider_id.to_string();
        let mid_str = model_id.map(|m| m.to_string());
        let jid_str = job_id.map(|j| j.to_string());
        let req_str = request_id.to_string();

        let billing_str = match billing_mode {
            BillingMode::Api => "api",
            BillingMode::SubscriptionCli => "subscription_cli",
            BillingMode::Unknown => "unknown",
        };

        let status_str = match status {
            UsageRecordStatus::Succeeded => "succeeded",
            UsageRecordStatus::Failed => "failed",
            UsageRecordStatus::Cancelled => "cancelled",
            UsageRecordStatus::Unknown => "unknown",
        };

        let in_tokens = usage.input_tokens.map(|t| t as i64);
        let out_tokens = usage.output_tokens.map(|t| t as i64);

        self.db
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO usage_records (
                        id, provider_id, model_id, job_id, request_id,
                        input_tokens, output_tokens, cost_coefficient, cost_scale,
                        currency, cost_source, billing_mode, status, at
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, NULL, NULL, NULL, ?8, ?9, ?10)",
                    params![
                        id_str,
                        pid_str,
                        mid_str,
                        jid_str,
                        req_str,
                        in_tokens,
                        out_tokens,
                        billing_str,
                        status_str,
                        at
                    ],
                )?;
                Ok(())
            })
            .await
    }
}

fn validate_provider_input(input: &ProviderInput) -> Result<()> {
    if !input.owner_ack {
        return Err(AppError::new(
            ErrorCode::ActionDenied,
            "Explicit owner acknowledgement is required",
        ));
    }
    if input.label.trim().is_empty() || input.label.len() > 80 {
        return Err(AppError::new(
            ErrorCode::InvalidRequest,
            "Provider label must be between 1 and 80 bytes",
        ));
    }
    let protocol_valid = match input.kind {
        ProviderKind::OpenaiResponses => input.protocol == "responses",
        ProviderKind::OpenaiCompatibleChat => input.protocol == "chat_completions",
        ProviderKind::Gemini => matches!(
            input.protocol.as_str(),
            "gemini_interactions" | "gemini_generate_content"
        ),
        ProviderKind::Anthropic => input.protocol == "anthropic_messages",
    };
    if !protocol_valid {
        return Err(AppError::new(
            ErrorCode::InvalidRequest,
            "Provider kind and protocol are inconsistent",
        ));
    }
    let endpoint = url::Url::parse(&input.endpoint)
        .map_err(|_| AppError::new(ErrorCode::InvalidRequest, "Provider endpoint is invalid"))?;
    if !endpoint.username().is_empty()
        || endpoint.password().is_some()
        || endpoint.query().is_some()
        || endpoint.fragment().is_some()
        || endpoint.host_str().is_none()
    {
        return Err(AppError::new(
            ErrorCode::InvalidRequest,
            "Provider endpoint cannot contain credentials, query, or fragment",
        ));
    }
    let loopback_host = endpoint.host_str().is_some_and(|host| {
        host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|address| address.is_loopback())
    });
    if endpoint.scheme() != "https"
        && !(endpoint.scheme() == "http" && input.local_endpoint_allowed && loopback_host)
    {
        return Err(AppError::new(
            ErrorCode::ScopeDenied,
            "Provider endpoint must use HTTPS unless an HTTP loopback endpoint is explicitly allowed",
        ));
    }
    Ok(())
}

fn parse_model_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Model> {
    let id_str: String = row.get(0)?;
    let pid_str: String = row.get(1)?;
    let model_id: String = row.get(2)?;
    let label: String = row.get(3)?;
    let enabled: i64 = row.get(4)?;
    let caps_json: String = row.get(5)?;
    let ctx_limit: i64 = row.get(6)?;
    let rev: i64 = row.get(7)?;

    let id = Uuid::parse_str(&id_str).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })?;
    let provider_id = Uuid::parse_str(&pid_str).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, Box::new(e))
    })?;
    let capabilities: Vec<ModelCapability> =
        serde_json::from_str(&caps_json).unwrap_or_else(|_| vec![ModelCapability::Text]);

    Ok(Model {
        id,
        provider_id,
        model_id,
        label,
        enabled: enabled != 0,
        capabilities,
        context_limit_bytes: ctx_limit as u32,
        revision: rev,
    })
}

fn parse_role(s: &str) -> Role {
    match s {
        "chat" => Role::Chat,
        "planning" => Role::Planning,
        "review" => Role::Review,
        "coding" => Role::Coding,
        "fallback" => Role::Fallback,
        "stt" => Role::Stt,
        "tts" => Role::Tts,
        _ => Role::Chat,
    }
}

fn role_to_str(r: Role) -> &'static str {
    match r {
        Role::Chat => "chat",
        Role::Planning => "planning",
        Role::Review => "review",
        Role::Coding => "coding",
        Role::Fallback => "fallback",
        Role::Stt => "stt",
        Role::Tts => "tts",
    }
}

fn is_local_or_private(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(ipv4) => {
            ipv4.is_loopback()
                || ipv4.is_private()
                || ipv4.is_link_local()
                || ipv4.is_broadcast()
                || ipv4.is_unspecified()
        }
        IpAddr::V6(ipv6) => ipv6.is_loopback() || ipv6.is_unspecified(),
    }
}
