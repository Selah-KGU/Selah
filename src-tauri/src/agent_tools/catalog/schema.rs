// ─────────────────────── Tool Spec & Arg Schema ───────────────────────

/// Describes how to sanitize tool arguments before dispatch.
#[derive(Clone, Copy)]
pub(in crate::agent_tools) enum ArgSchema {
    /// No arguments — always returns `{}`.
    Empty,
    /// Single integer arg with key, clamped to 0..=max.
    Int { key: &'static str, max: i64 },
    /// Single text arg with key, max_len.
    Text { key: &'static str, max_len: usize },
    /// Course code arg (alphanumeric, uppercased).
    CourseCode { key: &'static str },
    /// limit + optional keyword.
    LimitKeyword,
    /// Custom sanitizer (message_id with validation).
    MailMessageId,
    /// Downloaded file path (restricted to allowed roots).
    FilePath,
    /// Downloaded file path + body for safe text writes.
    FileWrite,
    /// Luna title + optional attachment name.
    TitleAttachment,
    /// Luna activity detail options (all optional, but should have title/luna_id).
    LunaActivityDetail,
    /// Luna explicit attachment download options.
    DownloadLunaAttachment,
    /// Luna course material explicit download by filename.
    DownloadCourseMaterial,
    /// Optional text arg, omitted when empty.
    OptionalText { key: &'static str, max_len: usize },
    /// URL arg.
    Url,
    /// A known Copilot page plus optional context.
    CopilotPage,
    /// URL + optional explicit filename for the saved file.
    DownloadUrl,
    /// Browser click action.
    BrowserClick,
    /// Browser viewport coordinate click.
    BrowserMouseClick,
    /// Browser viewport coordinate drag.
    BrowserMouseDrag,
    /// Browser fill action.
    BrowserFill,
    /// Browser select action.
    BrowserSelect,
    /// Browser key press action.
    BrowserPress,
    /// Browser scroll action.
    BrowserScroll,
    /// Browser wait action.
    BrowserWait,
    /// Screenshot of a window or target.
    ComputerScreenshot,
    /// System-level mouse click.
    ComputerMouseClick,
    /// System-level mouse drag.
    ComputerMouseDrag,
    /// System-level scroll wheel.
    ComputerScroll,
    /// Google Calendar single-event creation.
    CalendarEvent,
    /// Google Calendar event update (event_id required, rest optional).
    CalendarUpdate,
    /// Google Calendar event delete (event_id required).
    CalendarEventId,
}

pub(in crate::agent_tools) struct ToolSpec {
    pub(in crate::agent_tools) name: &'static str,
    pub(in crate::agent_tools) category: &'static str,
    pub(in crate::agent_tools) signature: &'static str,
    pub(in crate::agent_tools) purpose: &'static str,
    pub(in crate::agent_tools) schema: ArgSchema,
}
