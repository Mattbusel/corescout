//! Topology discovery on Windows, via `GetLogicalProcessorInformationEx`.
//!
//! # What this reads
//!
//! One call returns a packed, variable-length list of records describing cores,
//! caches, NUMA nodes and packages. It is the closest Windows analogue to
//! walking `/sys/devices/system/cpu`, and it is better in one respect: it comes
//! from a single consistent snapshot rather than from several hundred files
//! that can change underneath the reader.
//!
//! # Processor groups
//!
//! Windows partitions logical CPUs into *groups* of at most 64, because the
//! classic affinity APIs take a `usize` mask. A machine with 80 CPUs has two
//! groups, and `(group 1, bit 3)` is a different CPU from `(group 0, bit 3)`.
//!
//! CoreScout's `LogicalId` is a flat number, so the two are mapped as
//! `group * 64 + bit`. That is a convention, and it is the same convention used
//! by [`super::affinity`], which is what matters: an id means the same thing
//! everywhere or it means nothing.
//!
//! # What Windows does not tell us
//!
//! No per-core minimum frequency, no `core_id` that survives a reboot, and no
//! preferred-core ranking of the sort Linux exposes through CPPC. Those are
//! left `None` rather than guessed at, which is what the availability machinery
//! in the mirror exists to represent.

use std::collections::BTreeMap;

use windows_sys::Win32::System::SystemInformation::{
    CacheData, CacheInstruction, CacheUnified, GetLogicalProcessorInformationEx, RelationAll,
    RelationCache, RelationNumaNode, RelationProcessorCore, RelationProcessorPackage,
    PROCESSOR_CACHE_TYPE, SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX,
};

use corescout_core::error::{Error, Result};
use corescout_core::LogicalId;

use crate::topology::{
    Cache, CacheKind, CoreType, FrequencyInfo, LogicalCpu, NumaNode, PhysicalCore, Topology,
};

/// A `GROUP_AFFINITY`, flattened to the logical CPUs it names.
fn cpus_of(group: u16, mask: usize) -> Vec<LogicalId> {
    let mut cpus = Vec::new();
    for bit in 0..usize::BITS {
        if mask & (1usize << bit) != 0 {
            cpus.push(group as u32 * 64 + bit);
        }
    }
    cpus
}

/// Read the raw record list.
///
/// Called twice: once to learn the size, once to fill. The buffer is `u8` with
/// the records packed end to end, each carrying its own `Size`, so walking it
/// means stepping by that field rather than by a struct width.
fn raw_records() -> Result<Vec<u8>> {
    // SAFETY: a null buffer with a zero length is the documented way to ask for
    // the required size; the call writes only `length`.
    let mut length: u32 = 0;
    unsafe {
        GetLogicalProcessorInformationEx(RelationAll, std::ptr::null_mut(), &mut length);
    }
    if length == 0 {
        return Err(Error::unsupported(
            "GetLogicalProcessorInformationEx reported no processor information",
        ));
    }
    let mut buffer = vec![0u8; length as usize];
    // SAFETY: the buffer is exactly the size the previous call asked for.
    let ok = unsafe {
        GetLogicalProcessorInformationEx(
            RelationAll,
            buffer
                .as_mut_ptr()
                .cast::<SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX>(),
            &mut length,
        )
    };
    if ok == 0 {
        return Err(Error::syscall(
            "GetLogicalProcessorInformationEx",
            std::io::Error::last_os_error().raw_os_error().unwrap_or(0),
        ));
    }
    buffer.truncate(length as usize);
    Ok(buffer)
}

/// One decoded record.
enum Record {
    Core {
        cpus: Vec<LogicalId>,
        smt: bool,
        efficiency_class: u8,
    },
    Cache {
        level: u8,
        kind: CacheKind,
        size_bytes: u64,
        line_size: u32,
        associativity: Option<u32>,
        cpus: Vec<LogicalId>,
    },
    Numa {
        node: u32,
        cpus: Vec<LogicalId>,
    },
    Package {
        cpus: Vec<LogicalId>,
    },
}

