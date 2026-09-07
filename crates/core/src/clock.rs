//! High-resolution monotonic timing.
//!
//! # Why not `std::time::Instant` everywhere
//!
//! `Instant` is monotonic and perfectly adequate for millisecond work, but it
//! is a struct return through a generic abstraction, and on Linux it reads
//! `CLOCK_MONOTONIC`, which NTP *slews*: the clock rate itself is adjusted to
//! discipline the system time. For measuring 300 ns operations and reasoning
//! about tail jitter we want `CLOCK_MONOTONIC_RAW`, which is the untouched
//! hardware counter (usually the TSC, read through the vDSO with no syscall).
//!
//! The clock is a free function rather than a `corescout_substrate::platform::Platform`
//! method on purpose: it is called in the innermost measurement loop, and
//! dynamic dispatch there would add indirection to the thing being measured.

/// Read the monotonic clock in nanoseconds.
///
/// The absolute value is meaningless; only differences are.
#[inline(always)]
pub fn now_ns() -> u64 {
    #[cfg(target_os = "linux")]
    {
        let mut ts = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        // SAFETY: `ts` is a valid, writable timespec. CLOCK_MONOTONIC_RAW is
        // supported on every Linux since 2.6.28 and resolves through the vDSO,
        // so this is a function call, not a syscall, on normal systems.
        unsafe {
            libc::clock_gettime(libc::CLOCK_MONOTONIC_RAW, &mut ts);
        }
        (ts.tv_sec as u64) * 1_000_000_000 + (ts.tv_nsec as u64)
    }
    #[cfg(not(target_os = "linux"))]
    {
        // Portable fallback. Correct, slightly more expensive, and only used
        // where the benchmark engine is being exercised by tests rather than
        // producing publishable numbers.
        use std::time::Instant;
        static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
        let start = START.get_or_init(Instant::now);
        start.elapsed().as_nanos() as u64
    }
}

/// Measure the cost of reading the clock, in nanoseconds.
///
/// This is reported alongside results rather than subtracted from them.
/// Subtracting a noisy constant from noisy measurements makes the output look
/// more precise than it is; showing the overhead lets a reader judge whether a
/// measured latency is meaningfully above the noise floor. CoreScout keeps the
/// overhead irrelevant instead, by timing batches of operations rather than
/// individual ones.
pub fn overhead_ns() -> f64 {
    const REPS: u32 = 2000;
    // Warm the vDSO page and any branch predictors first.
    for _ in 0..REPS {
        std::hint::black_box(now_ns());
    }
    let start = now_ns();
    for _ in 0..REPS {
        std::hint::black_box(now_ns());
    }
    let end = now_ns();
    (end.saturating_sub(start)) as f64 / REPS as f64
}

/// Resolution of the clock: the smallest non-zero difference observable
/// between two consecutive reads.
///
/// A resolution larger than a microsecond means jitter figures should be
/// treated with suspicion; that happens on VMs without a reliable TSC, where
/// the kernel falls back to a slow clocksource such as HPET or ACPI PM timer.
pub fn resolution_ns() -> u64 {
    let mut best = u64::MAX;
    for _ in 0..1000 {
        let a = now_ns();
        let mut b = now_ns();
        // Spin until the clock actually advances.
        let mut guard = 0;
        while b == a && guard < 10_000 {
            b = now_ns();
            guard += 1;
        }
        if b > a {
            best = best.min(b - a);
        }
    }
    if best == u64::MAX {
        0
    } else {
        best
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_is_monotonic() {
        let mut last = now_ns();
        for _ in 0..10_000 {
            let now = now_ns();
            assert!(now >= last, "clock went backwards: {last} -> {now}");
            last = now;
        }
    }

    #[test]
    fn clock_advances_over_a_real_sleep() {
        let start = now_ns();
        std::thread::sleep(std::time::Duration::from_millis(5));
        let elapsed = now_ns() - start;
        // Generous bounds: this must not be flaky on a loaded CI box.
        assert!(
            elapsed >= 4_000_000,
            "5ms sleep measured as {elapsed}ns, clock is too coarse"
        );
    }

    #[test]
    fn overhead_is_plausible() {
        let o = overhead_ns();
        // A clock read costs tens of nanoseconds through the vDSO, and at most
        // a few microseconds if it degrades to a syscall on a bad clocksource.
        assert!(o > 0.0 && o < 10_000.0, "implausible clock overhead {o}ns");
    }

    #[test]
    fn resolution_is_reported() {
        let r = resolution_ns();
        assert!(r > 0, "clock never advanced between reads");
    }
}
