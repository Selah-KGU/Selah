//! An owned receipt admits an already committed voice turn without saving twice.
use super::{AgentError, Database, ImagePart};

#[cfg(any(target_os = "macos", target_os = "windows", test))]
pub(crate) struct SavedVoiceInput {
    conversation_id: String,
    text: String,
    message_id: i64,
}

#[cfg(any(target_os = "macos", target_os = "windows", test))]
impl SavedVoiceInput {
    pub(crate) fn conversation_id(&self) -> &str {
        &self.conversation_id
    }
}

/// The receipt has private fields and is never cloned: only a committed
/// transaction can produce it, and starting a turn consumes it.
#[cfg(any(target_os = "macos", target_os = "windows", test))]
pub(crate) fn save_voice_input(
    db: &Database,
    conversation_id: String,
    text: String,
) -> Result<SavedVoiceInput, String> {
    let message_id = db.agent_create_voice_turn(&conversation_id, &text)?;
    Ok(SavedVoiceInput {
        conversation_id,
        text,
        message_id,
    })
}

#[cfg(any(target_os = "macos", target_os = "windows", test))]
pub(crate) fn retry_voice_input(
    db: &Database,
    conversation_id: String,
    text: String,
) -> Result<SavedVoiceInput, String> {
    let message_id = db.agent_retry_voice_turn(&conversation_id, &text)?;
    Ok(SavedVoiceInput {
        conversation_id,
        text,
        message_id,
    })
}

pub(super) struct CommittedInput {
    pub(super) message_id: i64,
    pub(super) text: String,
    pub(super) images: Vec<ImagePart>,
}

pub(super) enum TurnInput {
    New {
        text: String,
        images: Vec<ImagePart>,
        save: crate::pending_persistence::SavePermit,
    },
    #[cfg(any(target_os = "macos", target_os = "windows", test))]
    Voice(SavedVoiceInput),
}

impl TurnInput {
    pub(super) fn persist_with(
        self,
        conversation_id: &str,
        persist: impl FnOnce(&str, &[ImagePart]) -> Result<i64, AgentError>,
    ) -> Result<CommittedInput, AgentError> {
        match self {
            Self::New { text, images, save } => {
                let result = persist(&text, &images);
                let failure = result.as_ref().err().map(ToString::to_string);
                save.complete(failure.as_deref().map_or(Ok(()), Err));
                let message_id = result?;
                Ok(CommittedInput {
                    message_id,
                    text,
                    images,
                })
            }
            #[cfg(any(target_os = "macos", target_os = "windows", test))]
            Self::Voice(saved) => {
                if saved.conversation_id != conversation_id {
                    return Err(AgentError::db("保存済み音声の会話が一致しません"));
                }
                Ok(CommittedInput {
                    message_id: saved.message_id,
                    text: saved.text,
                    images: Vec::new(),
                })
            }
        }
    }
}

#[cfg(test)]
#[path = "input/tests.rs"]
mod tests;
