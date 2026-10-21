use crate::artifacts::{ArtifactInput, ArtifactStore};
use crate::db::Db;
use crate::error::{AppError, ErrorCode, Result};
use crate::policy::ExecutionPermit;
use crate::types::{
    ActionClass, ArtifactKind, BrowserAction, BrowserActionKind, BrowserSettings, BrowserSite,
    BrowserTab, BrowserTabCapability, Capability, CapabilityDetails, CapabilityState,
    OperationResult, Provenance, ProvenanceSource,
};
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use std::net::IpAddr;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub struct BrowserService {
    db: Db,
    artifacts: Arc<ArtifactStore>,
}

fn find_executable(name: &str) -> bool {
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let full = dir.join(name);
            if full.is_file() {
                return true;
            }
        }
    }
    false
}

impl BrowserService {
    pub fn new(db: Db, artifacts: Arc<ArtifactStore>) -> Self {
        Self { db, artifacts }
    }

    pub fn db(&self) -> &Db {
        &self.db
    }

    pub async fn probe(&self) -> Result<Capability> {
        let settings = self.get_browser_settings().await?;
        let now = Utc::now().timestamp_millis();

        if !settings.enabled {
            return Ok(Capability {
                key: "browser".to_string(),
                state: crate::types::CapabilityState::Unavailable,
                reason: Some("Browser integration is disabled in settings".to_string()),
                version: None,
                checked_at: now,
                details: CapabilityDetails::default(),
            });
        }

        let Some(cdp_url) = settings.cdp_url.as_deref() else {
            let exe = settings.executable.as_deref().unwrap_or("chromium");
            let found = find_executable(exe)
                || find_executable("chromium-browser")
                || find_executable("google-chrome");
            return Ok(Capability {
                key: "browser".to_string(),
                state: CapabilityState::NeedsSetup,
                reason: Some(if found {
                    "Chromium is installed, but no loopback CDP endpoint is configured".to_string()
                } else {
                    "No supported Chromium browser binary or CDP endpoint was found".to_string()
                }),
                version: None,
                checked_at: now,
                details: CapabilityDetails::default(),
            });
        };
        validate_cdp_url(cdp_url)?;
        let version_url = format!("{}/json/version", cdp_url.trim_end_matches('/'));
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(std::time::Duration::from_secs(2))
            .build()
            .map_err(|error| AppError::new(ErrorCode::Internal, error.to_string()))?;
        if let Ok(response) = client.get(version_url).send().await
            && response.status().is_success()
        {
            let version = response
                .json::<serde_json::Value>()
                .await
                .ok()
                .and_then(|value| {
                    value
                        .get("Browser")
                        .and_then(|item| item.as_str())
                        .map(str::to_string)
                });
            Ok(Capability {
                key: "browser".to_string(),
                state: crate::types::CapabilityState::Available,
                reason: None,
                version,
                checked_at: now,
                details: CapabilityDetails {
                    operations: vec![
                        "open".to_string(),
                        "search".to_string(),
                        "read".to_string(),
                        "submit".to_string(),
                        "close".to_string(),
                    ],
                    ..Default::default()
                },
            })
        } else {
            Ok(Capability {
                key: "browser".to_string(),
                state: crate::types::CapabilityState::Unavailable,
                reason: Some("The configured loopback CDP endpoint is unreachable".to_string()),
                version: None,
                checked_at: now,
                details: CapabilityDetails::default(),
            })
        }
    }

    pub async fn tabs(&self) -> Result<Vec<BrowserTab>> {
        let settings = self.get_browser_settings().await?;
        if !settings.enabled {
            return Ok(vec![]);
        }

        let cdp_url = match settings.cdp_url {
            Some(u) => u,
            None => return Ok(vec![]),
        };
        validate_cdp_url(&cdp_url)?;

        // Query /json/list from CDP
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(std::time::Duration::from_secs(2))
            .build()
            .unwrap_or_default();

        let list_url = format!("{}/json/list", cdp_url.trim_end_matches('/'));
        let res = match client.get(&list_url).send().await {
            Ok(r) => r,
            Err(_) => return Ok(vec![]),
        };

        let items: Vec<serde_json::Value> = res.json().await.unwrap_or_default();
        let mut tabs = Vec::new();

        for item in items {
            let id = item
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let url = item
                .get("url")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let title = item
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let target_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("");

            if target_type == "page" && !id.is_empty() {
                let capabilities = vec![
                    BrowserTabCapability::Open,
                    BrowserTabCapability::Read,
                    BrowserTabCapability::Close,
                ];
                tabs.push(BrowserTab {
                    id,
                    url,
                    title,
                    owned: matches!(settings.cdp_mode, crate::types::CdpMode::Owned),
                    capabilities,
                });
            }
        }

        Ok(tabs)
    }

