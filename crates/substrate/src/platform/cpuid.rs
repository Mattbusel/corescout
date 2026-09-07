//! Thin, safe wrappers over the x86 `CPUID` instruction.
//!
//! Only used for facts the OS does not expose portably: the vendor string, the
//! processor brand string, and per-core hybrid classification. Everything else
//! comes from the OS, which knows about things CPUID does not (offlining,
//! cpusets, cgroups, cpufreq policy).
//!
//! # Why this needs pinning
//!
//! `CPUID` reports the core it executes on. On a hybrid part, leaf `0x1A`
//! returns a *different* answer on a P-core than on an E-core. Callers that
//! want per-core answers must pin the thread first; [`core_type_here`] is named
//! to make that obvious.

use crate::topology::CoreType;

#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::{__cpuid, __cpuid_count, __get_cpuid_max};

/// Raw CPUID output.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Regs {
    pub eax: u32,
    pub ebx: u32,
    pub ecx: u32,
    pub edx: u32,
}

/// Execute `CPUID` with the given leaf, or `None` off x86.
#[allow(unused_variables)]
pub fn cpuid(leaf: u32) -> Option<Regs> {
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: CPUID is unprivileged and has no side effects. We check the
        // maximum supported leaf first so unsupported leaves cannot return the
        // contents of an unrelated leaf (which is what real hardware does).
        unsafe {
            let max = __get_cpuid_max(leaf & 0x8000_0000).0;
            if leaf > max {
                return None;
            }
            let r = __cpuid(leaf);
            Some(Regs {
                eax: r.eax,
                ebx: r.ebx,
                ecx: r.ecx,
                edx: r.edx,
            })
        }
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        None
    }
}

/// Execute `CPUID` with an explicit sub-leaf in ECX.
#[allow(unused_variables)]
pub fn cpuid_count(leaf: u32, sub_leaf: u32) -> Option<Regs> {
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: as above.
        unsafe {
            let max = __get_cpuid_max(leaf & 0x8000_0000).0;
            if leaf > max {
                return None;
            }
            let r = __cpuid_count(leaf, sub_leaf);
            Some(Regs {
                eax: r.eax,
                ebx: r.ebx,
                ecx: r.ecx,
                edx: r.edx,
            })
        }
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        None
    }
}

/// The 12-character vendor string from leaf 0: `GenuineIntel`, `AuthenticAMD`.
pub fn vendor() -> Option<String> {
    let r = cpuid(0)?;
    let mut bytes = Vec::with_capacity(12);
    // The string is spread across EBX, EDX, ECX in that deliberately odd order.
    for reg in [r.ebx, r.edx, r.ecx] {
        bytes.extend_from_slice(&reg.to_le_bytes());
    }
    Some(String::from_utf8_lossy(&bytes).trim().to_string())
}

/// The 48-character brand string from extended leaves 0x8000_0002..=0x8000_0004.
pub fn brand_string() -> Option<String> {
    let mut bytes = Vec::with_capacity(48);
    for leaf in 0x8000_0002u32..=0x8000_0004 {
        let r = cpuid(leaf)?;
        for reg in [r.eax, r.ebx, r.ecx, r.edx] {
            bytes.extend_from_slice(&reg.to_le_bytes());
        }
    }
    let s = String::from_utf8_lossy(&bytes);
    let s = s.trim_end_matches('\0').trim();
    if s.is_empty() {
        None
    } else {
        // Brand strings are space padded internally on some parts.
        Some(collapse_spaces(s))
    }
}

/// True when the part advertises a hybrid (mixed core type) topology.
///
/// Leaf 7 sub-leaf 0, EDX bit 15.
pub fn is_hybrid() -> bool {
    cpuid_count(7, 0)
        .map(|r| r.edx & (1 << 15) != 0)
        .unwrap_or(false)
}

/// Classify the core the calling thread is *currently running on*.
///
/// Leaf `0x1A` sub-leaf 0, EAX bits 31:24: `0x40` = Core (P), `0x20` = Atom (E).
/// The caller is responsible for pinning; see the module docs.
pub fn core_type_here() -> CoreType {
    if !is_hybrid() {
        return CoreType::Unknown;
    }
    match cpuid_count(0x1A, 0).map(|r| r.eax >> 24) {
        Some(0x40) => CoreType::Performance,
        Some(0x20) => CoreType::Efficiency,
        _ => CoreType::Unknown,
    }
}

fn collapse_spaces(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_space = false;
    for ch in s.chars() {
        let is_space = ch == ' ';
        if is_space && last_space {
            continue;
        }
        last_space = is_space;
        out.push(ch);
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collapses_internal_padding_in_brand_strings() {
        assert_eq!(
            collapse_spaces("  AMD Ryzen 9    7950X 16-Core Processor "),
            "AMD Ryzen 9 7950X 16-Core Processor"
        );
    }

    #[test]
    #[cfg(target_arch = "x86_64")]
    fn vendor_string_is_twelve_sane_characters() {
        let v = vendor().expect("x86-64 always supports leaf 0");
        assert_eq!(v.len(), 12, "unexpected vendor string {v:?}");
        assert!(v.chars().all(|c| c.is_ascii_graphic()));
    }

    #[test]
    #[cfg(target_arch = "x86_64")]
    fn brand_string_is_non_empty() {
        // Every x86-64 part made since 2003 implements the brand leaves.
        let b = brand_string().expect("brand string leaves");
        assert!(!b.is_empty());
    }

    #[test]
    fn non_hybrid_parts_report_unknown_core_type() {
        // On a non-hybrid host this must not claim P or E.
        if !is_hybrid() {
            assert_eq!(core_type_here(), CoreType::Unknown);
        }
    }
}
