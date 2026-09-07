//! Shared test scaffolding: a builder for synthetic `/sys` and `/proc` trees.
//!
//! The point of this module is to let the real Linux topology parser be tested
//! against topologies nobody on the project physically owns, on a host that may
//! not even be Linux. Every file it writes is one the kernel really exposes,
//! with the kernel's own formatting, including the trailing newlines that a
//! naive parser trips over.

#![allow(dead_code)]

pub mod synthetic;

use std::path::{Path, PathBuf};

/// A temporary directory that deletes itself when dropped.
pub struct TempTree {
    root: PathBuf,
}

impl TempTree {
    pub fn new(tag: &str) -> TempTree {
        // A process id plus a nanosecond timestamp is enough to keep parallel
        // test binaries from colliding, without pulling in a tempfile crate.
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let root = std::env::temp_dir().join(format!(
            "corescout-test-{tag}-{}-{stamp}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).expect("create temp tree");
        TempTree { root }
    }

    pub fn path(&self) -> &Path {
        &self.root
    }

    pub fn sys(&self) -> PathBuf {
        self.root.join("sys")
    }

    pub fn proc(&self) -> PathBuf {
        self.root.join("proc")
    }

    /// Write a file, creating parent directories, appending the trailing
    /// newline that sysfs always emits.
    pub fn write(&self, relative: &str, contents: &str) {
        let path = self.root.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent");
        }
        std::fs::write(&path, format!("{contents}\n")).expect("write file");
    }
}

impl Drop for TempTree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// Description of a synthetic machine, before it is written to disk.
pub struct FakeMachine {
    pub model: String,
    pub vendor: String,
    /// `(logical cpu, package, core_id, numa node, siblings)`.
    pub cpus: Vec<FakeCpu>,
    pub present: String,
    pub online: String,
    pub offline: String,
    /// `(node id, cpu list, MemTotal kB)`.
    pub nodes: Vec<(u32, String, u64)>,
    /// CPUs the kernel classifies as P-cores, for hybrid machines.
    pub p_cores: Option<String>,
    pub e_cores: Option<String>,
    /// When true, write only structure and none of the interfaces the
    /// observation layer samples. Models a locked-down container.
    pub bare: bool,
}

pub struct FakeCpu {
    pub id: u32,
    pub package: u32,
    pub core_id: u32,
    /// Contents of `topology/core_cpus_list`.
    pub siblings: String,
    /// `(index, level, type, size, shared_cpu_list)`.
    pub caches: Vec<(u32, u8, &'static str, &'static str, String)>,
    /// `(current, min, max)` in kHz.
    pub freq: Option<(u64, u64, u64)>,
    /// ACPI CPPC `highest_perf`.
    pub highest_perf: Option<u32>,
}

impl FakeMachine {
    /// A 4-core, 8-thread, single-socket machine with two NUMA nodes, one
    /// offlined CPU and firmware preferred-core hints.
    ///
    /// The CPU numbering follows the usual Linux convention where the second
    /// SMT thread of core N is CPU N + 4, which is exactly the layout that
    /// makes people pin two "different" threads onto one physical core by
    /// accident.
    pub fn typical() -> FakeMachine {
        let mut cpus = Vec::new();
        // CPU 7 is offline, so core 3 has only one usable thread.
        for id in 0..7u32 {
            let core_id = id % 4;
            let sibling = if core_id == 3 {
                format!("{core_id}")
            } else {
                format!("{core_id},{}", core_id + 4)
            };
            let node = if core_id < 2 { 0 } else { 1 };
            let l3_cpus = if node == 0 { "0,1,4,5" } else { "2,3,6" };
            cpus.push(FakeCpu {
                id,
                package: 0,
                core_id,
                siblings: sibling.clone(),
                caches: vec![
                    (0, 1, "Data", "32K", sibling.clone()),
                    (1, 1, "Instruction", "32K", sibling.clone()),
                    (2, 2, "Unified", "512K", sibling.clone()),
                    (3, 3, "Unified", "16M", l3_cpus.to_string()),
                ],
                // Core 1 is clocked highest, to make sure nothing infers
                // "best core" from frequency alone.
                freq: Some((
                    if core_id == 1 { 4_800_000 } else { 3_600_000 },
                    550_000,
                    5_200_000,
                )),
                // The firmware's favourite is core 2 (CPUs 2 and 6).
                highest_perf: Some(match core_id {
                    2 => 255,
                    1 => 240,
                    0 => 200,
                    _ => 190,
                }),
            });
        }

        FakeMachine {
            model: "Synthetic Core Ultra 9 000X".to_string(),
            vendor: "GenuineTest".to_string(),
            cpus,
            present: "0-7".to_string(),
            online: "0-6".to_string(),
            offline: "7".to_string(),
            nodes: vec![
                (0, "0,1,4,5".to_string(), 16_000_000),
                (1, "2,3,6".to_string(), 16_000_000),
            ],
            p_cores: None,
            e_cores: None,
            bare: false,
        }
    }