    pub async fn action(
        &self,
        input: BrowserAction,
        permit: &ExecutionPermit,
        cancel: CancellationToken,
    ) -> Result<OperationResult> {
        match input.kind {
            BrowserActionKind::Read | BrowserActionKind::Search => {
                permit.permits(&ActionClass::BrowserRead, None)?;
            }
            BrowserActionKind::Submit => {
                permit.permits(&ActionClass::BrowserSubmit, None)?;
            }
            BrowserActionKind::Shortcut | BrowserActionKind::Open => {
                permit.permits(&ActionClass::OpenUrl, None)?;
            }
            BrowserActionKind::Close => {
                // Closing tab doesn't require high-risk grant
            }
        }

        if cancel.is_cancelled() {
            return Err(AppError::new(
                ErrorCode::Cancelled,
                "Browser action cancelled",
            ));
        }

        if let Some(ref target_url) = input.url {
            validate_browser_url_literal(target_url)?;
        }
        let cap = self.probe().await?;
        if cap.state != crate::types::CapabilityState::Available {
            return self.handle_fallback_action(input).await;
        }
        // Resolve and reject unsafe destinations only after a local controller is
        // available; unavailable CDP must report its own honest fallback without
        // attempting unrelated external DNS.
        if let Some(ref target_url) = input.url {
            validate_browser_url(target_url).await?;
        }
        let settings = self.get_browser_settings().await?;
        let cdp = settings.cdp_url.ok_or_else(|| {
            AppError::new(
                ErrorCode::UnsupportedCapability,
                "CDP endpoint is not configured",
            )
        })?;
        validate_cdp_url(&cdp)?;
        match input.kind {
            BrowserActionKind::Open => {
                let url = input
                    .url
                    .unwrap_or_else(|| "https://duckduckgo.com".to_string());
                let tab = cdp_open(&cdp, &url).await?;
                Ok(OperationResult {
                    kind: "browser_open".to_string(),
                    message: format!("Opened browser tab to {url}"),
                    artifact_id: None,
                    text: None,
                    tab_id: Some(tab),
                    exit: None,
                })
            }
            BrowserActionKind::Shortcut => {
                let target_site = match input.site {
                    Some(BrowserSite::Chatgpt) => "https://chatgpt.com",
                    Some(BrowserSite::Gemini) => "https://gemini.google.com",
                    Some(BrowserSite::Claude) => "https://claude.ai",
                    None => "https://duckduckgo.com",
                };
                let tab = cdp_open(&cdp, target_site).await?;
                Ok(OperationResult {
                    kind: "browser_shortcut".to_string(),
                    message: format!("Opened AI shortcut to {}", target_site),
                    artifact_id: None,
                    text: None,
                    tab_id: Some(tab),
                    exit: None,
                })
            }
            BrowserActionKind::Search => {
                let query = input.query.unwrap_or_default();
                let encoded_query = percent_encoding::utf8_percent_encode(
                    &query,
                    percent_encoding::NON_ALPHANUMERIC,
                )
                .to_string();
                let search_url = format!("https://duckduckgo.com/?q={}", encoded_query);
                let tab = cdp_open(&cdp, &search_url).await?;
                Ok(OperationResult {
                    kind: "browser_search".to_string(),
                    message: format!("Searched web for '{}' via {}", query, search_url),
                    artifact_id: None,
                    text: None,
                    tab_id: Some(tab),
                    exit: None,
                })
            }
            BrowserActionKind::Read => {
                let tab_id = input.tab_id.ok_or_else(|| {
                    AppError::new(ErrorCode::InvalidRequest, "tab_id is required for read")
                })?;
                let target = cdp_target(&cdp, &tab_id).await?;
                let ws = target
                    .get("webSocketDebuggerUrl")
                    .and_then(|value| value.as_str())
                    .ok_or_else(|| {
                        AppError::new(
                            ErrorCode::UnsupportedCapability,
                            "CDP target has no debugger socket",
                        )
                    })?;
                let value=cdp_command(ws,"Runtime.evaluate",json!({"expression":"document.body ? document.body.innerText.slice(0, 5242880) : ''","returnByValue":true,"awaitPromise":false})).await?;
                let content = value
                    .pointer("/result/result/value")
                    .and_then(|item| item.as_str())
                    .unwrap_or_default()
                    .to_string();
                let now = Utc::now().timestamp_millis();

                let artifact = self
                    .artifacts
                    .put_bytes(
                        ArtifactInput {
                            job_id: Some(permit.job_id()),
                            task_id: None,
                            kind: ArtifactKind::Scan,
                            mime: "text/plain".to_string(),
                            provenance: Provenance {
                                source: ProvenanceSource::Browser,
                                provider_id: None,
                                model_id: None,
                                url: input.url,
                                observed_at: now,
                            },
                            max_bytes: 5 * 1024 * 1024,
                            expires_at: None,
                        },
                        content.as_bytes(),
                    )
                    .await?;

                Ok(OperationResult {
                    kind: "browser_read".to_string(),
                    message: "Page content extracted".to_string(),
                    artifact_id: Some(artifact.id),
                    text: Some(content),
                    tab_id: Some(tab_id),
                    exit: None,
                })
            }
            BrowserActionKind::Submit => Err(AppError::new(
                ErrorCode::UnsupportedCapability,
                "Automated website prompt submission is unavailable. Use an official API or manual handoff.",
            )),
            BrowserActionKind::Close => {
                let tab_id = input.tab_id.ok_or_else(|| {
                    AppError::new(ErrorCode::InvalidRequest, "tab_id is required for close")
                })?;
                let response = reqwest::Client::builder()
                    .no_proxy()
                    .timeout(std::time::Duration::from_secs(5))
                    .build()
                    .map_err(|error| AppError::new(ErrorCode::Internal, error.to_string()))?
                    .get(format!(
                        "{}/json/close/{}",
                        cdp.trim_end_matches('/'),
                        percent_encoding::utf8_percent_encode(
                            &tab_id,
                            percent_encoding::NON_ALPHANUMERIC
                        )
                    ))
                    .send()
                    .await
                    .map_err(|error| {
                        AppError::new(ErrorCode::ProviderError, "CDP close request failed")
                            .with_source(error)
                    })?;
                if !response.status().is_success() {
                    return Err(AppError::new(
                        ErrorCode::ToolFailed,
                        "Chromium refused to close the tab",
                    ));
                }
                Ok(OperationResult {
                    kind: "browser_close".to_string(),
                    message: format!("Closed browser tab {}", tab_id),
                    artifact_id: None,
                    text: None,
                    tab_id: None,
                    exit: None,
                })
            }
        }
    }

