//! Cross-mapping tests of the self-state plane. Linux only.
//!
//! These need a real shared mapping: a writer with `PROT_READ|PROT_WRITE` and a
//! reader with `PROT_READ` over the same inode. That is the arrangement the
//! plane exists to support and the one the in-process unit tests cannot
//! reproduce.

#![cfg(target_os = "linux")]

mod common;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use corescout_mirror::plane::{PlaneMemory, PlaneReader, PlaneWriter};
use corescout_mirror::MirrorSnapshot;
use corescout_substrate::discovery::{Roots, Substrate};
use corescout_substrate::observation::default_sensors;
use corescout_substrate::platform::linux::sysfs::Sysfs;
use corescout_substrate::Reflector;

use common::FakeMachine;

fn plane_path(tag: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "corescout-plane-{tag}-{}.plane",
        std::process::id()
    ))
}

/// A mirror over a synthetic machine, so these tests do not depend on what the
/// host's own hardware happens to expose.
fn synthetic_snapshot(tag: &str) -> (common::TempTree, MirrorSnapshot) {
    let tree = FakeMachine::typical().write(tag);
    let topology = Sysfs::with_roots(tree.sys(), tree.proc())
        .read_topology(None)
        .expect("fixture topology");
    let substrate = Substrate::new(topology, Roots::new(tree.sys(), tree.proc()));
    let mut mirror = Reflector::build(substrate, default_sensors()).expect("mirror");
    mirror.observe();
    (tree, mirror.snapshot())
}

