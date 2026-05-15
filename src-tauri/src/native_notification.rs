#[cfg(target_os = "macos")]
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(target_os = "macos")]
static NOTIFICATION_ID: AtomicU64 = AtomicU64::new(1);

#[tauri::command]
pub fn native_notification_permission_granted() -> bool {
    true
}

#[cfg(target_os = "macos")]
pub fn send_native_notification(
    app: &tauri::AppHandle,
    title: &str,
    body: &str,
) -> Result<String, String> {
    use objc2_foundation::NSString;
    use objc2_user_notifications::{
        UNMutableNotificationContent, UNNotificationRequest, UNUserNotificationCenter,
    };

    let title = title.to_string();
    let body = body.to_string();
    let id = NOTIFICATION_ID.fetch_add(1, Ordering::Relaxed);

    app.run_on_main_thread(move || {
        let center = UNUserNotificationCenter::currentNotificationCenter();
        let content = UNMutableNotificationContent::new();
        content.setTitle(&NSString::from_str(&title));
        content.setBody(&NSString::from_str(&body));

        let identifier = NSString::from_str(&format!("selah-{}", id));
        let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
            &identifier,
            &content,
            None,
        );
        center.addNotificationRequest_withCompletionHandler(&request, None);
    })
    .map_err(|e| format!("Notification dispatch failed: {}", e))?;

    Ok("Notification queued".to_string())
}

#[cfg(not(target_os = "macos"))]
pub fn send_native_notification(
    app: &tauri::AppHandle,
    title: &str,
    body: &str,
) -> Result<String, String> {
    use tauri_plugin_notification::NotificationExt;

    app.notification()
        .builder()
        .title(title)
        .body(body)
        .show()
        .map(|_| "Notification sent".to_string())
        .map_err(|e| format!("Notification unavailable: {}", e))
}
