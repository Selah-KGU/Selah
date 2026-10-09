use super::{config::save_sync_state, AutoSyncBinding, CalendarSyncEntry};
use crate::{db::AccountContext, session_coordinator::SESSIONS};

tokio::task_local! { static SYNC_ACCOUNT: AccountContext; }

pub(super) fn ensure_sync_owner() -> Result<(), String> {
    if SYNC_ACCOUNT
        .try_with(|account| *account != SESSIONS.account_context())
        .unwrap_or(false)
    {
        return Err(crate::session_coordinator::CANCELLED.into());
    }
    Ok(())
}

impl super::GoogleCalendarClient {
    pub(crate) fn auto_sync_allowed(&self, account: &AccountContext) -> bool {
        self.token
            .as_ref()
            .zip(self.sync_state.auto_sync_binding.as_ref())
            .is_some_and(|(token, binding)| {
                binding.matches(
                    account.username.as_deref(),
                    &token.connection_id,
                    &self.sync_state.calendar_id,
                )
            })
    }

    pub(crate) fn ensure_confirmation(
        &self,
        connection: Option<&str>,
        calendar: Option<&str>,
    ) -> Result<(), String> {
        let current = self.token.as_ref().map(|t| t.connection_id.as_str());
        if connection.is_none_or(str::is_empty)
            || connection != current
            || calendar != Some(self.sync_state.calendar_id.as_str())
        {
            return Err(
                "Google の接続または同期先が変更されました。設定を開き直して確認してください"
                    .into(),
            );
        }
        Ok(())
    }

    pub(crate) async fn configure_auto_sync(
        &mut self,
        enabled: bool,
        account: &AccountContext,
        connection: Option<&str>,
        calendar: Option<&str>,
    ) -> Result<(), String> {
        if *account != SESSIONS.account_context() {
            return Err(crate::session_coordinator::CANCELLED.into());
        }
        if enabled {
            self.ensure_confirmation(connection, calendar)?;
            let username = account
                .username
                .as_ref()
                .filter(|s| !s.trim().is_empty())
                .ok_or("大学にログインしてから自動同期を有効にしてください")?
                .clone();
            self.save_token()?;
            let calendar_id = self
                .ensure_calendar_with_replacement(self.sync_state.calendar_id.is_empty())
                .await?;
            if *account != SESSIONS.account_context() {
                return Err(crate::session_coordinator::CANCELLED.into());
            }
            let connection_id = self
                .token
                .as_ref()
                .ok_or("Google Calendar に接続してください")?
                .connection_id
                .clone();
            let binding = AutoSyncBinding {
                university_username: username,
                connection_id,
                calendar_id,
            };
            let mut next = self.sync_state.clone();
            next.auto_sync_binding = Some(binding);
            save_sync_state(&next)?;
            self.sync_state = next;
        } else {
            self.sync_state.auto_sync_binding = None;
            save_sync_state(&self.sync_state)?;
        }
        Ok(())
    }

    pub(crate) async fn sync_timetable_automatically(
        &mut self,
        entries: Vec<CalendarSyncEntry>,
        week: String,
        account: &AccountContext,
    ) -> Result<String, String> {
        if *account != SESSIONS.account_context() || !self.auto_sync_allowed(account) {
            return Err("自動同期は停止中です。カレンダー設定で同期先を確認してください".into());
        }
        // Automatic work never selects or creates a replacement calendar. A
        // retired account cancels further requests; already sent writes cannot be recalled.
        let calendar = self.sync_state.calendar_id.clone();
        tokio::select! {
            biased;
            _ = SESSIONS.cancelled(account.generation) => Err(crate::session_coordinator::CANCELLED.into()),
            result = SYNC_ACCOUNT.scope(account.clone(), self.sync_to_calendar(entries, week, calendar)) => result,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn consent_is_scoped_to_account_connection_and_calendar() {
        let binding = AutoSyncBinding {
            university_username: "alice".into(),
            connection_id: "google-1".into(),
            calendar_id: "cal-1".into(),
        };
        assert!(binding.matches(Some("alice"), "google-1", "cal-1"));
        for (username, connection, calendar) in [
            (Some("bob"), "google-1", "cal-1"),
            (None, "google-1", "cal-1"),
            (Some("alice"), "google-2", "cal-1"),
            (Some("alice"), "google-1", "cal-2"),
            (Some("alice"), "", "cal-1"),
        ] {
            assert!(!binding.matches(username, connection, calendar));
        }
        let legacy: super::super::SyncState =
            serde_json::from_str(r#"{"calendar_id":"cal-1","event_map":{}}"#).unwrap();
        assert!(legacy.auto_sync_binding.is_none());
    }

    #[test]
    fn restored_connection_requires_matching_consent_even_with_global_auto_sync_enabled() {
        use super::super::{GoogleCalConfig, GoogleCalendarClient, SyncState, TokenData};
        let token: TokenData = serde_json::from_str(r#"{"access_token":"fake","refresh_token":"fake","expires_at":0,"connection_id":"google-1"}"#).unwrap();
        let state: SyncState = serde_json::from_str(r#"{"calendar_id":"cal-1","event_map":{},"auto_sync_binding":{"university_username":"alice","connection_id":"google-1","calendar_id":"cal-1"}}"#).unwrap();
        let mut client = GoogleCalendarClient {
            http: crate::oauth_http::Http::new(),
            lifecycle: Default::default(),
            new_login: false,
            logout_requested: false,
            token: Some(token),
            config: GoogleCalConfig {
                client_id: "fake".into(),
                client_secret: String::new(),
            },
            config_error: None,
            sync_state: state,
        };
        let alice = AccountContext {
            generation: 1,
            username: Some("alice".into()),
        };
        assert!(client.auto_sync_allowed(&alice));
        assert!(client
            .ensure_confirmation(Some("google-1"), Some("cal-1"))
            .is_ok());
        assert!(client
            .ensure_confirmation(Some("google-2"), Some("cal-1"))
            .is_err());
        assert!(client
            .ensure_confirmation(Some("google-1"), Some("cal-2"))
            .is_err());
        assert!(client.ensure_confirmation(None, Some("cal-1")).is_err());
        assert!(!client.auto_sync_allowed(&AccountContext {
            generation: 2,
            username: Some("bob".into())
        }));
        client.token.as_mut().unwrap().connection_id = "google-2".into();
        assert!(!client.auto_sync_allowed(&alice));
        client.token.as_mut().unwrap().connection_id.clear();
        assert!(!client.auto_sync_allowed(&alice));
    }
}
