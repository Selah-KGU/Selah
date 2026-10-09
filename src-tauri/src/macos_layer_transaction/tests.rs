use super::*;

fn outer_transaction() -> LayerTransaction {
    let transaction = LayerTransaction::begin();
    CATransaction::setDisableActions(false);
    CATransaction::setAnimationDuration(0.75);
    transaction
}

#[test]
fn returns_the_update_result_without_leaking_settings_to_the_caller() {
    let _outer = outer_transaction();
    let result = suppress_implicit_animations(|| {
        assert!(CATransaction::disableActions());
        assert_eq!(CATransaction::animationDuration(), 0.0);
        "updated"
    });
    assert_eq!(result, "updated");
    assert!(!CATransaction::disableActions());
    assert_eq!(CATransaction::animationDuration(), 0.75);
}

#[test]
fn nested_updates_restore_the_enclosing_transaction() {
    let _outer = outer_transaction();
    suppress_implicit_animations(|| {
        CATransaction::setAnimationDuration(0.125);
        suppress_implicit_animations(|| {
            assert!(CATransaction::disableActions());
            assert_eq!(CATransaction::animationDuration(), 0.0);
        });
        assert!(CATransaction::disableActions());
        assert_eq!(CATransaction::animationDuration(), 0.125);
    });
    assert!(!CATransaction::disableActions());
    assert_eq!(CATransaction::animationDuration(), 0.75);
}

#[test]
fn panicking_updates_close_the_transaction_before_the_next_ui_operation() {
    let _outer = outer_transaction();
    let result = std::panic::catch_unwind(|| {
        suppress_implicit_animations(|| panic!("interrupted layer update"));
    });
    assert!(result.is_err());
    assert!(!CATransaction::disableActions());
    assert_eq!(CATransaction::animationDuration(), 0.75);
    suppress_implicit_animations(|| assert!(CATransaction::disableActions()));
    assert!(!CATransaction::disableActions());
}
