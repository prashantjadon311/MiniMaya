use crate::error::{AppError, ErrorCode, Result};
use crate::provider::ModelProvider;
use crate::secrets::SecretStore;
use crate::types::{
    ModelOutputMode, ModelRequest, ModelResult, ModelResultFinish, Provider, ProviderCapabilities,
    ProviderEvent, UsageObservation,
};
use futures_util::StreamExt;
use futures_util::future::BoxFuture;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

// --- OpenAI Responses Adapter ---

pub struct OpenAiResponses {
    provider: Provider,
    secrets: Arc<dyn SecretStore>,
    client: reqwest::Client,
}

impl OpenAiResponses {
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

        // Check for error payload
        if let Some(err) = val.get("error") {
            let msg = err
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("OpenAI error");
            return Ok(ProviderEvent::Failed(if msg.contains("rate_limit") {
                ErrorCode::RateLimited
            } else {
                ErrorCode::ProviderError
            }));
        }

        // Check for refusal
        if let Some(refusal) = val.get("refusal").and_then(|r| r.as_str()) {
            return Ok(ProviderEvent::Refused(refusal.to_string()));
        }

        // Check for usage
        if let Some(usage_obj) = val.get("usage") {
            let in_toks = usage_obj.get("input_tokens").and_then(|t| t.as_u64());
            let out_toks = usage_obj.get("output_tokens").and_then(|t| t.as_u64());
            if in_toks.is_some() || out_toks.is_some() {
                return Ok(ProviderEvent::Usage(UsageObservation {
                    input_tokens: in_toks,
                    output_tokens: out_toks,
                    money: None,
                }));
            }
        }

        // Check for delta in response.text.delta or output delta
        if let Some(delta) = val.get("delta").and_then(|d| d.as_str()) {
            return Ok(ProviderEvent::TextDelta(delta.to_string()));
        }

        if let Some(delta) = val
            .get("response")
            .and_then(|r| r.get("output_item"))
            .and_then(|o| o.get("text"))
            .and_then(|t| t.as_str())
        {
            return Ok(ProviderEvent::TextDelta(delta.to_string()));
        }

        // Check for completed response
        if let Some(status) = val.get("status").and_then(|s| s.as_str())
            && status == "completed"
        {
            let model = val
                .get("model")
                .and_then(|m| m.as_str())
                .map(|s| s.to_string());
            return Ok(ProviderEvent::Completed {
                actual_model_id: model,
            });
        }

        Ok(ProviderEvent::Ignored)
    }
}

impl ModelProvider for OpenAiResponses {
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
                    "API credential missing for provider",
                )
            })?;

            let base = self.provider.endpoint.trim_end_matches('/');
            let url = format!("{base}/responses");

            // Format messages
            let mut inputs = Vec::new();
            for msg in &request.messages {
                let role = match msg.role {
                    crate::types::ModelMessageRole::System => "system",
                    crate::types::ModelMessageRole::User => "user",
                    crate::types::ModelMessageRole::Assistant => "assistant",
                };
                inputs.push(serde_json::json!({
                    "role": role,
                    "content": [{"type": "input_text", "text": msg.text}]
                }));
            }

            let mut body = serde_json::json!({
                "model": request.model.model_id,
                "input": inputs,
                "store": false,
                "stream": true
            });

            if request.output == ModelOutputMode::EditManifest {
                body["instructions"] = serde_json::json!(
                    "Output a strict JSON object conforming to EditManifest schema with keys schema_version (number), summary (string), changes (array)."
                );
            }

            let req_builder = self
                .client
                .post(&url)
                .header("Authorization", format!("Bearer {}", secret.as_str()))
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

                    // Process SSE stream
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
                                            usage = u;
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

// --- OpenAI Compatible Chat Adapter ---

pub struct OpenAiChatCompatible {
    provider: Provider,
    secrets: Arc<dyn SecretStore>,
    client: reqwest::Client,
}

impl OpenAiChatCompatible {
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

        if let Some(err) = val.get("error") {
            let msg = err
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("Chat error");
            return Ok(ProviderEvent::Failed(if msg.contains("rate_limit") {
                ErrorCode::RateLimited
            } else {
                ErrorCode::ProviderError
            }));
        }

        // Usage in chat stream
        if let Some(usage_obj) = val.get("usage") {
            let in_toks = usage_obj.get("prompt_tokens").and_then(|t| t.as_u64());
            let out_toks = usage_obj.get("completion_tokens").and_then(|t| t.as_u64());
            if in_toks.is_some() || out_toks.is_some() {
                return Ok(ProviderEvent::Usage(UsageObservation {
                    input_tokens: in_toks,
                    output_tokens: out_toks,
                    money: None,
                }));
            }
        }

        let model = val
            .get("model")
            .and_then(|m| m.as_str())
            .map(|s| s.to_string());

        if let Some(choices) = val.get("choices").and_then(|c| c.as_array())
            && let Some(first) = choices.first()
        {
            if let Some(finish_reason) = first.get("finish_reason").and_then(|f| f.as_str())
                && (finish_reason == "stop" || finish_reason == "length")
            {
                return Ok(ProviderEvent::Completed {
                    actual_model_id: model,
                });
            }
            if let Some(delta) = first.get("delta")
                && let Some(content) = delta.get("content").and_then(|c| c.as_str())
            {
                return Ok(ProviderEvent::TextDelta(content.to_string()));
            }
        }

        Ok(ProviderEvent::Ignored)
    }
}

impl ModelProvider for OpenAiChatCompatible {
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            text: true,
            structured_edits: false,
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
            let secret_opt = self.secrets.get(self.provider.id).await.ok();

            let base = self.provider.endpoint.trim_end_matches('/');
            let url = format!("{base}/chat/completions");

            let mut messages = Vec::new();
            for msg in &request.messages {
                let role = match msg.role {
                    crate::types::ModelMessageRole::System => "system",
                    crate::types::ModelMessageRole::User => "user",
                    crate::types::ModelMessageRole::Assistant => "assistant",
                };
                messages.push(serde_json::json!({
                    "role": role,
                    "content": msg.text
                }));
            }

            let body = serde_json::json!({
                "model": request.model.model_id,
                "messages": messages,
                "stream": true,
                "stream_options": {"include_usage": true}
            });

            let mut req_builder = self
                .client
                .post(&url)
                .header("Content-Type", "application/json")
                .timeout(std::time::Duration::from_millis(request.timeout_ms as u64))
                .json(&body);

            if let Some(sec) = secret_opt {
                req_builder =
                    req_builder.header("Authorization", format!("Bearer {}", sec.as_str()));
            }

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
                                            usage = u;
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
