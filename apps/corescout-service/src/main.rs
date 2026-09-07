//! `corescout-service`: the part that is always running.
//!
//! # What it does, in order
//!
//! Opens the store and the ring, discovers what machine it is on, starts
//! observing, binds a loopback port, and writes down where it is. From then on
//! it is two threads: one sampling the mirror, one answering requests.
//!
//! # Why observation lives in its own thread
//!
//! It has to keep a steady cadence. Sharing a thread with request handling
//! would make the sampling interval depend on whether the user happened to
//! have the window open, and a feature built from unevenly spaced samples is
//! partly a feature about the sampler.
//!
//! # What it costs
//!
//! An observation pass on the machine this was developed on takes about seven
//! microseconds. At four a second that is around three thousandths of one per
//! cent of one core. The exact figure for the machine it is running on is in
//! Settings, measured rather than quoted.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use corescout_core::error::Result;
use corescout_product_api::endpoint::{mint_token, Endpoint};
use corescout_product_api::{Api, Engine};
use corescout_storage::events::{Event, EventKind, Severity};
use corescout_substrate::observation::default_sensors;
use corescout_substrate::{platform, Reflector, Substrate};

/// How often the mirror is sampled.
///
/// Four times a second. Fast enough that a state change is noticed while it is
/// still the thing the user is looking at, slow enough to stay invisible.
const INTERVAL: Duration = Duration::from_millis(250);

/// How often learned state is written to disk.
const PERSIST: Duration = Duration::from_secs(20);

/// How often the Microsoft Store is asked about this copy.
///
/// Six hours. The answer changes when a trial ends or somebody asks for a
/// refund, and neither is worth polling for. Nothing depends on it being
/// current: the Store enforces the licence, and this only decides what the
/// Settings screen says.
const STORE_CHECK: Duration = Duration::from_secs(6 * 60 * 60);

const USAGE: &str = "\
corescout-service - the part of CoreScout that is always running

USAGE:
    corescout-service [OPTIONS]

OPTIONS:
    --port <PORT>      Bind a specific port instead of being given one
    --no-observe       Answer requests without sampling the mirror
    --ticks <N>        Stop after N observations. For tests and smoke checks.
    -h, --help         Show this help

CoreScout writes everything to one folder and sends nothing anywhere. The
Privacy page in the app names that folder and lists exactly what is in it.
";

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("corescout-service: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print!("{USAGE}");
        return Ok(());
    }
    let port: u16 = value(&args, "--port")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let ticks: Option<u64> = value(&args, "--ticks").and_then(|v| v.parse().ok());
    let observe = !args.iter().any(|a| a == "--no-observe");

    let mut engine = Engine::open_default()?;
    engine.set_interval(INTERVAL);
    engine.store().append(Event::new(
        EventKind::Lifecycle,
        Severity::Info,
        "CoreScout started",
    ))?;

    // What machine this is. A failure here is not fatal: the product still
    // watches AI activity and answers questions, and the Computer screen says
    // what could not be read instead of inventing it.
    match platform::detect().discover_topology() {
        Ok(topology) => engine.set_topology(topology),
        Err(error) => {
            engine.store().append(Event::new(
                EventKind::Fault,
                Severity::Warning,
                format!("CoreScout could not read this machine's layout: {error}"),
            ))?;
        }
    }
    engine.set_observing(observe);

    let engine = Arc::new(Mutex::new(engine));
    let api = Api::new(Arc::clone(&engine));
    let token = mint_token();
    let server = corescout_product_api::http::Server::bind(api, token.clone(), port)?;
    let endpoint = Endpoint::new(server.port(), token);
    endpoint.publish()?;

    let stop = Arc::new(AtomicBool::new(false));
    let observer = observe.then(|| {
        let engine = Arc::clone(&engine);
        let stop = Arc::clone(&stop);
        std::thread::Builder::new()
            .name("corescout-mirror".into())
            .spawn(move || observe_loop(engine, stop, ticks))
    });

    {
        let engine = Arc::clone(&engine);
        let stop = Arc::clone(&stop);
        let _ = std::thread::Builder::new()
            .name("corescout-store".into())
            .spawn(move || {
                // Once at startup, because somebody who bought CoreScout
                // yesterday should not have to wait six hours for it to
                // notice, and then on the slow cadence.
                loop {
                    {
                        let mut engine = engine
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                        // Only so Settings can say something true about the
                        // licence, and so a refund is noticed. Nothing in
                        // CoreScout is gated on the answer: the Store enforces
                        // the licence by not letting an unlicensed copy run.
                        // Failure is silent by design; see the method.
                        engine.recheck_licence();
                    }
                    for _ in 0..(STORE_CHECK.as_secs()) {
                        if stop.load(Ordering::Relaxed) {
                            return;
                        }
                        std::thread::sleep(Duration::from_secs(1));
                    }
                }
            });
    }

    {
        let engine = Arc::clone(&engine);
        let stop = Arc::clone(&stop);
        let _ = std::thread::Builder::new()
            .name("corescout-persist".into())
            .spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    std::thread::sleep(PERSIST);
                    let mut engine = engine
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    let _ = engine.persist();
                }
            });
    }

    if ticks.is_some() {
        // A bounded run: serve while the observation thread works, then stop.
        // This is what the installer smoke check and the integration tests use.
        if let Some(Ok(handle)) = observer {
            let _ = handle.join();
        }
        stop.store(true, Ordering::Relaxed);
        let mut engine = engine
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        engine.persist()?;
        Endpoint::withdraw();
        return Ok(());
    }

    server.serve();
    stop.store(true, Ordering::Relaxed);
    Endpoint::withdraw();
    Ok(())
}

