#[path = "mail/auth.rs"]
mod auth;
#[path = "mail/config.rs"]
mod config;
#[path = "mail/graph.rs"]
mod graph;
#[path = "mail/messages.rs"]
mod messages;
#[path = "mail/types.rs"]
mod types;

use reqwest::Client;

#[allow(unused_imports)]
pub use config::{load_config, save_config, MailConfig};
pub use graph::{graph_get_lockfree, graph_get_lockfree_with_headers};
pub(crate) use messages::validate_message_id;
pub(crate) use types::GraphListResponse;
#[allow(unused_imports)]
pub use types::{
    EmailAddress, MailAddress, MailAttachment, MailBody, MailDetail, MailMessage, MailProfile,
    TokenData,
};

pub struct MailClient {
    http: Client,
    pub token: Option<TokenData>,
    pub config: MailConfig,
}