#[test]
fn a_second_mapping_sees_what_the_writer_published() {
    let (_tree, snapshot) = synthetic_snapshot("shared");
    let path = plane_path("shared");
    let _ = std::fs::remove_file(&path);

    let mut writer = {
        let target = path.clone();
        PlaneWriter::new(move |len| PlaneMemory::create(&target, len), &snapshot).expect("writer")
    };
    writer.publish(&snapshot).expect("publish");

    let reader = PlaneReader::open(&path).expect("open plane");
    let back = reader.read_snapshot().expect("read");

    assert_eq!(back.epoch, snapshot.epoch);
    assert_eq!(back.sequence, snapshot.sequence);
    assert_eq!(back.entities, snapshot.entities);
    assert_eq!(back.relations, snapshot.relations);
    assert_eq!(back.state, snapshot.state);
    assert_eq!(
        back.lookup("cpu/1", "cpu.frequency.current"),
        snapshot.lookup("cpu/1", "cpu.frequency.current")
    );

    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_consumer_mapping_is_not_writable() {
    // The truth boundary. A consumer holds a PROT_READ mapping and cannot edit
    // its own reflection even if it tries.
    let (_tree, snapshot) = synthetic_snapshot("readonly");
    let path = plane_path("readonly");
    let _ = std::fs::remove_file(&path);

    let mut writer = {
        let target = path.clone();
        PlaneWriter::new(move |len| PlaneMemory::create(&target, len), &snapshot).expect("writer")
    };
    writer.publish(&snapshot).expect("publish");

    let memory = PlaneMemory::open_read_only(&path).expect("read-only map");
    assert!(
        !memory.is_writable(),
        "a consumer mapping must be read-only"
    );

    // And a writer cannot be constructed over it.
    let err = PlaneWriter::new(move |_| PlaneMemory::open_read_only(&path), &snapshot)
        .expect_err("publishing into a read-only plane must fail");
    assert!(err.to_string().contains("read-only"));
}

#[test]
fn updates_are_visible_through_an_already_open_mapping() {
    // A consumer maps once and keeps reading. It must see new snapshots without
    // reopening anything: that is what makes reading the mirror feel like
    // reading memory.
    let (_tree, mut snapshot) = synthetic_snapshot("live");
    let path = plane_path("live");
    let _ = std::fs::remove_file(&path);

    let mut writer = {
        let target = path.clone();
        PlaneWriter::new(move |len| PlaneMemory::create(&target, len), &snapshot).expect("writer")
    };
    writer.publish(&snapshot).expect("publish");

    let reader = PlaneReader::open(&path).expect("open plane");
    let first = reader.read_state().expect("first read");

    snapshot.sequence += 1;
    snapshot.monotonic_ns += 1_000_000;
    writer.publish(&snapshot).expect("republish");

    let second = reader.read_state().expect("second read");
    assert_eq!(second.sequence, first.sequence + 1);
    assert!(second.monotonic_ns > first.monotonic_ns);

    let _ = std::fs::remove_file(&path);
}

#[test]
fn concurrent_reads_never_observe_a_torn_snapshot() {
    // The seqlock, under contention.
    //
    // Every published snapshot fills the whole matrix with one value: the
    // sequence number. A reader that saw a partial write would see two
    // different values in one matrix, which is exactly what the seqlock exists
    // to prevent and what a naive shared buffer would produce constantly.
    let (_tree, base) = synthetic_snapshot("torn");
    let path = plane_path("torn");
    let _ = std::fs::remove_file(&path);

    let mut writer = {
        let target = path.clone();
        PlaneWriter::new(move |len| PlaneMemory::create(&target, len), &base).expect("writer")
    };

    let stop = Arc::new(AtomicBool::new(false));
    let writer_stop = Arc::clone(&stop);
    let publisher = std::thread::spawn(move || {
        let mut snapshot = base.clone();
        let cells = snapshot.state.rows() * snapshot.state.cols();
        let mut generation = 0u64;
        while !writer_stop.load(Ordering::Relaxed) {
            generation += 1;
            let uniform = vec![generation as f64; cells];
            snapshot.state.copy_from_slice(&uniform);
            snapshot.sequence = generation;
            writer.publish(&snapshot).expect("publish");
        }
        generation
    });

    let reader = PlaneReader::open(&path).expect("open plane");
    let mut reads = 0;
    let mut distinct_generations = 0;
    let mut last = f64::NAN;
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(300);

    while std::time::Instant::now() < deadline {
        let state = reader.read_state().expect("consistent read");
        if state.values.is_empty() {
            break;
        }
        let first = state.values[0];
        assert!(
            state.values.iter().all(|v| *v == first),
            "torn read: the matrix contained more than one generation"
        );
        // The sequence number must belong to the same generation as the data.
        assert_eq!(
            state.sequence as f64, first,
            "the header and the matrix disagreed about which snapshot this is"
        );
        if first != last {
            distinct_generations += 1;
            last = first;
        }
        reads += 1;
    }

    stop.store(true, Ordering::Relaxed);
    let published = publisher.join().expect("publisher");

    assert!(reads > 10, "only managed {reads} reads");
    assert!(published > 10, "only published {published} snapshots");
    assert!(
        distinct_generations > 1,
        "the reader never saw the plane change, so nothing was really tested"
    );

    let _ = std::fs::remove_file(&path);
}

#[test]
fn reading_the_plane_is_cheap() {
    // Not a benchmark, a sanity check on the claim the design rests on: reading
    // the whole machine's state should cost microseconds, not milliseconds, and
    // should not involve parsing anything.
    let (_tree, snapshot) = synthetic_snapshot("cost");
    let path = plane_path("cost");
    let _ = std::fs::remove_file(&path);

    let mut writer = {
        let target = path.clone();
        PlaneWriter::new(move |len| PlaneMemory::create(&target, len), &snapshot).expect("writer")
    };
    writer.publish(&snapshot).expect("publish");
    let reader = PlaneReader::open(&path).expect("open plane");

    // Warm the mapping.
    for _ in 0..100 {
        reader.read_state().expect("read");
    }
    let started = std::time::Instant::now();
    const READS: u32 = 1000;
    for _ in 0..READS {
        std::hint::black_box(reader.read_state().expect("read"));
    }
    let per_read = started.elapsed().as_secs_f64() / READS as f64;

    assert!(
        per_read < 1e-3,
        "a plane read took {:.1} us; the design assumes memcpy speed",
        per_read * 1e6
    );

    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_plane_from_a_different_machine_shape_is_refused() {
    // Publishing a differently shaped snapshot into an existing plane would
    // silently reinterpret every row. It must fail loudly instead.
    let (_tree, snapshot) = synthetic_snapshot("shape");
    let path = plane_path("shape");
    let _ = std::fs::remove_file(&path);

    let mut writer = {
        let target = path.clone();
        PlaneWriter::new(move |len| PlaneMemory::create(&target, len), &snapshot).expect("writer")
    };
    let mut changed = snapshot.clone();
    changed.entities.pop();
    assert!(writer.publish(&changed).is_err());

    let _ = std::fs::remove_file(&path);
}
