//! Turning a topology into the entities and relations the mirror reflects.
//!
//! # What this module decides
//!
//! The [`Topology`] the platform layer produces is a good description of a CPU
//! and a poor foundation for a general representation: it has typed collections
//! with different shapes, and reasoning about it means knowing which collection
//! to look in. This module flattens it into one uniform entity list plus an edge
//! list, losing nothing and gaining the ability to be traversed by something
//! that has never heard of a NUMA node.
//!
//! # Ordering is part of the contract
//!
//! Entity rows are assigned in a deterministic order: machine, packages, NUMA
//! nodes, physical cores, logical CPUs, caches, each sorted by natural index.
//! Two runs against the same machine therefore produce the same row layout,
//! which means a consumer can cache row indices for the lifetime of an epoch and
//! a recorded snapshot can be compared against a later one directly.
//!
//! Sensors append their own entities after these, in sensor registration order,
//! for the same reason.

use std::path::PathBuf;

use crate::platform::Platform;
use crate::topology::{CacheKind, Topology};
use corescout_core::error::Result;
use corescout_mirror::entity::{keys, Entity, EntityClass};
use corescout_mirror::relation::{Relation, RelationKind};

/// Filesystem roots the observation layer reads from.
///
/// Injectable for exactly the reason the sysfs parser's roots are injectable:
/// it lets every sensor be tested against a synthetic machine, on any host,
/// including topologies nobody on the project owns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Roots {
    pub sys: PathBuf,
    pub proc: PathBuf,
}

impl Default for Roots {
    fn default() -> Self {
        Roots::system()
    }
}

impl Roots {
    pub fn system() -> Roots {
        Roots {
            sys: PathBuf::from("/sys"),
            proc: PathBuf::from("/proc"),
        }
    }

    pub fn new(sys: impl Into<PathBuf>, proc: impl Into<PathBuf>) -> Roots {
        Roots {
            sys: sys.into(),
            proc: proc.into(),
        }
    }
}

/// The structural machine: what exists, and where to look to observe it.
#[derive(Debug, Clone)]
pub struct Substrate {
    pub topology: Topology,
    pub roots: Roots,
}

impl Substrate {
    /// Discover the substrate through a platform backend.
    pub fn discover(platform: &dyn Platform) -> Result<Substrate> {
        Ok(Substrate {
            topology: platform.discover_topology()?,
            roots: Roots::system(),
        })
    }

    pub fn new(topology: Topology, roots: Roots) -> Substrate {
        Substrate { topology, roots }
    }

    /// Re-read the machine's structure from the same roots.
    ///
    /// Used to detect hotplug: a CPU going offline changes the entity set, and
    /// every cached row index in every consumer becomes wrong. Cheap enough to
    /// call once a second, far too expensive to call every tick.
    pub fn rediscover(&self) -> Result<Substrate> {
        let sysfs = crate::platform::linux::sysfs::Sysfs::with_roots(
            self.roots.sys.clone(),
            self.roots.proc.clone(),
        );
        Ok(Substrate {
            topology: sysfs.read_topology(None)?,
            roots: self.roots.clone(),
        })
    }

