//! The Dock icon for an unbundled run. A bundled Maya.app gets its icon from
//! the bundle; the bare debug binary would show the generic one.

/// The app icon, embedded at build time.
pub const ICON_PNG: &[u8] = include_bytes!("../icons/icon.png");

/// Sets the running process's Dock icon. Must run on the main thread.
pub fn set_dock_icon() -> bool {
    use objc2::{AllocAnyThread, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;
    let Some(mtm) = MainThreadMarker::new() else { return false };
    let data = NSData::with_bytes(ICON_PNG);
    let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) else { return false };
    // SAFETY: called on the main thread with a valid image; AppKit copies it.
    unsafe { NSApplication::sharedApplication(mtm).setApplicationIconImage(Some(&image)) };
    true
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_embedded_icon_is_a_png() {
        assert!(super::ICON_PNG.starts_with(b"\x89PNG"));
        assert!(super::ICON_PNG.len() > 10_000);
    }
}
