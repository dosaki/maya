//! ElevenLabs as an optional voice. The key lives in the macOS Keychain (the
//! Windows Credential Manager on Windows, GNOME Keyring through `secret-tool`
//! on Linux), synthesised lines are cached under `<maya_dir>/voice/`, and
//! playback uses the built-in `afplay` (MCI on Windows, `paplay`, `aplay` or
//! `ffplay` on Linux). Anything that fails falls back to the built-in voice.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Stdio;

pub use maya_core::config::VoiceProvider;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Voice {
    pub voice_id: String,
    pub name: String,
}

#[cfg(any(target_os = "macos", windows))]
const KEYCHAIN_SERVICE: &str = "maya-elevenlabs";
#[cfg(any(target_os = "macos", windows))]
const KEYCHAIN_ACCOUNT: &str = "api-key";
pub const MODEL_ID: &str = "eleven_multilingual_v2";

/// Stores the API key in Credential Manager (replacing any previous one).
#[cfg(windows)]
pub fn store_key(key: &str) -> Result<(), String> {
    store_credential(KEYCHAIN_SERVICE, key)
}

/// The stored API key, if any.
#[cfg(windows)]
pub fn load_key() -> Option<String> {
    load_credential(KEYCHAIN_SERVICE)
}

#[cfg(windows)]
fn store_credential(service: &str, key: &str) -> Result<(), String> {
    use windows_sys::Win32::Security::Credentials::{CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC};
    let key = key.trim();
    if key.is_empty() {
        return Err("The key is empty.".into());
    }
    let mut target = wide(service);
    let mut user = wide(KEYCHAIN_ACCOUNT);
    let mut blob = key.as_bytes().to_vec();
    let mut cred: CREDENTIALW = unsafe { std::mem::zeroed() };
    cred.Type = CRED_TYPE_GENERIC;
    cred.TargetName = target.as_mut_ptr();
    cred.UserName = user.as_mut_ptr();
    cred.CredentialBlobSize = blob.len() as u32;
    cred.CredentialBlob = blob.as_mut_ptr();
    cred.Persist = CRED_PERSIST_LOCAL_MACHINE;
    // SAFETY: the credential's strings and blob outlive the call.
    if unsafe { CredWriteW(&cred, 0) } != 0 {
        Ok(())
    } else {
        Err(format!("Credential Manager refused the key: {}", std::io::Error::last_os_error()))
    }
}

#[cfg(windows)]
fn load_credential(service: &str) -> Option<String> {
    use windows_sys::Win32::Security::Credentials::{CredFree, CredReadW, CREDENTIALW, CRED_TYPE_GENERIC};
    let target = wide(service);
    let mut cred: *mut CREDENTIALW = std::ptr::null_mut();
    // SAFETY: CredReadW allocates the credential, which is read then freed.
    unsafe {
        if CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut cred) == 0 || cred.is_null() {
            return None;
        }
        let c = &*cred;
        let bytes = std::slice::from_raw_parts(c.CredentialBlob, c.CredentialBlobSize as usize).to_vec();
        CredFree(cred as *const _);
        let key = String::from_utf8(bytes).ok()?.trim().to_string();
        (!key.is_empty()).then_some(key)
    }
}

#[cfg(windows)]
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

/// Stores the API key in the login Keychain (replacing any previous one).
#[cfg(target_os = "macos")]
pub fn store_key(key: &str) -> Result<(), String> {
    let key = key.trim();
    if key.is_empty() {
        return Err("The key is empty.".into());
    }
    let out = maya_core::command("security")
        .args(["add-generic-password", "-U", "-s", KEYCHAIN_SERVICE, "-a", KEYCHAIN_ACCOUNT, "-w", key])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("could not run security: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!("Keychain refused the key: {}", String::from_utf8_lossy(&out.stderr).trim()))
    }
}

/// The stored API key, if any.
#[cfg(target_os = "macos")]
pub fn load_key() -> Option<String> {
    let out = maya_core::command("security")
        .args(["find-generic-password", "-s", KEYCHAIN_SERVICE, "-a", KEYCHAIN_ACCOUNT, "-w"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let key = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!key.is_empty()).then_some(key)
}

/// `secret-tool` arguments that store the key (read from stdin) in GNOME Keyring.
#[cfg(any(test, target_os = "linux"))]
pub fn secret_tool_store_args() -> Vec<String> {
    ["store", "--label", "Maya ElevenLabs", "service", "maya", "account", "elevenlabs"].map(String::from).to_vec()
}

/// `secret-tool` arguments that print the stored key.
#[cfg(any(test, target_os = "linux"))]
pub fn secret_tool_lookup_args() -> Vec<String> {
    ["lookup", "service", "maya", "account", "elevenlabs"].map(String::from).to_vec()
}

/// The first audio player on `path` and its arguments before the file:
/// PulseAudio's (PipeWire answers it too), then ALSA's, then ffmpeg's.
#[cfg(any(test, target_os = "linux"))]
pub fn player_command(path: &str) -> Option<(&'static str, Vec<String>)> {
    for (bin, args) in [("paplay", vec![]), ("aplay", vec![]), ("ffplay", vec!["-nodisp".to_string(), "-autoexit".to_string()])] {
        if maya_core::launch::find_on_path(path, bin).is_some() {
            return Some((bin, args));
        }
    }
    None
}

/// Stores the API key in GNOME Keyring (replacing any previous one). The
/// key goes through stdin, never the command line.
#[cfg(target_os = "linux")]
pub fn store_key(key: &str) -> Result<(), String> {
    use std::io::Write;
    let key = key.trim();
    if key.is_empty() {
        return Err("The key is empty.".into());
    }
    let mut child = maya_core::command("secret-tool")
        .args(secret_tool_store_args())
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not run secret-tool (install libsecret-tools): {e}"))?;
    child.stdin.take().ok_or("no stdin for secret-tool")?.write_all(key.as_bytes()).map_err(|e| e.to_string())?;
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!("GNOME Keyring refused the key: {}", String::from_utf8_lossy(&out.stderr).trim()))
    }
}

