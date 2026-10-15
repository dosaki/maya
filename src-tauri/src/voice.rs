//! ElevenLabs as an optional voice. The key lives in the macOS Keychain (the
//! Windows Credential Manager on Windows, GNOME Keyring through `secret-tool`
//! on Linux), synthesised lines are cached under `<maya_dir>/voice/`, and
//! playback uses the built-in `afplay` (MCI on Windows, `paplay` or `ffplay`
//! on Linux). Anything that fails falls back to the built-in voice.

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

pub const MODEL_ID: &str = "eleven_multilingual_v2";

/// Stores the API key in the OS key store (replacing any previous one).
pub fn store_key(key: &str) -> Result<(), String> {
    crate::keystore::store(&crate::keystore::ELEVENLABS, key)
}

/// The stored API key, if any.
pub fn load_key() -> Option<String> {
    crate::keystore::load(&crate::keystore::ELEVENLABS).ok().flatten()
}

/// The first audio player on `path` and its arguments before the file:
/// PulseAudio's (PipeWire answers it too), then ffmpeg's. Both decode the
/// MP3 ElevenLabs sends; ALSA's `aplay` does not (it plays it as static).
#[cfg(any(test, target_os = "linux"))]
pub fn player_command(path: &str) -> Option<(&'static str, Vec<String>)> {
    for (bin, args) in [("paplay", vec![]), ("ffplay", vec!["-nodisp".to_string(), "-autoexit".to_string()])] {
        if maya_core::launch::find_on_path(path, bin).is_some() {
            return Some((bin, args));
        }
    }
    None
}

/// The volume for an audio player, on its own scale; none at full volume:
/// `afplay -v` takes 1.0 as normal, PulseAudio's `paplay --volume` 65536 as
/// 100%, `ffplay -volume` 0..100.
#[cfg(any(test, unix))]
pub fn player_volume_args(bin: &str, volume: u8) -> Vec<String> {
    if volume >= 100 {
        return vec![];
    }
    match bin {
        "afplay" => vec!["-v".into(), format!("{:.2}", f32::from(volume) / 100.0)],
        "paplay" => vec![format!("--volume={}", u32::from(volume) * 65536 / 100)],
        "ffplay" => vec!["-volume".into(), volume.to_string()],
        _ => vec![],
    }
}

/// The MCI command that sets the voice's volume, out of 1000.
#[cfg(any(test, windows))]
fn mci_volume_command(volume: u8) -> String {
    format!("setaudio maya_voice volume to {}", u32::from(volume.min(100)) * 10)
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
pub fn play(path: &Path, volume: u8) -> Result<(), String> {
    use windows_sys::Win32::Media::Multimedia::mciSendStringW;
    let send = |cmd: String| -> Result<(), String> {
        let w = crate::keystore::wide(&cmd);
        // SAFETY: a NUL-terminated command; no return buffer or window.
        match unsafe { mciSendStringW(w.as_ptr(), std::ptr::null_mut(), 0, std::ptr::null_mut()) } {
            0 => Ok(()),
            code => Err(format!("could not play the voice (MCI error {code})")),
        }
    };
    send(format!("open \"{}\" type mpegvideo alias maya_voice", path.display()))?;
    if let Err(e) = send(mci_volume_command(volume)) {
        crate::log::line("voice", format!("could not set the volume: {e}"));
    }
    let played = send("play maya_voice wait".into());
    let _ = send("close maya_voice".into());
    played
}

/// Plays an audio file with the system player, waiting for it to finish.
#[cfg(target_os = "macos")]
pub fn play(path: &Path, volume: u8) -> Result<(), String> {
    let ok = maya_core::command("afplay").args(player_volume_args("afplay", volume)).arg(path).stdin(Stdio::null()).status().map_err(|e| format!("could not run afplay: {e}"))?;
    if ok.success() {
        Ok(())
    } else {
        Err("afplay failed".into())
    }
}

/// Plays an audio file with the first player found, waiting for it to finish.
#[cfg(target_os = "linux")]
pub fn play(path: &Path, volume: u8) -> Result<(), String> {
    let (bin, args) = player_command(&std::env::var("PATH").unwrap_or_default()).ok_or("No audio player found: install pulseaudio-utils.")?;
    let ok = maya_core::command(bin).args(args).args(player_volume_args(bin, volume)).arg(path).stdin(Stdio::null()).status().map_err(|e| format!("could not run {bin}: {e}"))?;
    if ok.success() {
        Ok(())
    } else {
        Err(format!("{bin} failed"))
    }
}

/// Speaks `text` with ElevenLabs, from the cache when possible.
pub fn speak(maya_dir: &Path, key: &str, voice_id: &str, text: &str, volume: u8) -> Result<(), String> {
    let path = cache_path(maya_dir, voice_id, text);
    if !path.is_file() {
        synthesize(key, voice_id, text, &path)?;
    }
    play(&path, volume)
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn players_are_told_the_volume_their_own_way() {
        for bin in ["afplay", "paplay", "ffplay"] {
            assert!(player_volume_args(bin, 100).is_empty(), "{bin} at full volume plays as before");
        }
        assert_eq!(player_volume_args("afplay", 80), ["-v", "0.80"]);
        // PulseAudio's 100% is 65536.
        assert_eq!(player_volume_args("paplay", 80), ["--volume=52428"]);
        assert_eq!(player_volume_args("paplay", 10), ["--volume=6553"]);
        assert_eq!(player_volume_args("ffplay", 80), ["-volume", "80"]);
        assert!(player_volume_args("aplay", 50).is_empty());
    }

    #[test]
    fn mci_is_told_the_volume_out_of_a_thousand() {
        assert_eq!(mci_volume_command(80), "setaudio maya_voice volume to 800");
        assert_eq!(mci_volume_command(100), "setaudio maya_voice volume to 1000");
    }

    #[cfg(unix)]
    #[test]
    fn player_is_paplay_then_ffplay_never_aplay() {
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
        assert_eq!(player_command(&path).map(|c| c.0), Some("ffplay"), "aplay cannot decode MP3");
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
