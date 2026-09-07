//! Human-facing projections of machine-native state.
//!
//! # This is debugging infrastructure
//!
//! Everything in this crate is a lossy rendering of something whose canonical
//! form is elsewhere: the binary matrix in the self-state plane, the numeric
//! findings of a model, the audit log. None of it is the representation.
//!
//! That ordering is easy to lose and expensive to lose. In the original
//! CoreScout the human table and the JSON *were* the output, so every design
//! decision was quietly shaped by what would read well in a terminal. Here the
//! rendering is downstream of representations designed for a consumer that
//! cannot read.
//!
//! The dependency direction enforces it: this crate depends on the mirror and
//! the model crates, and nothing depends on this one except the apps.

pub mod json;
pub mod mirror_debug;

/// Output format selected on a command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Human,
    Json,
}

/// Format a byte count the way a systems person expects to read it.
pub fn bytes(n: u64) -> String {
    const K: u64 = 1024;
    match n {
        n if n >= 16 * K * K => format!("{} MiB", n / (K * K)),
        n if n >= K * K => format!("{:.1} MiB", n as f64 / (K * K) as f64),
        n if n >= K => format!("{} KiB", n / K),
        n => format!("{n} B"),
    }
}

/// Format a kHz frequency as MHz, or `-` when unknown.
pub fn mhz(khz: Option<u64>) -> String {
    match khz {
        Some(v) => format!("{}", v / 1000),
        None => "-".to_string(),
    }
}

/// Render a nanosecond duration at a sensible scale.
pub fn duration(ns: f64) -> String {
    if ns >= 1_000_000_000.0 {
        format!("{:.2} s", ns / 1e9)
    } else if ns >= 1_000_000.0 {
        format!("{:.3} ms", ns / 1e6)
    } else if ns >= 1_000.0 {
        format!("{:.2} us", ns / 1e3)
    } else {
        format!("{ns:.0} ns")
    }
}

/// Render a value at a readable magnitude without losing its identity.
pub fn magnitude(value: f64) -> String {
    let size = value.abs();
    if !value.is_finite() {
        return "-".to_string();
    }
    if size >= 1e12 {
        format!("{:.3}e12", value / 1e12)
    } else if size >= 1e9 {
        format!("{:.3}e9", value / 1e9)
    } else if size == value.trunc() && size < 1e9 {
        format!("{value:.0}")
    } else {
        format!("{value:.3}")
    }
}

/// Right-pad to `width` columns.
pub fn pad(s: &str, width: usize) -> String {
    if s.len() >= width {
        s.to_string()
    } else {
        format!("{s}{}", " ".repeat(width - s.len()))
    }
}

/// Left-pad to `width` columns.
pub fn rpad(s: &str, width: usize) -> String {
    if s.len() >= width {
        s.to_string()
    } else {
        format!("{}{s}", " ".repeat(width - s.len()))
    }
}

/// A simple horizontal bar, for a value in `0.0 ..= 1.0`.
pub fn bar(fraction: f64, width: usize) -> String {
    let filled = ((fraction.clamp(0.0, 1.0)) * width as f64).round() as usize;
    format!("{}{}", "#".repeat(filled), "-".repeat(width - filled))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_formatting() {
        assert_eq!(bytes(512), "512 B");
        assert_eq!(bytes(32 * 1024), "32 KiB");
        assert_eq!(bytes(1536 * 1024), "1.5 MiB");
        assert_eq!(bytes(64 * 1024 * 1024), "64 MiB");
    }

    #[test]
    fn frequency_formatting() {
        assert_eq!(mhz(Some(3_400_000)), "3400");
        assert_eq!(mhz(None), "-");
    }

    #[test]
    fn duration_scales() {
        assert_eq!(duration(900.0), "900 ns");
        assert_eq!(duration(1_500.0), "1.50 us");
        assert_eq!(duration(2_500_000.0), "2.500 ms");
        assert_eq!(duration(3_000_000_000.0), "3.00 s");
    }

    #[test]
    fn magnitudes_stay_readable_without_losing_identity() {
        assert_eq!(magnitude(3_600_000.0), "3600000");
        assert_eq!(magnitude(1.5), "1.500");
        assert_eq!(magnitude(9_812_345_678_901.0), "9.812e12");
        assert_eq!(magnitude(f64::NAN), "-");
    }

    #[test]
    fn padding_never_truncates() {
        assert_eq!(pad("ab", 4), "ab  ");
        assert_eq!(rpad("ab", 4), "  ab");
        assert_eq!(pad("abcdef", 3), "abcdef");
    }

    #[test]
    fn bars_clamp_rather_than_overflow() {
        assert_eq!(bar(0.0, 4), "----");
        assert_eq!(bar(1.0, 4), "####");
        assert_eq!(bar(2.0, 4), "####");
        assert_eq!(bar(-1.0, 4), "----");
        assert_eq!(bar(0.5, 4), "##--");
    }
}
