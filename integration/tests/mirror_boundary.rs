//! Architectural tests: the layer boundaries, enforced.
//!
//! These read the source tree and assert things about which module may mention
//! which. That is a blunt instrument, and it catches the failure that actually
//! happens: someone needs one more number, the easiest way to get it is a
//! direct `/sys` read from the wrong layer, and the boundary quietly stops
//! being a boundary.
//!
//! A design document describing a boundary is a wish. A test is a boundary.

use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    // This package lives at <workspace>/integration.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the workspace root")
        .to_path_buf()
}

/// The crates a package declares as dependencies, and everything those pull in.
///
/// **Transitive on purpose.** An earlier version of this checked only the
/// declared dependencies and passed while the guarantee was broken: the
/// consumer package did not name `corescout-substrate`, but it named the
/// human-output crate, which named the analysis crate, which named the
/// substrate. The boundary held in every manifest and leaked through the graph.
///
/// A closure over the workspace is the only version of this check that means
/// anything, because linking is transitive and so is the ability to call.
fn declared_dependencies(package: &str) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    let mut queue = vec![package.to_string()];
    while let Some(current) = queue.pop() {
        for name in direct_dependencies(&current) {
            if seen.contains(&name) {
                continue;
            }
            seen.push(name.clone());
            // Only workspace crates have a directory to recurse into; third
            // party crates cannot reach our hardware layer by definition.
            if let Some(dir) = workspace_dir(&name) {
                queue.push(dir);
            }
        }
    }
    seen.sort();
    seen
}

/// The `corescout-*` crates one package names directly.
fn direct_dependencies(package: &str) -> Vec<String> {
    let manifest = read(&format!("{package}/Cargo.toml"));
    let mut deps = Vec::new();
    let mut in_deps = false;
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_deps = line.contains("dependencies");
            continue;
        }
        if in_deps && !line.starts_with('#') {
            if let Some(name) = line.split(['=', ' ']).next() {
                if !name.is_empty() {
                    deps.push(name.to_string());
                }
            }
        }
    }
    deps
}

/// Where a workspace crate's manifest lives, by package name.
fn workspace_dir(package: &str) -> Option<String> {
    let short = package.strip_prefix("corescout-")?;
    // `selfmodel` and friends keep their directory name; check both spellings.
    [format!("crates/{short}"), format!("apps/{package}")]
        .into_iter()
        .find(|candidate| repo_root().join(candidate).join("Cargo.toml").exists())
}

fn read(relative: &str) -> String {
    let path = repo_root().join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// Every `.rs` file under a directory, with its repo-relative path.
fn sources_under(relative: &str) -> Vec<(PathBuf, String)> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }
    let mut files = Vec::new();
    walk(&repo_root().join(relative), &mut files);
    assert!(!files.is_empty(), "no sources found under {relative}");
    files
        .into_iter()
        .map(|path| {
            let text = std::fs::read_to_string(&path).expect("read source");
            (path, text)
        })
        .collect()
}

