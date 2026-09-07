//! The topology data model.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub use corescout_core::{LogicalId, PhysicalId};

/// Which class of core this is on a hybrid (big.LITTLE style) processor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoreType {
    /// Performance core: Intel "Core"/P-core, AMD classic Zen core.
    Performance,
    /// Efficiency core: Intel "Atom"/E-core, AMD Zen-c dense core.
    Efficiency,
    /// Non-hybrid part, or a hybrid part whose class we could not determine.
    Unknown,
}

impl CoreType {
    /// One-character tag used in table output.
    pub fn short(&self) -> &'static str {
        match self {
            CoreType::Performance => "P",
            CoreType::Efficiency => "E",
            CoreType::Unknown => "-",
        }
    }
}

/// Level of a cache in the memory hierarchy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheKind {
    L1Data,
    L1Instruction,
    L2Unified,
    L3Unified,
    /// Anything else the kernel reports (L4/eDRAM, unified L1, ...).
    Other,
}

/// One cache instance, plus the set of logical CPUs that share it.
///
/// Sharing is the load-bearing field. Two threads that share an L2 contend for
/// it; two threads that *want* to share data are much cheaper to co-locate
/// under one L3 than to spread across sockets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cache {
    pub level: u8,
    pub kind: CacheKind,
    /// Size in bytes. `None` when the kernel did not expose it.
    pub size_bytes: Option<u64>,
    pub line_size_bytes: Option<u32>,
    pub ways_of_associativity: Option<u32>,
    /// Logical CPUs sharing this exact cache instance, ascending.
    pub shared_cpus: Vec<LogicalId>,
}

/// Per-CPU frequency information, in kHz, as reported by cpufreq.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrequencyInfo {
    pub current_khz: Option<u64>,
    pub min_khz: Option<u64>,
    pub max_khz: Option<u64>,
    /// Base (non-turbo) frequency where the platform exposes it.
    pub base_khz: Option<u64>,
}

/// A manufacturer preferred/favored-core hint.
///
/// Intel exposes Turbo Boost Max 3.0 ordering through ACPI CPPC
/// (`highest_perf`); AMD's Preferred Core works the same way. Both are
/// *factory bin-sort* results: they describe the silicon as it left the fab,
/// not how the core behaves in this chassis, at this ambient temperature, with
/// this interrupt routing and this neighbour thread. CoreScout records the hint
/// precisely so it can tell you when measurement disagrees with it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FavoredHint {
    /// Raw ACPI CPPC "highest performance" value. Higher means the firmware
    /// believes this core boosts further.
    pub highest_perf: u32,
    pub nominal_perf: Option<u32>,
    /// Rank among all CPUs by `highest_perf`, 1 = the firmware's favourite.
    pub rank: u32,
    /// Where the value came from, e.g. `acpi_cppc/highest_perf`.
    pub source: String,
}

/// A single logical CPU (an SMT thread, or a whole core on a non-SMT part).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogicalCpu {
    pub id: LogicalId,
    /// Synthetic machine-wide physical core id.
    pub physical: PhysicalId,
    /// Socket / package this CPU lives in.
    pub package_id: u32,
    /// Kernel `core_id` (unique only within a package).
    pub core_id: u32,
    /// NUMA node, when the kernel exposes NUMA at all.
    pub numa_node: Option<u32>,
    /// Other logical CPUs on the same physical core, excluding `self.id`.
    pub smt_siblings: Vec<LogicalId>,
    pub core_type: CoreType,
    pub online: bool,
    pub frequency: FrequencyInfo,
    /// Manufacturer "this is one of the good ones" hint, if the OS exposes it.
    pub favored: Option<FavoredHint>,
}

impl LogicalCpu {
    /// True when this CPU shares its physical core with another logical CPU.
    pub fn is_smt(&self) -> bool {
        !self.smt_siblings.is_empty()
    }
}

/// A physical core: one or more logical CPUs sharing execution resources.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhysicalCore {
    pub id: PhysicalId,
    pub package_id: u32,
    pub core_id: u32,
    pub numa_node: Option<u32>,
    pub core_type: CoreType,
    /// Logical CPUs belonging to this core, ascending. The first entry is the
    /// one CoreScout benchmarks on.
    pub logical_cpus: Vec<LogicalId>,
}

impl PhysicalCore {
    /// The logical CPU CoreScout pins to when measuring this core.
    ///
    /// Which sibling we pick is arbitrary but must be *stable*, otherwise
    /// re-running the benchmark silently changes what was measured.
    pub fn primary_cpu(&self) -> LogicalId {
        self.logical_cpus[0]
    }
}

/// A NUMA node and the CPUs attached to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NumaNode {
    pub id: u32,
    pub cpus: Vec<LogicalId>,
    /// Total memory on the node in kB, when available.
    pub memory_kb: Option<u64>,
}

