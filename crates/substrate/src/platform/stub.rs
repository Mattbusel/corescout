//! Fallback backend for platforms CoreScout does not support yet.
//!
//! It exists so the crate builds and its platform-neutral logic (CPU set
//! parsing, statistics, ranking, output formatting) stays unit-testable on any
//! developer machine, including the Windows and macOS boxes where the Linux
//! backend cannot even be compiled.

use std::process::{Child, Command};

use crate::platform::{Platform, SwitchCounters};
use crate::topology::{LogicalId, Topology};
use corescout_core::cpuset::CpuSet;
use corescout_core::error::{Error, Result};

/// A [`Platform`] whose every operation reports "unsupported".
pub struct StubPlatform;

impl StubPlatform {
    pub fn new() -> Self {
        StubPlatform
    }
}

impl Default for StubPlatform {
    fn default() -> Self {
        Self::new()
    }
}

fn unsupported<T>(what: &str) -> Result<T> {
    Err(Error::unsupported(format!(
        "{what} (CoreScout currently implements Linux only; \
         see platform/mod.rs for the Windows porting notes)"
    )))
}

impl Platform for StubPlatform {
    fn name(&self) -> &'static str {
        "unsupported"
    }

    fn discover_topology(&self) -> Result<Topology> {
        unsupported("topology discovery")
    }

    fn pin_current_thread(&self, _cpu: LogicalId) -> Result<()> {
        unsupported("thread pinning")
    }

    fn set_current_thread_affinity(&self, _cpus: &CpuSet) -> Result<()> {
        unsupported("thread affinity")
    }

    fn current_thread_affinity(&self) -> Result<CpuSet> {
        unsupported("thread affinity")
    }

    fn process_affinity(&self) -> Result<CpuSet> {
        unsupported("process affinity")
    }

    fn thread_switch_counters(&self) -> Option<SwitchCounters> {
        None
    }

    fn spawn_with_affinity(&self, _command: &mut Command, _cpus: &CpuSet) -> Result<Child> {
        unsupported("affinity-constrained process launch")
    }
}