/// Walk the packed buffer, decoding what we understand and skipping what we do
/// not.
fn decode(buffer: &[u8]) -> Vec<Record> {
    let mut records = Vec::new();
    let mut offset = 0usize;
    while offset + std::mem::size_of::<u32>() * 2 <= buffer.len() {
        // SAFETY: the buffer came from the OS in this exact layout, and the
        // bounds are checked against `Size` before each step.
        let header = unsafe {
            &*buffer
                .as_ptr()
                .add(offset)
                .cast::<SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX>()
        };
        let size = header.Size as usize;
        if size == 0 || offset + size > buffer.len() {
            break;
        }

        // SAFETY: each branch reads only the union member the relationship
        // names. These are constants rather than enum variants, so they are
        // compared explicitly: as match patterns they would bind as new
        // variables and every record would take the first arm.
        unsafe {
            let relationship = header.Relationship;
            if relationship == RelationProcessorCore || relationship == RelationProcessorPackage {
                let processor = &header.Anonymous.Processor;
                let mut cpus = Vec::new();
                for index in 0..processor.GroupCount as usize {
                    let affinity = processor.GroupMask[index];
                    cpus.extend(cpus_of(affinity.Group, affinity.Mask));
                }
                if relationship == RelationProcessorCore {
                    records.push(Record::Core {
                        smt: processor.Flags != 0,
                        efficiency_class: processor.EfficiencyClass,
                        cpus,
                    });
                } else {
                    records.push(Record::Package { cpus });
                }
            } else if relationship == RelationCache {
                let cache = &header.Anonymous.Cache;
                let affinity = cache.Anonymous.GroupMask;
                records.push(Record::Cache {
                    level: cache.Level,
                    kind: cache_kind(cache.Level, cache.Type),
                    size_bytes: cache.CacheSize as u64,
                    line_size: cache.LineSize as u32,
                    // Windows reports 0xFF for "fully associative", which is a
                    // fact about the cache rather than a number of ways.
                    associativity: match cache.Associativity {
                        0xFF => None,
                        ways => Some(ways as u32),
                    },
                    cpus: cpus_of(affinity.Group, affinity.Mask),
                });
            } else if relationship == RelationNumaNode {
                let numa = &header.Anonymous.NumaNode;
                let affinity = numa.Anonymous.GroupMask;
                records.push(Record::Numa {
                    node: numa.NodeNumber,
                    cpus: cpus_of(affinity.Group, affinity.Mask),
                });
            }
            // Groups and dies are skipped: not needed, and skipping is better
            // than guessing at a mapping we would then have to defend.
        }
        offset += size;
    }
    records
}

fn cache_kind(level: u8, kind: PROCESSOR_CACHE_TYPE) -> CacheKind {
    // Explicit comparisons for the same reason as above: these are constants.
    if level == 1 && kind == CacheData {
        CacheKind::L1Data
    } else if level == 1 && kind == CacheInstruction {
        CacheKind::L1Instruction
    } else if level == 2 && kind == CacheUnified {
        CacheKind::L2Unified
    } else if level == 3 && kind == CacheUnified {
        CacheKind::L3Unified
    } else {
        CacheKind::Other
    }
}