    /// A minimal machine: one core, no SMT, no NUMA, no cache directory, no
    /// cpufreq. This is what a small VM or a restricted container looks like,
    /// and it must not make the parser fall over.
    pub fn minimal() -> FakeMachine {
        FakeMachine {
            model: "Synthetic VM CPU".to_string(),
            vendor: "GenuineTest".to_string(),
            cpus: vec![FakeCpu {
                id: 0,
                package: 0,
                core_id: 0,
                siblings: "0".to_string(),
                caches: vec![],
                freq: None,
                highest_perf: None,
            }],
            present: "0".to_string(),
            online: "0".to_string(),
            offline: String::new(),
            nodes: vec![],
            p_cores: None,
            e_cores: None,
            bare: true,
        }
    }

    /// A hybrid part: two P-cores with SMT, four E-cores without.
    pub fn hybrid() -> FakeMachine {
        let mut cpus = Vec::new();
        // P-cores: physical 0 and 1, CPUs 0/1 and 2/3.
        for (id, core_id, sibling) in [
            (0u32, 0u32, "0,1"),
            (1, 0, "0,1"),
            (2, 1, "2,3"),
            (3, 1, "2,3"),
        ] {
            cpus.push(FakeCpu {
                id,
                package: 0,
                core_id,
                siblings: sibling.to_string(),
                caches: vec![(0, 1, "Data", "48K", sibling.to_string())],
                freq: Some((5_000_000, 800_000, 5_400_000)),
                highest_perf: Some(255),
            });
        }
        // E-cores: physical 2..5, CPUs 4..7, no SMT, sharing an L2 in a cluster.
        for (offset, id) in (4u32..8).enumerate() {
            cpus.push(FakeCpu {
                id,
                package: 0,
                core_id: 2 + offset as u32,
                siblings: format!("{id}"),
                caches: vec![
                    (0, 1, "Data", "32K", format!("{id}")),
                    (1, 2, "Unified", "4M", "4-7".to_string()),
                ],
                freq: Some((3_800_000, 800_000, 4_000_000)),
                highest_perf: Some(128),
            });
        }

        FakeMachine {
            model: "Synthetic Hybrid 12900".to_string(),
            vendor: "GenuineTest".to_string(),
            cpus,
            present: "0-7".to_string(),
            online: "0-7".to_string(),
            offline: String::new(),
            nodes: vec![(0, "0-7".to_string(), 32_000_000)],
            p_cores: Some("0-3".to_string()),
            e_cores: Some("4-7".to_string()),
            bare: false,
        }
    }

