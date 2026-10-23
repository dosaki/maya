//! A text-to-speech server that speaks the OpenAI audio API (Chatterbox,
//! Kokoro, OpenAI itself, ...). Maya asks it for a WAV through
//! `POST /v1/audio/speech` and lists its voices through
//! `GET /v1/audio/voices`. The key, when there is one, goes only in the
//! `Authorization` header: never in a log line or an error.

use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
/// The most audio one line may come back as.
const MAX_AUDIO: u64 = 128 * 1024 * 1024;

/// The server's root from what the user typed, or `None` when it is not an
/// `http://` or `https://` address. A trailing `/` and `/v1` are dropped, so
/// both `http://h:8004` and `http://h:8004/v1` work.
pub fn base_url(raw: &str) -> Option<String> {
    let s = raw.trim();
    if !(s.starts_with("http://") || s.starts_with("https://")) {
        return None;
    }
    let s = s.trim_end_matches('/');
    let s = s.strip_suffix("/v1").unwrap_or(s);
    let s = s.trim_end_matches('/');
    if s == "http:" || s == "https:" {
        return None;
    }
    Some(s.to_string())
}

pub fn speech_url(base: &str) -> String {
    format!("{base}/v1/audio/speech")
}

pub fn voices_url(base: &str) -> String {
    format!("{base}/v1/audio/voices")
}

/// The JSON for one line. `voice` and `model` are left out when blank: the
/// server then uses its own default (some reject a name they do not know).
pub fn request_body(text: &str, voice: &str, model: &str) -> String {
    let mut v = json!({ "input": text, "response_format": "wav" });
    if !voice.trim().is_empty() {
        v["voice"] = json!(voice);
    }
    if !model.trim().is_empty() {
        v["model"] = json!(model);
    }
    v.to_string()
}

/// Voice names from a voices reply: strings, or objects with a `name`.
pub fn parse_voices(json: &str) -> Result<Vec<String>, String> {
    let v: Value = serde_json::from_str(json).map_err(|e| format!("the voice list is not JSON: {e}"))?;
    let Some(list) = v["voices"].as_array() else { return Ok(Vec::new()) };
    Ok(list.iter().filter_map(|x| x.as_str().or_else(|| x["name"].as_str())).map(str::to_string).collect())
}

fn agent() -> ureq::Agent {
    let tls = ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::PlatformVerifier).build();
    let config = ureq::Agent::config_builder()
        .timeout_connect(Some(CONNECT_TIMEOUT))
        .timeout_global(Some(REQUEST_TIMEOUT))
        .http_status_as_error(false)
        .tls_config(tls)
        .build();
    ureq::Agent::new_with_config(config)
}

fn unreachable(base: &str, e: impl std::fmt::Display) -> String {
    format!("could not reach the voice server at {base}: {e}")
}

fn answered(status: u16, body: &[u8]) -> String {
    let text: String = String::from_utf8_lossy(body).chars().take(200).collect();
    format!("the voice server answered {status}: {text}")
}

/// The voices the server offers. A 404 means it has no list, which is not
/// an error: the user types the voice name instead.
pub fn list_voices(base: &str, key: Option<&str>) -> Result<Vec<String>, String> {
    let mut req = agent().get(voices_url(base));
    if let Some(k) = key {
        req = req.header("Authorization", &format!("Bearer {k}"));
    }
    let mut resp = req.call().map_err(|e| unreachable(base, e))?;
    let status = resp.status().as_u16();
    let body = resp.body_mut().read_to_vec().map_err(|e| unreachable(base, e))?;
    match status {
        404 => Ok(Vec::new()),
        200..=299 => parse_voices(&String::from_utf8_lossy(&body)),
        _ => Err(answered(status, &body)),
    }
}