/// Sample the mirror on a steady cadence.
///
/// A shape change means the machine is not the machine it was: a CPU went
/// offline, or a device appeared. The reflector is rebuilt, and the feature
/// series breaks at that point rather than differencing across it, which would
/// produce a number about two different machines.
fn observe_loop(engine: Arc<Mutex<Engine>>, stop: Arc<AtomicBool>, ticks: Option<u64>) {
    let mut reflector = match build_reflector() {
        Ok(reflector) => reflector,
        Err(error) => {
            let engine = engine
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let _ = engine.store().append(Event::new(
                EventKind::Fault,
                Severity::Error,
                format!("CoreScout cannot watch this machine: {error}"),
            ));
            return;
        }
    };

    let mut taken = 0u64;
    while !stop.load(Ordering::Relaxed) {
        let started = Instant::now();

        if reflector.shape_changed() {
            match build_reflector() {
                Ok(fresh) => reflector = fresh,
                Err(_) => break,
            }
        }
        reflector.observe();
        let snapshot = reflector.snapshot();

        {
            let mut engine = engine
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            engine.record_reflection(&snapshot);
            // Cheap, and only interesting when something has changed, so it is
            // refreshed every few seconds rather than every pass.
            if taken % 40 == 0 {
                engine.refresh_self_description();
            }
        }

        taken += 1;
        if ticks.is_some_and(|limit| taken >= limit) {
            break;
        }
        if let Some(remaining) = INTERVAL.checked_sub(started.elapsed()) {
            std::thread::sleep(remaining);
        }
    }
}

fn build_reflector() -> Result<Reflector> {
    let platform = platform::detect();
    let substrate = Substrate::discover(platform.as_ref())?;
    Reflector::build(substrate, default_sensors())
}

fn value(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|at| args.get(at + 1))
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sampling_interval_is_small_enough_to_be_useful_and_large_enough_to_be_invisible() {
        // A quarter of a second: a state change is noticed while it is still
        // the thing the user is looking at, and the cost stays in the noise.
        assert!(INTERVAL >= Duration::from_millis(100));
        assert!(INTERVAL <= Duration::from_millis(1000));
    }

    #[test]
    fn learned_state_is_written_often_enough_that_a_crash_loses_little() {
        assert!(PERSIST <= Duration::from_secs(60));
    }

    #[test]
    fn arguments_are_read_by_name() {
        let args = vec!["--port".to_string(), "51234".to_string()];
        assert_eq!(value(&args, "--port").as_deref(), Some("51234"));
        assert_eq!(value(&args, "--ticks"), None);
        assert_eq!(value(&["--port".to_string()], "--port"), None);
    }
}
