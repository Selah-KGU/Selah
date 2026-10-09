use super::{Health, Service, SessionError, SessionLease, SESSIONS};

/// Network work holds no state lock. The manager atomically checks the lease
/// before applying evidence, so a delayed probe cannot expire a newer login.
pub(crate) async fn verify(service: Service) -> Result<SessionLease, SessionError> {
    SESSIONS.restore(service)?;
    let lease = SESSIONS.lease(service);
    if !lease.has_credentials() {
        return Ok(lease);
    }
    let (base, path, predicate): (_, _, fn(&str) -> bool) = match service {
        Service::Kgc => (
            crate::config::KG_COURSE_BASE,
            "/uniasv2/ARF010.do?REQ_PRFR_MNU_ID=MNUIDSTD0102014",
            crate::client::is_session_expired_body,
        ),
        Service::Luna => (
            crate::config::LUNA_BASE,
            "/lms/timetable",
            crate::luna_client::is_luna_session_expired,
        ),
        Service::Kwic => (
            crate::config::KWIC_BASE,
            "/portal/home",
            crate::kwic_client::is_kwic_session_expired,
        ),
    };
    match crate::client::fetch_session_page(lease.http(), &format!("{base}{path}"), base, predicate)
        .await
    {
        Ok(html) => {
            let identity = if service == Service::Kgc {
                let info = crate::parser::parse_student_info(&html);
                if info.student_id.is_empty() && info.name.is_empty() {
                    SESSIONS
                        .validated(&lease, Health::Unavailable, None)
                        .map_err(|_| SessionError::Cancelled)?;
                    return Err(SessionError::Unavailable(
                        "KGC returned an unrecognized verification page".into(),
                    ));
                }
                let mut identity = lease.identity().cloned().expect("KGC record has identity");
                if !info.student_id.is_empty() {
                    identity.username = info.student_id.clone();
                    identity.student_id = info.student_id;
                }
                if !info.name.is_empty() {
                    identity.display_name = info.name;
                }
                identity.faculty = info.faculty;
                identity.department = info.department;
                Some(identity)
            } else {
                None
            };
            SESSIONS
                .validated(&lease, Health::Valid, identity)
                .map_err(|_| SessionError::Cancelled)?;
        }
        Err(SessionError::NeedsLogin) => {
            SESSIONS
                .validated(&lease, Health::NeedsLogin, None)
                .map_err(|_| SessionError::Cancelled)?;
        }
        Err(error) => {
            SESSIONS
                .validated(&lease, Health::Unavailable, None)
                .map_err(|_| SessionError::Cancelled)?;
            return Err(error);
        }
    }
    Ok(SESSIONS.lease(service))
}
