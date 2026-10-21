use crate::error::{AppError, ErrorCode, Result};
use crate::provider::ModelProvider;
use crate::secrets::SecretStore;
use crate::types::{
    ModelRequest, ModelResult, ModelResultFinish, Provider, ProviderCapabilities, ProviderEvent,
    UsageObservation,
};
use futures_util::StreamExt;
use futures_util::future::BoxFuture;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

pub struct AnthropicProvider {
    provider: Provider,
    secrets: Arc<dyn SecretStore>,
    client: reqwest::Client,
}

impl AnthropicProvider {
    pub const API_VERSION: &'static str = "2023-06-01";

    pub fn new(provider: Provider, secrets: Arc<dyn SecretStore>) -> Self {
        Self {
            provider,
            secrets,
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap_or_default(),
        }
    }

    pub fn decode_event(bytes: &[u8]) -> Result<ProviderEvent> {
        let text = std::str::from_utf8(bytes).map_err(|e| {
            AppError::new(
                ErrorCode::ProviderError,
                format!("Invalid UTF-8 in event: {e}"),
            )
        })?;

        let trimmed = text.trim();
        if trimmed.is_empty() || trimmed.starts_with(':') {
            return Ok(ProviderEvent::Ignored);
        }

        let payload = if let Some(stripped) = trimmed.strip_prefix("data:") {
            stripped.trim()
        } else {
            trimmed
        };

        if payload == "[DONE]" {
            return Ok(ProviderEvent::Completed {
                actual_model_id: None,
            });
        }

        let val: serde_json::Value = match serde_json::from_str(payload) {
            Ok(v) => v,
            Err(_) => return Ok(ProviderEvent::Ignored),
        };

        let ev_type = val.get("type").and_then(|t| t.as_str()).unwrap_or("");

        match ev_type {
            "ping" => Ok(ProviderEvent::Ignored),
            "error" => {
                let err_obj = val.get("error");
                let err_type = err_obj
                    .and_then(|e| e.get("type"))
                    .and_then(|t| t.as_str())
                    .unwrap_or("");
                let msg = err_obj
                    .and_then(|e| e.get("message"))
                    .and_then(|m| m.as_str())
                    .unwrap_or("Anthropic error");
                if err_type == "rate_limit_error" || msg.contains("rate_limit") {
                    Ok(ProviderEvent::Failed(ErrorCode::RateLimited))
                } else if err_type == "overloaded_error" {
                    Ok(ProviderEvent::Failed(ErrorCode::Busy))
                } else {
                    Ok(ProviderEvent::Failed(ErrorCode::ProviderError))
                }
            }
            "message_start" => {
                let mut in_tokens = None;
                let mut model = None;
                if let Some(msg) = val.get("message") {
                    model = msg
                        .get("model")
                        .and_then(|m| m.as_str())
                        .map(|s| s.to_string());
                    if let Some(usage) = msg.get("usage") {
                        in_tokens = usage.get("input_tokens").and_then(|t| t.as_u64());
                    }
                }
                if let Some(in_t) = in_tokens {
                    Ok(ProviderEvent::Usage(UsageObservation {
                        input_tokens: Some(in_t),
                        output_tokens: None,
                        money: None,
                    }))
                } else if model.is_some() {
                    Ok(ProviderEvent::Completed {
                        actual_model_id: model,
                    })
                } else {
                    Ok(ProviderEvent::Ignored)
                }
            }
            "content_block_delta" => {
                if let Some(delta) = val.get("delta")
                    && let Some(text_val) = delta.get("text").and_then(|t| t.as_str())
                {
                    return Ok(ProviderEvent::TextDelta(text_val.to_string()));
                }
                Ok(ProviderEvent::Ignored)
            }
            "message_delta" => {
                if let Some(usage) = val.get("usage") {
                    let out_tokens = usage.get("output_tokens").and_then(|t| t.as_u64());
                    if let Some(out_t) = out_tokens {
                        return Ok(ProviderEvent::Usage(UsageObservation {
                            input_tokens: None,
                            output_tokens: Some(out_t),
                            money: None,
                        }));
                    }
                }
                Ok(ProviderEvent::Ignored)
            }
            "message_stop" => Ok(ProviderEvent::Completed {
                actual_model_id: None,
            }),
            _ => Ok(ProviderEvent::Ignored),
        }
    }
}