    async fn handle_fallback_action(&self, _input: BrowserAction) -> Result<OperationResult> {
        Err(AppError::new(
            ErrorCode::UnsupportedCapability,
            "Browser CDP is unavailable. Use an approved desktop launch and import the result with manual handoff.",
        ))
    }

    async fn get_browser_settings(&self) -> Result<BrowserSettings> {
        self.db
            .read(|conn| {
                let mut stmt =
                    conn.prepare("SELECT value_json FROM settings WHERE key = 'browser'")?;
                let mut rows = stmt.query([])?;
                if let Some(r) = rows.next()? {
                    let json_str: String = r.get(0)?;
                    let settings: BrowserSettings =
                        serde_json::from_str(&json_str).unwrap_or(BrowserSettings {
                            enabled: false,
                            executable: None,
                            profile_dir: None,
                            cdp_mode: crate::types::CdpMode::Disabled,
                            cdp_url: None,
                        });
                    Ok(settings)
                } else {
                    Ok(BrowserSettings {
                        enabled: false,
                        executable: None,
                        profile_dir: None,
                        cdp_mode: crate::types::CdpMode::Disabled,
                        cdp_url: None,
                    })
                }
            })
            .await
    }
}

pub(crate) fn validate_action_preview(input: &BrowserAction) -> Result<()> {
    if !input.owner_ack {
        return Err(AppError::new(
            ErrorCode::ActionDenied,
            "Explicit owner acknowledgement is required",
        ));
    }
    match input.kind {
        BrowserActionKind::Open => {
            let url = input.url.as_deref().ok_or_else(|| {
                AppError::new(ErrorCode::InvalidRequest, "url is required for open")
            })?;
            validate_browser_url_literal(url)
        }
        BrowserActionKind::Shortcut => {
            if input.site.is_none() {
                return Err(AppError::new(
                    ErrorCode::InvalidRequest,
                    "site is required for a browser shortcut",
                ));
            }
            Ok(())
        }
        BrowserActionKind::Search => {
            let query = input.query.as_deref().unwrap_or_default().trim();
            if query.is_empty() || query.len() > 200 {
                return Err(AppError::new(
                    ErrorCode::InvalidRequest,
                    "Search query must be between 1 and 200 bytes",
                ));
            }
            Ok(())
        }
        BrowserActionKind::Read | BrowserActionKind::Close => {
            if input.tab_id.as_deref().unwrap_or_default().is_empty() {
                return Err(AppError::new(
                    ErrorCode::InvalidRequest,
                    "tab_id is required for this browser action",
                ));
            }
            if let Some(url) = input.url.as_deref() {
                validate_browser_url_literal(url)?;
            }
            Ok(())
        }
        BrowserActionKind::Submit => Err(AppError::new(
            ErrorCode::UnsupportedCapability,
            "Automated website prompt submission is unavailable. Use an official API or manual handoff.",
        )),
    }
}

