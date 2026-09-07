//! Topology discovery from Linux `sysfs` and `procfs`.
//!
//! # Why sysfs and not CPUID
//!
//! CPUID describes the silicon; sysfs describes the *machine as the kernel has
//! configured it*. Only the kernel knows which CPUs are offline, how a cpuset
//! or container has restricted us, what cpufreq policy is in force, and how
//! caches are actually shared after firmware fusing. CoreScout therefore treats
//! sysfs as authoritative and uses CPUID (see [`crate::platform::cpuid`]) only
//! to fill gaps such as the brand string.
//!
//! # Root injection
//!
//! [`Sysfs`] holds the filesystem roots rather than hardcoding `/sys` and
//! `/proc`. That is not an abstraction for its own sake: it lets the parser be
//! exercised against captured sysfs trees in unit and integration tests, on any
//! host OS, including machines with topologies nobody on the project owns.
//!
//! This module is compiled on every platform precisely so those tests can run
//! everywhere; only the syscall layer next door is Linux-only.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::topology::{
    Cache, CacheKind, CoreType, FavoredHint, FrequencyInfo, LogicalCpu, LogicalId, NumaNode,
    PhysicalCore, Topology,
};
use corescout_core::cpuset::CpuSet;
use corescout_core::error::{Error, Result};

/// Reader for a (possibly simulated) `/sys` + `/proc` pair.
#[derive(Debug, Clone)]
pub struct Sysfs {
    sys_root: PathBuf,
    proc_root: PathBuf,
}

impl Default for Sysfs {
    fn default() -> Self {
        Self::system()
    }
}

impl Sysfs {
    /// The real filesystem.
    pub fn system() -> Self {
        Sysfs {
            sys_root: PathBuf::from("/sys"),
            proc_root: PathBuf::from("/proc"),
        }
    }

    /// A simulated tree, used by tests and by `--sysroot` for debugging a
    /// captured topology from another machine.
    pub fn with_roots(sys_root: impl Into<PathBuf>, proc_root: impl Into<PathBuf>) -> Self {
        Sysfs {
            sys_root: sys_root.into(),
            proc_root: proc_root.into(),
        }
    }

    fn cpu_dir(&self) -> PathBuf {
        self.sys_root.join("devices/system/cpu")
    }

    fn node_dir(&self) -> PathBuf {
        self.sys_root.join("devices/system/node")
    }

