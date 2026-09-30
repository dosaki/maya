//! Banners, Maya's built-in voice and "do not disturb" on Windows: toasts,
//! SAPI, and Windows' notification state.

/// The Start-menu identity toasts are shown under: the installer registers
/// Maya's; an uninstalled debug build borrows PowerShell's, as Tauri does.
fn app_id() -> &'static str {
    if cfg!(debug_assertions) {
        tauri_winrt_notification::Toast::POWERSHELL_APP_ID
    } else {
        "com.dosaki.maya"
    }
}

/// Shows a toast: the session's name, the project (and machine), the ask.
pub fn show_toast(title: &str, subtitle: &str, body: &str, sound: bool) {
    use tauri_winrt_notification::{Sound, Toast};
    let toast = Toast::new(app_id()).title(title).text1(subtitle).text2(body).sound(if sound { Some(Sound::Default) } else { None });
    if let Err(e) = toast.show() {
        crate::log::line("notify", format!("could not show a notification: {e}"));
    }
}

/// `SHQueryUserNotificationState` values under which Windows itself holds
/// banners back: busy (a full-screen app), full-screen Direct3D,
/// presentation mode, and quiet time.
pub fn state_is_quiet(state: i32) -> bool {
    matches!(state, 2 | 3 | 4 | 6)
}

/// The Focus (formerly Focus Assist, now Do not disturb) profile: 0 off, 1
/// priority only, 2 alarms only. Windows publishes it only as a WNF state,
/// read here as Windows' own shell does; None when it cannot be read.
fn focus_profile() -> Option<u32> {
    #[link(name = "ntdll")]
    extern "system" {
        fn NtQueryWnfStateData(state: *const u64, type_id: *const u8, scope: *const u8, stamp: *mut u32, buffer: *mut u32, size: *mut u32) -> i32;
    }
    const WNF_SHEL_QUIETHOURS_ACTIVE_PROFILE_CHANGED: u64 = 0x0D83_063E_A3BF_1C75;
    let (mut stamp, mut value, mut size) = (0u32, 0u32, 4u32);
    // SAFETY: a 4-byte buffer whose size is passed in; the other pointers are optional and null.
    let status = unsafe { NtQueryWnfStateData(&WNF_SHEL_QUIETHOURS_ACTIVE_PROFILE_CHANGED, std::ptr::null(), std::ptr::null(), &mut stamp, &mut value, &mut size) };
    (status >= 0 && size == 4).then_some(value)
}

/// Whether Windows is keeping notifications quiet right now: Do not
/// disturb (or a Focus session) is on, or it holds banners back itself.
pub fn quiet_now() -> bool {
    if focus_profile().is_some_and(|p| p != 0) {
        return true;
    }
    let mut state = 0;
    // SAFETY: writes one i32.
    let ok = unsafe { windows_sys::Win32::UI::Shell::SHQueryUserNotificationState(&mut state) } >= 0;
    ok && state_is_quiet(state)
}

/// Token attributes tried in turn for Maya's voice: a female US English
/// voice (Zira), a British one (Hazel), then any English female voice.
const VOICE_PREFERENCES: &[&str] = &["Gender=Female;Language=409", "Gender=Female;Language=809", "Gender=Female;Language=c09", "Gender=Female;Language=1009", "Language=409"];

thread_local! {
    static VOICE: std::cell::OnceCell<Option<windows::Win32::Media::Speech::ISpVoice>> = const { std::cell::OnceCell::new() };
}

fn make_voice() -> windows::core::Result<windows::Win32::Media::Speech::ISpVoice> {
    use windows::core::HSTRING;
    use windows::Win32::Media::Speech::{ISpObjectTokenCategory, ISpVoice, SpObjectTokenCategory, SpVoice, SPCAT_VOICES};
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED};
    // SAFETY: COM calls on this thread, which initialises COM first.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let voice: ISpVoice = CoCreateInstance(&SpVoice, None, CLSCTX_ALL)?;
        let category: ISpObjectTokenCategory = CoCreateInstance(&SpObjectTokenCategory, None, CLSCTX_ALL)?;
        category.SetId(SPCAT_VOICES, false)?;
        for attrs in VOICE_PREFERENCES {
            let Ok(tokens) = category.EnumTokens(&HSTRING::from(*attrs), None) else { continue };
            let mut token = None;
            if tokens.Next(1, &mut token, None).is_ok() {
                if let Some(t) = token {
                    voice.SetVoice(&t)?;
                    break;
                }
            }
        }
        Ok(voice)
    }
}

/// Speaks `line` with the built-in voice and returns once it is said.
pub fn say(line: &str) {
    use windows::core::HSTRING;
    use windows::Win32::Media::Speech::SPF_IS_NOT_XML;
    VOICE.with(|cell| {
        let voice = cell.get_or_init(|| make_voice().inspect_err(|e| crate::log::line("speech", format!("no Windows voice: {e}"))).ok());
        if let Some(v) = voice {
            // SAFETY: a live voice and a string that outlives the call; without
            // SPF_ASYNC, Speak returns when the line has been spoken.
            if let Err(e) = unsafe { v.Speak(&HSTRING::from(line), SPF_IS_NOT_XML.0 as u32, None) } {
                crate::log::line("speech", format!("could not speak: {e}"));
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quiet_states_are_the_ones_windows_holds_banners_for() {
        // QUNS_NOT_PRESENT, QUNS_ACCEPTS_NOTIFICATIONS and QUNS_APP are not quiet.
        for s in [1, 5, 7] {
            assert!(!state_is_quiet(s), "{s}");
        }
        for s in [2, 3, 4, 6] {
            assert!(state_is_quiet(s), "{s}");
        }
    }

    #[test]
    #[ignore = "needs a Windows speech voice installed"]
    fn a_voice_is_found_without_speaking() {
        use windows::Win32::Media::Speech::ISpObjectToken;
        let v = make_voice().unwrap();
        let token: ISpObjectToken = unsafe { v.GetVoice() }.unwrap();
        let name = unsafe { token.GetStringValue(None) }.unwrap();
        println!("voice: {}", unsafe { name.to_string() }.unwrap());
    }

    #[test]
    fn the_focus_profile_can_be_read() {
        assert!(matches!(focus_profile(), Some(0..=2)), "{:?}", focus_profile());
    }
}