impl ModelProvider for AnthropicProvider {
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
        request: ModelRequest,
        cancel: CancellationToken,
    ) -> BoxFuture<'a, Result<ModelResult>> {
        Box::pin(async move {
            let secret = self.secrets.get(self.provider.id).await.map_err(|_| {
                AppError::new(
                    ErrorCode::AuthRequired,
                    "API key missing for Anthropic provider",
                )
            })?;

            let base = self.provider.endpoint.trim_end_matches('/');
            let url = format!("{base}/v1/messages");

            let mut system_text = String::new();
            let mut messages = Vec::new();

            for msg in &request.messages {
                match msg.role {
                    crate::types::ModelMessageRole::System => {
                        system_text.push_str(&msg.text);
                        system_text.push('\n');
                    }
                    crate::types::ModelMessageRole::User => {
                        messages.push(serde_json::json!({
                            "role": "user",
                            "content": msg.text
                        }));
                    }
                    crate::types::ModelMessageRole::Assistant => {
                        messages.push(serde_json::json!({
                            "role": "assistant",
                            "content": msg.text
                        }));
                    }
                }
            }

            let mut body = serde_json::json!({
                "model": request.model.model_id,
                "max_tokens": 4096,
                "messages": messages,
                "stream": true
            });

            if !system_text.is_empty() {
                body["system"] = serde_json::json!(system_text.trim());
            }

            let req_builder = self
                .client
                .post(&url)
                .header("x-api-key", secret.as_str())
                .header("anthropic-version", Self::API_VERSION)
                .header("Content-Type", "application/json")
                .timeout(std::time::Duration::from_millis(request.timeout_ms as u64))
                .json(&body);

            tokio::select! {
                _ = cancel.cancelled() => {
                    Err(AppError::new(ErrorCode::Cancelled, "Request cancelled"))
                }
                resp_res = req_builder.send() => {
                    let resp = resp_res.map_err(|e| {
                        if e.is_timeout() {
                            AppError::new(ErrorCode::Timeout, "Upstream request timed out")
                        } else {
                            AppError::new(ErrorCode::ProviderError, format!("HTTP transport error: {e}"))
                        }
                    })?;

                    let status = resp.status();
                    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
                        return Err(AppError::new(ErrorCode::AuthRequired, "Authentication rejected by upstream provider"));
                    }
                    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                        return Err(AppError::new(ErrorCode::RateLimited, "Upstream provider rate limit"));
                    }
                    if !status.is_success() {
                        return Err(AppError::new(ErrorCode::ProviderError, format!("Upstream HTTP status {status}")));
                    }

                    let mut stream = resp.bytes_stream();
                    let mut accumulated_text = String::new();
                    let mut actual_model: Option<String> = None;
                    let mut usage = UsageObservation::default();
                    let mut finish = ModelResultFinish::Complete;
                    let mut ignored_count = 0;

                    let mut line_buf = Vec::new();

                    while let Some(chunk_res) = stream.next().await {
                        if cancel.is_cancelled() {
                            return Err(AppError::new(ErrorCode::Cancelled, "Stream cancelled"));
                        }

                        let chunk = chunk_res.map_err(|e| {
                            AppError::new(ErrorCode::ProviderError, format!("Error reading SSE stream: {e}"))
                        })?;

                        for &b in &chunk {
                            if b == b'\n' {
                                if !line_buf.is_empty() {
                                    let ev = Self::decode_event(&line_buf)?;
                                    match ev {
                                        ProviderEvent::TextDelta(delta) => {
                                            accumulated_text.push_str(&delta);
                                            if accumulated_text.len() as u32 > request.max_output_bytes {
                                                accumulated_text.truncate(request.max_output_bytes as usize);
                                                finish = ModelResultFinish::Truncated;
                                                return Ok(ModelResult {
                                                    text: accumulated_text,
                                                    actual_model_id: actual_model,
                                                    usage,
                                                    finish,
                                                });
                                            }
                                        }
                                        ProviderEvent::Usage(u) => {
                                            if let Some(in_t) = u.input_tokens {
                                                usage.input_tokens = Some(in_t);
                                            }
                                            if let Some(out_t) = u.output_tokens {
                                                usage.output_tokens = Some(out_t);
                                            }
                                        }
                                        ProviderEvent::Completed { actual_model_id } => {
                                            if actual_model_id.is_some() {
                                                actual_model = actual_model_id;
                                            }
                                        }
                                        ProviderEvent::Refused(reason) => {
                                            return Err(AppError::new(ErrorCode::ProviderError, format!("Model refused: {reason}")));
                                        }
                                        ProviderEvent::Failed(err_code) => {
                                            return Err(AppError::new(err_code, "Provider emitted stream failure"));
                                        }
                                        ProviderEvent::Ignored => {
                                            ignored_count += 1;
                                            if ignored_count > 1000 {
                                                return Err(AppError::new(ErrorCode::ProviderError, "Exceeded 1000 ignored stream events"));
                                            }
                                        }
                                    }
                                    line_buf.clear();
                                }
                            } else if b != b'\r' {
                                line_buf.push(b);
                            }
                        }
                    }

                    Ok(ModelResult {
                        text: accumulated_text,
                        actual_model_id: actual_model,
                        usage,
                        finish,
                    })
                }
            }
        })
    }
}