    /// Build the structural entity list and the static edges between them.
    ///
    /// "Static" means constant for the lifetime of an epoch: containment, SMT
    /// siblinghood, cache membership and NUMA locality do not change while the
    /// set of online CPUs is unchanged. Sensors add further entities and edges
    /// on top of these.
    pub fn structure(&self) -> (Vec<Entity>, Vec<Relation>) {
        let t = &self.topology;
        let mut entities = Vec::new();
        let mut relations = Vec::new();

        // Row 0 is always the machine. A consumer with no other knowledge can
        // start there and walk outwards.
        let machine = push(
            &mut entities,
            Entity::new(keys::machine(), EntityClass::Machine, None),
        );

        let mut packages: Vec<u32> = t.physical_cores.iter().map(|c| c.package_id).collect();
        packages.sort_unstable();
        packages.dedup();
        let package_rows: Vec<(u32, u32)> = packages
            .iter()
            .map(|pkg| {
                let row = push(
                    &mut entities,
                    Entity::new(keys::package(*pkg), EntityClass::Package, Some(*pkg)),
                );
                relations.push(Relation::new(machine, row, RelationKind::Contains));
                (*pkg, row)
            })
            .collect();
        let package_row = |pkg: u32| -> Option<u32> {
            package_rows
                .iter()
                .find(|(p, _)| *p == pkg)
                .map(|(_, r)| *r)
        };

        let numa_rows: Vec<(u32, u32)> = t
            .numa_nodes
            .iter()
            .map(|node| {
                let row = push(
                    &mut entities,
                    Entity::new(
                        keys::numa_node(node.id),
                        EntityClass::NumaNode,
                        Some(node.id),
                    ),
                );
                relations.push(Relation::new(machine, row, RelationKind::Contains));
                (node.id, row)
            })
            .collect();

        // Physical cores, then logical CPUs. Cores first so that a containment
        // walk from the machine reaches a core before the CPUs inside it.
        let mut core_rows: Vec<(u32, u32)> = Vec::new();
        for core in &t.physical_cores {
            let row = push(
                &mut entities,
                Entity::new(
                    keys::physical_core(core.package_id, core.core_id),
                    EntityClass::PhysicalCore,
                    Some(core.core_id),
                ),
            );
            if let Some(pkg) = package_row(core.package_id) {
                relations.push(Relation::new(pkg, row, RelationKind::Contains));
            }
            core_rows.push((core.id, row));
        }
        let core_row =
            |id: u32| -> Option<u32> { core_rows.iter().find(|(c, _)| *c == id).map(|(_, r)| *r) };

        let mut cpu_rows: Vec<(u32, u32)> = Vec::new();
        for cpu in t.logical_cpus.iter().filter(|c| c.online) {
            let row = push(
                &mut entities,
                Entity::new(
                    keys::logical_cpu(cpu.id),
                    EntityClass::LogicalCpu,
                    Some(cpu.id),
                ),
            );
            if let Some(core) = core_row(cpu.physical) {
                relations.push(Relation::new(core, row, RelationKind::Contains));
            }
            cpu_rows.push((cpu.id, row));
        }
        let cpu_row =
            |id: u32| -> Option<u32> { cpu_rows.iter().find(|(c, _)| *c == id).map(|(_, r)| *r) };

        // SMT siblinghood, emitted in both directions so a consumer never has
        // to guess which way an edge was written.
        for cpu in t.logical_cpus.iter().filter(|c| c.online) {
            let Some(from) = cpu_row(cpu.id) else {
                continue;
            };
            for sibling in &cpu.smt_siblings {
                if let Some(to) = cpu_row(*sibling) {
                    relations.push(Relation::new(from, to, RelationKind::SmtSibling));
                }
            }
        }

        // NUMA locality.
        for (node_id, node_row) in &numa_rows {
            let Some(node) = t.numa_nodes.iter().find(|n| n.id == *node_id) else {
                continue;
            };
            for cpu in &node.cpus {
                if let Some(row) = cpu_row(*cpu) {
                    relations.push(Relation::new(*node_row, row, RelationKind::NumaLocal));
                }
            }
        }

        // Caches. Each becomes an entity, anchored to the lowest core id among
        // its sharers so that its identity does not depend on how many of its
        // CPUs happen to be online elsewhere in the machine.
        for cache in &t.caches {
            let anchor_cpu = cache.shared_cpus.iter().copied().min();
            let Some(anchor_cpu) = anchor_cpu else {
                continue;
            };
            let anchor_core = t.core_of_cpu(anchor_cpu);
            let (package_id, anchor_core_id) = match anchor_core {
                Some(core) => (core.package_id, core.core_id),
                None => (0, anchor_cpu),
            };
            let kind = cache_kind_key(cache.kind);
            let row = push(
                &mut entities,
                Entity::new(
                    keys::cache(package_id, cache.level, kind, anchor_core_id),
                    EntityClass::Cache,
                    Some(cache.level as u32),
                ),
            );
            if let Some(pkg) = package_row(package_id) {
                relations.push(Relation::new(pkg, row, RelationKind::Contains));
            }
            for cpu in &cache.shared_cpus {
                if let Some(cpu_row) = cpu_row(*cpu) {
                    relations.push(
                        Relation::new(row, cpu_row, RelationKind::CacheMember)
                            .with(0, cache.level as f64)
                            .with(1, cache.size_bytes.map(|b| b as f64).unwrap_or(f64::NAN))
                            .with(2, cache.shared_cpus.len() as f64),
                    );
                }
            }
        }

        (entities, relations)
    }
}

fn push(entities: &mut Vec<Entity>, entity: Entity) -> u32 {
    entities.push(entity);
    (entities.len() - 1) as u32
}