async fn validate_browser_url(url_str: &str) -> Result<()> {
    validate_browser_url_literal(url_str)?;
    let parsed = url::Url::parse(url_str).map_err(|e| {
        AppError::new(
            ErrorCode::InvalidRequest,
            format!("Invalid browser URL '{}': {}", url_str, e),
        )
    })?;

    let scheme = parsed.scheme();
    if scheme != "http" && scheme != "https" {
        return Err(AppError::new(
            ErrorCode::ScopeDenied,
            format!("URL scheme '{}' is forbidden in browser actions", scheme),
        ));
    }

    // Browser navigation is limited to public destinations.
    if let Some(host_str) = parsed.host_str() {
        if host_str.eq_ignore_ascii_case("localhost")
            || host_str.eq_ignore_ascii_case("metadata.google.internal")
        {
            return Err(AppError::new(
                ErrorCode::ScopeDenied,
                "Local, private, and metadata destinations are forbidden",
            ));
        }
        let port = parsed.port_or_known_default().unwrap_or(443);
        let addresses = tokio::net::lookup_host((host_str, port))
            .await
            .map_err(|_| {
                AppError::new(
                    ErrorCode::Offline,
                    "Browser destination could not be resolved",
                )
            })?;
        if addresses
            .into_iter()
            .any(|address| private_address(address.ip()))
        {
            return Err(AppError::new(
                ErrorCode::ScopeDenied,
                "Local, private, and metadata destinations are forbidden",
            ));
        }
    }

    Ok(())
}

fn validate_browser_url_literal(url_str: &str) -> Result<()> {
    let parsed = url::Url::parse(url_str).map_err(|error| {
        AppError::new(
            ErrorCode::InvalidRequest,
            format!("Invalid browser URL '{url_str}': {error}"),
        )
    })?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(AppError::new(
            ErrorCode::ScopeDenied,
            "Only HTTP and HTTPS browser destinations are allowed",
        ));
    }
    if let Some(host) = parsed.host_str()
        && let Ok(ip) = host.parse::<IpAddr>()
        && private_address(ip)
    {
        return Err(AppError::new(
            ErrorCode::ScopeDenied,
            "Local, private, and metadata destinations are forbidden",
        ));
    }
    Ok(())
}