    /// Build the complete [`Topology`].
    ///
    /// `process_affinity` is passed in rather than read here because it comes
    /// from a syscall, not a file. When `None`, the online set is assumed,
    /// which is what a fresh process without an inherited mask sees.
    pub fn read_topology(&self, process_affinity: Option<CpuSet>) -> Result<Topology> {
        let present = self.read_cpu_list("present")?;
        let online = self.read_cpu_list("online")?;
        // `offline` is absent on kernels built without CPU hotplug.
        let offline = self.read_cpu_list("offline").unwrap_or_default();

        if present.is_empty() {
            return Err(Error::parse(
                self.cpu_dir().join("present"),
                "kernel reported no present CPUs",
            ));
        }

        let hybrid_map = self.read_hybrid_core_types();
        let caches = self.read_caches(&online)?;

        // Pass 1: read per-CPU attributes and collect the distinct
        // (package, core) pairs so physical ids can be assigned densely.
        struct Raw {
            id: LogicalId,
            package_id: u32,
            core_id: u32,
            siblings: Vec<LogicalId>,
            online: bool,
            frequency: FrequencyInfo,
            cppc: Option<(u32, Option<u32>)>,
        }

        let mut raws = Vec::new();
        for cpu in present.iter() {
            let is_online = online.contains(cpu);
            let dir = self.cpu_dir().join(format!("cpu{cpu}"));
            // An offline CPU has no topology/ directory at all, so its package
            // and core ids are simply unknown. We keep it in the CPU list
            // (users want to see it) but it cannot be grouped or benchmarked.
            let (package_id, core_id, siblings) = if is_online {
                let package_id = read_u32(dir.join("topology/physical_package_id")).unwrap_or(0);
                let core_id = read_u32(dir.join("topology/core_id")).unwrap_or(cpu);
                let siblings = self.read_thread_siblings(&dir, cpu)?;
                (package_id, core_id, siblings)
            } else {
                (u32::MAX, cpu, vec![cpu])
            };

            raws.push(Raw {
                id: cpu,
                package_id,
                core_id,
                siblings,
                online: is_online,
                frequency: self.read_frequency(&dir),
                cppc: self.read_cppc(&dir),
            });
        }

        // Assign machine-wide physical core ids. Linux `core_id` is unique only
        // within a package, so a two-socket box would otherwise collide core 0
        // of socket 0 with core 0 of socket 1.
        let mut pairs: Vec<(u32, u32)> = raws.iter().map(|r| (r.package_id, r.core_id)).collect();
        pairs.sort_unstable();
        pairs.dedup();
        let phys_index: BTreeMap<(u32, u32), u32> = pairs
            .iter()
            .enumerate()
            .map(|(i, pair)| (*pair, i as u32))
            .collect();

        let numa_nodes = self.read_numa_nodes()?;
        let numa_of_cpu = |cpu: LogicalId| -> Option<u32> {
            numa_nodes
                .iter()
                .find(|n| n.cpus.contains(&cpu))
                .map(|n| n.id)
        };

        // Pass 2: rank the CPPC hints so the "firmware's favourite" is explicit
        // rather than something the user has to eyeball out of raw numbers.
        let mut perf_values: Vec<u32> = raws.iter().filter_map(|r| r.cppc.map(|c| c.0)).collect();
        perf_values.sort_unstable_by(|a, b| b.cmp(a));
        perf_values.dedup();

        let mut logical_cpus = Vec::with_capacity(raws.len());
        for raw in &raws {
            let physical = *phys_index
                .get(&(raw.package_id, raw.core_id))
                .expect("every raw CPU contributed its pair to the index");
            let smt_siblings: Vec<LogicalId> = raw
                .siblings
                .iter()
                .copied()
                .filter(|s| *s != raw.id)
                .collect();
            let favored = raw.cppc.map(|(highest, nominal)| FavoredHint {
                highest_perf: highest,
                nominal_perf: nominal,
                rank: perf_values
                    .iter()
                    .position(|v| *v == highest)
                    .map(|p| p as u32 + 1)
                    .unwrap_or(u32::MAX),
                source: "acpi_cppc/highest_perf".to_string(),
            });

            logical_cpus.push(LogicalCpu {
                id: raw.id,
                physical,
                package_id: raw.package_id,
                core_id: raw.core_id,
                numa_node: numa_of_cpu(raw.id),
                smt_siblings,
                core_type: hybrid_map
                    .get(&raw.id)
                    .copied()
                    .unwrap_or(CoreType::Unknown),
                online: raw.online,
                frequency: raw.frequency,
                favored,
            });
        }
        logical_cpus.sort_by_key(|c| c.id);

        // Group logical CPUs into physical cores. Offline CPUs are excluded:
        // a core we cannot schedule on is not a core we can rank.
        let mut cores: BTreeMap<u32, PhysicalCore> = BTreeMap::new();
        for cpu in logical_cpus.iter().filter(|c| c.online) {
            let entry = cores.entry(cpu.physical).or_insert_with(|| PhysicalCore {
                id: cpu.physical,
                package_id: cpu.package_id,
                core_id: cpu.core_id,
                numa_node: cpu.numa_node,
                core_type: cpu.core_type,
                logical_cpus: Vec::new(),
            });
            entry.logical_cpus.push(cpu.id);
            if entry.core_type == CoreType::Unknown {
                entry.core_type = cpu.core_type;
            }
        }
        let mut physical_cores: Vec<PhysicalCore> = cores.into_values().collect();
        for core in &mut physical_cores {
            core.logical_cpus.sort_unstable();
        }

        let (vendor, model_name) = self.read_cpu_identity();
        let hybrid = physical_cores
            .iter()
            .any(|c| c.core_type == CoreType::Efficiency)
            && physical_cores
                .iter()
                .any(|c| c.core_type == CoreType::Performance);

        let process_affinity = process_affinity.unwrap_or_else(|| online.clone());

        Ok(Topology {
            model_name,
            vendor,
            hybrid,
            logical_cpus,
            physical_cores,
            caches,
            numa_nodes,
            online_cpus: online.to_vec(),
            offline_cpus: offline.to_vec(),
            process_affinity: process_affinity.to_vec(),
        })
    }

