//! Secrets in the OS key store: the login Keychain on macOS, Credential
//! Manager on Windows, GNOME Keyring through `secret-tool` on Linux. A
//! secret never goes on a command line: `security -i` and `secret-tool`
//! read it from stdin.

#[cfg(any(target_os = "macos", target_os = "linux"))]
use std::process::Stdio;

pub struct Item {
    /// The Keychain service and the Credential Manager target.
    pub service: &'static str,
    /// The Keychain account and the Credential Manager user name.
    pub account: &'static str,
    /// What GNOME Keyring shows.
    pub label: &'static str,
    /// The `account` attribute on Linux (`service` is always `maya` there).
    pub linux_account: &'static str,
}

pub const ELEVENLABS: Item = Item { service: "maya-elevenlabs", account: "api-key", label: "Maya ElevenLabs", linux_account: "elevenlabs" };
pub const NETWORK_KEY: Item = Item { service: "maya-network-key", account: "private-key", label: "Maya network key", linux_account: "network-key" };

/// `security` exits with this when the item is not in the keychain.
#[cfg(target_os = "macos")]
const ERR_SEC_ITEM_NOT_FOUND: i32 = 44;

/// Stores the secret in the login Keychain (replacing any previous one).
/// `security -i` reports a failed command on stderr but may still exit 0,
/// hence the stderr check.
#[cfg(target_os = "macos")]
pub fn store(item: &Item, secret: &str) -> Result<(), String> {
    use std::io::Write;
    let secret = secret.trim();
    if secret.is_empty() {
        return Err("The key is empty.".into());
    }
    if secret.contains(char::is_whitespace) || secret.contains('"') {
        return Err("The secret cannot be stored as given.".into());
    }
    let mut child = maya_core::command("security")
        .arg("-i")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not run security: {e}"))?;
    let line = format!("add-generic-password -U -s {} -a {} -w {}\n", item.service, item.account, secret);
    child.stdin.take().ok_or("no stdin for security")?.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    let err = String::from_utf8_lossy(&out.stderr);
    if out.status.success() && err.trim().is_empty() {
        Ok(())
    } else {
        Err(format!("Keychain refused it: {}", err.trim()))
    }
}

/// The stored secret: `Ok(None)` when none is stored, `Err` when the
/// Keychain could not be read.
#[cfg(target_os = "macos")]
pub fn load(item: &Item) -> Result<Option<String>, String> {
    let out = maya_core::command("security")
        .args(["find-generic-password", "-s", item.service, "-a", item.account, "-w"])
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("could not run security: {e}"))?;
    match out.status.code() {
        Some(0) => Ok(Some(String::from_utf8_lossy(&out.stdout).trim().to_string()).filter(|s| !s.is_empty())),
        Some(ERR_SEC_ITEM_NOT_FOUND) => Ok(None),
        _ => Err(format!("Keychain: {}", String::from_utf8_lossy(&out.stderr).trim())),
    }
}

/// Stores the secret in Credential Manager (replacing any previous one).
#[cfg(windows)]
pub fn store(item: &Item, secret: &str) -> Result<(), String> {
    use windows_sys::Win32::Security::Credentials::{CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC};
    let secret = secret.trim();
    if secret.is_empty() {
        return Err("The key is empty.".into());
    }
    let mut target = wide(item.service);
    let mut user = wide(item.account);
    let mut blob = secret.as_bytes().to_vec();
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
        Err(format!("Credential Manager refused it: {}", std::io::Error::last_os_error()))
    }
}

/// The stored secret: `Ok(None)` when none is stored, `Err` when Credential
/// Manager could not be read.
#[cfg(windows)]
pub fn load(item: &Item) -> Result<Option<String>, String> {
    use windows_sys::Win32::Foundation::{GetLastError, ERROR_NOT_FOUND};
    use windows_sys::Win32::Security::Credentials::{CredFree, CredReadW, CREDENTIALW, CRED_TYPE_GENERIC};
    let target = wide(item.service);
    let mut cred: *mut CREDENTIALW = std::ptr::null_mut();
    // SAFETY: CredReadW allocates the credential, which is read then freed.
    unsafe {
        if CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut cred) == 0 || cred.is_null() {
            if GetLastError() == ERROR_NOT_FOUND {
                return Ok(None);
            }
            return Err(format!("Credential Manager: {}", std::io::Error::last_os_error()));
        }
        let c = &*cred;
        let bytes = std::slice::from_raw_parts(c.CredentialBlob, c.CredentialBlobSize as usize).to_vec();
        CredFree(cred as *const _);
        let secret = String::from_utf8(bytes).map_err(|_| "Credential Manager: the secret is not text".to_string())?;
        Ok(Some(secret.trim().to_string()).filter(|s| !s.is_empty()))
    }
}

