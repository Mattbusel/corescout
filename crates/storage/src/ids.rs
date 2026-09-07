//! Identifiers that sort by creation time and do not need a dependency.
//!
//! A UUID crate would do, but every dependency in this workspace has to earn
//! itself, and what is actually needed here is narrower than a UUID: an
//! identifier that is unique on one machine, sorts in creation order so a range
//! scan of the document store reads chronologically, and is short enough to
//! paste into a support message.

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// A monotonic counter, so two ids minted in the same millisecond differ.
static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A locally unique, time-ordered identifier.
///
/// Rendered as 27 lowercase base32 characters: 48 bits of millisecond
/// timestamp, 16 bits of in-process counter, 64 bits of process-seeded noise.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Id(String);

const ALPHABET: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";

impl Id {
    /// Mint a fresh identifier.
    pub fn new() -> Id {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
            & 0xffff_ffff_ffff;
        let count = COUNTER.fetch_add(1, Ordering::Relaxed) & 0xffff;
        let noise = splitmix(millis ^ (count << 24) ^ seed());
        let mut out = String::with_capacity(27);
        encode(millis, 48, &mut out);
        encode(count, 16, &mut out);
        encode(noise, 64, &mut out);
        Id(out)
    }

    /// Wrap an identifier that came from somewhere else.
    ///
    /// Used when reading back from storage or accepting an id an agent quoted.
    /// No validation: an id from a caller is a key, not a claim.
    pub fn from_raw(raw: impl Into<String>) -> Id {
        Id(raw.into())
    }

    /// The identifier as a string.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// A short prefix, for display where the full id is noise.
    pub fn short(&self) -> &str {
        let end = self.0.len().min(8);
        &self.0[..end]
    }
}

impl Default for Id {
    fn default() -> Id {
        Id::new()
    }
}

impl fmt::Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Id({})", self.0)
    }
}

impl From<&str> for Id {
    fn from(raw: &str) -> Id {
        Id::from_raw(raw)
    }
}

impl From<String> for Id {
    fn from(raw: String) -> Id {
        Id::from_raw(raw)
    }
}

/// Append `bits` bits of `value`, most significant first, five at a time.
fn encode(value: u64, bits: u32, out: &mut String) {
    let mut remaining = bits;
    while remaining > 0 {
        let take = remaining.min(5);
        remaining -= take;
        let chunk = (value >> remaining) & ((1u64 << take) - 1);
        out.push(ALPHABET[chunk as usize % 32] as char);
    }
}

/// Per-process entropy without a random number generator.
///
/// The address of a static and the process id are not cryptographic, and this
/// is not a cryptographic identifier: it exists so two CoreScout processes on
/// one machine do not mint colliding ids in the same millisecond.
fn seed() -> u64 {
    static ANCHOR: AtomicU64 = AtomicU64::new(0);
    let address = &ANCHOR as *const _ as u64;
    address ^ (std::process::id() as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
}

fn splitmix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn ids_are_unique_within_a_process() {
        let minted: BTreeSet<Id> = (0..10_000).map(|_| Id::new()).collect();
        assert_eq!(minted.len(), 10_000);
    }

    #[test]
    fn ids_sort_in_creation_order() {
        // A range scan of the document store has to read chronologically, so
        // this is a storage property rather than a cosmetic one.
        let first = Id::new();
        std::thread::sleep(std::time::Duration::from_millis(3));
        let second = Id::new();
        assert!(first < second, "{first} should sort before {second}");
    }

    #[test]
    fn ids_are_a_fixed_width_of_lowercase_base32() {
        let id = Id::new();
        assert_eq!(id.as_str().len(), 27);
        assert!(id.as_str().bytes().all(|b| ALPHABET.contains(&b)));
        assert_eq!(id.short().len(), 8);
    }

    #[test]
    fn an_id_from_storage_round_trips() {
        let id = Id::new();
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, format!("\"{id}\""));
        let back: Id = serde_json::from_str(&json).unwrap();
        assert_eq!(back, id);
    }
}