/// Build a [`Topology`] from what Windows reports.
pub fn discover() -> Result<Topology> {
    let records = decode(&raw_records()?);
    if records.is_empty() {
        return Err(Error::unsupported(
            "no processor records were decodable from GetLogicalProcessorInformationEx",
        ));
    }

    // Cores first: they define the logical CPU set and the SMT relation.
    let mut cores: Vec<(Vec<LogicalId>, bool, u8)> = Vec::new();
    for record in &records {
        if let Record::Core {
            cpus,
            smt,
            efficiency_class,
        } = record
        {
            cores.push((cpus.clone(), *smt, *efficiency_class));
        }
    }
    cores.sort_by_key(|(cpus, _, _)| cpus.first().copied().unwrap_or(u32::MAX));

    // Which package each CPU belongs to.
    let mut package_of: BTreeMap<LogicalId, u32> = BTreeMap::new();
    let mut package_index = 0u32;
    for record in &records {
        if let Record::Package { cpus } = record {
            for cpu in cpus {
                package_of.insert(*cpu, package_index);
            }
            package_index += 1;
        }
    }

    // Which NUMA node each CPU belongs to.
    let mut numa_of: BTreeMap<LogicalId, u32> = BTreeMap::new();
    let mut numa_nodes: Vec<NumaNode> = Vec::new();
    for record in &records {
        if let Record::Numa { node, cpus } = record {
            for cpu in cpus {
                numa_of.insert(*cpu, *node);
            }
            numa_nodes.push(NumaNode {
                id: *node,
                cpus: cpus.clone(),
                // Per-node memory needs GetNumaAvailableMemoryNodeEx, which
                // reports *available* rather than total. Reporting a different
                // quantity under the same name would be worse than reporting
                // nothing.
                memory_kb: None,
            });
        }
    }
    numa_nodes.sort_by_key(|node| node.id);

    // Hybrid parts report an efficiency class per core; higher is more
    // performant. A part where every core reports the same class is not hybrid,
    // and calling its cores "performance" would be inventing a distinction.
    let classes: Vec<u8> = cores.iter().map(|(_, _, class)| *class).collect();
    let best = classes.iter().copied().max().unwrap_or(0);
    let worst = classes.iter().copied().min().unwrap_or(0);
    let hybrid = best != worst;

    let mut logical_cpus: Vec<LogicalCpu> = Vec::new();
    let mut physical_cores: Vec<PhysicalCore> = Vec::new();

    for (physical, (cpus, smt, class)) in cores.iter().enumerate() {
        let physical = physical as u32;
        let package_id = cpus
            .first()
            .and_then(|cpu| package_of.get(cpu).copied())
            .unwrap_or(0);
        let numa_node = cpus.first().and_then(|cpu| numa_of.get(cpu).copied());
        let core_type = if !hybrid {
            CoreType::Unknown
        } else if *class == best {
            CoreType::Performance
        } else {
            CoreType::Efficiency
        };

        for cpu in cpus {
            let siblings: Vec<LogicalId> =
                cpus.iter().copied().filter(|other| other != cpu).collect();
            debug_assert!(*smt || siblings.is_empty());
            logical_cpus.push(LogicalCpu {
                id: *cpu,
                physical,
                package_id,
                core_id: physical,
                numa_node,
                smt_siblings: siblings,
                core_type,
                // Windows does not enumerate offline CPUs here at all, so every
                // CPU that appears is online by construction.
                online: true,
                // Filled in below, from the power interface.
                frequency: FrequencyInfo::default(),
                // No CPPC-style ranking is exposed. Left absent rather than
                // approximated from the efficiency class, which is a different
                // thing.
                favored: None,
            });
        }

        physical_cores.push(PhysicalCore {
            id: physical,
            package_id,
            core_id: physical,
            numa_node,
            core_type,
            logical_cpus: cpus.clone(),
        });
    }

    logical_cpus.sort_by_key(|cpu| cpu.id);

    let mut caches: Vec<Cache> = Vec::new();
    for record in &records {
        if let Record::Cache {
            level,
            kind,
            size_bytes,
            line_size,
            associativity,
            cpus,
        } = record
        {
            caches.push(Cache {
                level: *level,
                kind: *kind,
                size_bytes: (*size_bytes > 0).then_some(*size_bytes),
                line_size_bytes: (*line_size > 0).then_some(*line_size),
                ways_of_associativity: *associativity,
                shared_cpus: cpus.clone(),
            });
        }
    }
    caches.sort_by_key(|cache| (cache.level, cache.shared_cpus.first().copied().unwrap_or(0)));

    let online: Vec<LogicalId> = logical_cpus.iter().map(|cpu| cpu.id).collect();
    let mut topology = Topology {
        model_name: super::cpu_brand().unwrap_or_else(|| "unknown".to_string()),
        vendor: super::cpu_vendor().unwrap_or_else(|| "unknown".to_string()),
        hybrid,
        logical_cpus,
        physical_cores,
        caches,
        numa_nodes,
        online_cpus: online.clone(),
        offline_cpus: Vec::new(),
        process_affinity: super::affinity::process_affinity()
            .map(|set| set.to_vec())
            .unwrap_or(online),
    };

    super::frequency::fill(&mut topology);
    Ok(topology)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_group_mask_expands_to_the_cpus_it_names() {
        assert_eq!(cpus_of(0, 0b1011), vec![0, 1, 3]);
        // Group 1 bit 0 is CPU 64, not CPU 0: an id means the same thing
        // everywhere or it means nothing.
        assert_eq!(cpus_of(1, 0b101), vec![64, 66]);
        assert!(cpus_of(0, 0).is_empty());
    }

    #[test]
    fn cache_levels_map_to_the_kinds_the_topology_uses() {
        assert_eq!(cache_kind(1, CacheData), CacheKind::L1Data);
        assert_eq!(cache_kind(1, CacheInstruction), CacheKind::L1Instruction);
        assert_eq!(cache_kind(2, CacheUnified), CacheKind::L2Unified);
        assert_eq!(cache_kind(3, CacheUnified), CacheKind::L3Unified);
        assert_eq!(cache_kind(4, CacheUnified), CacheKind::Other);
    }

    #[test]
    fn this_machine_has_a_coherent_topology() {
        // A live test. It runs on the machine doing the testing, which is the
        // entire point of writing this backend.
        let topology = discover().expect("Windows should describe its own CPUs");
        assert!(!topology.logical_cpus.is_empty());
        assert!(!topology.physical_cores.is_empty());
        assert!(topology.logical_cpus.len() >= topology.physical_cores.len());

        // Every logical CPU belongs to exactly one physical core, and that
        // core agrees that it owns it.
        for cpu in &topology.logical_cpus {
            let core = topology
                .physical_cores
                .iter()
                .find(|core| core.id == cpu.physical)
                .expect("every CPU has a core");
            assert!(core.logical_cpus.contains(&cpu.id));
        }

        // SMT is symmetric: if A is a sibling of B, B is a sibling of A.
        for cpu in &topology.logical_cpus {
            for sibling in &cpu.smt_siblings {
                let other = topology
                    .logical_cpus
                    .iter()
                    .find(|c| c.id == *sibling)
                    .expect("a sibling exists");
                assert!(other.smt_siblings.contains(&cpu.id));
            }
        }
    }

    #[test]
    fn this_machine_reports_caches_that_make_sense() {
        let topology = discover().expect("discoverable");
        assert!(!topology.caches.is_empty(), "no caches were reported");
        for cache in &topology.caches {
            assert!(cache.level >= 1 && cache.level <= 4);
            assert!(!cache.shared_cpus.is_empty());
            if let Some(size) = cache.size_bytes {
                assert!(size > 0);
            }
        }
        // A last-level cache should be shared by more CPUs than an L1.
        let l1 = topology.caches.iter().find(|c| c.level == 1);
        let last = topology.caches.iter().max_by_key(|c| c.level);
        if let (Some(l1), Some(last)) = (l1, last) {
            if last.level > 1 {
                assert!(last.shared_cpus.len() >= l1.shared_cpus.len());
            }
        }
    }
}
