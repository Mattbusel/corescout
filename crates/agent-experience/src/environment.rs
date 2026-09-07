//! Failures caused by the machine rather than by the operation.
//!
//! # Why these do not wait for a pattern
//!
//! Everything else in this crate is deliberately slow to believe: an operation
//! must fail several times before CoreScout will call it a failure mode, and a
//! procedure must survive randomised trials before it is offered. That bar is
//! right for claims about *tendencies*. Builds fail here more often than they
//! succeed is a statistical claim and needs samples.
//!
//! This is a different kind of claim. When `CARGO_HOME` points at a drive that
//! is not mounted, cargo does not fail more often; it fails every time, for
//! every subcommand, until the variable or the drive changes. There is no
//! distribution to sample. Waiting for five attempts before mentioning it means
//! the user pays for the same diagnosis five times, and the fifth is no better
//! evidenced than the first.
//!
//! So the evidence here is not repetition. It is that the missing thing is
//! still missing, which CoreScout can check directly whenever it is asked.
//! [`Environment::resolve`] is that check, and a fault the machine no longer
//! has is forgotten rather than remembered.
//!
//! # Why they are not filed under the operation
//!
//! A failure mode is keyed by the normalised command, which is right when the
//! command is what went wrong. Here the command is the one detail that does not
//! matter: `cargo test` and `cargo build --release` fail identically, and so
//! would anything else that read the same variable. Keying these by the missing
//! path means one observation covers every operation that trips over it, which
//! is the whole point of noticing.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// A path something needed and the machine does not have.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fault {
    /// The missing path, as it appeared in the failure.
    pub path: String,
    /// The environment variable that pointed at it, when the failure named one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variable: Option<String>,
    /// An operation seen to fail this way, for the explanation.
    pub example: String,
    /// How many reports have named it. Not a threshold, only context.
    pub seen: u64,
    /// When it was first reported.
    pub first_ms: u64,
    /// When it was last reported.
    pub last_ms: u64,
    /// What the agent did about it, if the report said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workaround: Option<String>,
}

impl Fault {
    /// A line for an agent about to run into this.
    pub fn headline(&self) -> String {
        match &self.variable {
            Some(variable) => format!(
                "{variable} points at {}, which does not exist on this machine. \
                 Anything that reads it fails here, whatever the arguments.",
                self.path
            ),
            None => format!(
                "{} does not exist on this machine. Operations needing it fail here.",
                self.path
            ),
        }
    }
}

/// Something a failure blamed, before anyone has checked whether it is true.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Blamed {
    /// The path the failure named.
    pub path: String,
    /// The variable that pointed at it, if the text said so.
    pub variable: Option<String>,
}

/// The faults this machine is known to have.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Environment {
    #[serde(default)]
    faults: BTreeMap<String, Fault>,
}

impl Environment {
    /// Record that something was blamed and found missing.
    ///
    /// The caller does the finding: this crate holds no opinion about the
    /// filesystem, and a test that had to create drives would test nothing.
    pub fn record(&mut self, candidate: &Blamed, example: &str, now_ms: u64) -> &Fault {
        let fault = self
            .faults
            .entry(candidate.path.to_lowercase())
            .or_insert_with(|| Fault {
                path: candidate.path.clone(),
                variable: candidate.variable.clone(),
                example: example.to_string(),
                seen: 0,
                first_ms: now_ms,
                last_ms: now_ms,
                workaround: None,
            });
        // A later report may name the variable when the first did not. Keeping
        // the more specific of the two costs nothing and reads better.
        if fault.variable.is_none() {
            fault.variable = candidate.variable.clone();
        }
        fault.seen += 1;
        fault.last_ms = now_ms;
        fault
    }

    /// Note how someone got past it.
    pub fn note_workaround(&mut self, path: &str, workaround: &str) {
        if let Some(fault) = self.faults.get_mut(&path.to_lowercase()) {
            fault.workaround = Some(workaround.to_string());
        }
    }

    /// Every fault held, whether or not it is still true.
    pub fn all(&self) -> impl Iterator<Item = &Fault> {
        self.faults.values()
    }

    /// Drop the ones that are no longer missing.
    ///
    /// Called with a test the caller supplies, so the crate stays pure and the
    /// caller decides what "still missing" means. Returns what was resolved,
    /// because a machine that quietly fixed itself is worth an entry in the
    /// activity log.
    pub fn resolve(&mut self, mut exists: impl FnMut(&str) -> bool) -> Vec<Fault> {
        let resolved: Vec<Fault> = self
            .faults
            .values()
            .filter(|fault| exists(&fault.path))
            .cloned()
            .collect();
        for fault in &resolved {
            self.faults.remove(&fault.path.to_lowercase());
        }
        resolved
    }

    /// How many faults are held.
    pub fn len(&self) -> usize {
        self.faults.len()
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.faults.is_empty()
    }
}

/// Read a failure message for paths it blames.
///
/// Conservative in the same direction as the fingerprinting: it would rather
/// miss a fault than invent one, because the caller is going to go and check
/// the filesystem for whatever comes back, and a wrong guess there is a wasted
/// syscall rather than a wrong claim. What it will not do is return a path that
/// is merely mentioned; the text has to be a failure, which the caller
/// establishes before asking.
pub fn candidates(detail: &str) -> Vec<Blamed> {
    let mut out: Vec<Blamed> = Vec::new();
    let variables = variables_in(detail);

    for path in paths_in(detail) {
        if out.iter().any(|c| c.path.eq_ignore_ascii_case(&path)) {
            continue;
        }
        // A variable claims a path when its value is that path or contains it,
        // so `CARGO_HOME=R:\cargo` claims `R:\cargo` and `R:\`.
        let variable = variables
            .iter()
            .find(|(_, value)| {
                value.eq_ignore_ascii_case(&path)
                    || value.to_lowercase().starts_with(&path.to_lowercase())
            })
            .map(|(name, _)| name.clone());
        out.push(Blamed { path, variable });
    }
    out
}

