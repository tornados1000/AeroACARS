//! OS keyring wrapper for secrets (phpVMS API key, future tokens).
//!
//! Backends used by the `keyring` crate (must be enabled via Cargo features
//! since keyring v3 — see this crate's Cargo.toml in the workspace root):
//!   * Windows: Credential Manager  (feature `windows-native`)
//!   * macOS:   Keychain            (feature `apple-native`)
//!   * Linux:   Secret Service      (not enabled in Phase 1; not a target OS yet)
//!
//! See requirements spec §29 — "API-Key sicher speichern, keine Passwörter im Klartext".

#![allow(dead_code)]

use thiserror::Error;

const SERVICE: &str = "CloudeAcars";

#[derive(Debug, Error)]
pub enum SecretError {
    #[error("keyring error: {0}")]
    Keyring(#[from] keyring::Error),
}

pub fn store_api_key(account: &str, api_key: &str) -> Result<(), SecretError> {
    let entry = keyring::Entry::new(SERVICE, account)?;
    entry.set_password(api_key)?;
    tracing::debug!(service = SERVICE, account, "stored credential");
    // Round-trip self-check: if our backend turned out to be the no-op mock
    // (would happen when no platform feature is enabled), the read-back below
    // would still return Ok with the cached value, so it does not catch
    // mock-vs-real on its own. But it does catch real keyring failures
    // (permission denied, store unavailable) that some backends report
    // asynchronously on read but not on write.
    match entry.get_password() {
        Ok(_) => Ok(()),
        Err(e) => {
            tracing::error!(error = %e, "credential read-back failed after write");
            Err(SecretError::Keyring(e))
        }
    }
}

pub fn load_api_key(account: &str) -> Result<Option<String>, SecretError> {
    let entry = keyring::Entry::new(SERVICE, account)?;
    match entry.get_password() {
        Ok(s) => {
            tracing::debug!(service = SERVICE, account, "loaded credential");
            Ok(Some(s))
        }
        Err(keyring::Error::NoEntry) => {
            tracing::debug!(service = SERVICE, account, "no credential stored");
            Ok(None)
        }
        Err(e) => {
            tracing::error!(error = %e, "credential load failed");
            Err(SecretError::Keyring(e))
        }
    }
}

pub fn delete_api_key(account: &str) -> Result<(), SecretError> {
    let entry = keyring::Entry::new(SERVICE, account)?;
    match entry.delete_credential() {
        Ok(()) => {
            tracing::debug!(service = SERVICE, account, "deleted credential");
            Ok(())
        }
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => {
            tracing::error!(error = %e, "credential delete failed");
            Err(SecretError::Keyring(e))
        }
    }
}
