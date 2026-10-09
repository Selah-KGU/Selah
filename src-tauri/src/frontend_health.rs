//! Passive main-WebView diagnostics. A healthy DOM does not prove that WebKit's
//! GPU painted it, so a timeout is logged, never used to restart a recording.

use serde::Deserialize;
use std::sync::{LazyLock, Mutex};
use std::time::Instant;
use tauri::Manager;

#[derive(Default)]
struct ProbeState {
    sequence: u64,
    pending: Option<(u64, Instant)>,
}

impl ProbeState {
    fn begin(&mut self, now: Instant) -> (u64, Option<u64>) {
        let missed = self.pending.take().map(|(seq, _)| seq);
        self.sequence = self.sequence.wrapping_add(1);
        self.pending = Some((self.sequence, now));
        (self.sequence, missed)
    }

    fn acknowledge(&mut self, sequence: u64, now: Instant) -> Option<u128> {
        let (pending, sent) = self.pending?;
        if sequence != pending {
            return None;
        }
        self.pending = None;
        Some(now.duration_since(sent).as_millis())
    }
}

static PROBE: LazyLock<Mutex<ProbeState>> = LazyLock::new(|| Mutex::new(ProbeState::default()));

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FrontendHealth {
    sequence: u64,
    ready: String,
    visibility: String,
    root_children: u64,
    root_text_length: u64,
    width: u64,
    height: u64,
    recovery: bool,
    errors: u64,
}

#[tauri::command]
pub(crate) fn frontend_health_report(webview: tauri::Webview, report: FrontendHealth) {
    if webview.label() != "main" {
        return;
    }
    let elapsed = PROBE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .acknowledge(report.sequence, Instant::now());
    let Some(elapsed) = elapsed else { return };
    // No transcript, course name, URL or page text is included.
    log::info!(
        "[frontend-health] seq={} latency_ms={} ready={} visibility={} children={} text_length={} viewport={}x{} recovery={} errors={}",
        report.sequence, elapsed, report.ready.chars().take(16).collect::<String>(),
        report.visibility.chars().take(16).collect::<String>(), report.root_children,
        report.root_text_length, report.width, report.height, report.recovery, report.errors,
    );
}

#[tauri::command]
pub(crate) fn frontend_report_error(webview: tauri::Webview, message: String) {
    log::error!(
        "[frontend-error] surface={} {}",
        webview.label(),
        message.chars().take(8192).collect::<String>()
    );
}

pub(crate) fn start(app: &tauri::AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            let Some(state) = app.try_state::<crate::live::LiveState>() else {
                continue;
            };
            if !state.has_active_session() {
                PROBE.lock().unwrap_or_else(|e| e.into_inner()).pending = None;
                continue;
            }
            let Some(window) = app.get_webview_window("main") else {
                continue;
            };
            if !window.is_visible().unwrap_or(false)
                || window.is_minimized().unwrap_or(true)
                || !window.is_focused().unwrap_or(false)
            {
                // Background WebViews can be suspended normally.
                PROBE.lock().unwrap_or_else(|e| e.into_inner()).pending = None;
                continue;
            }
            let (sequence, missed) = PROBE
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .begin(Instant::now());
            if let Some(missed) = missed {
                log::warn!("[frontend-health] no response to seq={} within 30s; visible main WebView may be unresponsive (recording continues)", missed);
            }
            let script = format!(
                r#"(() => {{
                const inv = window.__TAURI_INTERNALS__?.invoke;
                if (!inv) return;
                const root = document.getElementById('app');
                // Count UTF-16 units without concatenating the entire mounted
                // dashboard (including hidden pages) into a temporary string.
                let rootTextLength = 0;
                if (root) {{
                    const text = document.createTreeWalker(root, NodeFilter.SHOW_TEXT | NodeFilter.SHOW_CDATA_SECTION);
                    while (text.nextNode()) rootTextLength += text.currentNode.data.length;
                }}
                let errors = 0;
                for (const entry of window.__SELAH_PREBOOT_LOGS__ || []) {{
                    if (entry.type === 'error') errors += 1;
                }}
                inv('frontend_health_report', {{ report: {{
                    sequence: {sequence}, ready: document.readyState,
                    visibility: document.visibilityState,
                    rootChildren: root?.childElementCount || 0,
                    rootTextLength,
                    width: Math.max(0, window.innerWidth), height: Math.max(0, window.innerHeight),
                    recovery: !!document.querySelector('.render-recovery'),
                    errors
                }} }}).catch(() => {{}});
            }})()"#
            );
            if let Err(error) = window.eval(script) {
                log::warn!("[frontend-health] failed to submit probe: {error}");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delayed_response_cannot_acknowledge_a_new_probe() {
        let mut state = ProbeState::default();
        let now = Instant::now();
        let (first, _) = state.begin(now);
        let (second, missed) = state.begin(now);
        assert_eq!(missed, Some(first));
        assert_eq!(state.acknowledge(first, now), None);
        assert_eq!(state.acknowledge(second, now), Some(0));
        assert_eq!(state.acknowledge(second, now), None);
        assert_eq!(state.begin(now).1, None);
    }
}