    /// Read one of the `cpu/{present,online,offline,possible}` masks.
    fn read_cpu_list(&self, name: &str) -> Result<CpuSet> {
        let path = self.cpu_dir().join(name);
        let raw = read_string(&path)?;
        CpuSet::parse_list(&raw).map_err(|e| Error::parse(&path, e.to_string()))
    }

    /// Thread siblings, i.e. the logical CPUs sharing this physical core.
    ///
    /// `core_cpus_list` is the modern name (5.3+); `thread_siblings_list` is
    /// the legacy one. Both may be missing on kernels without
    /// `CONFIG_SCHED_SMT` or inside restricted containers, in which case the
    /// CPU is treated as its own core, which is the correct conservative
    /// answer: we would rather under-report SMT than pin two benchmark threads
    /// onto one physical core believing they are independent.
    fn read_thread_siblings(&self, dir: &Path, cpu: LogicalId) -> Result<Vec<LogicalId>> {
        for name in ["topology/core_cpus_list", "topology/thread_siblings_list"] {
            let path = dir.join(name);
            match read_string(&path) {
                Ok(raw) => {
                    let set =
                        CpuSet::parse_list(&raw).map_err(|e| Error::parse(&path, e.to_string()))?;
                    if !set.is_empty() {
                        return Ok(set.to_vec());
                    }
                }
                Err(e) if e.is_not_found() => continue,
                Err(e) => return Err(e),
            }
        }
        Ok(vec![cpu])
    }

    fn read_frequency(&self, dir: &Path) -> FrequencyInfo {
        let cpufreq = dir.join("cpufreq");
        FrequencyInfo {
            // `scaling_cur_freq` is the governor's request;
            // `cpuinfo_cur_freq` is the measured value but needs privileges on
            // many systems, so the former is the pragmatic default.
            current_khz: read_u64(cpufreq.join("scaling_cur_freq"))
                .or_else(|| read_u64(cpufreq.join("cpuinfo_cur_freq"))),
            min_khz: read_u64(cpufreq.join("cpuinfo_min_freq"))
                .or_else(|| read_u64(cpufreq.join("scaling_min_freq"))),
            max_khz: read_u64(cpufreq.join("cpuinfo_max_freq"))
                .or_else(|| read_u64(cpufreq.join("scaling_max_freq"))),
            // intel_pstate and amd_pstate expose the non-turbo base clock here.
            base_khz: read_u64(cpufreq.join("base_frequency")),
        }
    }

    /// ACPI CPPC performance hints: the kernel's view of the manufacturer's
    /// preferred-core ordering (Intel Turbo Boost Max 3.0, AMD Preferred Core).
    fn read_cppc(&self, dir: &Path) -> Option<(u32, Option<u32>)> {
        let cppc = dir.join("acpi_cppc");
        let highest = read_u32(cppc.join("highest_perf"))?;
        Some((highest, read_u32(cppc.join("nominal_perf"))))
    }

    /// Hybrid core classification from the per-core-type PMU devices that the
    /// kernel creates on Intel hybrid parts (`cpu_core` = P, `cpu_atom` = E).
    ///
    /// This is preferred over CPUID leaf 0x1A because it needs no pinning and
    /// works for CPUs we are not allowed to run on.
    fn read_hybrid_core_types(&self) -> BTreeMap<LogicalId, CoreType> {
        let mut map = BTreeMap::new();
        let sources = [
            ("devices/cpu_core/cpus", CoreType::Performance),
            ("devices/cpu_atom/cpus", CoreType::Efficiency),
        ];
        for (rel, kind) in sources {
            if let Ok(raw) = read_string(self.sys_root.join(rel)) {
                if let Ok(set) = CpuSet::parse_list(&raw) {
                    for cpu in set.iter() {
                        map.insert(cpu, kind);
                    }
                }
            }
        }
        map
    }

