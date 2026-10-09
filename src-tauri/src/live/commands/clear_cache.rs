use super::{LiveCourseInfo, LiveState};

pub(super) async fn clear(
    state: LiveState,
    course: LiveCourseInfo,
    remove: impl FnOnce(&LiveCourseInfo) -> Result<(), String> + Send + 'static,
) -> Result<(), String> {
    if course.is_free_note {
        return Ok(());
    }
    if course.course_name.trim().is_empty() {
        return Err("講義名が空です".into());
    }
    tokio::task::spawn_blocking(move || {
        let _storage = state
            .persistence
            .gate
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state
            .session
            .lock()
            .map_err(|_| "Live state lock failed".to_string())?
            .as_ref()
            .is_some_and(|session| {
                session.course.course_name == course.course_name && !session.course.is_free_note
            })
        {
            return Err("録音中または保存中の講義データは削除できません".into());
        }
        remove(&course)
    })
    .await
    .map_err(|error| format!("Liveキャッシュ削除処理失敗: {error}"))?
}

#[cfg(test)]
mod tests;
