pub mod anthropic;
pub mod gemini;
pub mod openai;

use crate::error::{AppError, ErrorCode, Result};
use crate::provider::ModelProvider;
use crate::secrets::SecretStore;
use crate::types::{Provider, ProviderKind};
use std::sync::Arc;

pub fn create(
    config: &Provider,
    secret_store: Arc<dyn SecretStore>,
) -> Result<Arc<dyn ModelProvider>> {
    match config.kind {
        ProviderKind::OpenaiResponses => {
            if config.protocol != "responses" {
                return Err(AppError::new(
                    ErrorCode::UnsupportedCapability,
                    format!(
                        "Invalid protocol '{}' for openai_responses, expected 'responses'",
                        config.protocol
                    ),
                ));
            }
            Ok(Arc::new(openai::OpenAiResponses::new(
                config.clone(),
                secret_store,
            )))
        }
        ProviderKind::OpenaiCompatibleChat => {
            if config.protocol != "chat_completions" {
                return Err(AppError::new(
                    ErrorCode::UnsupportedCapability,
                    format!(
                        "Invalid protocol '{}' for openai_compatible_chat, expected 'chat_completions'",
                        config.protocol
                    ),
                ));
            }
            Ok(Arc::new(openai::OpenAiChatCompatible::new(
                config.clone(),
                secret_store,
            )))
        }
        ProviderKind::Gemini => {
            if config.protocol != "gemini_interactions"
                && config.protocol != "gemini_generate_content"
            {
                return Err(AppError::new(
                    ErrorCode::UnsupportedCapability,
                    format!(
                        "Invalid protocol '{}' for gemini, expected 'gemini_interactions' or 'gemini_generate_content'",
                        config.protocol
                    ),
                ));
            }
            Ok(Arc::new(gemini::GeminiProvider::new(
                config.clone(),
                secret_store,
            )))
        }
        ProviderKind::Anthropic => {
            if config.protocol != "anthropic_messages" {
                return Err(AppError::new(
                    ErrorCode::UnsupportedCapability,
                    format!(
                        "Invalid protocol '{}' for anthropic, expected 'anthropic_messages'",
                        config.protocol
                    ),
                ));
            }
            Ok(Arc::new(anthropic::AnthropicProvider::new(
                config.clone(),
                secret_store,
            )))
        }
    }
}