    /// Materialise the machine as a filesystem tree.
    pub fn write(&self, tag: &str) -> TempTree {
        let tree = TempTree::new(tag);
        let cpu_dir = "sys/devices/system/cpu";

        tree.write(&format!("{cpu_dir}/present"), &self.present);
        tree.write(&format!("{cpu_dir}/online"), &self.online);
        if !self.offline.is_empty() {
            tree.write(&format!("{cpu_dir}/offline"), &self.offline);
        }
        if let Some(p) = &self.p_cores {
            tree.write("sys/devices/cpu_core/cpus", p);
        }
        if let Some(e) = &self.e_cores {
            tree.write("sys/devices/cpu_atom/cpus", e);
        }

        for cpu in &self.cpus {
            let base = format!("{cpu_dir}/cpu{}", cpu.id);
            tree.write(
                &format!("{base}/topology/physical_package_id"),
                &cpu.package.to_string(),
            );
            tree.write(
                &format!("{base}/topology/core_id"),
                &cpu.core_id.to_string(),
            );
            tree.write(&format!("{base}/topology/core_cpus_list"), &cpu.siblings);
            tree.write(
                &format!("{base}/topology/thread_siblings_list"),
                &cpu.siblings,
            );

            for (index, level, kind, size, shared) in &cpu.caches {
                let dir = format!("{base}/cache/index{index}");
                tree.write(&format!("{dir}/level"), &level.to_string());
                tree.write(&format!("{dir}/type"), kind);
                tree.write(&format!("{dir}/size"), size);
                tree.write(&format!("{dir}/coherency_line_size"), "64");
                tree.write(&format!("{dir}/ways_of_associativity"), "8");
                tree.write(&format!("{dir}/shared_cpu_list"), shared);
            }

            if let Some((cur, min, max)) = cpu.freq {
                tree.write(
                    &format!("{base}/cpufreq/scaling_cur_freq"),
                    &cur.to_string(),
                );
                tree.write(
                    &format!("{base}/cpufreq/cpuinfo_min_freq"),
                    &min.to_string(),
                );
                tree.write(
                    &format!("{base}/cpufreq/cpuinfo_max_freq"),
                    &max.to_string(),
                );
            }
            if let Some(perf) = cpu.highest_perf {
                tree.write(&format!("{base}/acpi_cppc/highest_perf"), &perf.to_string());
                tree.write(&format!("{base}/acpi_cppc/nominal_perf"), "180");
            }
        }

        for (id, cpulist, mem) in &self.nodes {
            tree.write(
                &format!("sys/devices/system/node/node{id}/cpulist"),
                cpulist,
            );
            tree.write(
                &format!("sys/devices/system/node/node{id}/meminfo"),
                &format!("Node {id} MemTotal:       {mem} kB\nNode {id} MemFree: 100 kB"),
            );
        }

        // /proc/cpuinfo, in the kernel's own format.
        let mut cpuinfo = String::new();
        for cpu in &self.cpus {
            cpuinfo.push_str(&format!(
                "processor\t: {}\nvendor_id\t: {}\nmodel name\t: {}\ncore id\t\t: {}\n\n",
                cpu.id, self.vendor, self.model, cpu.core_id
            ));
        }
        tree.write("proc/cpuinfo", cpuinfo.trim_end());

        if !self.bare {
            self.write_observable_state(&tree);
        }
        tree
    }