/// `NAME=value` pairs in the text.
fn variables_in(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for token in text.split(|c: char| c.is_whitespace() || c == ',') {
        let Some((name, value)) = token.split_once('=') else {
            continue;
        };
        let name = name.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '_');
        // Environment variables are conventionally shouted, and requiring that
        // keeps `--flag=value` and `key=1` out of the results.
        if name.len() < 2
            || !name
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        {
            continue;
        }
        let value = value.trim_end_matches(['.', ',', ';', ')', '"', '\'']);
        if value.is_empty() {
            continue;
        }
        out.push((name.to_string(), value.to_string()));
    }
    out
}

/// Absolute paths in the text, Windows or Unix.
fn paths_in(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for token in text.split(|c: char| c.is_whitespace() || c == ',' || c == '"' || c == '\'') {
        // The value side of an assignment is the path, not the whole token.
        let token = token.split_once('=').map_or(token, |(_, value)| value);
        let token = token.trim_end_matches(['.', ';', ':', ')', '`']);
        if token.len() < 3 {
            continue;
        }
        let windows = {
            let bytes = token.as_bytes();
            bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && (bytes[2] == b'\\')
        };
        // A bare Unix path is too common in prose to take, so it has to have a
        // second component: `/tmp/build` counts, `/` and `/tmp` do not.
        let unix = token.starts_with('/') && token[1..].contains('/');
        if windows || unix {
            out.push(token.to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The failure that prompted all of this, verbatim.
    const REAL: &str = "failed to acquire package cache lock: failed to create directory \
                        R:\\cargo. CARGO_HOME=R:\\cargo and CARGO_TARGET_DIR=R:\\cargo-target, \
                        but the R: drive does not exist on this boot.";

    #[test]
    fn the_failure_that_started_this_is_read_correctly() {
        let found = candidates(REAL);
        let cargo_home = found
            .iter()
            .find(|c| c.path.eq_ignore_ascii_case("R:\\cargo"))
            .expect("R:\\cargo");
        assert_eq!(cargo_home.variable.as_deref(), Some("CARGO_HOME"));
        let target = found
            .iter()
            .find(|c| c.path.eq_ignore_ascii_case("R:\\cargo-target"))
            .expect("R:\\cargo-target");
        assert_eq!(target.variable.as_deref(), Some("CARGO_TARGET_DIR"));
    }

    #[test]
    fn prose_without_a_path_blames_nothing() {
        // The expensive mistake here is inventing a fault, so ordinary failure
        // text must come back empty.
        assert!(candidates("the test suite failed: 3 assertions did not hold").is_empty());
        assert!(candidates("connection refused").is_empty());
        assert!(candidates("error[E0308]: mismatched types").is_empty());
    }

    #[test]
    fn a_flag_that_looks_like_a_variable_is_not_one() {
        let found = candidates("ran with --target=/opt/build/out and it failed");
        assert_eq!(found.len(), 1, "the path is still worth checking");
        assert_eq!(found[0].variable, None, "--target is not an env var");
    }

    #[test]
    fn a_unix_path_needs_two_components_to_count() {
        assert!(candidates("failed at / today").is_empty());
        assert_eq!(
            candidates("cannot open /var/lib/thing")[0].path,
            "/var/lib/thing"
        );
    }

    #[test]
    fn the_same_fault_from_two_operations_is_one_fault() {
        let mut environment = Environment::default();
        let found = candidates(REAL);
        environment.record(&found[0], "cargo test", 1);
        environment.record(&found[0], "cargo build", 2);
        assert_eq!(environment.len(), 1, "keyed by the path, not the command");
        let fault = environment.all().next().expect("the fault");
        assert_eq!(fault.seen, 2);
        assert_eq!(fault.example, "cargo test", "the first one seen");
        assert_eq!(fault.last_ms, 2);
    }

    #[test]
    fn a_fault_the_machine_no_longer_has_is_forgotten() {
        // Otherwise CoreScout warns about a drive that came back, which is the
        // failure mode that makes people stop reading warnings.
        let mut environment = Environment::default();
        environment.record(&candidates(REAL)[0], "cargo test", 1);
        let resolved = environment.resolve(|_| true);
        assert_eq!(resolved.len(), 1);
        assert!(environment.is_empty());
    }

    #[test]
    fn a_fault_that_is_still_missing_survives_the_check() {
        let mut environment = Environment::default();
        environment.record(&candidates(REAL)[0], "cargo test", 1);
        assert!(environment.resolve(|_| false).is_empty());
        assert_eq!(environment.len(), 1);
    }

    #[test]
    fn the_headline_names_the_variable_when_there_is_one() {
        let mut environment = Environment::default();
        let fault = environment
            .record(&candidates(REAL)[0], "cargo test", 1)
            .clone();
        let headline = fault.headline();
        assert!(headline.contains("CARGO_HOME"), "{headline}");
        assert!(headline.contains("whatever the arguments"), "{headline}");
    }
}
