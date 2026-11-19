use zeroize::{Zeroize, Zeroizing};

#[cfg(any(
    all(feature = "aes-256-gcm-siv", feature = "xchacha20-poly1305"),
    all(feature = "aes-256-gcm-siv", feature = "aes-256-gcm"),
    all(feature = "xchacha20-poly1305", feature = "aes-256-gcm"),
))]
compile_error!("enable exactly one of the features: aes-256-gcm-siv, xchacha20-poly1305, aes-256-gcm");

#[cfg(feature = "aes-256-gcm-siv")]
mod algorithm {
    pub use aes_gcm_siv::aead::{Aead, KeyInit, Nonce, Payload};
    pub type Cipher = aes_gcm_siv::Aes256GcmSiv;
    pub const ID: u8 = 1;
    pub const NAME: &str = "AES-256-GCM-SIV";
    pub const NONCE_LEN: usize = 12;
}

#[cfg(all(feature = "xchacha20-poly1305", not(feature = "aes-256-gcm-siv")))]
mod algorithm {
    pub use chacha20poly1305::aead::{Aead, KeyInit, Nonce, Payload};
    pub type Cipher = chacha20poly1305::XChaCha20Poly1305;
    pub const ID: u8 = 2;
    pub const NAME: &str = "XChaCha20-Poly1305";
    pub const NONCE_LEN: usize = 24;
}

#[cfg(all(feature = "aes-256-gcm", not(any(feature = "aes-256-gcm-siv", feature = "xchacha20-poly1305"))))]
mod algorithm {
    pub use aes_gcm::aead::{Aead, KeyInit, Nonce, Payload};
    pub type Cipher = aes_gcm::Aes256Gcm;
    pub const ID: u8 = 3;
    pub const NAME: &str = "AES-256-GCM";
    pub const NONCE_LEN: usize = 12;
}

use algorithm::{Aead, Cipher, KeyInit, Nonce, Payload};

pub const MAGIC: &[u8; 4] = b"PAPV";
pub const VERSION: u8 = 1;
pub const KEY_LEN: usize = 32;
pub const ALGORITHM: &str = algorithm::NAME;
const DOMAIN: &[u8] = b"papilla-vault-v1";

pub struct Key(Box<[u8; KEY_LEN]>);

impl Key {
    fn blank() -> Self {
        Key(Box::new([0u8; KEY_LEN]))
    }

    pub fn from_slice(raw: &[u8]) -> Option<Key> {
        if raw.len() != KEY_LEN {
            return None;
        }
        let mut key = Key::blank();
        key.0.copy_from_slice(raw);
        Some(key)
    }

    pub fn as_slice(&self) -> &[u8] {
        self.0.as_slice()
    }
}

impl Drop for Key {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl std::fmt::Debug for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Key")
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum VaultError {
    NotSealed,
    Version(u8),
    Algorithm(u8),
    Rejected,
    Random,
}

impl VaultError {
    pub fn key(&self) -> &'static str {
        match self {
            VaultError::NotSealed => "papilla-vault-not-sealed",
            VaultError::Version(_) => "papilla-vault-version",
            VaultError::Algorithm(_) => "papilla-vault-algorithm",
            VaultError::Rejected => "papilla-vault-rejected",
            VaultError::Random => "papilla-vault-random",
        }
    }

    pub fn args(&self) -> Vec<(&'static str, String)> {
        match self {
            VaultError::Version(v) => vec![("version", v.to_string())],
            VaultError::Algorithm(id) => vec![("found", name_of(*id).to_string()), ("expected", ALGORITHM.to_string())],
            _ => Vec::new(),
        }
    }
}

impl std::fmt::Display for VaultError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.key())?;
        for (name, value) in self.args() {
            write!(f, " {name}={value}")?;
        }
        Ok(())
    }
}

impl std::error::Error for VaultError {}

fn name_of(id: u8) -> &'static str {
    match id {
        1 => "AES-256-GCM-SIV",
        2 => "XChaCha20-Poly1305",
        3 => "AES-256-GCM",
        _ => "?",
    }
}

