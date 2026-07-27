//! Stub secrets module — vault encryption is excluded from the Dystil product.
//!
//! All keychain operations return `NotFound`/`false`; `is_encryption_enabled`
//! always returns `false` so `decrypt_store_file` and `encrypt_store_file` skip
//! their crypto paths without touching any dystil crate.

#[derive(Debug)]
pub enum KeyResult {
    NotFound,
}

pub fn is_encryption_enabled() -> bool {
    false
}

pub fn get_key_if_encryption_enabled() -> KeyResult {
    KeyResult::NotFound
}

pub fn get_key() -> KeyResult {
    KeyResult::NotFound
}
