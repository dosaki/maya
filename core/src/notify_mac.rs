//! Banners Maya posts herself on a Mac, through the User Notifications
//! framework, so macOS files them under Maya and a click comes back to her
//! (a banner from `osascript` belongs to Script Editor, which a click opens).

use crate::notify::{BannerClick, ClickBook};
use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{Bool, ProtocolObject};
use objc2::{define_class, msg_send, AllocAnyThread};
use objc2_foundation::{NSBundle, NSError, NSObject, NSObjectProtocol, NSString};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNAuthorizationStatus, UNMutableNotificationContent, UNNotification, UNNotificationPresentationOptions, UNNotificationRequest, UNNotificationResponse, UNNotificationSettings, UNNotificationSound, UNUserNotificationCenter,
    UNUserNotificationCenterDelegate,
};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};

static BOOK: LazyLock<Mutex<ClickBook>> = LazyLock::new(Mutex::default);

/// Set once `start` found Maya running from an app bundle and became the
/// notification centre's delegate.
static READY: AtomicBool = AtomicBool::new(false);

define_class!(
    // SAFETY: NSObject has no subclassing requirements and `Delegate` does not implement `Drop`.
    #[unsafe(super(NSObject))]
    #[name = "MayaBannerDelegate"]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl UNUserNotificationCenterDelegate for Delegate {
        /// A banner shows (and sounds) even while Maya is the frontmost app,
        /// as `osascript`'s did: macOS hides it then unless told otherwise.
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(&self, _center: &UNUserNotificationCenter, _notification: &UNNotification, done: &block2::DynBlock<dyn Fn(UNNotificationPresentationOptions)>) {
            done.call((UNNotificationPresentationOptions::Banner | UNNotificationPresentationOptions::List | UNNotificationPresentationOptions::Sound,));
        }

        /// A click on the banner (macOS has already brought Maya forward).
        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn did_receive(&self, _center: &UNUserNotificationCenter, response: &UNNotificationResponse, done: &block2::DynBlock<dyn Fn()>) {
            let id = response.notification().request().identifier().to_string();
            crate::notify::clicked_id(&BOOK, &id);
            done.call(());
        }
    }
);

impl Delegate {
    fn new() -> Retained<Self> {
        let this = Self::alloc().set_ivars(());
        // SAFETY: NSObject's `init`, on a freshly allocated object.
        unsafe { msg_send![super(this), init] }
    }
}

/// True when this process runs from an app bundle: the notification
/// centre raises an exception for a bare binary (a debug build run straight
/// from `target/`), so that one keeps to `osascript`.
fn in_app_bundle() -> bool {
    let bundle = NSBundle::mainBundle();
    bundle.bundleIdentifier().is_some() && bundle.bundlePath().to_string().ends_with(".app")
}

fn describe(err: *mut NSError) -> String {
    // SAFETY: the framework hands either null or a valid NSError.
    match unsafe { err.as_ref() } {
        Some(e) => e.localizedDescription().to_string(),
        None => "no reason given".into(),
    }
}

/// Becomes the notification centre's delegate, so clicks reach Maya, and
/// asks for permission to show banners (macOS asks the user only once).
pub fn start() {
    if !in_app_bundle() {
        crate::log::line("notify", "not running from an app bundle: banners go through osascript, and a click on one opens Script Editor");
        return;
    }
    let center = UNUserNotificationCenter::currentNotificationCenter();
    let delegate = Delegate::new();
    center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    // The centre holds its delegate weakly: this one lives as long as Maya.
    std::mem::forget(delegate);
    let answered = RcBlock::new(|granted: Bool, err: *mut NSError| {
        if granted.as_bool() {
            crate::log::line("notify", "macOS allows Maya's banners");
        } else {
            crate::log::line("notify", format!("macOS does not allow Maya's banners ({}): they go through osascript until it is allowed in System Settings → Notifications → Maya", describe(err)));
        }
    });
    center.requestAuthorizationWithOptions_completionHandler(UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound, &answered);
    READY.store(true, Ordering::SeqCst);
}

/// Posts a banner under Maya; false when she cannot (not started from an
/// app bundle), for the caller to use `osascript`. macOS's current answer
/// is read before each banner, since it can change in System Settings at
/// any time: while Maya's banners are not allowed (macOS would take the
/// request and show nothing), or when macOS refuses one, it is shown
/// through `osascript` instead.
pub fn post(title: &str, subtitle: &str, body: &str, sound: bool, click: BannerClick) -> bool {
    if !READY.load(Ordering::SeqCst) {
        return false;
    }
    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(title));
    content.setSubtitle(&NSString::from_str(subtitle));
    content.setBody(&NSString::from_str(body));
    if sound {
        content.setSound(Some(&UNNotificationSound::defaultSound()));
    }
    let id = BOOK.lock().map(|mut b| b.add(click)).unwrap_or_else(|_| "maya-banner".into());
    let request = UNNotificationRequest::requestWithIdentifier_content_trigger(&NSString::from_str(&id), &content, None);
    let text = (title.to_string(), subtitle.to_string(), body.to_string());
    let via_osascript = move |(title, subtitle, body): (String, String, String)| {
        std::thread::spawn(move || crate::notify::banner_via_osascript(&title, &subtitle, &body, sound));
    };
    let settings = RcBlock::new(move |settings: NonNull<UNNotificationSettings>| {
        // SAFETY: the framework hands a valid settings object.
        let status = unsafe { settings.as_ref() }.authorizationStatus();
        if !allowed(status) {
            NOT_ALLOWED.call_once(|| crate::log::line("notify", "Maya's banners are not allowed in System Settings → Notifications → Maya: they go through osascript"));
            via_osascript(text.clone());
            return;
        }
        let text = text.clone();
        let posted = RcBlock::new(move |err: *mut NSError| {
            if !err.is_null() {
                crate::log::line("notify", format!("macOS refused Maya's banner ({}): showing it through osascript", describe(err)));
                via_osascript(text.clone());
            }
        });
        UNUserNotificationCenter::currentNotificationCenter().addNotificationRequest_withCompletionHandler(&request, Some(&posted));
    });
    UNUserNotificationCenter::currentNotificationCenter().getNotificationSettingsWithCompletionHandler(&settings);
    true
}

static NOT_ALLOWED: std::sync::Once = std::sync::Once::new();

/// True when macOS shows Maya's banners: allowed, or allowed quietly
/// (provisional, to Notification Center only).
fn allowed(status: UNAuthorizationStatus) -> bool {
    matches!(status, UNAuthorizationStatus::Authorized | UNAuthorizationStatus::Provisional | UNAuthorizationStatus::Ephemeral)
}