/// Stable short name for a cache kind, used in natural keys. Changing these
/// strings changes every cache entity's identity, so they are frozen.
fn cache_kind_key(kind: CacheKind) -> &'static str {
    match kind {
        CacheKind::L1Data => "data",
        CacheKind::L1Instruction => "instruction",
        CacheKind::L2Unified | CacheKind::L3Unified => "unified",
        CacheKind::Other => "other",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::fake_topology;
    use corescout_mirror::relation::RelationView;

    fn structure() -> (Vec<Entity>, Vec<Relation>) {
        Substrate::new(
            fake_topology(),
            Roots::new("/nonexistent/sys", "/nonexistent/proc"),
        )
        .structure()
    }

    #[test]
    fn the_machine_is_always_row_zero() {
        let (entities, _) = structure();
        assert_eq!(entities[0].class_hint, EntityClass::Machine);
    }

    #[test]
    fn every_structural_component_becomes_an_entity() {
        let (entities, _) = structure();
        let count = |class: EntityClass| entities.iter().filter(|e| e.class_hint == class).count();
        assert_eq!(count(EntityClass::Machine), 1);
        assert_eq!(count(EntityClass::Package), 1);
        assert_eq!(count(EntityClass::NumaNode), 1);
        assert_eq!(count(EntityClass::PhysicalCore), 2);
        assert_eq!(count(EntityClass::LogicalCpu), 4);
        assert_eq!(count(EntityClass::Cache), 3, "two L2s and one L3");
    }

    #[test]
    fn entity_keys_are_unique() {
        let (entities, _) = structure();
        let mut keys: Vec<&str> = entities.iter().map(|e| e.key.as_str()).collect();
        keys.sort_unstable();
        let before = keys.len();
        keys.dedup();
        assert_eq!(keys.len(), before, "duplicate entity key");
    }

    #[test]
    fn identity_is_derived_from_the_key() {
        let (entities, _) = structure();
        for entity in &entities {
            assert_eq!(entity.id, corescout_mirror::EntityId::derive(&entity.key));
        }
    }

    #[test]
    fn containment_forms_a_tree_from_the_machine() {
        let (entities, relations) = structure();
        let view = RelationView::new(&relations);
        // Every entity except the machine has exactly one container, except
        // logical CPUs which are contained by their core only.
        for row in 1..entities.len() as u32 {
            let containers = view
                .to(row)
                .filter(|r| r.kind == RelationKind::Contains)
                .count();
            assert!(
                containers <= 1,
                "entity {} has {} containers",
                entities[row as usize].key,
                containers
            );
        }
        assert!(view.from(0).count() >= 2, "the machine contains things");
    }

    #[test]
    fn smt_siblings_are_symmetric_edges() {
        let (entities, relations) = structure();
        let view = RelationView::new(&relations);
        let row_of = |key: &str| entities.iter().position(|e| e.key == key).unwrap() as u32;
        let cpu0 = row_of("cpu/0");
        let cpu2 = row_of("cpu/2");
        assert!(view.connected(cpu0, cpu2, RelationKind::SmtSibling));
        assert!(view.connected(cpu2, cpu0, RelationKind::SmtSibling));
    }

    #[test]
    fn cache_membership_carries_level_and_size() {
        let (entities, relations) = structure();
        let view = RelationView::new(&relations);
        let l3_row = entities
            .iter()
            .position(|e| e.key.contains("L3"))
            .expect("an L3 entity") as u32;
        let members: Vec<&Relation> = view
            .from(l3_row)
            .filter(|r| r.kind == RelationKind::CacheMember)
            .collect();
        assert_eq!(members.len(), 4, "the fixture L3 is shared by four CPUs");
        assert_eq!(members[0].attributes[0], 3.0, "level in slot 0");
        assert_eq!(
            members[0].attributes[1],
            (32 << 20) as f64,
            "size in slot 1"
        );
        assert_eq!(members[0].attributes[2], 4.0, "sharer count in slot 2");
    }

    #[test]
    fn offline_cpus_do_not_become_entities() {
        // The mirror reflects what is, and an offline CPU is not running.
        let mut topology = fake_topology();
        topology.logical_cpus[3].online = false;
        topology.online_cpus = vec![0, 1, 2];
        topology.offline_cpus = vec![3];
        let (entities, _) = Substrate::new(topology, Roots::system()).structure();
        assert!(!entities.iter().any(|e| e.key == "cpu/3"));
        assert!(entities.iter().any(|e| e.key == "cpu/1"));
    }

    #[test]
    fn structure_is_deterministic_across_runs() {
        // Row indices must be reproducible, or cached indices and recorded
        // snapshots both break.
        let (a_entities, a_relations) = structure();
        let (b_entities, b_relations) = structure();
        assert_eq!(a_entities, b_entities);
        assert_eq!(a_relations.len(), b_relations.len());
    }
}
