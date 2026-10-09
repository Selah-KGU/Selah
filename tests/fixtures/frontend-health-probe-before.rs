// Frozen native format string for isolated diagnostic comparison only.
const BEFORE: &str = r#"(() => {{
                const inv = window.__TAURI_INTERNALS__?.invoke;
                if (!inv) return;
                const root = document.getElementById('app');
                inv('frontend_health_report', {{ report: {{
                    sequence: {sequence}, ready: document.readyState,
                    visibility: document.visibilityState,
                    rootChildren: root?.childElementCount || 0,
                    rootTextLength: root?.textContent?.length || 0,
                    width: Math.max(0, window.innerWidth), height: Math.max(0, window.innerHeight),
                    recovery: !!document.querySelector('.render-recovery'),
                    errors: (window.__SELAH_PREBOOT_LOGS__ || []).filter(x => x.type === 'error').length
                }} }}).catch(() => {{}});
            }})()"#;
