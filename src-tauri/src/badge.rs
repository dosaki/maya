//! The Dock or taskbar indicator: a badge with the number of sessions
//! waiting for a decision, and a request for attention when a new one
//! arrives (the Dock icon bounces once on macOS, the taskbar button
//! flashes on Windows, the window gets the urgency hint on Linux).

use maya_core::log;
use maya_core::model::{Card, State};
use std::sync::atomic::{AtomicUsize, Ordering};
use tauri::{AppHandle, Manager, UserAttentionType};

/// What the badge counts: sessions waiting for a decision, this machine's
/// and the assistants' alike. Finished sessions are left out, since they
/// never clear on their own and the badge would stick.
pub fn awaiting_count(cards: &[Card]) -> usize {
    cards.iter().filter(|c| c.state == State::Awaiting).count()
}

/// The count last shown, so an unchanged count costs no call into the shell.
static SHOWN: AtomicUsize = AtomicUsize::new(0);

/// Shows `count` on the app's icon, or clears the badge at zero. Windows
/// has no count on a taskbar button, so it gets a dot overlay instead.
pub fn show(app: &AppHandle, count: usize) {
    if SHOWN.swap(count, Ordering::Relaxed) == count {
        return;
    }
    let Some(window) = app.get_webview_window("main") else { return };
    #[cfg(target_os = "windows")]
    let result = window.set_overlay_icon(if count == 0 { None } else { Some(tauri::image::Image::new_owned(dot_rgba(DOT_SIZE), DOT_SIZE, DOT_SIZE)) });
    #[cfg(not(target_os = "windows"))]
    let result = window.set_badge_count(if count == 0 { None } else { Some(count as i64) });
    if let Err(e) = result {
        log::line("app", format!("could not show {count} on the app icon: {e}"));
    }
}

/// Asks for the user's attention once, for a new ask. Does nothing while
/// Maya is the focused app.
pub fn bounce(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else { return };
    if let Err(e) = window.request_user_attention(Some(UserAttentionType::Informational)) {
        log::line("app", format!("could not ask for attention: {e}"));
    }
}

/// The overlay dot's side, in pixels (Windows draws it at 16 or so).
#[allow(dead_code)]
pub const DOT_SIZE: u32 = 32;

/// A red disc on a transparent square, as RGBA rows, for the Windows
/// taskbar overlay. The edge is blended over one pixel.
#[allow(dead_code)]
pub fn dot_rgba(size: u32) -> Vec<u8> {
    let (r, g, b) = (0xe5u8, 0x48u8, 0x4du8);
    let centre = size as f32 / 2.0;
    let radius = centre - 1.0;
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let d = ((x as f32 + 0.5 - centre).powi(2) + (y as f32 + 0.5 - centre).powi(2)).sqrt();
            let alpha = (radius - d + 0.5).clamp(0.0, 1.0);
            out.extend_from_slice(&[r, g, b, (alpha * 255.0).round() as u8]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use maya_core::model::Harness;

    fn card(state: State) -> Card {
        Card {
            session_id: String::new(), pid: 0, name: String::new(), cwd: String::new(), state, state_since: 0, snippet: String::new(),
            awaiting: None, has_inbox: true, harness: Harness::ClaudeCode, pr: None, context: None, machine: None,
            machine_address: None, machine_platform: None, terminal: None, stale: false,
        }
    }

    #[test]
    fn the_badge_counts_sessions_waiting_for_a_decision_only() {
        let cards = [card(State::Awaiting), card(State::Completed), card(State::Working), card(State::Awaiting), card(State::Idle)];
        assert_eq!(awaiting_count(&cards), 2);
        assert_eq!(awaiting_count(&[]), 0);
    }

    #[test]
    fn the_dot_is_an_opaque_red_disc_on_a_transparent_square() {
        let px = dot_rgba(DOT_SIZE);
        assert_eq!(px.len(), (DOT_SIZE * DOT_SIZE * 4) as usize);
        let at = |x: u32, y: u32| {
            let i = ((y * DOT_SIZE + x) * 4) as usize;
            (px[i], px[i + 1], px[i + 2], px[i + 3])
        };
        assert_eq!(at(0, 0).3, 0, "the corner is transparent");
        assert_eq!(at(DOT_SIZE / 2, DOT_SIZE / 2), (0xe5, 0x48, 0x4d, 255), "the centre is solid red");
        assert!(at(DOT_SIZE / 2, 0).3 < 255, "the edge is blended");
    }
}