    /// All cache instances, deduplicated by their sharing set.
    ///
    /// sysfs lists every cache once *per participating CPU*, so an L3 shared by
    /// 32 CPUs appears 32 times. We key on (level, kind, shared set) to collapse
    /// those into one instance, which is what makes "which cores share this
    /// cache" answerable.
    fn read_caches(&self, online: &CpuSet) -> Result<Vec<Cache>> {
        let mut seen: BTreeSet<(u8, CacheKind, Vec<LogicalId>)> = BTreeSet::new();
        let mut caches = Vec::new();

        for cpu in online.iter() {
            let cache_dir = self.cpu_dir().join(format!("cpu{cpu}/cache"));
            // Index numbering is dense but its length is unknown; probe until
            // a gap. Containers frequently expose no cache directory at all.
            for index in 0..16u32 {
                let dir = cache_dir.join(format!("index{index}"));
                let level = match read_u32(dir.join("level")) {
                    Some(l) => l as u8,
                    None => break,
                };
                let type_raw = read_string(dir.join("type")).unwrap_or_default();
                let kind = classify_cache(level, type_raw.trim());
                let shared = match read_string(dir.join("shared_cpu_list")) {
                    Ok(raw) => CpuSet::parse_list(&raw).unwrap_or_default().to_vec(),
                    Err(_) => vec![cpu],
                };
                let key = (level, kind, shared.clone());
                if !seen.insert(key) {
                    continue;
                }
                caches.push(Cache {
                    level,
                    kind,
                    size_bytes: read_string(dir.join("size"))
                        .ok()
                        .and_then(|s| parse_size(s.trim())),
                    line_size_bytes: read_u32(dir.join("coherency_line_size")),
                    ways_of_associativity: read_u32(dir.join("ways_of_associativity")),
                    shared_cpus: shared,
                });
            }
        }

        caches.sort_by(|a, b| {
            a.level
                .cmp(&b.level)
                .then(a.kind.cmp(&b.kind))
                .then(a.shared_cpus.cmp(&b.shared_cpus))
        });
        Ok(caches)
    }

    fn read_numa_nodes(&self) -> Result<Vec<NumaNode>> {
        let dir = self.node_dir();
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            // No CONFIG_NUMA, or a container hiding it. A single implicit node
            // is the right model in that case, and we signal it with an empty
            // node list rather than inventing a node 0 that sysfs never showed.
            Err(_) => return Ok(Vec::new()),
        };

        let mut nodes = Vec::new();
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let Some(id) = name
                .strip_prefix("node")
                .and_then(|n| n.parse::<u32>().ok())
            else {
                continue;
            };
            let path = dir.join(&*name);
            let cpus = read_string(path.join("cpulist"))
                .ok()
                .and_then(|raw| CpuSet::parse_list(&raw).ok())
                .unwrap_or_default()
                .to_vec();
            nodes.push(NumaNode {
                id,
                cpus,
                memory_kb: read_string(path.join("meminfo"))
                    .ok()
                    .and_then(|s| parse_node_mem_total_kb(&s)),
            });
        }
        nodes.sort_by_key(|n| n.id);
        Ok(nodes)
    }

    /// Vendor and model name, preferring `/proc/cpuinfo` (which the kernel has
    /// already normalised) and falling back to the CPUID brand string.
    fn read_cpu_identity(&self) -> (String, String) {
        let cpuinfo = read_string(self.proc_root.join("cpuinfo")).unwrap_or_default();
        let vendor = first_cpuinfo_field(&cpuinfo, "vendor_id")
            .or_else(|| first_cpuinfo_field(&cpuinfo, "CPU implementer"))
            .or_else(crate::platform::cpuid::vendor)
            .unwrap_or_else(|| "unknown".to_string());
        let model = first_cpuinfo_field(&cpuinfo, "model name")
            .or_else(|| first_cpuinfo_field(&cpuinfo, "Model"))
            .or_else(crate::platform::cpuid::brand_string)
            .unwrap_or_else(|| "unknown".to_string());
        (vendor, model)
    }
}

// ---------------------------------------------------------------------------
// Small parsing helpers. Kept free-standing so they can be unit tested without
// constructing a filesystem.
// ---------------------------------------------------------------------------

fn read_string(path: impl AsRef<Path>) -> Result<String> {
    let path = path.as_ref();
    std::fs::read_to_string(path).map_err(|e| Error::io(path, e))
}

