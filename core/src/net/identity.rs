//! This Maya's long-term X25519 key pair: its identity on the network
//! (protocol 2). Peers pin the public half; the private half never leaves
//! this machine and is never logged.

use base64::Engine;
use snow::params::DHChoice;
use snow::resolvers::{CryptoResolver, DefaultResolver};
use std::path::Path;

pub const KEY_LEN: usize = 32;
/// The key file of the CLI and the phone, beside their `config.json`.
pub const FILE: &str = "identity.key";
pub const BAD_KEY: &str = "That is not a Maya key.";

#[derive(Clone)]
pub struct Keypair {
    private: [u8; KEY_LEN],
    pub public: [u8; KEY_LEN],
}

impl std::fmt::Debug for Keypair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Keypair({})", self.public_b64())
    }
}

impl Keypair {
    pub fn generate() -> Keypair {
        let bytes: [u8; KEY_LEN] = super::protocol::random(KEY_LEN).try_into().expect("32 random bytes");
        Keypair::from_private(bytes)
    }

    pub fn from_private(private: [u8; KEY_LEN]) -> Keypair {
        let mut dh = DefaultResolver.resolve_dh(&DHChoice::Curve25519).expect("snow resolves Curve25519");
        dh.set(&private);
        let public = dh.pubkey().try_into().expect("a 32-byte public key");
        Keypair { private, public }
    }

    /// The private key as stored: base64.
    pub fn from_secret(secret: &str) -> Result<Keypair, String> {
        decode_key(secret).map(Keypair::from_private)
    }

    pub fn to_secret(&self) -> String {
        encode_key(&self.private)
    }

    pub fn private(&self) -> &[u8; KEY_LEN] {
        &self.private
    }

    pub fn public_b64(&self) -> String {
        encode_key(&self.public)
    }
}

pub fn encode_key(key: &[u8; KEY_LEN]) -> String {
    base64::engine::general_purpose::STANDARD.encode(key)
}

pub fn decode_key(text: &str) -> Result<[u8; KEY_LEN], String> {
    let bytes = base64::engine::general_purpose::STANDARD.decode(text.trim()).map_err(|_| BAD_KEY.to_string())?;
    bytes.try_into().map_err(|_| BAD_KEY.to_string())
}

/// The key pair in `path`, an owner-only file; made and saved there when
/// the file does not exist. A file that does not hold a key is an error,
/// never overwritten: a new key would silently drop every pairing.
pub fn load_or_create(path: &Path) -> Result<Keypair, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Keypair::from_secret(&text).map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => replace(path),
        Err(e) => Err(format!("Could not read {}: {e}", path.display())),
    }
}

/// A fresh key pair, saved over `path`.
pub fn replace(path: &Path) -> Result<Keypair, String> {
    let k = Keypair::generate();
    save(path, &k)?;
    Ok(k)
}

/// `key` saved over `path`, owner-only.
pub fn save(path: &Path, key: &Keypair) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    crate::config::write_private(path, key.to_secret().as_bytes()).map_err(|e| format!("Could not save {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_pair_derives_its_public_key_and_round_trips_as_a_secret() {
        let k = Keypair::generate();
        let again = Keypair::from_secret(&k.to_secret()).unwrap();
        assert_eq!(again.public, k.public);
        assert_eq!(again.private(), k.private());
        assert_ne!(Keypair::generate().public, k.public, "fresh pairs differ");
        assert_eq!(decode_key(&k.public_b64()).unwrap(), k.public);
    }

    #[test]
    fn debug_shows_the_public_key_only() {
        let k = Keypair::generate();
        let shown = format!("{k:?}");
        assert!(shown.contains(&k.public_b64()), "{shown}");
        assert!(!shown.contains(&k.to_secret()), "the private key never prints");
    }

    #[test]
    fn a_key_that_is_not_32_bytes_of_base64_is_refused() {
        assert_eq!(decode_key("").unwrap_err(), BAD_KEY);
        assert_eq!(decode_key("not base64!").unwrap_err(), BAD_KEY);
        assert_eq!(decode_key("AAAA").unwrap_err(), BAD_KEY, "three bytes");
        assert!(Keypair::from_secret("AAAA").is_err());
    }

    #[test]
    fn the_key_file_is_made_once_owner_only_and_replaced_on_request() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("maya").join(FILE);
        let first = load_or_create(&path).unwrap();
        assert_eq!(load_or_create(&path).unwrap().public, first.public, "loaded, not made again");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let fresh = replace(&path).unwrap();
        assert_ne!(fresh.public, first.public);
        assert_eq!(load_or_create(&path).unwrap().public, fresh.public);
    }

    #[test]
    fn a_given_key_is_saved_over_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("maya").join(FILE);
        let old = load_or_create(&path).unwrap();
        let fresh = Keypair::generate();
        save(&path, &fresh).unwrap();
        let loaded = load_or_create(&path).unwrap();
        assert_eq!(loaded.public, fresh.public);
        assert_ne!(loaded.public, old.public);
    }

    #[test]
    fn a_garbled_key_file_is_an_error_not_a_new_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE);
        std::fs::write(&path, "garbage").unwrap();
        assert!(load_or_create(&path).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "garbage", "the file is left alone");
    }
}
