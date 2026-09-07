//! Schema versioning.
//!
//! # The rule
//!
//! An older binary opening a newer database refuses, loudly, and says which
//! version it found. It does not try to read it. The alternative is a build
//! that silently ignores fields it does not understand and then writes the
//! record back without them, which destroys data belonging to a version the
//! user may go back to.
//!
//! A newer binary opening an older database runs the migrations between them
//! in order and stamps the new version.

use corescout_core::error::{Error, Result};

/// The schema this build writes.
///
/// Bump this and add a [`Migration`] whenever the meaning of stored bytes
/// changes. Adding an optional field to a document body is not a schema
/// change: document bodies are JSON and tolerate it.
pub const SCHEMA_VERSION: u32 = 1;

/// One step from `from` to `from + 1`.
pub struct Migration {
    /// The version this migration upgrades from.
    pub from: u32,
    /// What it does, for the log.
    pub describe: &'static str,
}

/// Every migration this build knows, in order.
///
/// Version 1 is the first shipped schema, so there is nothing to migrate from
/// yet. The machinery exists now because adding it after the first release is
/// how databases get abandoned.
pub fn migrations() -> Vec<Migration> {
    Vec::new()
}

/// Decide what to do with a database stamped `found`.
pub fn plan(found: u32) -> Result<Vec<&'static str>> {
    if found > SCHEMA_VERSION {
        return Err(Error::invalid(format!(
            "this data was written by a newer CoreScout (schema {found}, this build reads \
             {SCHEMA_VERSION}). Update CoreScout, or move {} aside to start fresh.",
            crate::paths::database().display()
        )));
    }
    let steps = migrations()
        .into_iter()
        .filter(|migration| migration.from >= found && migration.from < SCHEMA_VERSION)
        .map(|migration| migration.describe)
        .collect();
    Ok(steps)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_current_version_needs_no_migration() {
        assert_eq!(plan(SCHEMA_VERSION).expect("a plan"), Vec::<&str>::new());
    }

    #[test]
    fn a_newer_database_is_refused_and_says_what_to_do() {
        let error = plan(SCHEMA_VERSION + 1)
            .expect_err("should refuse")
            .to_string();
        assert!(error.contains("newer CoreScout"), "{error}");
        assert!(error.contains("Update CoreScout"), "{error}");
    }

    #[test]
    fn every_migration_covers_exactly_one_step_and_they_chain() {
        // A gap means an old database upgrades to a version that never
        // existed; an overlap means a step runs twice.
        let mut steps = migrations();
        steps.sort_by_key(|migration| migration.from);
        for (index, migration) in steps.iter().enumerate() {
            assert_eq!(
                migration.from,
                index as u32 + 1,
                "migrations must start at 1 and not skip"
            );
        }
        assert_eq!(
            steps.len() as u32 + 1,
            SCHEMA_VERSION,
            "SCHEMA_VERSION was bumped without adding the migration for it"
        );
    }
}
