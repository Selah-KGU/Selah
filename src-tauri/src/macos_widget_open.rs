//! Desktop widget clicks must raise Selah inside the AppKit callback.
//!
//! macOS 14+ ignores `activateIgnoringOtherApps`. A widget click only brings a
//! background or hidden window forward when we accept the handoff and call
//! `NSApp.activate()` before that callback returns. tao drops non-web user
//! activities, which is the path WidgetKit uses when the click has no URL.

use std::ffi::c_void;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::{Mutex, Once, OnceLock};

use objc2::ffi::{class_getInstanceMethod, class_replaceMethod, method_getTypeEncoding};
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool, Imp, NSObject, Sel};
use objc2::{msg_send, sel, ClassType, MainThreadMarker};
use objc2_app_kit::NSApp;
use objc2_foundation::{
    NSDistributedNotificationCenter, NSNotificationSuspensionBehavior, NSString,
};
use tauri::AppHandle;

const WIDGET_OPEN_NOTIFICATION: &str = "com.kgu.selah.widget-open";

static APP: Mutex<Option<AppHandle>> = Mutex::new(None);
static CONTINUE_ORIGINAL: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static WILL_CONTINUE_ORIGINAL: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static INSTALLED: Once = Once::new();

thread_local! {
    static OBSERVER: std::cell::RefCell<Option<Retained<NSObject>>> = std::cell::RefCell::new(None);
}

pub(crate) fn install(app: &AppHandle) {
    if let Ok(mut slot) = APP.lock() {
        *slot = Some(app.clone());
    }
    INSTALLED.call_once(|| {
        install_continue_hooks();
        install_open_observer();
    });
}

pub(crate) fn is_widget_activity_type(activity_type: &str) -> bool {
    let lower = activity_type.to_ascii_lowercase();
    lower.contains("widget") || lower.contains("selah")
}

fn app_handle() -> Option<AppHandle> {
    APP.lock().ok().and_then(|slot| slot.clone())
}

fn present_main_window() {
    let Some(app) = app_handle() else {
        return;
    };
    log::info!("widget click: presenting main window");
    crate::app_lifecycle::present_main_window(&app);
}

fn install_continue_hooks() {
    let Some(mtm) = MainThreadMarker::new() else {
        log::error!("widget activation hook must be installed on the main thread");
        return;
    };
    let app = NSApp(mtm);
    let Some(delegate) = app.delegate() else {
        log::error!("NSApp has no delegate; widget clicks will not raise the window");
        return;
    };
    let class = unsafe {
        let class_ptr: *const AnyClass = msg_send![&*delegate, class];
        class_ptr.as_ref()
    };
    let Some(class) = class else {
        log::error!("NSApp delegate has no class; widget clicks will not raise the window");
        return;
    };
    replace_method(
        class,
        sel!(application:willContinueUserActivityWithType:),
        fn_imp(application_will_continue as WillContinueFn),
        &WILL_CONTINUE_ORIGINAL,
    );
    replace_method(
        class,
        sel!(application:continueUserActivity:restorationHandler:),
        fn_imp(application_continue_user_activity as ContinueFn),
        &CONTINUE_ORIGINAL,
    );
}

fn fn_imp<F>(function: F) -> Imp {
    assert_eq!(std::mem::size_of::<F>(), std::mem::size_of::<Imp>());
    unsafe { std::mem::transmute_copy(&function) }
}

fn replace_method(class: &AnyClass, name: Sel, imp: Imp, slot: &AtomicPtr<c_void>) {
    let class_ptr = std::ptr::from_ref(class);
    let method = unsafe { class_getInstanceMethod(class_ptr, name) };
    if method.is_null() {
        log::error!(
            "missing AppKit delegate method {name:?}; widget clicks may not raise the window"
        );
        return;
    }
    let types = unsafe { method_getTypeEncoding(method) };
    let previous = unsafe { class_replaceMethod(class_ptr.cast_mut(), name, imp, types) };
    if let Some(previous) = previous {
        slot.store(previous as *mut c_void, Ordering::Release);
    }
}

type ContinueFn = unsafe extern "C-unwind" fn(
    *mut AnyObject,
    Sel,
    *mut AnyObject,
    *mut AnyObject,
    *mut AnyObject,
) -> Bool;

type WillContinueFn =
    unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject, *mut AnyObject) -> Bool;

