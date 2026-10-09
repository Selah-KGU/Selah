//! Immutable native request identity and events passed directly by the backend.
use std::borrow::Cow;
use std::sync::Arc;

pub(crate) struct StreamOwner {
    conversation_id: String,
    request_id: String,
}
impl StreamOwner {
    pub(crate) fn new(conversation_id: String) -> Arc<Self> {
        Arc::new(Self {
            conversation_id,
            request_id: uuid::Uuid::new_v4().to_string(),
        })
    }
    pub(crate) fn conversation_id(&self) -> &str {
        &self.conversation_id
    }
    pub(crate) fn request_id(&self) -> &str {
        &self.request_id
    }
}

pub(crate) enum StreamEvent<'a> {
    Token(Cow<'a, str>),
    Done,
    Error(Cow<'a, str>),
}
