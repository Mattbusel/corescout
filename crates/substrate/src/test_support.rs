//! Fixtures shared with the other crates' tests.
//!
//! # Why this is not behind `cfg(test)`
//!
//! A `#[cfg(test)]` module is invisible outside its own crate, so once the
//! project became a workspace the fixtures every downstream crate's tests were
//! already using stopped compiling. Exporting them properly, hidden from the
//! documentation, is honest about what they are: shipped code that exists for
//! tests. They are never used by the runtime, and nothing in the runtime reads
//! this module.
//!
//! These are fixtures, not fallbacks. Nothing here is ever substituted for a
//! real reading of the machine.

use crate::topology::*;

/// Two physical cores, SMT width 2, one shared L3, split L2s.
pub fn fake_topology() -> Topology {
    let mk_cpu = |id: LogicalId, phys: PhysicalId, sib: LogicalId| LogicalCpu {
        id,
        physical: phys,
        package_id: 0,
        core_id: phys,
        numa_node: Some(0),
        smt_siblings: vec![sib],
        core_type: CoreType::Unknown,
        online: true,
        frequency: FrequencyInfo::default(),
        favored: None,
    };
    Topology {
        model_name: "Test CPU".into(),
        vendor: "TestVendor".into(),
        hybrid: false,
        logical_cpus: vec![
            mk_cpu(0, 0, 2),
            mk_cpu(1, 1, 3),
            mk_cpu(2, 0, 0),
            mk_cpu(3, 1, 1),
        ],
        physical_cores: vec![
            PhysicalCore {
                id: 0,
                package_id: 0,
                core_id: 0,
                numa_node: Some(0),
                core_type: CoreType::Unknown,
                logical_cpus: vec![0, 2],
            },
            PhysicalCore {
                id: 1,
                package_id: 0,
                core_id: 1,
                numa_node: Some(0),
                core_type: CoreType::Unknown,
                logical_cpus: vec![1, 3],
            },
        ],
        caches: vec![
            Cache {
                level: 2,
                kind: CacheKind::L2Unified,
                size_bytes: Some(1 << 20),
                line_size_bytes: Some(64),
                ways_of_associativity: Some(8),
                shared_cpus: vec![0, 2],
            },
            Cache {
                level: 2,
                kind: CacheKind::L2Unified,
                size_bytes: Some(1 << 20),
                line_size_bytes: Some(64),
                ways_of_associativity: Some(8),
                shared_cpus: vec![1, 3],
            },
            Cache {
                level: 3,
                kind: CacheKind::L3Unified,
                size_bytes: Some(32 << 20),
                line_size_bytes: Some(64),
                ways_of_associativity: Some(16),
                shared_cpus: vec![0, 1, 2, 3],
            },
        ],
        numa_nodes: vec![NumaNode {
            id: 0,
            cpus: vec![0, 1, 2, 3],
            memory_kb: Some(16 * 1024 * 1024),
        }],
        online_cpus: vec![0, 1, 2, 3],
        offline_cpus: vec![],
        process_affinity: vec![0, 1, 2, 3],
    }
}