/// The stored API key, if any.
#[cfg(target_os = "linux")]
pub fn load_key() -> Option<String> {
    let out = maya_core::command("secret-tool").args(secret_tool_lookup_args()).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let key = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!key.is_empty()).then_some(key)
}

/// True when the provider is ElevenLabs, a key is stored and a voice chosen.
pub fn use_elevenlabs(provider: VoiceProvider, has_key: bool, voice_id: Option<&str>) -> bool {
    provider == VoiceProvider::Elevenlabs && has_key && voice_id.map_or(false, |v| !v.trim().is_empty())
}

pub fn parse_voices(json: &str) -> Result<Vec<Voice>, String> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| format!("ElevenLabs answered with something that is not JSON: {e}"))?;
    if let Some(detail) = v["detail"]["message"].as_str().or_else(|| v["detail"].as_str()) {
        return Err(format!("ElevenLabs: {detail}"));
    }
    Ok(v["voices"]
        .as_array()
        .map(|a| a.iter().filter_map(|x| Some(Voice { voice_id: x["voice_id"].as_str()?.to_string(), name: x["name"].as_str()?.to_string() })).collect())
        .unwrap_or_default())
}

/// The account's voices.
pub fn list_voices(key: &str) -> Result<Vec<Voice>, String> {
    let out = maya_core::command("curl")
        .args(["-fsS", "--max-time", "15", "-H", &format!("xi-api-key: {key}"), "https://api.elevenlabs.io/v1/voices"])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("could not run curl: {e}"))?;
    if !out.status.success() {
        return Err(format!("could not reach ElevenLabs: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    parse_voices(&String::from_utf8_lossy(&out.stdout))
}

pub fn request_body(text: &str) -> String {
    serde_json::json!({ "text": text, "model_id": MODEL_ID }).to_string()
}

fn hash(s: &str) -> u64 {
    // FNV-1a: stable across runs, unlike the std hasher.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// Where the audio for `text` in `voice_id` is cached.
pub fn cache_path(maya_dir: &Path, voice_id: &str, text: &str) -> PathBuf {
    let safe_voice: String = voice_id.chars().filter(|c| c.is_ascii_alphanumeric()).take(24).collect();
    maya_dir.join("voice").join(format!("{safe_voice}-{:016x}.mp3", hash(text)))
}

/// Synthesises `text` to `path`, returning the bytes written.
pub fn synthesize(key: &str, voice_id: &str, text: &str, path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("part");
    let url = format!("https://api.elevenlabs.io/v1/text-to-speech/{voice_id}?output_format=mp3_44100_128");
    let out = maya_core::command("curl")
        .args(["-fsS", "--max-time", "20", "-X", "POST", "-H", &format!("xi-api-key: {key}"), "-H", "Content-Type: application/json", "-d", &request_body(text), "-o"])
        .arg(&tmp)
        .arg(&url)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("could not run curl: {e}"))?;
    if !out.status.success() {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("ElevenLabs synthesis failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

/// Plays an audio file through MCI, waiting for it to finish.
#[cfg(windows)]
pub fn play(path: &Path) -> Result<(), String> {
    use windows_sys::Win32::Media::Multimedia::mciSendStringW;
    let send = |cmd: String| -> Result<(), String> {
        let w = wide(&cmd);
        // SAFETY: a NUL-terminated command; no return buffer or window.
        match unsafe { mciSendStringW(w.as_ptr(), std::ptr::null_mut(), 0, std::ptr::null_mut()) } {
            0 => Ok(()),
            code => Err(format!("could not play the voice (MCI error {code})")),
        }
    };
    send(format!("open \"{}\" type mpegvideo alias maya_voice", path.display()))?;
    let played = send("play maya_voice wait".into());
    let _ = send("close maya_voice".into());
    played
}

/// Plays an audio file with the system player, waiting for it to finish.
#[cfg(target_os = "macos")]
pub fn play(path: &Path) -> Result<(), String> {
    let ok = maya_core::command("afplay").arg(path).stdin(Stdio::null()).status().map_err(|e| format!("could not run afplay: {e}"))?;
    if ok.success() {
        Ok(())
    } else {
        Err("afplay failed".into())
    }
}

/// Plays an audio file with the first player found, waiting for it to finish.
#[cfg(target_os = "linux")]
pub fn play(path: &Path) -> Result<(), String> {
    let (bin, args) = player_command(&std::env::var("PATH").unwrap_or_default()).ok_or("No audio player found: install pulseaudio-utils.")?;
    let ok = maya_core::command(bin).args(args).arg(path).stdin(Stdio::null()).status().map_err(|e| format!("could not run {bin}: {e}"))?;
    if ok.success() {
        Ok(())
    } else {
        Err(format!("{bin} failed"))
    }
}

/// Speaks `text` with ElevenLabs, from the cache when possible.
pub fn speak(maya_dir: &Path, key: &str, voice_id: &str, text: &str) -> Result<(), String> {
    let path = cache_path(maya_dir, voice_id, text);
    if !path.is_file() {
        synthesize(key, voice_id, text, &path)?;
    }
    play(&path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn credential_manager_round_trips_and_replaces_the_key() {
        use windows_sys::Win32::Security::Credentials::{CredDeleteW, CRED_TYPE_GENERIC};
        let service = format!("maya-test-{}", std::process::id());
        assert_eq!(load_credential(&service), None);
        store_credential(&service, "  first-key
").unwrap();
        assert_eq!(load_credential(&service).as_deref(), Some("first-key"));
        store_credential(&service, "second").unwrap();
        assert_eq!(load_credential(&service).as_deref(), Some("second"));
        assert!(store_credential(&service, "  ").unwrap_err().contains("empty"));
        unsafe { CredDeleteW(wide(&service).as_ptr(), CRED_TYPE_GENERIC, 0) };
        assert_eq!(load_credential(&service), None);
    }
    use std::path::Path;

    #[test]
    fn cache_path_is_stable_per_voice_and_text_and_a_safe_file_name() {
        let a = cache_path(Path::new("/m"), "v1", "hexgrid d3 needs a decision");
        let b = cache_path(Path::new("/m"), "v1", "hexgrid d3 needs a decision");
        let c = cache_path(Path::new("/m"), "v2", "hexgrid d3 needs a decision");
        let d = cache_path(Path::new("/m"), "v1", "other line");
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_ne!(a, d);
        assert!(a.starts_with("/m/voice"));
        let name = a.file_name().unwrap().to_str().unwrap();
        assert!(name.ends_with(".mp3"));
        assert!(name.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '.'));
    }

    #[test]
    fn voices_parse_from_the_api_listing() {
        let json = r#"{"voices":[{"voice_id":"abc","name":"Rachel","category":"premade"},{"voice_id":"def","name":"Custom"}]}"#;
        let v = parse_voices(json).unwrap();
        assert_eq!(v, vec![Voice { voice_id: "abc".into(), name: "Rachel".into() }, Voice { voice_id: "def".into(), name: "Custom".into() }]);
        assert!(parse_voices("{}").unwrap().is_empty());
        assert!(parse_voices("nope").is_err());
    }

    #[test]
    fn elevenlabs_is_used_only_when_fully_configured() {
        assert!(!use_elevenlabs(VoiceProvider::Builtin, true, Some("v")));
        assert!(!use_elevenlabs(VoiceProvider::Elevenlabs, false, Some("v")));
        assert!(!use_elevenlabs(VoiceProvider::Elevenlabs, true, None));
        assert!(!use_elevenlabs(VoiceProvider::Elevenlabs, true, Some("  ")));
        assert!(use_elevenlabs(VoiceProvider::Elevenlabs, true, Some("v")));
    }

    #[test]
    fn secret_tool_args_name_the_maya_service_and_account() {
        assert_eq!(secret_tool_store_args(), ["store", "--label", "Maya ElevenLabs", "service", "maya", "account", "elevenlabs"].map(String::from).to_vec());
        assert_eq!(secret_tool_lookup_args(), ["lookup", "service", "maya", "account", "elevenlabs"].map(String::from).to_vec());
    }

    #[cfg(unix)]
    #[test]
    fn player_is_paplay_then_aplay_then_ffplay() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().to_string_lossy().into_owned();
        let exe = |name: &str| {
            let p = d.path().join(name);
            std::fs::write(&p, "#!/bin/sh\n").unwrap();
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        };
        assert_eq!(player_command(""), None);
        assert_eq!(player_command(&path), None);
        exe("ffplay");
        assert_eq!(player_command(&path), Some(("ffplay", vec!["-nodisp".to_string(), "-autoexit".to_string()])));
        exe("aplay");
        assert_eq!(player_command(&path).map(|c| c.0), Some("aplay"));
        exe("paplay");
        assert_eq!(player_command(&path).map(|c| c.0), Some("paplay"));
    }

    #[test]
    fn request_body_carries_text_and_model() {
        let b = request_body("hi \"there\"");
        let v: serde_json::Value = serde_json::from_str(&b).unwrap();
        assert_eq!(v["text"], "hi \"there\"");
        assert_eq!(v["model_id"], "eleven_multilingual_v2");
    }
}
