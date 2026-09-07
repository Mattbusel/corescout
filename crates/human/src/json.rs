//! JSON projections of the mirror.
//!
//! A *projection*, never the canonical form. The canonical representation is
//! the binary state matrix in the self-state plane; this exists so a person, or
//! a script that does not want to map shared memory, can look at one
//! reflection. Unobserved cells become `null`, which is what `NaN` means here.
//!
//! Making the text format authoritative is what the mirror refactor moved away
//! from, and nothing here should be read back in as truth.
//!
//! The renderers for topology, benchmarks and rankings live in
//! `corescout_report`, because they need the crates that reach hardware and
//! this one must not: everything that consumes only the mirror links this
//! crate, and would inherit a path to `/sys` along with it.

use corescout_core::error::{Error, Result};
use serde::Serialize;

/// One reflection as JSON.
pub fn snapshot(snapshot: &corescout_mirror::MirrorSnapshot) -> Result<String> {
    encode(snapshot, "mirror snapshot")
}

/// Serialise, turning a failure into our own error type with the subject named.
///
/// Worth a helper because "key must be a string" is a real failure mode here
/// (several models key maps by tuples) and a bare serde message gives no clue
/// which structure produced it.
pub(crate) fn encode<T: Serialize>(value: &T, what: &str) -> Result<String> {
    serde_json::to_string_pretty(value)
        .map_err(|error| Error::invalid(format!("could not serialise the {what}: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use corescout_mirror::test_support::fixture;

    #[test]
    fn a_snapshot_projects_to_parseable_json() {
        let text = snapshot(&fixture()).expect("serialises");
        let value: serde_json::Value = serde_json::from_str(&text).expect("parses");
        assert!(value["entities"].as_array().unwrap().len() >= 2);
        assert!(value["channels"].as_array().is_some());
        assert_eq!(value["format_version"], corescout_mirror::FORMAT_VERSION);
    }

    #[test]
    fn an_unobserved_cell_becomes_null_rather_than_zero() {
        // The whole point of the projection: a gap must stay a gap. Zero would
        // read as a measurement.
        let mut snap = fixture();
        snap.state.set(0, 0, f64::NAN);
        let text = snapshot(&snap).expect("serialises");
        let value: serde_json::Value = serde_json::from_str(&text).expect("parses");
        let cells = value["state"]["values"]
            .as_array()
            .expect("the state matrix is an array");
        assert!(cells[0].is_null(), "an unobserved cell must be null");
    }

    #[test]
    fn availability_reasons_survive_the_projection_as_codes() {
        // A consumer reading JSON must still be able to tell "permission
        // denied" from "not applicable", and it can: the reason travels as its
        // numeric code rather than its name.
        //
        // Codes rather than strings because this same representation is what a
        // recording writes for every frame, and a machine-readable trace should
        // not spend a dozen bytes per cell on a word nothing parses. The names
        // are one `Availability::from_u8` away, and `mirror-inspect` prints
        // them.
        let snap = fixture();
        let text = snapshot(&snap).expect("serialises");
        let value: serde_json::Value = serde_json::from_str(&text).expect("parses");
        let codes = value["availability"]["codes"]
            .as_array()
            .expect("the availability matrix travels with the state");
        assert_eq!(codes.len(), snap.entities.len() * snap.channels.len());
        assert!(
            codes.iter().any(|code| code.as_u64()
                == Some(corescout_mirror::schema::Availability::PermissionDenied.as_u8() as u64)),
            "the fixture's privilege gap did not reach the JSON"
        );
    }

    #[test]
    fn a_projected_snapshot_reads_back_identically() {
        // The projection is not canonical, but it is what a recording holds, so
        // it has to survive the round trip exactly.
        let snap = fixture();
        let text = snapshot(&snap).expect("serialises");
        let back: corescout_mirror::MirrorSnapshot =
            serde_json::from_str(&text).expect("deserialises");
        assert_eq!(back, snap);
    }
}
