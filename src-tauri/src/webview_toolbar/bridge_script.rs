//! Injected browser bridge for managed webviews.

pub(super) const BROWSER_BRIDGE_SCRIPT: &str = concat!(
    include_str!("bridge_script/dom.inc"),
    include_str!("bridge_script/find.inc"),
    include_str!("bridge_script/pointer.inc"),
    include_str!("bridge_script/runner.inc"),
);
