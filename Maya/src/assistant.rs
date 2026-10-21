use crate::auth::SessionContext;
use crate::conversation::ConversationService;
use crate::error::{AppError, ErrorCode, Result};
use crate::provider::ProviderRegistry;
use crate::tasks::TaskService;
use crate::types::*;
use crate::workflow::WorkflowEngine;
use chrono_tz::Tz;
use std::sync::Arc;

#[derive(Clone)]
pub struct AssistantService {
    conversations: Arc<ConversationService>,
    tasks: Arc<TaskService>,
    providers: Arc<ProviderRegistry>,
    workflow: Arc<WorkflowEngine>,
}

impl AssistantService {
    pub fn new(
        conversations: Arc<ConversationService>,
        tasks: Arc<TaskService>,
        providers: Arc<ProviderRegistry>,
        workflow: Arc<WorkflowEngine>,
    ) -> Self {
        Self {
            conversations,
            tasks,
            providers,
            workflow,
        }
    }

    pub async fn receive(
        &self,
        id: Id,
        input: UserMessage,
        session: &SessionContext,
    ) -> Result<MessageReceipt> {
        validate_message(&input)?;
        let owner_message = self.conversations.append(id, input.clone()).await?;
        let actor = Actor {
            kind: ActorKind::Owner,
            id: Some(session.id),
        };

        if let Some(task_draft) = input.confirmed_task {
            let task = self.tasks.create(task_draft, actor).await?;
            return Ok(MessageReceipt {
                message: owner_message,
                assistant_message: None,
                draft: None,
                task: Some(task),
                job_id: None,
            });
        }

        let draft = Self::classify(
            &input.text,
            chrono_tz::UTC,
            chrono::Utc::now().timestamp_millis(),
        )?;
        if !matches!(draft.intent, Intent::Question) {
            return Ok(MessageReceipt {
                message: owner_message,
                assistant_message: None,
                draft: Some(draft),
                task: None,
                job_id: None,
            });
        }

        let resolved = self
            .providers
            .resolve(ModelSelection {
                role: Role::Chat,
                override_id: input.model_override_id,
                project_id: self.conversations.get(id).await?.project_id,
                fallback_consent: false,
            })
            .await;
        let resolved = match resolved {
            Ok(value) => value,
            Err(error) if error.code() == ErrorCode::ModelUnavailable => {
                let response = self
                    .conversations
                    .append_assistant_message(
                        id,
                        "No chat model is configured. Configure an enabled provider and assign the Chat role in Workspace.".to_string(),
                        None,
                        None,
                    )
                    .await?;
                return Ok(MessageReceipt {
                    message: owner_message,
                    assistant_message: Some(response),
                    draft: None,
                    task: None,
                    job_id: None,
                });
            }
            Err(error) => return Err(error),
        };
        let context = self.conversations.context(id).await?;
        let job = self
            .workflow
            .create_chat_job(id, &input.text, context, resolved, actor)
            .await?;
        Ok(MessageReceipt {
            message: owner_message,
            assistant_message: None,
            draft: None,
            task: None,
            job_id: Some(job.id),
        })
    }

    pub fn classify(text: &str, timezone: Tz, now: UtcMs) -> Result<IntentDraft> {
        let trimmed = text.trim();
        if trimmed.is_empty() || trimmed.len() > 65_536 {
            return Err(AppError::new(
                ErrorCode::InvalidRequest,
                "Message text must be between 1 and 65536 bytes",
            ));
        }
        let lower = trimmed.to_ascii_lowercase();
        let task_intent = lower.starts_with("task:")
            || lower.starts_with("todo:")
            || lower.starts_with("remind me")
            || lower.starts_with("please implement")
            || lower.starts_with("implement ");
        if task_intent {
            let title_source = trimmed
                .split_once(':')
                .map_or(trimmed, |(_, value)| value.trim());
            let title: String = title_source.chars().take(200).collect();
            let reminder = if lower.starts_with("remind me") {
                Some(ScheduleDraft {
                    task_id: None,
                    kind: ScheduleKind::Reminder,
                    label: title.clone(),
                    first_at: now,
                    timezone: timezone.name().to_string(),
                    recurrence: Recurrence::Once,
                    weekdays: Vec::new(),
                    local_time: chrono::DateTime::from_timestamp_millis(now)
                        .map(|value| value.format("%H:%M").to_string())
                        .unwrap_or_else(|| "09:00".to_string()),
                    enabled: true,
                })
            } else {
                None
            };
            return Ok(IntentDraft {
                intent: if reminder.is_some() {
                    Intent::Reminder
                } else if lower.contains("implement") {
                    Intent::ProjectWork
                } else {
                    Intent::Task
                },
                task: Some(TaskDraft {
                    title,
                    description: trimmed.to_string(),
                    project_id: None,
                    parent_task_id: None,
                    priority: Priority::Normal,
                    due_at: None,
                    timezone: timezone.name().to_string(),
                    estimated_minutes: None,
                    reminder,
                }),
                operation: None,
                clarification: if lower.starts_with("remind me") {
                    Some("Confirm the reminder time before saving this draft.".to_string())
                } else {
                    None
                },
                source: IntentSource::Rule,
            });
        }
        Ok(IntentDraft {
            intent: Intent::Question,
            task: None,
            operation: None,
            clarification: None,
            source: IntentSource::Rule,
        })
    }
}

fn validate_message(input: &UserMessage) -> Result<()> {
    if input.text.trim().is_empty() || input.text.len() > 65_536 {
        return Err(AppError::new(
            ErrorCode::InvalidRequest,
            "Message text must be between 1 and 65536 bytes",
        ));
    }
    Ok(())
}
