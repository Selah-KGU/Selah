#[cfg(target_os = "macos")]
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(target_os = "macos")]
static NOTIFICATION_ID: AtomicU64 = AtomicU64::new(1);

#[tauri::command]
pub fn native_notification_permission_granted() -> bool {
    true
}

#[cfg(target_os = "macos")]
fn has_main_bundle_identifier() -> bool {
    use objc2_foundation::NSBundle;
    use std::sync::OnceLock;

    static CACHED: OnceLock<bool> = OnceLock::new();
    *CACHED.get_or_init(|| {
        // UNUserNotificationCenter aborts (NSInternalInconsistencyException)
        // when the main bundle has no identifier — i.e. when the binary is
        // launched directly rather than as a packaged .app. Detect that here
        // so we can fall back instead of crashing the whole process.
        let bundle = NSBundle::mainBundle();
        bundle.bundleIdentifier().is_some()
    })
}

#[cfg(target_os = "macos")]
fn osascript_escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' | '\r' => out.push(' '),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(target_os = "macos")]
fn send_via_osascript(title: &str, body: &str) -> Result<String, String> {
    use std::process::Command;

    let script = format!(
        "display notification \"{}\" with title \"{}\"",
        osascript_escape(body),
        osascript_escape(title),
    );

    let output = Command::new("osascript")
        .arg("-e")
        .arg(&script)
        .output()
        .map_err(|e| format!("osascript launch failed: {}", e))?;

    if output.status.success() {
        Ok("Notification dispatched via osascript fallback".to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!(
            "osascript exited with {:?}: {}",
            output.status.code(),
            stderr.trim()
        ))
    }
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

    if !has_main_bundle_identifier() {
        return send_via_osascript(title, body);
    }

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