fn read_u64(path: impl AsRef<Path>) -> Option<u64> {
    read_string(path).ok()?.trim().parse().ok()
}

fn read_u32(path: impl AsRef<Path>) -> Option<u32> {
    read_string(path).ok()?.trim().parse().ok()
}

/// Map sysfs `level` + `type` onto our cache taxonomy.
fn classify_cache(level: u8, type_raw: &str) -> CacheKind {
    match (level, type_raw) {
        (1, "Data") => CacheKind::L1Data,
        (1, "Instruction") => CacheKind::L1Instruction,
        (2, "Unified") => CacheKind::L2Unified,
        (3, "Unified") => CacheKind::L3Unified,
        _ => CacheKind::Other,
    }
}

/// Parse the sysfs cache `size` attribute: `"32K"`, `"1280K"`, `"32M"`.
fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let (digits, mult) = match s.as_bytes()[s.len() - 1] {
        b'K' | b'k' => (&s[..s.len() - 1], 1024),
        b'M' | b'm' => (&s[..s.len() - 1], 1024 * 1024),
        b'G' | b'g' => (&s[..s.len() - 1], 1024 * 1024 * 1024),
        _ => (s, 1),
    };
    digits.trim().parse::<u64>().ok().map(|v| v * mult)
}

/// Pull `MemTotal` out of a NUMA node's `meminfo`, whose lines look like
/// `Node 0 MemTotal:  16316296 kB`.
fn parse_node_mem_total_kb(s: &str) -> Option<u64> {
    for line in s.lines() {
        if let Some(rest) = line.split("MemTotal:").nth(1) {
            return rest.split_whitespace().next()?.parse().ok();
        }
    }
    None
}

/// First value of a `key : value` field in `/proc/cpuinfo`.
///
/// "First" because cpuinfo repeats every field per CPU and, on hybrid parts,
/// the model name genuinely differs between entries; the leading one is the
/// conventional choice and matches what `lscpu` prints.
fn first_cpuinfo_field(cpuinfo: &str, key: &str) -> Option<String> {
    for line in cpuinfo.lines() {
        let (k, v) = line.split_once(':')?;
        if k.trim() == key {
            let v = v.trim();
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cache_sizes_with_and_without_suffix() {
        assert_eq!(parse_size("32K"), Some(32 * 1024));
        assert_eq!(parse_size("1280K"), Some(1280 * 1024));
        assert_eq!(parse_size("32M"), Some(32 * 1024 * 1024));
        assert_eq!(parse_size("512"), Some(512));
        assert_eq!(parse_size(""), None);
        assert_eq!(parse_size("banana"), None);
    }

    #[test]
    fn classifies_cache_levels() {
        assert_eq!(classify_cache(1, "Data"), CacheKind::L1Data);
        assert_eq!(classify_cache(1, "Instruction"), CacheKind::L1Instruction);
        assert_eq!(classify_cache(2, "Unified"), CacheKind::L2Unified);
        assert_eq!(classify_cache(3, "Unified"), CacheKind::L3Unified);
        assert_eq!(classify_cache(4, "Unified"), CacheKind::Other);
        assert_eq!(classify_cache(1, "Unified"), CacheKind::Other);
    }

    #[test]
    fn extracts_node_memory() {
        let s = "Node 0 MemTotal:       16316296 kB\nNode 0 MemFree: 100 kB\n";
        assert_eq!(parse_node_mem_total_kb(s), Some(16316296));
        assert_eq!(parse_node_mem_total_kb("Node 0 MemFree: 1 kB"), None);
    }

    #[test]
    fn reads_first_cpuinfo_field() {
        let s = "processor\t: 0\nvendor_id\t: AuthenticAMD\nmodel name\t: AMD Ryzen 9 7950X\n\
                 processor\t: 1\nmodel name\t: AMD Ryzen 9 7950X\n";
        assert_eq!(
            first_cpuinfo_field(s, "vendor_id").as_deref(),
            Some("AuthenticAMD")
        );
        assert_eq!(
            first_cpuinfo_field(s, "model name").as_deref(),
            Some("AMD Ryzen 9 7950X")
        );
        assert_eq!(first_cpuinfo_field(s, "flags"), None);
    }
}
