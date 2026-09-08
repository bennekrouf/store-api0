// src/infra/secret_box.rs
//
// Authenticated encryption for the secrets this store holds on a tenant's or a
// user's behalf — Azure DevOps PATs today, Entra refresh tokens next.
//
// AES-256-GCM. Every ciphertext is laid out as
//
//     version(1) ‖ nonce(12) ‖ ciphertext ‖ tag(16)
//
// The version byte is what makes key rotation possible later without a flag day:
// a reader can tell which key a record was sealed with. There is one version so
// far, and it is still worth writing.
//
// The nonce is fresh random per seal. Reusing a nonce under one AES-GCM key
// destroys the guarantee entirely, which is why sealing lives here and nowhere
// else — no caller is in a position to get it wrong.
//
// The associated data binds a ciphertext to where it lives: a value sealed for
// (tenant A, "pat") will not open as (tenant B, "pat") or as (tenant A,
// "entra_refresh"). Moving a row between tenants or columns turns it into
// undecryptable bytes rather than a working credential.

use aes_gcm::aead::{Aead, AeadCore, KeyInit, OsRng, Payload};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use anyhow::{anyhow, Context, Result};
use base64::Engine;

const VERSION: u8 = 1;
const NONCE_LEN: usize = 12;

/// Where a secret lives, mixed into the AEAD as associated data.
pub struct SecretContext<'a> {
    pub tenant_id: &'a str,
    pub purpose: &'a str,
}

impl SecretContext<'_> {
    fn associated_data(&self) -> Vec<u8> {
        format!("{}\u{1f}{}", self.tenant_id, self.purpose).into_bytes()
    }
}

/// Encrypt `plaintext` for storage.
pub fn seal(plaintext: &str, context: &SecretContext<'_>) -> Result<Vec<u8>> {
    let cipher = cipher()?;
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let aad = context.associated_data();

    let ciphertext = cipher
        .encrypt(
            &nonce,
            Payload {
                msg: plaintext.as_bytes(),
                aad: &aad,
            },
        )
        .map_err(|_| anyhow!("could not seal the secret"))?;

    let mut out = Vec::with_capacity(1 + NONCE_LEN + ciphertext.len());
    out.push(VERSION);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

/// Decrypt a stored value. Fails if the key is wrong, the bytes were tampered
/// with, or the record was written for a different tenant or purpose.
pub fn open(sealed: &[u8], context: &SecretContext<'_>) -> Result<String> {
    if sealed.len() < 1 + NONCE_LEN {
        return Err(anyhow!("sealed value is too short to be valid"));
    }
    if sealed[0] != VERSION {
        return Err(anyhow!(
            "sealed value has unknown version {} — written by a newer release?",
            sealed[0]
        ));
    }

    let cipher = cipher()?;
    let nonce = Nonce::from_slice(&sealed[1..1 + NONCE_LEN]);
    let aad = context.associated_data();

    let plaintext = cipher
        .decrypt(
            nonce,
            Payload {
                msg: &sealed[1 + NONCE_LEN..],
                aad: &aad,
            },
        )
        .map_err(|_| anyhow!("could not open the secret — wrong key, or the record was altered"))?;

    String::from_utf8(plaintext).context("sealed value did not contain valid UTF-8")
}

/// True when a usable key is configured. Lets a handler answer "encryption is
/// not set up" clearly instead of failing on the first write.
pub fn is_configured() -> bool {
    cipher().is_ok()
}

fn cipher() -> Result<Aes256Gcm> {
    let raw = std::env::var("API0_ENCRYPTION_KEY")
        .ok()
        .filter(|k| !k.is_empty())
        .ok_or_else(|| anyhow!("API0_ENCRYPTION_KEY is not set"))?;

    let bytes = base64::engine::general_purpose::STANDARD
        .decode(raw.trim())
        .context("API0_ENCRYPTION_KEY is not valid base64")?;

    if bytes.len() != 32 {
        return Err(anyhow!(
            "API0_ENCRYPTION_KEY must decode to 32 bytes, got {}",
            bytes.len()
        ));
    }

    Ok(Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// These tests set a process-wide variable, so they run behind one mutex
    /// rather than racing each other across the test harness's threads.
    static GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn with_key<T>(body: impl FnOnce() -> T) -> T {
        let _lock = GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let key = base64::engine::general_purpose::STANDARD.encode([7u8; 32]);
        std::env::set_var("API0_ENCRYPTION_KEY", key);
        let out = body();
        std::env::remove_var("API0_ENCRYPTION_KEY");
        out
    }

    fn ctx<'a>(tenant: &'a str, purpose: &'a str) -> SecretContext<'a> {
        SecretContext { tenant_id: tenant, purpose }
    }

    #[test]
    fn a_sealed_secret_opens_again() {
        with_key(|| {
            let sealed = seal("pat-abc123", &ctx("tenant-1", "pat")).unwrap();
            assert_eq!(open(&sealed, &ctx("tenant-1", "pat")).unwrap(), "pat-abc123");
        });
    }

    #[test]
    fn the_ciphertext_does_not_contain_the_plaintext() {
        with_key(|| {
            let sealed = seal("pat-abc123", &ctx("tenant-1", "pat")).unwrap();
            assert!(!String::from_utf8_lossy(&sealed).contains("pat-abc123"));
            assert_eq!(sealed[0], VERSION);
        });
    }

    #[test]
    fn sealing_twice_gives_different_bytes() {
        // A fixed nonce would make equal secrets visibly equal in the database.
        with_key(|| {
            let a = seal("same", &ctx("t", "pat")).unwrap();
            let b = seal("same", &ctx("t", "pat")).unwrap();
            assert_ne!(a, b);
        });
    }

    #[test]
    fn a_record_will_not_open_for_another_tenant_or_purpose() {
        with_key(|| {
            let sealed = seal("pat-abc123", &ctx("tenant-1", "pat")).unwrap();
            assert!(open(&sealed, &ctx("tenant-2", "pat")).is_err());
            assert!(open(&sealed, &ctx("tenant-1", "entra_refresh")).is_err());
        });
    }

    #[test]
    fn tampering_is_detected() {
        with_key(|| {
            let mut sealed = seal("pat-abc123", &ctx("t", "pat")).unwrap();
            let last = sealed.len() - 1;
            sealed[last] ^= 0x01;
            assert!(open(&sealed, &ctx("t", "pat")).is_err());
        });
    }

    #[test]
    fn without_a_key_nothing_seals() {
        let _lock = GUARD.lock().unwrap_or_else(|e| e.into_inner());
        std::env::remove_var("API0_ENCRYPTION_KEY");
        assert!(!is_configured());
        assert!(seal("x", &ctx("t", "pat")).is_err());
    }
}