fn validate_cdp_url(value: &str) -> Result<()> {
    let url = url::Url::parse(value)
        .map_err(|_| AppError::new(ErrorCode::InvalidRequest, "Invalid CDP URL"))?;
    if url.scheme() != "http" || !matches!(url.host_str(), Some("127.0.0.1" | "localhost")) {
        return Err(AppError::new(
            ErrorCode::ScopeDenied,
            "CDP endpoint must use loopback HTTP",
        ));
    }
    Ok(())
}
fn validate_cdp_websocket_url(value: &str) -> Result<()> {
    let url = url::Url::parse(value)
        .map_err(|_| AppError::new(ErrorCode::InvalidRequest, "Invalid CDP websocket URL"))?;
    if url.scheme() != "ws" || !matches!(url.host_str(), Some("127.0.0.1" | "localhost")) {
        return Err(AppError::new(
            ErrorCode::ScopeDenied,
            "CDP debugger socket must use loopback WebSocket",
        ));
    }
    Ok(())
}
fn private_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_broadcast()
                || ip.is_unspecified()
                || ip.octets()[0] == 0
        }
        IpAddr::V6(ip) => {
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
        }
    }
}
async fn cdp_open(base: &str, url: &str) -> Result<String> {
    let encoded = percent_encoding::utf8_percent_encode(url, percent_encoding::NON_ALPHANUMERIC);
    let response = reqwest::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|error| AppError::new(ErrorCode::Internal, error.to_string()))?
        .put(format!(
            "{}/json/new?{}",
            base.trim_end_matches('/'),
            encoded
        ))
        .send()
        .await
        .map_err(|error| {
            AppError::new(ErrorCode::ProviderError, "CDP open request failed").with_source(error)
        })?;
    if !response.status().is_success() {
        return Err(AppError::new(
            ErrorCode::ToolFailed,
            "Chromium refused to open the tab",
        ));
    }
    let value = response.json::<serde_json::Value>().await.map_err(|_| {
        AppError::new(
            ErrorCode::ProviderError,
            "Chromium returned an invalid target response",
        )
    })?;
    value
        .get("id")
        .and_then(|item| item.as_str())
        .map(str::to_string)
        .ok_or_else(|| {
            AppError::new(
                ErrorCode::ProviderError,
                "Chromium target response omitted id",
            )
        })
}
async fn cdp_target(base: &str, id: &str) -> Result<serde_json::Value> {
    let response = reqwest::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|error| AppError::new(ErrorCode::Internal, error.to_string()))?
        .get(format!("{}/json/list", base.trim_end_matches('/')))
        .send()
        .await
        .map_err(|error| {
            AppError::new(ErrorCode::ProviderError, "CDP target query failed").with_source(error)
        })?;
    let items = response
        .json::<Vec<serde_json::Value>>()
        .await
        .map_err(|_| {
            AppError::new(
                ErrorCode::ProviderError,
                "Chromium returned an invalid target list",
            )
        })?;
    items
        .into_iter()
        .find(|item| item.get("id").and_then(|value| value.as_str()) == Some(id))
        .ok_or_else(|| AppError::new(ErrorCode::NotFound, "Browser tab not found"))
}
async fn cdp_command(
    socket: &str,
    method: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value> {
    validate_cdp_websocket_url(socket)?;
    let (mut stream, _) = tokio_tungstenite::connect_async(socket)
        .await
        .map_err(|error| {
            AppError::new(ErrorCode::ProviderError, "CDP websocket connection failed")
                .with_source(error)
        })?;
    let id = 1u64;
    let message = json!({"id":id,"method":method,"params":params}).to_string();
    stream
        .send(tokio_tungstenite::tungstenite::Message::Text(
            message.into(),
        ))
        .await
        .map_err(|error| {
            AppError::new(ErrorCode::ProviderError, "CDP command send failed").with_source(error)
        })?;
    while let Some(item) = stream.next().await {
        let item = item.map_err(|error| {
            AppError::new(ErrorCode::ProviderError, "CDP response failed").with_source(error)
        })?;
        if let Ok(text) = item.to_text() {
            let value: serde_json::Value = serde_json::from_str(text).map_err(|_| {
                AppError::new(ErrorCode::ProviderError, "CDP returned invalid JSON")
            })?;
            if value.get("id").and_then(|value| value.as_u64()) == Some(id) {
                if value.get("error").is_some() {
                    return Err(AppError::new(
                        ErrorCode::ToolFailed,
                        "Chromium rejected the bounded CDP command",
                    ));
                }
                return Ok(value);
            }
        }
    }
    Err(AppError::new(
        ErrorCode::ProviderError,
        "CDP connection closed before response",
    ))
}
