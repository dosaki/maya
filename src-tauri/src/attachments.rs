use std::path::{Path, PathBuf};

/// The largest file the composer will save.
pub const MAX_BYTES: usize = 20 * 1024 * 1024;

/// `name` reduced to a safe file name: path separators and odd characters
/// become dashes, the extension is lowercased, and blanks become "file".
fn safe_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("").trim();
    let (stem, ext) = match base.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() && !e.is_empty() => (s, Some(e.to_ascii_lowercase())),
        _ => (base, None),
    };
    let clean = |s: &str| -> String {
        let mut out = String::new();
        let mut dash = false;
        for c in s.chars() {
            if c.is_alphanumeric() || c == '_' {
                out.push(c);
                dash = false;
            } else if !dash {
                out.push('-');
                dash = true;
            }
        }
        out.trim_matches('-').chars().take(80).collect()
    };
    let stem = clean(stem);
    let stem = if stem.is_empty() { "file".to_string() } else { stem };
    match ext.map(|e| clean(&e)).filter(|e| !e.is_empty()) {
        Some(e) => format!("{stem}.{e}"),
        None => stem,
    }
}

/// Writes `bytes` to `<maya_dir>/attachments/<now>-<safe name>` and returns the path.
pub fn save(maya_dir: &Path, name: &str, bytes: &[u8], now_ms: u64) -> Result<PathBuf, String> {
    if bytes.len() > MAX_BYTES {
        return Err("The file is too large (over 20 MB).".into());
    }
    let dir = maya_dir.join("attachments");
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let base = safe_name(name);
    let mut path = dir.join(format!("{now_ms}-{base}"));
    let mut n = 1;
    while path.exists() {
        path = dir.join(format!("{now_ms}-{n}-{base}"));
        n += 1;
    }
    std::fs::write(&path, bytes).map_err(|e| format!("cannot write the attachment: {e}"))?;
    Ok(path)
}

/// True for links the app will hand to the browser.
pub fn is_web_url(url: &str) -> bool {
    let u = url.trim().to_ascii_lowercase();
    u.starts_with("https://") || u.starts_with("http://")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_under_the_attachments_dir_with_a_safe_unique_name() {
        let t = tempfile::tempdir().unwrap();
        let p = save(t.path(), "../../evil name?.PNG", b"abc", 1_700_000_000_000).unwrap();
        assert!(p.starts_with(t.path().join("attachments")));
        assert_eq!(p.file_name().unwrap().to_str().unwrap(), "1700000000000-evil-name.png");
        assert_eq!(std::fs::read(&p).unwrap(), b"abc");
        let q = save(t.path(), "shot.png", b"x", 1_700_000_000_000).unwrap();
        assert_ne!(p, q);
        assert!(save(t.path(), "", b"x", 1).unwrap().file_name().unwrap().to_str().unwrap().ends_with("-file"));
    }

    #[test]
    fn refuses_files_over_the_limit() {
        let t = tempfile::tempdir().unwrap();
        let big = vec![0u8; MAX_BYTES + 1];
        assert!(save(t.path(), "big.bin", &big, 1).unwrap_err().contains("20 MB"));
    }

    #[test]
    fn only_http_and_https_links_open() {
        assert!(is_web_url("https://github.com/o/r/pull/1"));
        assert!(is_web_url("http://localhost:1420/x"));
        assert!(!is_web_url("javascript:alert(1)"));
        assert!(!is_web_url("file:///etc/passwd"));
        assert!(!is_web_url("ftp://x"));
    }
}
