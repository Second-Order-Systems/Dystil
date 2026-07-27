use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};

pub(crate) fn parse_sqlite_timestamp(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .or_else(|_| {
            chrono::NaiveDateTime::parse_from_str(
                value.trim_end_matches(" UTC"),
                "%Y-%m-%d %H:%M:%S%.f",
            )
            .map(|value| value.and_utc())
        })
        .unwrap_or(DateTime::UNIX_EPOCH)
}

pub(crate) fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex_string(hasher.finalize().as_slice())
}

pub(crate) fn hex_string(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push_str(&format!("{:02x}", byte));
    }
    output
}
