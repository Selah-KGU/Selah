use super::Database;

pub(super) fn delete_conversation(db: &Database, id: &str) -> Result<(), String> {
    db.agent_delete_conversation(id)?;
    // This belongs to the durable worker, not its async response waiter or a
    // frontend listener. A failed deletion must leave the request running.
    crate::agent_turn_scope::retire_conversation(id);
    Ok(())
}

#[cfg(test)]
#[path = "lifecycle_tests.rs"]
mod tests;
