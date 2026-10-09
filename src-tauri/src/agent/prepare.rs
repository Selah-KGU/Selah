//! Slow turn preparation stays off the async executor; persist input first.
use crate::agent_error::AgentError;

pub(super) async fn blocking<R: Send + 'static>(
    work: impl FnOnce() -> Result<R, AgentError> + Send + 'static,
) -> Result<R, AgentError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(AgentError::task)?
}

pub(super) fn persisted_turn<I, P, H>(
    persist: impl FnOnce() -> Result<I, AgentError>,
    provider: impl FnOnce() -> Result<P, AgentError>,
    history: impl FnOnce() -> Result<H, AgentError>,
) -> Result<(I, P, H), AgentError> {
    let input = persist()?;
    let provider = provider()?;
    let history = history()?;
    Ok((input, provider, history))
}
#[cfg(test)]
#[path = "prepare/tests.rs"]
mod tests;
