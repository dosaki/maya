//! ElevenLabs as an optional voice. The key lives in the macOS Keychain,
//! synthesised lines are cached under `<maya_dir>/voice/`, and playback uses
//! the built-in `afplay`. Anything that fails falls back to the built-in voice.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum VoiceProvider {
    #[default]
    Builtin,
    Elevenlabs,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Voice {
    pub voice_id: String,
    pub name: String,
}

const KEYCHAIN_SERVICE: &str = "maya-elevenlabs";
const KEYCHAIN_ACCOUNT: &str = "api-key";
pub const MODEL_ID: &str = "eleven_multilingual_v2";

/// Stores the API key in the login Keychain (replacing any previous one).
pub fn store_key(key: &str) -> Result<(), String> {
    let key = key.trim();
    if key.is_empty() {
        return Err("The key is empty.".into());
    }
    let out = Command::new("security")
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
pub fn load_key() -> Option<String> {
    let out = Command::new("security")
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
    let out = Command::new("curl")
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
    let out = Command::new("curl")
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

/// Plays an audio file with the system player, waiting for it to finish.
pub fn play(path: &Path) -> Result<(), String> {
    let ok = Command::new("afplay").arg(path).stdin(Stdio::null()).status().map_err(|e| format!("could not run afplay: {e}"))?;
    if ok.success() {
        Ok(())
    } else {
        Err("afplay failed".into())
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
    fn request_body_carries_text_and_model() {
        let b = request_body("hi \"there\"");
        let v: serde_json::Value = serde_json::from_str(&b).unwrap();
        assert_eq!(v["text"], "hi \"there\"");
        assert_eq!(v["model_id"], "eleven_multilingual_v2");
    }
}