    /// Write the interfaces the observation layer samples.
    ///
    /// Structure alone is not enough to exercise a mirror: the sensors need
    /// something to read. These are the same files a Linux machine exposes,
    /// with the same formats, so the production sensors run unmodified against
    /// this tree on any host.
    ///
    /// One real interface is missing: `/sys/class/powercap/intel-rapl:0`. Its
    /// directory name contains a colon, which cannot appear in a filename on
    /// Windows, where these tests also run. The power sensor is therefore
    /// exercised on Linux only.
    fn write_observable_state(&self, tree: &TempTree) {
        let cpu_dir = "sys/devices/system/cpu";
        let online: Vec<u32> = self.cpus.iter().map(|c| c.id).collect();

        // cpuidle: two states per CPU, with cumulative counters.
        for cpu in &self.cpus {
            for state in 0..2u32 {
                let base = format!("{cpu_dir}/cpu{}/cpuidle/state{state}", cpu.id);
                tree.write(
                    &format!("{base}/name"),
                    if state == 0 { "POLL" } else { "C1" },
                );
                tree.write(
                    &format!("{base}/usage"),
                    &(1000 * (cpu.id + 1) + state).to_string(),
                );
                tree.write(
                    &format!("{base}/time"),
                    &(5_000_000 * (cpu.id as u64 + 1) + state as u64).to_string(),
                );
            }
        }

        // A package thermal zone, plus an hwmon chip with a per-core sensor.
        tree.write("sys/class/thermal/thermal_zone0/type", "x86_pkg_temp");
        tree.write("sys/class/thermal/thermal_zone0/temp", "47000");
        tree.write("sys/class/hwmon/hwmon0/name", "coretemp");
        tree.write("sys/class/hwmon/hwmon0/temp1_input", "52000");
        tree.write("sys/class/hwmon/hwmon0/temp1_label", "Core 0");
        tree.write("sys/class/hwmon/hwmon0/temp2_input", "49000");
        tree.write("sys/class/hwmon/hwmon0/temp2_label", "Core 1");

        // /proc/stat, in USER_HZ ticks, with the aggregate line first.
        let mut stat = String::from("cpu  1000 20 3000 400000 50 6 70 8 0 0\n");
        for cpu in &self.cpus {
            let base = (cpu.id as u64 + 1) * 100;
            stat.push_str(&format!(
                "cpu{} {} {} {} {} {} {} {} {} 0 0\n",
                cpu.id,
                base,
                base / 10,
                base * 2,
                base * 40,
                base / 20,
                base / 50,
                base / 25,
                base / 100,
            ));
        }
        stat.push_str("intr 123456 1 2 3\nctxt 987654\nbtime 1700000000\nprocesses 4242\n");
        stat.push_str("procs_running 3\nprocs_blocked 0");
        tree.write("proc/stat", &stat);

        // /proc/schedstat, version 15 shape.
        let mut schedstat = String::from("version 15\ntimestamp 4294900000\n");
        for cpu in &self.cpus {
            schedstat.push_str(&format!(
                "cpu{} 0 0 0 0 0 0 {} {} {}\n",
                cpu.id,
                100_000_000 * (cpu.id as u64 + 1),
                2_000_000 * (cpu.id as u64 + 1),
                500 * (cpu.id as u64 + 1),
            ));
            schedstat.push_str("domain0 00000000,00000003 0 0 0\n");
        }
        tree.write("proc/schedstat", schedstat.trim_end());

        // The load averages here are deliberately non-zero: a test asserts they
        // do not reach the mirror.
        tree.write("proc/loadavg", "0.52 0.61 0.70 3/1234 5678");

        // /proc/interrupts, with the load concentrated on CPU 0 and a NIC queue
        // pinned to the second CPU.
        let mut header = String::new();
        for cpu in &online {
            header.push_str(&format!("           CPU{cpu}"));
        }
        let mut interrupts = format!("{header}\n");
        let row = |label: &str, values: Vec<u64>, tail: &str| {
            let mut line = format!("{label:>4}:");
            for value in values {
                line.push_str(&format!(" {value:>10}"));
            }
            line.push_str(&format!("   {tail}\n"));
            line
        };
        interrupts.push_str(&row(
            "0",
            online
                .iter()
                .map(|c| if *c == 0 { 31 } else { 0 })
                .collect(),
            "IO-APIC   2-edge      timer",
        ));
        interrupts.push_str(&row(
            "124",
            online
                .iter()
                .map(|c| if *c == 1 { 88_192 } else { 0 })
                .collect(),
            "PCI-MSI 524288-edge   eth0-rx-0",
        ));
        interrupts.push_str(&row(
            "LOC",
            online.iter().map(|c| 1_000_000 * (*c as u64 + 1)).collect(),
            "Local timer interrupts",
        ));
        tree.write("proc/interrupts", interrupts.trim_end());
    }
}
