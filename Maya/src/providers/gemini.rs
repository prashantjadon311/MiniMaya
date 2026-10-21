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

pub struct GeminiProvider {
    provider: Provider,
    secrets: Arc<dyn SecretStore>,
    client: reqwest::Client,
}

impl GeminiProvider {
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

        // Error check
        if let Some(err) = val.get("error") {
            let msg = err
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("Gemini error");
            return Ok(ProviderEvent::Failed(
                if msg.contains("RESOURCE_EXHAUSTED") || msg.contains("rate_limit") {
                    ErrorCode::RateLimited
                } else {
                    ErrorCode::ProviderError
                },
            ));
        }

        // Check promptFeedback / blockReason
        if let Some(feedback) = val.get("promptFeedback")
            && let Some(block_reason) = feedback.get("blockReason").and_then(|b| b.as_str())
        {
            return Ok(ProviderEvent::Refused(format!(
                "Prompt blocked: {block_reason}"
            )));
        }

        // Check usage metadata (generateContent)
        if let Some(usage_meta) = val.get("usageMetadata") {
            let prompt_toks = usage_meta.get("promptTokenCount").and_then(|t| t.as_u64());
            let cand_toks = usage_meta
                .get("candidatesTokenCount")
                .and_then(|t| t.as_u64());
            if prompt_toks.is_some() || cand_toks.is_some() {
                return Ok(ProviderEvent::Usage(UsageObservation {
                    input_tokens: prompt_toks,
                    output_tokens: cand_toks,
                    money: None,
                }));
            }
        }

        // Check usage (interactions)
        if let Some(usage_obj) = val.get("usage") {
            let prompt_toks = usage_obj.get("prompt_tokens").and_then(|t| t.as_u64());
            let cand_toks = usage_obj.get("completion_tokens").and_then(|t| t.as_u64());
            if prompt_toks.is_some() || cand_toks.is_some() {
                return Ok(ProviderEvent::Usage(UsageObservation {
                    input_tokens: prompt_toks,
                    output_tokens: cand_toks,
                    money: None,
                }));
            }
        }

        // Check output in interactions
        if let Some(out_text) = val.get("output").and_then(|o| o.as_str()) {
            return Ok(ProviderEvent::TextDelta(out_text.to_string()));
        }

        // Check candidates in generateContent
        if let Some(candidates) = val.get("candidates").and_then(|c| c.as_array())
            && let Some(first) = candidates.first()
        {
            if let Some(reason) = first.get("finishReason").and_then(|r| r.as_str()) {
                if reason == "SAFETY" || reason == "RECITATION" {
                    return Ok(ProviderEvent::Refused(format!(
                        "Generation stopped due to {reason}"
                    )));
                }
                if reason == "STOP" {
                    return Ok(ProviderEvent::Completed {
                        actual_model_id: None,
                    });
                }
            }

            if let Some(content) = first.get("content")
                && let Some(parts) = content.get("parts").and_then(|p| p.as_array())
            {
                let mut text_part = String::new();
                for p in parts {
                    if let Some(t) = p.get("text").and_then(|txt| txt.as_str()) {
                        text_part.push_str(t);
                    }
                }
                if !text_part.is_empty() {
                    return Ok(ProviderEvent::TextDelta(text_part));
                }
            }
        }

        Ok(ProviderEvent::Ignored)
    }
}

impl ModelProvider for GeminiProvider {
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
                    "API key missing for Gemini provider",
                )
            })?;

            let base = self.provider.endpoint.trim_end_matches('/');

            let is_interactions = self.provider.protocol == "gemini_interactions";

            let (url, body) = if is_interactions {
                let u = format!("{base}/v1beta/interactions");
                let mut system_text = String::new();
                let mut user_text = String::new();

                for msg in &request.messages {
                    match msg.role {
                        crate::types::ModelMessageRole::System => {
                            system_text.push_str(&msg.text);
                            system_text.push('\n');
                        }
                        _ => {
                            user_text.push_str(&msg.text);
                            user_text.push('\n');
                        }
                    }
                }

                let b = serde_json::json!({
                    "model": request.model.model_id,
                    "input": user_text,
                    "system_instruction": system_text,
                    "stream": true,
                    "store": false
                });
                (u, b)
            } else {
                // gemini_generate_content
                let encoded_model = percent_encoding::utf8_percent_encode(
                    &request.model.model_id,
                    percent_encoding::NON_ALPHANUMERIC,
                );
                let u =
                    format!("{base}/v1beta/models/{encoded_model}:streamGenerateContent?alt=sse");

                let mut contents = Vec::new();
                let mut system_parts = Vec::new();

                for msg in &request.messages {
                    match msg.role {
                        crate::types::ModelMessageRole::System => {
                            system_parts.push(serde_json::json!({"text": msg.text}));
                        }
                        crate::types::ModelMessageRole::User => {
                            contents.push(serde_json::json!({
                                "role": "user",
                                "parts": [{"text": msg.text}]
                            }));
                        }
                        crate::types::ModelMessageRole::Assistant => {
                            contents.push(serde_json::json!({
                                "role": "model",
                                "parts": [{"text": msg.text}]
                            }));
                        }
                    }
                }

                let mut b = serde_json::json!({
                    "contents": contents
                });

                if !system_parts.is_empty() {
                    b["systemInstruction"] = serde_json::json!({
                        "parts": system_parts
                    });
                }

                (u, b)
            };

            let req_builder = self
                .client
                .post(&url)
                .header("x-goog-api-key", secret.as_str())
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
