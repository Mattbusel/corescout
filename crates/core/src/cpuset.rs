//! CPU sets and CPU-list parsing.
//!
//! The `0-3,8,10-11` list format used here is the one the Linux kernel uses
//! throughout sysfs (`thread_siblings_list`, `shared_cpu_list`, `online`, ...)
//! and the one `taskset -c` accepts, so it doubles as our user-facing syntax.
//! Parsing and formatting are pure string work with no platform dependency,
//! which is why they live above `corescout_substrate::platform`.

use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::LogicalId;

/// An ordered, deduplicated set of logical CPU ids.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CpuSet {
    cpus: BTreeSet<LogicalId>,
}

impl CpuSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, cpu: LogicalId) -> &mut Self {
        self.cpus.insert(cpu);
        self
    }

    pub fn contains(&self, cpu: LogicalId) -> bool {
        self.cpus.contains(&cpu)
    }

    pub fn len(&self) -> usize {
        self.cpus.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cpus.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = LogicalId> + '_ {
        self.cpus.iter().copied()
    }

    pub fn to_vec(&self) -> Vec<LogicalId> {
        self.cpus.iter().copied().collect()
    }

    /// Highest CPU id in the set, used to size kernel bitmaps.
    pub fn max(&self) -> Option<LogicalId> {
        self.cpus.iter().next_back().copied()
    }

    /// Parse a kernel-style CPU list: `"0-3,8,10-11"`. An empty or
    /// whitespace-only string yields an empty set, which is what sysfs writes
    /// for e.g. `offline` on a machine with every CPU online.
    pub fn parse_list(s: &str) -> Result<Self> {
        let mut set = CpuSet::new();
        let s = s.trim();
        if s.is_empty() {
            return Ok(set);
        }
        for part in s.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            // Ranges may carry a stride suffix (`0-7:2/4`) in some sysfs
            // files. We only need the endpoints; the stride form is rejected
            // rather than silently misparsed.
            if part.contains(':') {
                return Err(Error::invalid(format!(
                    "strided CPU range `{part}` is not supported"
                )));
            }
            match part.split_once('-') {
                Some((lo, hi)) => {
                    let lo: LogicalId = parse_id(lo)?;
                    let hi: LogicalId = parse_id(hi)?;
                    if hi < lo {
                        return Err(Error::invalid(format!("inverted CPU range `{part}`")));
                    }
                    for cpu in lo..=hi {
                        set.insert(cpu);
                    }
                }
                None => {
                    set.insert(parse_id(part)?);
                }
            }
        }
        Ok(set)
    }

    /// Render as a compact kernel-style list, collapsing runs into ranges.
    pub fn to_list(&self) -> String {
        let cpus = self.to_vec();
        format_list(&cpus)
    }
}

impl FromIterator<LogicalId> for CpuSet {
    fn from_iter<I: IntoIterator<Item = LogicalId>>(iter: I) -> Self {
        CpuSet {
            cpus: iter.into_iter().collect(),
        }
    }
}

impl From<&[LogicalId]> for CpuSet {
    fn from(v: &[LogicalId]) -> Self {
        v.iter().copied().collect()
    }
}

impl fmt::Display for CpuSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_list())
    }
}

fn parse_id(s: &str) -> Result<LogicalId> {
    s.trim()
        .parse::<LogicalId>()
        .map_err(|_| Error::invalid(format!("`{s}` is not a CPU number")))
}

/// Collapse a sorted-or-unsorted slice of CPU ids into `0-3,8` form.
pub fn format_list(cpus: &[LogicalId]) -> String {
    let mut cpus: Vec<LogicalId> = cpus.to_vec();
    cpus.sort_unstable();
    cpus.dedup();
    let mut out = String::new();
    let mut i = 0;
    while i < cpus.len() {
        let start = cpus[i];
        let mut end = start;
        while i + 1 < cpus.len() && cpus[i + 1] == end + 1 {
            i += 1;
            end = cpus[i];
        }
        if !out.is_empty() {
            out.push(',');
        }
        if start == end {
            out.push_str(&start.to_string());
        } else {
            out.push_str(&format!("{start}-{end}"));
        }
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_singletons_ranges_and_mixtures() {
        assert_eq!(CpuSet::parse_list("0").unwrap().to_vec(), vec![0]);
        assert_eq!(
            CpuSet::parse_list("0-3").unwrap().to_vec(),
            vec![0, 1, 2, 3]
        );
        assert_eq!(
            CpuSet::parse_list("0-3,8,10-11").unwrap().to_vec(),
            vec![0, 1, 2, 3, 8, 10, 11]
        );
    }

    #[test]
    fn tolerates_whitespace_and_trailing_newline() {
        // sysfs reads come back with a trailing newline.
        assert_eq!(CpuSet::parse_list(" 0-1 \n").unwrap().to_vec(), vec![0, 1]);
    }

    #[test]
    fn empty_string_is_an_empty_set_not_an_error() {
        assert!(CpuSet::parse_list("").unwrap().is_empty());
        assert!(CpuSet::parse_list("\n").unwrap().is_empty());
    }

    #[test]
    fn rejects_garbage_and_inverted_ranges() {
        assert!(CpuSet::parse_list("abc").is_err());
        assert!(CpuSet::parse_list("5-1").is_err());
        assert!(CpuSet::parse_list("0-7:2/4").is_err());
    }

    #[test]
    fn deduplicates_and_orders() {
        let set = CpuSet::parse_list("3,1,1,2").unwrap();
        assert_eq!(set.to_vec(), vec![1, 2, 3]);
    }

    #[test]
    fn formatting_collapses_runs() {
        assert_eq!(format_list(&[0, 1, 2, 3, 8, 10, 11]), "0-3,8,10-11");
        assert_eq!(format_list(&[]), "");
        assert_eq!(format_list(&[7]), "7");
        assert_eq!(format_list(&[5, 3, 4]), "3-5");
    }

    #[test]
    fn parse_and_format_round_trip() {
        for s in ["0", "0-3", "0-3,8,10-11", "1,3,5,7"] {
            assert_eq!(CpuSet::parse_list(s).unwrap().to_list(), s);
        }
    }
}
