//! Serde helpers for shapes JSON does not natively support.
//!
//! Two problems recur throughout this workspace, and both fail at *runtime*
//! rather than at compile time, which is the worst place to discover them.
//!
//! # Tuple-keyed maps
//!
//! `BTreeMap<(usize, usize), V>` is the natural type for "something per cell of
//! the state matrix". `serde_json` derives `Serialize` for it happily and then
//! fails when asked to write it, because a JSON object key must be a string.
//! The code compiles, the tests that never serialise pass, and the first
//! attempt to persist a model fails with `key must be a string`.
//!
//! [`cell_map`] stores such a map as an array of `[row, col, value]` triples.
//!
//! # Non-finite floats
//!
//! `NaN` means "unobserved" throughout the mirror, and infinities appear in
//! prediction intervals that express ignorance. JSON has neither. `serde_json`
//! writes them as `null` and then refuses to read `null` back into an `f64`, so
//! a document round-trips only until the first hole appears in it.
//!
//! [`maybe_finite`] maps non-finite values to `null` and back, symmetrically.

use std::collections::BTreeMap;

use serde::de::Deserializer;
use serde::ser::Serializer;
use serde::{Deserialize, Serialize};

/// A `BTreeMap` keyed by `(row, col)`, stored as `[row, col, value]` triples.
pub mod cell_map {
    use super::*;

    pub fn serialize<S, V>(
        map: &BTreeMap<(usize, usize), V>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
        V: Serialize,
    {
        let entries: Vec<(usize, usize, &V)> = map
            .iter()
            .map(|((row, col), value)| (*row, *col, value))
            .collect();
        entries.serialize(serializer)
    }

    pub fn deserialize<'de, D, V>(deserializer: D) -> Result<BTreeMap<(usize, usize), V>, D::Error>
    where
        D: Deserializer<'de>,
        V: Deserialize<'de>,
    {
        let entries: Vec<(usize, usize, V)> = Vec::deserialize(deserializer)?;
        Ok(entries
            .into_iter()
            .map(|(row, col, value)| ((row, col), value))
            .collect())
    }
}

/// Any `BTreeMap` whose key JSON cannot express, stored as an array of
/// `[key, value]` pairs.
///
/// The general case of [`cell_map`]. A struct or tuple key derives
/// `Serialize` happily and then fails at runtime with "key must be a string",
/// which is the kind of defect that surfaces the first time something tries to
/// persist a model rather than when it is written.
pub mod pairs {
    use super::*;

    pub fn serialize<S, K, V>(map: &BTreeMap<K, V>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
        K: Serialize,
        V: Serialize,
    {
        let entries: Vec<(&K, &V)> = map.iter().collect();
        entries.serialize(serializer)
    }

    pub fn deserialize<'de, D, K, V>(deserializer: D) -> Result<BTreeMap<K, V>, D::Error>
    where
        D: Deserializer<'de>,
        K: Deserialize<'de> + Ord,
        V: Deserialize<'de>,
    {
        let entries: Vec<(K, V)> = Vec::deserialize(deserializer)?;
        Ok(entries.into_iter().collect())
    }
}

/// An `f64` that may be `NaN` or infinite, stored as `null` when it is not a
/// finite number.
///
/// Deserialising `null` yields `NaN` rather than an infinity: "unobserved" is
/// by far the commonest reason a value is missing, and `NaN` propagates through
/// arithmetic instead of silently dominating it the way an infinity does.
pub mod maybe_finite {
    use super::*;

    pub fn serialize<S: Serializer>(value: &f64, serializer: S) -> Result<S::Ok, S::Error> {
        if value.is_finite() {
            serializer.serialize_some(value)
        } else {
            serializer.serialize_none()
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<f64, D::Error> {
        let value = Option::<f64>::deserialize(deserializer)?;
        Ok(value.unwrap_or(f64::NAN))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Holder {
        #[serde(with = "cell_map")]
        cells: BTreeMap<(usize, usize), f64>,
        #[serde(with = "maybe_finite")]
        value: f64,
    }

    #[test]
    fn a_tuple_keyed_map_round_trips_through_json() {
        // Without the helper this fails at runtime with "key must be a string",
        // which is exactly the kind of bug that only appears once something
        // tries to persist a model.
        let holder = Holder {
            cells: [((0, 1), 5.0), ((2, 3), 7.5)].into_iter().collect(),
            value: 1.0,
        };
        let json = serde_json::to_string(&holder).expect("serialise");
        let back: Holder = serde_json::from_str(&json).expect("deserialise");
        assert_eq!(holder, back);
        assert_eq!(back.cells[&(2, 3)], 7.5);
    }

    #[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
    struct StructKey {
        family: String,
        target: Option<usize>,
    }

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct PairHolder {
        #[serde(with = "pairs")]
        map: BTreeMap<StructKey, u32>,
    }

    #[test]
    fn a_struct_keyed_map_round_trips_through_json() {
        let holder = PairHolder {
            map: [
                (
                    StructKey {
                        family: "affinity".into(),
                        target: Some(2),
                    },
                    7,
                ),
                (
                    StructKey {
                        family: "priority".into(),
                        target: None,
                    },
                    1,
                ),
            ]
            .into_iter()
            .collect(),
        };
        let json = serde_json::to_string(&holder).expect("serialise");
        let back: PairHolder = serde_json::from_str(&json).expect("deserialise");
        assert_eq!(holder, back);
    }

    #[test]
    fn an_empty_map_round_trips() {
        let holder = Holder {
            cells: BTreeMap::new(),
            value: 0.0,
        };
        let json = serde_json::to_string(&holder).unwrap();
        let back: Holder = serde_json::from_str(&json).unwrap();
        assert!(back.cells.is_empty());
    }

    #[test]
    fn non_finite_values_survive_as_null() {
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let holder = Holder {
                cells: BTreeMap::new(),
                value,
            };
            let json = serde_json::to_string(&holder).unwrap();
            assert!(json.contains("null"), "expected null in {json}");
            let back: Holder = serde_json::from_str(&json).unwrap();
            assert!(
                back.value.is_nan(),
                "a non-finite value comes back as unobserved, not as an infinity"
            );
        }
    }

    #[test]
    fn finite_values_are_unchanged() {
        let holder = Holder {
            cells: BTreeMap::new(),
            value: -2.5,
        };
        let json = serde_json::to_string(&holder).unwrap();
        let back: Holder = serde_json::from_str(&json).unwrap();
        assert_eq!(back.value, -2.5);
    }
}