#[cfg(windows)]
pub(crate) fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

/// `secret-tool` arguments that store a secret (read from stdin) in GNOME
/// Keyring, that print it, and that list the item's attributes whether or
/// not the keyring is unlocked.
#[cfg(any(test, target_os = "linux"))]
fn secret_tool_args(item: &Item) -> (Vec<String>, Vec<String>, Vec<String>) {
    let store = ["store", "--label", item.label, "service", "maya", "account", item.linux_account].map(String::from).to_vec();
    let lookup = ["lookup", "service", "maya", "account", item.linux_account].map(String::from).to_vec();
    let search = ["search", "--all", "service", "maya", "account", item.linux_account].map(String::from).to_vec();
    (store, lookup, search)
}

/// Stores the secret in GNOME Keyring (replacing any previous one). The
/// secret goes through stdin, never the command line.
#[cfg(target_os = "linux")]
pub fn store(item: &Item, secret: &str) -> Result<(), String> {
    use std::io::Write;
    let secret = secret.trim();
    if secret.is_empty() {
        return Err("The key is empty.".into());
    }
    let mut child = maya_core::command("secret-tool")
        .args(secret_tool_args(item).0)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not run secret-tool (install libsecret-tools): {e}"))?;
    child.stdin.take().ok_or("no stdin for secret-tool")?.write_all(secret.as_bytes()).map_err(|e| e.to_string())?;
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!("GNOME Keyring refused it: {}", String::from_utf8_lossy(&out.stderr).trim()))
    }
}

/// The stored secret: `Ok(None)` when none is stored (`secret-tool` exits
/// non-zero and says nothing), `Err` when GNOME Keyring could not be read.
#[cfg(target_os = "linux")]
pub fn load(item: &Item) -> Result<Option<String>, String> {
    let out = maya_core::command("secret-tool")
        .args(secret_tool_args(item).1)
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("could not run secret-tool (install libsecret-tools): {e}"))?;
    if out.status.success() {
        return Ok(Some(String::from_utf8_lossy(&out.stdout).trim().to_string()).filter(|s| !s.is_empty()));
    }
    let err = String::from_utf8_lossy(&out.stderr);
    if err.trim().is_empty() {
        // `lookup` exits 1 and says nothing both for an absent item and for
        // an unlock prompt the user dismissed: `search` tells them apart.
        let found = maya_core::command("secret-tool")
            .args(secret_tool_args(item).2)
            .stdin(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .map_err(|e| format!("could not run secret-tool (install libsecret-tools): {e}"))?;
        let search_err = String::from_utf8_lossy(&found.stderr);
        if found.stdout.iter().any(|b| !b.is_ascii_whitespace()) {
            Err("GNOME Keyring holds the item but did not unlock it".into())
        } else if !search_err.trim().is_empty() {
            // A search that failed proves nothing: never take it for "no key".
            Err(format!("GNOME Keyring: {}", search_err.trim()))
        } else {
            Ok(None)
        }
    } else {
        Err(format!("GNOME Keyring: {}", err.trim()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_tool_args_name_the_maya_service_and_account() {
        let (store, lookup, search) = secret_tool_args(&ELEVENLABS);
        assert_eq!(store, ["store", "--label", "Maya ElevenLabs", "service", "maya", "account", "elevenlabs"].map(String::from).to_vec());
        assert_eq!(lookup, ["lookup", "service", "maya", "account", "elevenlabs"].map(String::from).to_vec());
        assert_eq!(search, ["search", "--all", "service", "maya", "account", "elevenlabs"].map(String::from).to_vec());
        let (store, lookup, search) = secret_tool_args(&NETWORK_KEY);
        assert_eq!(search.last().unwrap(), "network-key");
        assert_eq!(store[2], "Maya network key");
        assert_eq!(lookup.last().unwrap(), "network-key");
    }

    #[cfg(windows)]
    #[test]
    fn credential_manager_round_trips_and_replaces_the_secret() {
        use windows_sys::Win32::Security::Credentials::{CredDeleteW, CRED_TYPE_GENERIC};
        let service: &'static str = Box::leak(format!("maya-test-{}", std::process::id()).into_boxed_str());
        let item = Item { service, account: "test", label: "test", linux_account: "test" };
        assert_eq!(load(&item), Ok(None));
        store(&item, "  first-key\n").unwrap();
        assert_eq!(load(&item), Ok(Some("first-key".to_string())));
        store(&item, "second").unwrap();
        assert_eq!(load(&item), Ok(Some("second".to_string())));
        assert!(store(&item, "  ").unwrap_err().contains("empty"));
        unsafe { CredDeleteW(wide(service).as_ptr(), CRED_TYPE_GENERIC, 0) };
        assert_eq!(load(&item), Ok(None));
    }
}