/// Whole-machine CPU description.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Topology {
    pub model_name: String,
    pub vendor: String,
    /// True when the part mixes core types (Intel hybrid, AMD dense/classic).
    pub hybrid: bool,
    pub logical_cpus: Vec<LogicalCpu>,
    pub physical_cores: Vec<PhysicalCore>,
    pub caches: Vec<Cache>,
    pub numa_nodes: Vec<NumaNode>,
    /// CPUs the kernel currently reports as online.
    pub online_cpus: Vec<LogicalId>,
    /// CPUs present but offlined.
    pub offline_cpus: Vec<LogicalId>,
    /// Affinity mask of the CoreScout process itself at discovery time. This
    /// matters more than it looks: inside a container or under a cpuset, most
    /// of the machine may be off limits, and benchmarking cores we cannot
    /// actually be scheduled on would produce nonsense.
    pub process_affinity: Vec<LogicalId>,
}

impl Topology {
    pub fn logical_count(&self) -> usize {
        self.logical_cpus.len()
    }

    pub fn physical_count(&self) -> usize {
        self.physical_cores.len()
    }

    /// True when at least one physical core carries more than one logical CPU.
    pub fn smt_enabled(&self) -> bool {
        self.physical_cores.iter().any(|c| c.logical_cpus.len() > 1)
    }

    /// SMT width, i.e. threads per core. 1 when SMT is off or absent.
    pub fn smt_width(&self) -> usize {
        self.physical_cores
            .iter()
            .map(|c| c.logical_cpus.len())
            .max()
            .unwrap_or(1)
    }

    pub fn cpu(&self, id: LogicalId) -> Option<&LogicalCpu> {
        self.logical_cpus.iter().find(|c| c.id == id)
    }

    pub fn core(&self, id: PhysicalId) -> Option<&PhysicalCore> {
        self.physical_cores.iter().find(|c| c.id == id)
    }

    /// The physical core owning a given logical CPU.
    pub fn core_of_cpu(&self, id: LogicalId) -> Option<&PhysicalCore> {
        let cpu = self.cpu(id)?;
        self.core(cpu.physical)
    }

    /// Caches at `level` that the logical CPU participates in.
    pub fn caches_for_cpu(&self, id: LogicalId, level: u8) -> Vec<&Cache> {
        self.caches
            .iter()
            .filter(|c| c.level == level && c.shared_cpus.contains(&id))
            .collect()
    }

    /// True when the two logical CPUs share a cache at `level`.
    pub fn shares_cache(&self, a: LogicalId, b: LogicalId, level: u8) -> bool {
        self.caches
            .iter()
            .any(|c| c.level == level && c.shared_cpus.contains(&a) && c.shared_cpus.contains(&b))
    }

    /// Physical cores that are online *and* inside the process affinity mask:
    /// the cores CoreScout is actually allowed to benchmark.
    pub fn benchmarkable_cores(&self) -> Vec<&PhysicalCore> {
        self.physical_cores
            .iter()
            .filter(|core| {
                core.logical_cpus.iter().any(|cpu| {
                    self.online_cpus.contains(cpu) && self.process_affinity.contains(cpu)
                })
            })
            .collect()
    }

    /// Count of cores per type, for hybrid parts.
    pub fn cores_by_type(&self) -> BTreeMap<&'static str, usize> {
        let mut map = BTreeMap::new();
        for core in &self.physical_cores {
            let label = match core.core_type {
                CoreType::Performance => "P-core",
                CoreType::Efficiency => "E-core",
                CoreType::Unknown => "core",
            };
            *map.entry(label).or_insert(0) += 1;
        }
        map
    }

    /// The firmware's favourite CPU, if any hint was found.
    pub fn firmware_favored_cpu(&self) -> Option<&LogicalCpu> {
        self.logical_cpus
            .iter()
            .filter(|c| c.favored.is_some())
            .min_by_key(|c| c.favored.as_ref().map(|f| f.rank).unwrap_or(u32::MAX))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::fake_topology;

    #[test]
    fn smt_detection() {
        let t = fake_topology();
        assert!(t.smt_enabled());
        assert_eq!(t.smt_width(), 2);
        assert_eq!(t.physical_count(), 2);
        assert_eq!(t.logical_count(), 4);
    }

    #[test]
    fn cache_sharing_follows_the_hierarchy() {
        let t = fake_topology();
        // SMT siblings share their L2.
        assert!(t.shares_cache(0, 2, 2));
        // Different physical cores do not.
        assert!(!t.shares_cache(0, 1, 2));
        // ...but they do share the package L3.
        assert!(t.shares_cache(0, 1, 3));
    }

    #[test]
    fn benchmarkable_cores_respect_affinity_and_offline() {
        let mut t = fake_topology();
        t.process_affinity = vec![0, 2];
        t.online_cpus = vec![0, 2];
        let cores = t.benchmarkable_cores();
        assert_eq!(cores.len(), 1);
        assert_eq!(cores[0].id, 0);
    }

    #[test]
    fn core_lookup_by_logical_cpu() {
        let t = fake_topology();
        assert_eq!(t.core_of_cpu(3).unwrap().id, 1);
        assert_eq!(t.core(1).unwrap().primary_cpu(), 1);
    }

    #[test]
    fn topology_round_trips_through_json() {
        let t = fake_topology();
        let s = serde_json::to_string(&t).unwrap();
        let back: Topology = serde_json::from_str(&s).unwrap();
        assert_eq!(t, back);
    }
}