fn associated(context: &[u8]) -> Vec<u8> {
    let mut ad = Vec::with_capacity(DOMAIN.len() + 2 + context.len());
    ad.extend_from_slice(DOMAIN);
    ad.push(VERSION);
    ad.push(algorithm::ID);
    ad.extend_from_slice(context);
    ad
}

pub fn new_key() -> Result<Key, VaultError> {
    let mut key = Key::blank();
    getrandom::fill(key.0.as_mut_slice()).map_err(|_| VaultError::Random)?;
    Ok(key)
}

pub fn is_sealed(data: &[u8]) -> bool {
    data.starts_with(MAGIC)
}

pub fn seal(key: &Key, context: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, VaultError> {
    let cipher = Cipher::new_from_slice(key.as_slice()).map_err(|_| VaultError::Rejected)?;
    let mut nonce = [0u8; algorithm::NONCE_LEN];
    getrandom::fill(&mut nonce).map_err(|_| VaultError::Random)?;
    let ad = associated(context);
    let n = Nonce::<Cipher>::try_from(&nonce[..]).map_err(|_| VaultError::Rejected)?;
    let sealed = cipher.encrypt(&n, Payload { msg: plaintext, aad: &ad }).map_err(|_| VaultError::Rejected)?;
    let mut out = Vec::with_capacity(6 + nonce.len() + sealed.len());
    out.extend_from_slice(MAGIC);
    out.push(VERSION);
    out.push(algorithm::ID);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&sealed);
    Ok(out)
}

pub fn open(key: &Key, context: &[u8], data: &[u8]) -> Result<Zeroizing<Vec<u8>>, VaultError> {
    if !is_sealed(data) || data.len() < 6 {
        return Err(VaultError::NotSealed);
    }
    if data[4] != VERSION {
        return Err(VaultError::Version(data[4]));
    }
    if data[5] != algorithm::ID {
        return Err(VaultError::Algorithm(data[5]));
    }
    let body = &data[6..];
    if body.len() < algorithm::NONCE_LEN {
        return Err(VaultError::Rejected);
    }
    let (nonce, sealed) = body.split_at(algorithm::NONCE_LEN);
    let cipher = Cipher::new_from_slice(key.as_slice()).map_err(|_| VaultError::Rejected)?;
    let n = Nonce::<Cipher>::try_from(nonce).map_err(|_| VaultError::Rejected)?;
    let ad = associated(context);
    cipher.decrypt(&n, Payload { msg: sealed, aad: &ad }).map(Zeroizing::new).map_err(|_| VaultError::Rejected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let key = new_key().unwrap();
        let sealed = seal(&key, b"uid=1000/right-index", b"template").unwrap();
        assert!(is_sealed(&sealed));
        assert_eq!(open(&key, b"uid=1000/right-index", &sealed).unwrap().as_slice(), b"template");
    }

    #[test]
    fn nonce_is_fresh_every_time() {
        let key = new_key().unwrap();
        assert_ne!(seal(&key, b"c", b"same").unwrap(), seal(&key, b"c", b"same").unwrap());
    }

    #[test]
    fn wrong_context_key_or_tampering_is_rejected() {
        let key = new_key().unwrap();
        let sealed = seal(&key, b"uid=1000/right-index", b"template").unwrap();
        assert_eq!(open(&key, b"uid=1001/right-index", &sealed).err(), Some(VaultError::Rejected));
        assert_eq!(open(&key, b"uid=1000/left-index", &sealed).err(), Some(VaultError::Rejected));
        assert_eq!(open(&new_key().unwrap(), b"uid=1000/right-index", &sealed).err(), Some(VaultError::Rejected));
        let mut flipped = sealed.clone();
        let last = flipped.len() - 1;
        flipped[last] ^= 1;
        assert_eq!(open(&key, b"uid=1000/right-index", &flipped).err(), Some(VaultError::Rejected));
    }

    #[test]
    fn other_algorithm_is_refused_not_tried() {
        let key = new_key().unwrap();
        let mut sealed = seal(&key, b"c", b"t").unwrap();
        sealed[5] = if algorithm::ID == 1 { 2 } else { 1 };
        assert_eq!(open(&key, b"c", &sealed).err(), Some(VaultError::Algorithm(sealed[5])));
        assert_eq!(open(&key, b"c", b"PAPL....").err(), Some(VaultError::NotSealed));
    }
}
