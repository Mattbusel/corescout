//! Serde helpers mapping `NaN` to JSON `null`.
//!
//! # Why this exists
//!
//! The mirror uses `NaN` to mean "not observed", and that distinction is load
//! bearing: an unobserved cell must never be confused with a zero. JSON has no
//! `NaN`. `serde_json` writes one as `null` and then refuses to read `null` back
//! into an `f64`, so a snapshot containing any unobserved cell would serialise
//! successfully and fail to deserialise. That is the worst possible failure
//! mode, because it only appears once a real machine produces a real hole.
//!
//! These helpers make the mapping explicit and symmetric: `NaN` becomes `null`,
//! `null` becomes `NaN`, and a JSON snapshot round trips.
//!
//! The binary self-state plane has no such problem. It stores raw IEEE-754 bit
//! patterns, so `NaN` survives untouched, which is one more reason the binary
//! form is canonical and JSON is a projection.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::relation::RELATION_ATTRS;

/// `Vec<f64>` where `NaN` is written as `null`.
pub mod vec {
    use super::*;

    pub fn serialize<S: Serializer>(values: &[f64], serializer: S) -> Result<S::Ok, S::Error> {
        let mapped: Vec<Option<f64>> = values
            .iter()
            .map(|v| if v.is_nan() { None } else { Some(*v) })
            .collect();
        mapped.serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<f64>, D::Error> {
        let mapped = Vec::<Option<f64>>::deserialize(deserializer)?;
        Ok(mapped.into_iter().map(|v| v.unwrap_or(f64::NAN)).collect())
    }
}

/// `[f64; RELATION_ATTRS]` where `NaN` is written as `null`.
pub mod attrs {
    use super::*;

    pub fn serialize<S: Serializer>(
        values: &[f64; RELATION_ATTRS],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let mapped: Vec<Option<f64>> = values
            .iter()
            .map(|v| if v.is_nan() { None } else { Some(*v) })
            .collect();
        mapped.serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<[f64; RELATION_ATTRS], D::Error> {
        let mapped = Vec::<Option<f64>>::deserialize(deserializer)?;
        let mut out = [f64::NAN; RELATION_ATTRS];
        for (slot, value) in out.iter_mut().zip(mapped) {
            *slot = value.unwrap_or(f64::NAN);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Serialize, Deserialize)]
    struct Holder {
        #[serde(with = "vec")]
        values: Vec<f64>,
        #[serde(with = "attrs")]
        attributes: [f64; RELATION_ATTRS],
    }

    #[test]
    fn unobserved_values_survive_a_json_round_trip() {
        let holder = Holder {
            values: vec![1.0, f64::NAN, 3.0],
            attributes: [f64::NAN, 2.0, f64::NAN, f64::NAN],
        };
        let json = serde_json::to_string(&holder).unwrap();
        assert!(
            json.contains("null"),
            "NaN should be written as null: {json}"
        );

        let back: Holder = serde_json::from_str(&json).unwrap();
        assert_eq!(back.values[0], 1.0);
        assert!(back.values[1].is_nan(), "null must come back as unobserved");
        assert_eq!(back.values[2], 3.0);
        assert!(back.attributes[0].is_nan());
        assert_eq!(back.attributes[1], 2.0);
    }

    #[test]
    fn a_short_attribute_array_fills_the_rest_with_unobserved() {
        // Forward compatibility: a plane written with fewer attribute slots
        // must not fail to parse.
        let back: [f64; RELATION_ATTRS] =
            attrs::deserialize(&mut serde_json::Deserializer::from_str("[1.0]")).unwrap();
        assert_eq!(back[0], 1.0);
        assert!(back[1..].iter().all(|v| v.is_nan()));
    }
}