/// Strip `#[cfg(test)]` modules and doc comments before checking for forbidden
/// references.
///
/// Tests legitimately construct things from other layers, and documentation
/// legitimately *names* other layers; neither is a dependency. Without this the
/// checks below would be unusable and would get deleted, which is worse than
/// them being slightly approximate.
fn code_only(text: &str) -> String {
    let mut out = String::new();
    let mut in_tests = false;
    let mut depth = 0i32;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("//") {
            continue;
        }
        if !in_tests && trimmed.starts_with("#[cfg(test)]") {
            in_tests = true;
            depth = 0;
            continue;
        }
        if in_tests {
            depth += line.matches('{').count() as i32;
            depth -= line.matches('}').count() as i32;
            if depth <= 0 && line.contains('}') {
                in_tests = false;
            }
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

#[test]
fn mirror_inspect_has_no_hardware_access() {
    // The whole point of the second binary: it proves the mirror is a real
    // abstraction by consuming it with nothing else.
    let source = code_only(&read("apps/mirror-tools/src/bin/mirror_inspect.rs"));
    for forbidden in [
        "/proc",
        "/sys",
        "perf_event",
        "observation::",
        "substrate::",
        "platform::",
        "experiment::",
        "libc::",
    ] {
        assert!(
            !source.contains(forbidden),
            "mirror-inspect must not reference `{forbidden}`; it may only read the plane"
        );
    }
    // What it *is* allowed to do.
    assert!(source.contains("PlaneReader"));
}

#[test]
fn mirror_inspect_only_reads() {
    let source = code_only(&read("apps/mirror-tools/src/bin/mirror_inspect.rs"));
    for forbidden in ["PlaneWriter", "publish(", "as_mut_slice"] {
        assert!(
            !source.contains(forbidden),
            "a consumer must not be able to write the mirror: found `{forbidden}`"
        );
    }
}

#[test]
fn the_mirror_does_not_depend_on_the_layers_above_it() {
    // Mirror(t) -> decision -> action -> machine -> Mirror(t+1).
    // The arrow never points backwards, and it is not allowed to in the code
    // either: the mirror cannot see memory, models, policy or experiments.
    for (path, text) in sources_under("crates/mirror/src") {
        let code = code_only(&text);
        for forbidden in [
            "crate::memory",
            "crate::model",
            "crate::agency",
            "crate::autonomy",
            "crate::analysis",
            "crate::experiment",
        ] {
            assert!(
                !code.contains(forbidden),
                "{} references `{forbidden}`; the mirror must not depend on a layer above it",
                path.display()
            );
        }
    }
}

#[test]
fn observation_does_not_depend_on_experiment_or_on_judgement() {
    // The rule that keeps observation from becoming probing.
    for (path, text) in sources_under("crates/substrate/src/observation") {
        let code = code_only(&text);
        for forbidden in [
            "crate::experiment",
            "crate::analysis",
            "crate::agency",
            "crate::autonomy",
            "crate::memory",
            "crate::model",
        ] {
            assert!(
                !code.contains(forbidden),
                "{} references `{forbidden}`; observation is passive and judges nothing",
                path.display()
            );
        }
    }
}

#[test]
fn no_sensor_writes_to_the_machine() {
    // Observation reads. Anything that sets, writes or spawns belongs in
    // agency or experiment.
    for (path, text) in sources_under("crates/substrate/src/observation") {
        let code = code_only(&text);
        for forbidden in [
            "sched_setaffinity",
            "fs::write",
            "OpenOptions",
            "Command::new",
            "set_len",
            "pin_current_thread",
        ] {
            assert!(
                !code.contains(forbidden),
                "{} contains `{forbidden}`; a sensor must not modify anything",
                path.display()
            );
        }
    }
}

#[test]
fn every_sensor_declares_its_observation_cost() {
    // Enforced by the type system already, but the point is important enough to
    // assert at the source level too: a new sensor cannot be added without
    // saying what looking at it costs.
    for (path, text) in sources_under("crates/substrate/src/observation") {
        if !text.contains("impl Sensor for") {
            continue;
        }
        for required in [
            "perturbation:",
            "physical_fact:",
            "uncertainty:",
            "max_rate_hz:",
        ] {
            assert!(
                text.contains(required),
                "{} implements Sensor without declaring `{required}`",
                path.display()
            );
        }
    }
}

#[test]
fn no_sensor_declares_material_perturbation() {
    // `Material` means observation changes hardware state to read it. Such a
    // thing is an experiment, not a reflection, and there is nowhere in the
    // mirror for it to live.
    for (path, text) in sources_under("crates/substrate/src/observation") {
        let code = code_only(&text);
        assert!(
            !code.contains("perturbation: Perturbation::Material"),
            "{} declares Material perturbation and belongs in experiment/",
            path.display()
        );
    }
}

#[test]
fn the_substrate_layer_stays_structural() {
    for (path, text) in sources_under("crates/substrate/src") {
        let code = code_only(&text);
        for forbidden in ["crate::experiment", "crate::analysis", "crate::autonomy"] {
            assert!(
                !code.contains(forbidden),
                "{} references `{forbidden}`",
                path.display()
            );
        }
    }
}

#[test]
fn the_layers_that_were_once_only_boundaries_are_now_implemented() {
    // This test used to assert the opposite: that `memory` and `autonomy`
    // contained nothing but a module doc, so that filling them in would force a
    // deliberate decision rather than drifting in a commit at a time. They have
    // since been filled in, so the tripwire has done its job and been replaced
    // by the check that matters now.
    for layer in [
        "crates/memory/src",
        "crates/autonomy/src",
        "crates/selfmodel/src",
        "crates/counterfactual/src",
        "crates/identity/src",
    ] {
        let files = sources_under(layer);
        let code: usize = files
            .iter()
            .map(|(_, text)| {
                code_only(text)
                    .lines()
                    .filter(|l| !l.trim().is_empty())
                    .count()
            })
            .sum();
        assert!(
            code > 100,
            "{layer} has {code} lines of code; it is declared but not built"
        );
    }
}

#[test]
fn mirror_observer_has_no_hardware_access() {
    // The experiment is void if the learner can peek at the machine directly.
    // Everything it reports must have come through the plane.
    let source = code_only(&read("apps/mirror-tools/src/bin/mirror_observer.rs"));
    for forbidden in [
        "/proc",
        "/sys",
        "perf_event",
        "observation::",
        "substrate::",
        "platform::",
        "experiment::",
        "libc::",
        "cpuid",
    ] {
        assert!(
            !source.contains(forbidden),
            "mirror-observer must not reference `{forbidden}`; it may only read the mirror"
        );
    }
    assert!(source.contains("PlaneReader") || source.contains("MirrorSnapshot"));
}

#[test]
fn the_model_layer_touches_only_the_mirror() {
    // The observer's algorithms must be reachable only from mirror data. If
    // `discover` could read a sysfs file, every result it produced would be
    // uninterpretable.
    for (path, text) in sources_under("crates/selfmodel/src") {
        let code = code_only(&text);
        for forbidden in [
            "crate::observation",
            "crate::substrate",
            "crate::platform",
            "crate::experiment",
            "crate::analysis",
            "crate::agency",
            "std::fs",
            "/proc",
            "/sys",
        ] {
            assert!(
                !code.contains(forbidden),
                "{} references `{forbidden}`; the model layer sees only the mirror",
                path.display()
            );
        }
    }
}

#[test]
fn the_unlabelled_lens_cannot_reach_past_itself() {
    // The A/B experiment measures nothing if the unlabelled path quietly looks
    // up a friendlier name. Every human name must come from `Lens`.
    let lens = read("crates/observer/src/lens.rs");
    assert!(
        lens.contains("Lens::Unlabelled => format!(\"entity_{row}\")"),
        "the unlabelled lens must synthesise names rather than pass keys through"
    );

    // And nothing else in the model may read an entity or channel key
    // directly: it all has to go through the lens.
    for (path, text) in sources_under("crates/selfmodel/src") {
        if path.ends_with("lens.rs") || path.ends_with("mod.rs") {
            continue;
        }
        let code = code_only(&text);
        for forbidden in [".key.clone()", "entity.key", "channel.key", "kind.label()"] {
            assert!(
                !code.contains(forbidden),
                "{} reads a name directly instead of going through the lens: `{forbidden}`",
                path.display()
            );
        }
    }
}

#[test]
fn the_read_only_boundary_is_enforced_by_the_kernel() {
    // Not by a `&` in an API. If this mapping ever becomes writable for
    // convenience, an agent could edit its own reflection.
    let source = read("crates/mirror/src/plane/memory.rs");
    assert!(
        source.contains("libc::PROT_READ,"),
        "the consumer mapping must be PROT_READ only"
    );
    assert!(
        source.contains("File::open(path)"),
        "the consumer must open the plane read-only"
    );
}

#[test]
fn the_experiment_layer_is_still_present_and_reclassified() {
    // Nothing was deleted in the refactor; the benchmarks moved.
    for expected in [
        "crates/experiment/src/benchmarks/mod.rs",
        "crates/experiment/src/benchmarks/workloads/compute.rs",
        "crates/experiment/src/benchmarks/workloads/memory.rs",
        "crates/experiment/src/benchmarks/workloads/latency.rs",
        "crates/experiment/src/benchmarks/stats.rs",
        "crates/analysis/src/lib.rs",
        "crates/analysis/src/placement.rs",
        "crates/agency/src/launch.rs",
    ] {
        assert!(
            repo_root().join(expected).exists(),
            "{expected} is missing; the refactor reclassifies, it does not delete"
        );
    }
}

#[test]
fn the_pure_consumers_cannot_link_hardware_access() {
    // The strongest form of the project's central claim, and the one the
    // workspace split exists to make available.
    //
    // Every other test in this file reads source text and asserts that nobody
    // *wrote* a hardware access. This one reads the manifest and asserts that
    // nobody *could*: `corescout-substrate` is the only crate that touches
    // /proc, /sys, CPUID, MSRs or perf, and it is not among the dependencies of
    // the package that holds mirror-inspect, mirror-observer, mirror-learn and
    // mirror-replay. Those programs' inability to peek at the machine is a
    // property of the linked executable, not a promise in a comment.
    let deps = declared_dependencies("apps/mirror-tools");
    assert!(
        !deps.is_empty(),
        "the manifest parser found nothing; it has probably drifted"
    );
    assert!(
        deps.iter().any(|d| d == "corescout-mirror"),
        "expected the mirror crate among {deps:?}"
    );
    for forbidden in [
        "corescout-substrate",
        "corescout-experiment",
        "corescout-analysis",
        "corescout-agency",
        "corescout-autonomy",
    ] {
        assert!(
            !deps.iter().any(|d| d == forbidden),
            "mirror-tools declares {forbidden}, so its programs can reach past the \
             mirror: {deps:?}"
        );
    }
}

#[test]
fn the_crates_that_reason_about_the_machine_do_not_depend_on_the_one_that_reads_it() {
    // The same argument one layer down. A model, an observer or a memory that
    // could read hardware directly would eventually do so for one field, and
    // the mirror would stop being the only route to the machine.
    for package in [
        "crates/memory",
        "crates/represent",
        "crates/selfmodel",
        "crates/observer",
        "crates/counterfactual",
        "crates/identity",
        "crates/intent",
    ] {
        let deps = declared_dependencies(package);
        assert!(
            !deps.iter().any(|d| d == "corescout-substrate"),
            "{package} depends on corescout-substrate; it should know the machine \
             only through the mirror. Declared: {deps:?}"
        );
    }
}

#[test]
fn only_the_substrate_crate_reaches_the_machine() {
    // Whichever crate does the reading, there must be exactly one. If a second
    // one starts opening /sys, the mirror is no longer the single source of
    // observation and the layering is decorative.
    let mut readers = Vec::new();
    for package in [
        "core",
        "mirror",
        "substrate",
        "memory",
        "represent",
        "selfmodel",
        "identity",
        "intent",
        "counterfactual",
        "observer",
        "agency",
        "experiment",
        "analysis",
        "autonomy",
        "human",
    ] {
        let touches = sources_under(&format!("crates/{package}/src"))
            .iter()
            .any(|(_, text)| {
                let code = code_only(text);
                code.contains("/sys/devices") || code.contains("/proc/stat")
            });
        if touches {
            readers.push(package);
        }
    }
    assert_eq!(
        readers,
        vec!["substrate"],
        "exactly one crate should read the machine, found {readers:?}"
    );
}
