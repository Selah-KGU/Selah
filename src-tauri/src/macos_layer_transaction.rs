//! Group programmatically generated layer updates without implicit interpolation.
//! AppKit window operations stay outside this scope.

use objc2_quartz_core::CATransaction;

struct LayerTransaction;

impl LayerTransaction {
    fn begin() -> Self {
        CATransaction::begin();
        Self
    }
}

impl Drop for LayerTransaction {
    fn drop(&mut self) {
        CATransaction::commit();
    }
}

/// The nested transaction restores the caller's settings on return or unwind.
/// Keep this synchronous: a transaction belongs to its current thread.
pub(crate) fn suppress_implicit_animations<T>(updates: impl FnOnce() -> T) -> T {
    let _transaction = LayerTransaction::begin();
    CATransaction::setDisableActions(true);
    CATransaction::setAnimationDuration(0.0);
    updates()
}

#[cfg(test)]
#[path = "macos_layer_transaction/tests.rs"]
mod tests;
