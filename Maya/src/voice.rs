use crate::artifacts::{ArtifactInput, ArtifactStore};
use crate::db::Db;
use crate::error::{AppError, ErrorCode, Result};
use crate::policy::{ExecutionPermit, Policy};
use crate::secrets::SecretStore;
use crate::types::{
    ActionClass, ArtifactKind, AudioInput, AudioOutput, AudioOutputMode, Capability,
    CapabilityDetails, CapabilityState, Digest, Language, Provenance, ProvenanceSource,
    SpeechInput, Transcript, VoiceInfo,
};
use rusqlite::params;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const MAX_AUDIO_BYTES: usize = 8 * 1024 * 1024; // 8 MiB
const MAX_SPEECH_TEXT_BYTES: usize = 8192; // 8 KiB

#[derive(Clone)]
pub struct VoiceService {
    db: Db,
    secrets: Arc<dyn SecretStore>,
    artifacts: Option<Arc<ArtifactStore>>,
    client: reqwest::Client,
}

impl VoiceService {
    pub fn new(
        db: Db,
        secrets: Arc<dyn SecretStore>,
        artifacts: Option<Arc<ArtifactStore>>,
    ) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        Self {
            db,
            secrets,
            artifacts,
            client,
        }
    }

    pub fn db(&self) -> &Db {
        &self.db
    }

    pub fn secrets(&self) -> Arc<dyn SecretStore> {
        self.secrets.clone()
    }

    pub async fn transcribe(
        &self,
        audio: AudioInput,
        permit: &ExecutionPermit,
        cancel: CancellationToken,
    ) -> Result<Transcript> {
        if cancel.is_cancelled() {
            return Err(AppError::new(
                ErrorCode::Cancelled,
                "Transcription cancelled",
            ));
        }

        permit.permits(&ActionClass::VoiceUpload, None)?;

        if audio.data.is_empty() {
            return Err(AppError::new(
                ErrorCode::InvalidRequest,
                "Audio data cannot be empty",
            ));
        }

        if audio.data.len() > MAX_AUDIO_BYTES {
            return Err(AppError::new(
                ErrorCode::BodyTooLarge,
                format!(
                    "Audio input size {} exceeds maximum 8 MiB limit",
                    audio.data.len()
                ),
            ));
        }

        let mime = audio.mime.to_lowercase();
        let extension = match mime.as_str() {
            "audio/webm" => "audio.webm",
            "audio/ogg" => "audio.ogg",
            "audio/wav" | "audio/x-wav" => "audio.wav",
            "audio/mpeg" | "audio/mp3" => "audio.mp3",
            "audio/mp4" | "audio/m4a" => "audio.m4a",
            _ => {
                return Err(AppError::new(
                    ErrorCode::UnsupportedMedia,
                    format!("Unsupported audio MIME: {}", audio.mime),
                ));
            }
        };

        if audio.payload_digest != Digest::ZERO {
            let computed = Digest::of(&audio.data);
            if computed != audio.payload_digest {
                return Err(AppError::new(
                    ErrorCode::IntegrityFailure,
                    "Audio payload digest does not match actual bytes",
                ));
            }
        }

        let model_id = audio.model_id;
        let (provider_id, external_id) = self
            .db
            .read(move |conn| {
                let mut stmt = conn
                    .prepare("SELECT provider_id, model_id, enabled FROM models WHERE id = ?1")?;
                let (prov_id, ext_id, enabled): (String, String, i64) = stmt
                    .query_row(params![model_id.to_string()], |row| {
                        Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                    })
                    .map_err(|e| match e {
                        rusqlite::Error::QueryReturnedNoRows => AppError::new(
                            ErrorCode::NotFound,
                            format!("Model {} not found", model_id),
                        ),
                        other => AppError::from(other),
                    })?;

                if enabled == 0 {
                    return Err(AppError::new(
                        ErrorCode::InvalidRequest,
                        format!("Model {} is disabled", model_id),
                    ));
                }

                Ok((prov_id, ext_id))
            })
            .await?;

        let provider_uuid = Uuid::parse_str(&provider_id).map_err(|_| {
            AppError::new(ErrorCode::IntegrityFailure, "Stored provider ID is invalid")
        })?;

        let (base_url, secret_ref) = self
            .db
            .read(move |conn| {
                let mut stmt = conn
                    .prepare("SELECT endpoint, secret_ref, enabled FROM providers WHERE id = ?1")?;
                let (url, sref, enabled): (String, Option<String>, i64) = stmt
                    .query_row(params![provider_id], |row| {
                        Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                    })
                    .map_err(|e| match e {
                        rusqlite::Error::QueryReturnedNoRows => {
                            AppError::new(ErrorCode::NotFound, "Provider not found")
                        }
                        other => AppError::from(other),
                    })?;

                if enabled == 0 {
                    return Err(AppError::new(
                        ErrorCode::InvalidRequest,
                        "Provider is disabled",
                    ));
                }

                Ok((url, sref))
            })
            .await?;

        let secret_ref = secret_ref.ok_or_else(|| {
            AppError::new(
                ErrorCode::ModelUnavailable,
                "Provider credential is not configured",
            )
        })?;
        let secret_id = secret_reference_id(&secret_ref, provider_uuid).ok_or_else(|| {
            AppError::new(
                ErrorCode::ModelUnavailable,
                format!("Invalid secret ID format in provider: {secret_ref}"),
            )
        })?;

        let secret = self.secrets.get(secret_id).await.map_err(|_| {
            AppError::new(
                ErrorCode::ModelUnavailable,
                format!("Provider credential {secret_ref} not configured"),
            )
        })?;

        let endpoint = format!("{}/audio/transcriptions", base_url.trim_end_matches('/'));

        let part = reqwest::multipart::Part::bytes(audio.data)
            .file_name(extension)
            .mime_str(&audio.mime)
            .map_err(|e| {
                AppError::new(ErrorCode::InvalidRequest, format!("Invalid MIME type: {e}"))
            })?;

        let form = reqwest::multipart::Form::new()
            .part("file", part)
            .text("model", external_id)
            .text("language", audio.language.to_string());

        let req = self
            .client
            .post(&endpoint)
            .header(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {}", secret.expose_str()),
            )
            .multipart(form);

        tokio::select! {
            _ = cancel.cancelled() => Err(AppError::new(ErrorCode::Cancelled, "Transcription cancelled")),
            res = req.send() => {
                let res = res.map_err(|e| {
                    AppError::new(
                        ErrorCode::ProviderError,
                        format!("Audio transcription request failed: {e}"),
                    )
                })?;

                if !res.status().is_success() {
                    let status = res.status();
                    let body = res.text().await.unwrap_or_default();
                    return Err(AppError::new(
                        ErrorCode::ProviderError,
                        format!("Provider returned error {status}: {body}"),
                    ));
                }

                let json_res: serde_json::Value = res.json().await.map_err(|e| {
                    AppError::new(
                        ErrorCode::ProviderError,
                        format!("Failed to parse transcription response: {e}"),
                    )
                })?;

                let raw_text = json_res
                    .get("text")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        AppError::new(
                            ErrorCode::ProviderError,
                            "Missing text field in transcript response",
                        )
                    })?;

                let clean_text = Policy::redact_static(raw_text);

                Ok(Transcript {
                    text: clean_text,
                    language: audio.language,
                    model_id: audio.model_id,
                    usage_record_id: None,
                })
            }
        }
    }

    pub async fn synthesize(
        &self,
        input: SpeechInput,
        permit: &ExecutionPermit,
        cancel: CancellationToken,
    ) -> Result<AudioOutput> {
        if cancel.is_cancelled() {
            return Err(AppError::new(
                ErrorCode::Cancelled,
                "Speech synthesis cancelled",
            ));
        }

        if input.text.is_empty() {
            return Err(AppError::new(
                ErrorCode::InvalidRequest,
                "Speech text cannot be empty",
            ));
        }

        if input.text.len() > MAX_SPEECH_TEXT_BYTES {
            return Err(AppError::new(
                ErrorCode::InvalidRequest,
                format!(
                    "Speech text length {} exceeds 8 KiB limit",
                    input.text.len()
                ),
            ));
        }

        if input.rate < 0.5 || input.rate > 2.0 {
            return Err(AppError::new(
                ErrorCode::InvalidRequest,
                format!("Speech rate {} must be between 0.5 and 2.0", input.rate),
            ));
        }

        let model_id = match input.model_id {
            None => {
                // Local / browser synthesis mode
                return Ok(AudioOutput {
                    mode: AudioOutputMode::Browser,
                    audio_artifact_id: None,
                    mime: None,
                    text: Some(input.text),
                });
            }
            Some(m) => m,
        };

        // Cloud speech synthesis requires VoiceUpload permit
        permit.permits(&ActionClass::VoiceUpload, None)?;

        let (provider_id_str, external_id) = self
            .db
            .read(move |conn| {
                let mut stmt = conn
                    .prepare("SELECT provider_id, model_id, enabled FROM models WHERE id = ?1")?;
                let (prov_id, ext_id, enabled): (String, String, i64) = stmt
                    .query_row(params![model_id.to_string()], |row| {
                        Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                    })
                    .map_err(|e| match e {
                        rusqlite::Error::QueryReturnedNoRows => AppError::new(
                            ErrorCode::NotFound,
                            format!("Model {} not found", model_id),
                        ),
                        other => AppError::from(other),
                    })?;

                if enabled == 0 {
                    return Err(AppError::new(
                        ErrorCode::InvalidRequest,
                        format!("Model {} is disabled", model_id),
                    ));
                }

                Ok((prov_id, ext_id))
            })
            .await?;

        let provider_id = Uuid::parse_str(&provider_id_str).map_err(|_| {
            AppError::new(
                ErrorCode::Internal,
                format!("Invalid provider ID: {provider_id_str}"),
            )
        })?;

        let (base_url, secret_ref) = self
            .db
            .read(move |conn| {
                let mut stmt = conn
                    .prepare("SELECT endpoint, secret_ref, enabled FROM providers WHERE id = ?1")?;
                let (url, sref, enabled): (String, Option<String>, i64) = stmt
                    .query_row(params![provider_id.to_string()], |row| {
                        Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                    })
                    .map_err(|e| match e {
                        rusqlite::Error::QueryReturnedNoRows => {
                            AppError::new(ErrorCode::NotFound, "Provider not found")
                        }
                        other => AppError::from(other),
                    })?;

                if enabled == 0 {
                    return Err(AppError::new(
                        ErrorCode::InvalidRequest,
                        "Provider is disabled",
                    ));
                }

                Ok((url, sref))
            })
            .await?;

        let secret_ref = secret_ref.ok_or_else(|| {
            AppError::new(
                ErrorCode::ModelUnavailable,
                "Provider credential is not configured",
            )
        })?;
        let secret_id = secret_reference_id(&secret_ref, provider_id).ok_or_else(|| {
            AppError::new(
                ErrorCode::ModelUnavailable,
                format!("Invalid secret ID format in provider: {secret_ref}"),
            )
        })?;

        let secret = self.secrets.get(secret_id).await.map_err(|_| {
            AppError::new(
                ErrorCode::ModelUnavailable,
                format!("Provider credential {secret_ref} not configured"),
            )
        })?;

        let endpoint = format!("{}/audio/speech", base_url.trim_end_matches('/'));

        let payload = json!({
            "model": external_id,
            "input": input.text,
            "voice": input.voice,
            "speed": input.rate,
        });

        let req = self
            .client
            .post(&endpoint)
            .header(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {}", secret.expose_str()),
            )
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .json(&payload);

        tokio::select! {
            _ = cancel.cancelled() => Err(AppError::new(ErrorCode::Cancelled, "Speech synthesis cancelled")),
            res = req.send() => {
                let res = res.map_err(|e| {
                    AppError::new(
                        ErrorCode::ProviderError,
                        format!("Speech synthesis request failed: {e}"),
                    )
                })?;

                if !res.status().is_success() {
                    let status = res.status();
                    let body = res.text().await.unwrap_or_default();
                    return Err(AppError::new(
                        ErrorCode::ProviderError,
                        format!("Provider returned error {status}: {body}"),
                    ));
                }

                let bytes = res.bytes().await.map_err(|e| {
                    AppError::new(
                        ErrorCode::ProviderError,
                        format!("Failed to read speech audio stream: {e}"),
                    )
                })?;

                let artifact_id = if let Some(ref artifacts) = self.artifacts {
                    let meta = ArtifactInput {
                        job_id: Some(permit.job_id()),
                        task_id: None,
                        kind: ArtifactKind::Audio,
                        mime: "audio/mpeg".to_string(),
                        provenance: Provenance {
                            source: ProvenanceSource::Model,
                            provider_id: Some(provider_id),
                            model_id: Some(model_id),
                            url: None,
                            observed_at: chrono::Utc::now().timestamp_millis(),
                        },
                        max_bytes: MAX_AUDIO_BYTES as u64,
                        expires_at: None,
                    };

                    let art = artifacts.put_bytes(meta, &bytes).await?;
                    Some(art.id)
                } else {
                    None
                };

                Ok(AudioOutput {
                    mode: AudioOutputMode::Audio,
                    audio_artifact_id: artifact_id,
                    mime: Some("audio/mpeg".to_string()),
                    text: None,
                })
            }
        }
    }

    pub async fn capabilities(&self) -> Result<Vec<Capability>> {
        let now = chrono::Utc::now().timestamp_millis();

        // 1. Browser TTS capability
        let browser_tts = Capability {
            key: "voice_browser_tts".to_string(),
            state: CapabilityState::Available,
            reason: None,
            version: Some("1.0".to_string()),
            checked_at: now,
            details: CapabilityDetails {
                protocols: vec!["speechSynthesis".to_string()],
                languages: vec![Language::English, Language::Hindi],
                voices: vec![VoiceInfo {
                    id: "system-default".to_string(),
                    label: "System Default Voice".to_string(),
                    languages: vec![Language::English, Language::Hindi],
                }],
                operations: vec!["browser_speech_synthesis".to_string()],
                limitations: vec![
                    "Local synthesis depends on available browser/system voices".to_string(),
                    "No guaranteed Hinglish synthesized voice without external adapter".to_string(),
                ],
                sandbox_verified: true,
            },
        };

        // 2. Check cloud STT and TTS models in database
        let models_info = self
            .db
            .read(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT m.id, m.model_id, m.enabled, p.secret_ref, p.enabled
                     FROM models m
                     JOIN providers p ON m.provider_id = p.id",
                )?;
                let rows = stmt
                    .query_map([], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, i64>(2)?,
                            row.get::<_, Option<String>>(3)?,
                            row.get::<_, i64>(4)?,
                        ))
                    })?
                    .collect::<std::result::Result<Vec<_>, rusqlite::Error>>()?;
                Ok(rows)
            })
            .await?;

        let mut stt_available = false;
        let mut tts_available = false;
        let mut stt_needs_setup = false;
        let mut tts_needs_setup = false;

        for (_id, ext_id, model_enabled, secret_ref, prov_enabled) in models_info {
            let ext_lower = ext_id.to_lowercase();
            let is_stt = ext_lower.contains("whisper") || ext_lower.contains("transcribe");
            let is_tts = ext_lower.contains("tts") || ext_lower.contains("speech");

            if is_stt && model_enabled != 0 && prov_enabled != 0 {
                if let Some(sec_id) = secret_ref
                    .as_deref()
                    .and_then(|value| secret_reference_id(value, Uuid::nil()))
                {
                    if self.secrets.get(sec_id).await.is_ok() {
                        stt_available = true;
                    } else {
                        stt_needs_setup = true;
                    }
                } else {
                    stt_needs_setup = true;
                }
            }

            if is_tts && model_enabled != 0 && prov_enabled != 0 {
                if let Some(sec_id) = secret_ref
                    .as_deref()
                    .and_then(|value| secret_reference_id(value, Uuid::nil()))
                {
                    if self.secrets.get(sec_id).await.is_ok() {
                        tts_available = true;
                    } else {
                        tts_needs_setup = true;
                    }
                } else {
                    tts_needs_setup = true;
                }
            }
        }

        let cloud_stt_state = if stt_available {
            CapabilityState::Available
        } else if stt_needs_setup {
            CapabilityState::NeedsSetup
        } else {
            CapabilityState::Unavailable
        };

        let cloud_stt = Capability {
            key: "voice_cloud_stt".to_string(),
            state: cloud_stt_state,
            reason: if stt_available {
                None
            } else if stt_needs_setup {
                Some("Audio transcription model configured but credentials missing".to_string())
            } else {
                Some("No audio transcription model configured".to_string())
            },
            version: None,
            checked_at: now,
            details: CapabilityDetails {
                protocols: vec!["openai_audio_transcriptions".to_string()],
                languages: vec![Language::English, Language::Hindi],
                voices: vec![],
                operations: vec!["transcribe".to_string()],
                limitations: vec![
                    "Requires explicit gesture activation (PTT)".to_string(),
                    "Audio recordings capped at 60s / 8 MiB".to_string(),
                ],
                sandbox_verified: true,
            },
        };

        let cloud_tts_state = if tts_available {
            CapabilityState::Available
        } else if tts_needs_setup {
            CapabilityState::NeedsSetup
        } else {
            CapabilityState::Unavailable
        };

        let cloud_tts = Capability {
            key: "voice_cloud_tts".to_string(),
            state: cloud_tts_state,
            reason: if tts_available {
                None
            } else if tts_needs_setup {
                Some("Cloud speech model configured but credentials missing".to_string())
            } else {
                Some("No cloud speech model configured".to_string())
            },
            version: None,
            checked_at: now,
            details: CapabilityDetails {
                protocols: vec!["openai_audio_speech".to_string()],
                languages: vec![Language::English, Language::Hindi],
                voices: vec![
                    VoiceInfo {
                        id: "alloy".to_string(),
                        label: "Alloy".to_string(),
                        languages: vec![Language::English],
                    },
                    VoiceInfo {
                        id: "echo".to_string(),
                        label: "Echo".to_string(),
                        languages: vec![Language::English],
                    },
                ],
                operations: vec!["synthesize".to_string()],
                limitations: vec!["Text length capped at 8 KiB".to_string()],
                sandbox_verified: true,
            },
        };

        Ok(vec![browser_tts, cloud_stt, cloud_tts])
    }
}

fn secret_reference_id(reference: &str, fallback: Uuid) -> Option<Uuid> {
    reference
        .split_once(':')
        .and_then(|(_, id)| Uuid::parse_str(id).ok())
        .or_else(|| Uuid::parse_str(reference).ok())
        .or_else(|| (reference == "session" || reference == "keyring").then_some(fallback))
        .filter(|id| !id.is_nil())
}