extern "C-unwind" fn application_will_continue(
    this: *mut AnyObject,
    cmd: Sel,
    application: *mut AnyObject,
    activity_type: *mut AnyObject,
) -> Bool {
    if activity_pointer_is_widget_type(activity_type) {
        return Bool::new(true);
    }
    let original = WILL_CONTINUE_ORIGINAL.load(Ordering::Acquire);
    if original.is_null() {
        return Bool::new(false);
    }
    let original: WillContinueFn = unsafe { std::mem::transmute(original) };
    unsafe { original(this, cmd, application, activity_type) }
}

extern "C-unwind" fn application_continue_user_activity(
    this: *mut AnyObject,
    cmd: Sel,
    application: *mut AnyObject,
    activity: *mut AnyObject,
    restoration: *mut AnyObject,
) -> Bool {
    let widget = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
        is_widget_activity(activity)
    }))
    .unwrap_or(false);
    if widget {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(present_main_window));
        return Bool::new(true);
    }
    let original = CONTINUE_ORIGINAL.load(Ordering::Acquire);
    if original.is_null() {
        return Bool::new(false);
    }
    let original: ContinueFn = unsafe { std::mem::transmute(original) };
    unsafe { original(this, cmd, application, activity, restoration) }
}

fn activity_pointer_is_widget_type(activity_type: *mut AnyObject) -> bool {
    if activity_type.is_null() {
        return false;
    }
    let text = unsafe { &*activity_type.cast::<NSString>() }.to_string();
    is_widget_activity_type(&text)
}

unsafe fn is_widget_activity(activity: *mut AnyObject) -> bool {
    if activity.is_null() {
        return false;
    }
    let activity = &*activity;
    let activity_type: *mut AnyObject = msg_send![activity, activityType];
    if activity_pointer_is_widget_type(activity_type) {
        return true;
    }
    let webpage: *mut AnyObject = msg_send![activity, webpageURL];
    if !webpage.is_null() {
        let scheme: *mut AnyObject = msg_send![webpage, scheme];
        if !scheme.is_null()
            && unsafe { &*scheme.cast::<NSString>() }
                .to_string()
                .eq_ignore_ascii_case("selah")
        {
            return true;
        }
    }
    let info: *mut AnyObject = msg_send![activity, userInfo];
    if info.is_null() {
        return false;
    }
    dictionary_has_key(info, "WGWidgetUserInfoKeyKind")
        || dictionary_has_key(info, "WGWidgetUserInfoKeyFamily")
}

unsafe fn dictionary_has_key(info: *mut AnyObject, key: &str) -> bool {
    let info = &*info;
    let key = NSString::from_str(key);
    let value: *mut AnyObject = msg_send![info, objectForKey: &*key];
    !value.is_null()
}

fn install_open_observer() {
    if MainThreadMarker::new().is_none() {
        return;
    }
    let class = observer_class();
    let observer = unsafe {
        let allocated: *mut NSObject = msg_send![class, alloc];
        let initialized: *mut NSObject = msg_send![allocated, init];
        Retained::from_raw(initialized)
    };
    let Some(observer) = observer else {
        log::error!("failed to allocate widget-open observer");
        return;
    };
    let name = NSString::from_str(WIDGET_OPEN_NOTIFICATION);
    let center = NSDistributedNotificationCenter::defaultCenter();
    unsafe {
        center.addObserver_selector_name_object_suspensionBehavior(
            &observer,
            sel!(widgetOpen:),
            Some(&name),
            None,
            NSNotificationSuspensionBehavior::DeliverImmediately,
        );
    }
    OBSERVER.with(|slot| {
        *slot.borrow_mut() = Some(observer);
    });
}

fn observer_class() -> &'static AnyClass {
    static CLASS: OnceLock<&'static AnyClass> = OnceLock::new();
    CLASS.get_or_init(|| {
        if let Some(existing) = AnyClass::get(c"SelahWidgetOpenObserver") {
            return existing;
        }
        let mut builder =
            objc2::runtime::ClassBuilder::new(c"SelahWidgetOpenObserver", NSObject::class())
                .expect("SelahWidgetOpenObserver class name is unused");
        let opened: extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject) = widget_open;
        unsafe {
            builder.add_method(sel!(widgetOpen:), opened);
        }
        builder.register()
    })
}

extern "C-unwind" fn widget_open(_this: *mut AnyObject, _cmd: Sel, _note: *mut AnyObject) {
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(present_main_window));
}

#[cfg(test)]
mod tests {
    use super::is_widget_activity_type;

    #[test]
    fn recognizes_widget_activity_types() {
        assert!(is_widget_activity_type("selah.today"));
        assert!(is_widget_activity_type("com.apple.widgetkit.interaction"));
        assert!(is_widget_activity_type("com.kgu.selah.widget"));
        assert!(!is_widget_activity_type("NSUserActivityTypeBrowsingWeb"));
    }
}
