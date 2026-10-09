#[path = "mail/auth.rs"]
mod auth;
#[path = "mail/cache.rs"]
pub(crate) mod cache;
#[path = "mail/config.rs"]
mod config;
#[path = "mail/graph.rs"]
mod graph;
#[path = "mail/messages.rs"]
mod messages;
#[path = "mail/oauth.rs"]
pub(crate) mod oauth;
#[path = "mail/types.rs"]
mod types;

use crate::oauth_http::Http;

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
    http: Http,
    pub(crate) lifecycle: crate::oauth_lifecycle::Lifecycle,
    new_login: bool,
    logout_requested: bool,
    pub token: Option<TokenData>,
    pub config: MailConfig,
}
