//! The platform keychain behind [`Secrets`](crate::Secrets): the `keyring`
//! crate's one store per operating system, Secret Service on Linux (over
//! D-Bus through zbus), Keychain Services on macOS and the Credential
//! Manager on Windows. Every secret is an entry of the service `rick`
//! whose user name is the caller's key.
//!
//! The calls block on the keychain; callers run them on the blocking pool.

use types::Error;

/// The service every entry is filed under.
pub const SERVICE: &str = "rick";

/// The keychain entry for `key`.
fn entry(key: &str) -> Result<keyring::Entry, Error> {
    keyring::Entry::new(SERVICE, key).map_err(map)
}

/// Stores `secret` under `key`.
///
/// # Errors
///
/// `Error::Unsupported` when no keychain is reachable; otherwise the
/// platform error as `Error::Io`.
pub fn store(key: &str, secret: &str) -> Result<(), Error> {
    entry(key)?.set_password(secret).map_err(map)
}

/// The secret under `key`, or `None` when there is none.
///
/// # Errors
///
/// `Error::Unsupported` when no keychain is reachable; otherwise the
/// platform error as `Error::Io`.
pub fn get(key: &str) -> Result<Option<String>, Error> {
    match entry(key)?.get_password() {
        Ok(secret) => Ok(Some(secret)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(map(error)),
    }
}

/// Forgets the secret under `key`; nothing to forget is fine.
///
/// # Errors
///
/// `Error::Unsupported` when no keychain is reachable; otherwise the
/// platform error as `Error::Io`.
pub fn remove(key: &str) -> Result<(), Error> {
    match entry(key)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(map(error)),
    }
}

/// The kernel error for a keyring error: an unreachable or absent store is
/// `Unsupported`, the rest I/O errors with the store's own message.
fn map(error: keyring::Error) -> Error {
    match error {
        keyring::Error::NoDefaultStore | keyring::Error::NoStorageAccess(_) => {
            Error::Unsupported("no keychain is reachable on this system")
        }
        keyring::Error::NoEntry => Error::NotFound,
        other => Error::Io(std::io::Error::other(other.to_string())),
    }
}