/// Speak `text` and write the WAV to `path`. The audio lands in
/// `<path>.part` first, so a half-written file is never taken for a line;
/// on any failure no file is left at `path`.
pub fn synthesize(base: &str, key: Option<&str>, voice: &str, model: &str, text: &str, path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("could not save the audio: {e}"))?;
    }
    let mut req = agent().post(speech_url(base)).header("Content-Type", "application/json");
    if let Some(k) = key {
        req = req.header("Authorization", &format!("Bearer {k}"));
    }
    let mut resp = req.send(request_body(text, voice, model)).map_err(|e| unreachable(base, e))?;
    let status = resp.status().as_u16();
    let kind = resp.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_ascii_lowercase();
    let body = resp.body_mut().with_config().limit(MAX_AUDIO).read_to_vec().map_err(|e| unreachable(base, e))?;
    if !(200..=299).contains(&status) {
        return Err(answered(status, &body));
    }
    if body.is_empty() {
        return Err(format!("the voice server at {base} sent no audio"));
    }
    if kind.starts_with("application/json") || kind.starts_with("text/") {
        return Err(answered(status, &body));
    }
    let mut part = PathBuf::from(path);
    part.as_mut_os_string().push(".part");
    let written = std::fs::File::create(&part).and_then(|mut f| f.write_all(&body)).and_then(|_| std::fs::rename(&part, path));
    written.map_err(|e| {
        let _ = std::fs::remove_file(&part);
        format!("could not save the audio: {e}")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::net::TcpListener;

    #[test]
    fn base_url_is_normalised_and_needs_a_scheme() {
        assert_eq!(base_url(" http://localhost:8004/ ").as_deref(), Some("http://localhost:8004"));
        assert_eq!(base_url("http://mac.local:8004/v1/").as_deref(), Some("http://mac.local:8004"));
        assert_eq!(base_url("https://api.openai.com/v1").as_deref(), Some("https://api.openai.com"));
        assert_eq!(base_url("localhost:8004"), None);
        assert_eq!(base_url("   "), None);
        assert_eq!(base_url("http://"), None);
        assert_eq!(base_url("https:///"), None);
        assert_eq!(speech_url("http://h:1"), "http://h:1/v1/audio/speech");
        assert_eq!(voices_url("http://h:1"), "http://h:1/v1/audio/voices");
    }

    #[test]
    fn request_asks_for_wav_and_sends_the_model_only_when_given() {
        let v: serde_json::Value = serde_json::from_str(&request_body("hi \"x\"", "tiago.wav", " ")).unwrap();
        assert_eq!(v["input"], "hi \"x\"");
        assert_eq!(v["voice"], "tiago.wav");
        assert_eq!(v["response_format"], "wav");
        assert!(v.get("model").is_none());
        let v: serde_json::Value = serde_json::from_str(&request_body("hi", "alloy", "tts-1")).unwrap();
        assert_eq!(v["model"], "tts-1");
        let v: serde_json::Value = serde_json::from_str(&request_body("hi", " ", "tts-1")).unwrap();
        assert!(v.get("voice").is_none(), "a blank voice leaves the server's default");
    }

    #[test]
    fn voices_parse_from_names_or_objects() {
        assert_eq!(parse_voices(r#"{"status":"ok","voices":["default","tiago.wav"]}"#).unwrap(), vec!["default", "tiago.wav"]);
        assert_eq!(parse_voices(r#"{"voices":[{"name":"amy"},{"id":"x"}]}"#).unwrap(), vec!["amy"]);
        assert!(parse_voices("{}").unwrap().is_empty());
        assert!(parse_voices("nope").is_err());
    }

    /// A server that answers one request: the raw request text comes back
    /// from the thread.
    fn serve_once(status: &str, content_type: &str, body: &[u8]) -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let head = format!("HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
        let body = body.to_vec();
        let handle = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut raw = Vec::new();
            let mut buf = [0u8; 1024];
            let split = loop {
                let n = s.read(&mut buf).unwrap();
                assert!(n > 0, "the request ended before its head did");
                raw.extend_from_slice(&buf[..n]);
                if let Some(i) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                    break i + 4;
                }
            };
            let head_text = String::from_utf8_lossy(&raw[..split]).to_ascii_lowercase();
            let len: usize = head_text.lines().find_map(|l| l.strip_prefix("content-length:")).map_or(0, |v| v.trim().parse().unwrap());
            while raw.len() < split + len {
                let n = s.read(&mut buf).unwrap();
                assert!(n > 0, "the request ended before its body did");
                raw.extend_from_slice(&buf[..n]);
            }
            s.write_all(head.as_bytes()).unwrap();
            s.write_all(&body).unwrap();
            String::from_utf8_lossy(&raw).into_owned()
        });
        (base, handle)
    }

    #[test]
    fn synthesize_writes_the_audio_and_sends_the_key_as_a_bearer_header() {
        let dir = tempfile::tempdir().unwrap();
        // The voice folder does not exist yet, as on a device that never used ElevenLabs.
        let path = dir.path().join("voice/line.wav");
        let (base, server) = serve_once("200 OK", "audio/wav", b"RIFF....WAVE");
        synthesize(&base, Some("k1"), "tiago.wav", "", "hello", &path).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"RIFF....WAVE");
        assert!(!dir.path().join("voice/line.wav.part").exists());
        let req = server.join().unwrap();
        assert!(req.contains("POST /v1/audio/speech"));
        assert!(req.to_ascii_lowercase().contains("authorization: bearer k1"));
        assert!(req.contains("\"response_format\":\"wav\""));
    }

    #[test]
    fn synthesize_leaves_no_file_for_an_error_or_a_non_audio_answer() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("line.wav");
        let (base, server) = serve_once("500 Internal Server Error", "text/plain", b"boom");
        let e = synthesize(&base, None, "v", "", "hi", &path).unwrap_err();
        server.join().unwrap();
        assert!(e.contains("500") && e.contains("boom"), "{e}");
        assert!(!path.exists());
        let (base, server) = serve_once("200 OK", "application/json", br#"{"error":"x"}"#);
        assert!(synthesize(&base, None, "v", "", "hi", &path).is_err());
        server.join().unwrap();
        assert!(!path.exists());
        let (base, server) = serve_once("200 OK", "audio/wav", b"");
        assert!(synthesize(&base, None, "v", "", "hi", &path).is_err());
        server.join().unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn list_voices_treats_a_404_as_no_list() {
        let (base, server) = serve_once("404 Not Found", "text/plain", b"no");
        assert_eq!(list_voices(&base, None).unwrap(), Vec::<String>::new());
        server.join().unwrap();
        let (base, server) = serve_once("200 OK", "application/json", br#"{"voices":["a.wav"]}"#);
        assert_eq!(list_voices(&base, None).unwrap(), vec!["a.wav"]);
        server.join().unwrap();
    }

    #[test]
    fn an_unreachable_server_fails_fast() {
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let dir = tempfile::tempdir().unwrap();
        let started = std::time::Instant::now();
        let e = synthesize(&format!("http://127.0.0.1:{port}"), None, "v", "", "hi", &dir.path().join("x.wav")).unwrap_err();
        assert!(e.contains("could not reach the voice server"), "{e}");
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}
