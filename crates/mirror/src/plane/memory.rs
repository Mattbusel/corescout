//! The shared memory region behind the self-state plane.
//!
//! # Read-only is enforced by the kernel, not by convention
//!
//! This is the architectural point of the whole module. The writer maps the
//! region `PROT_READ | PROT_WRITE`; every consumer opens the file `O_RDONLY` and
//! maps it `PROT_READ`. A consumer that tries to modify the mirror takes a
//! `SIGSEGV` from the MMU.
//!
//! That matters because the alternative, a `&` rather than a `&mut` in an API,
//! only protects consumers written in Rust that go through the API. The mirror
//! is meant to be read by arbitrary software, including software not written
//! yet and not written in this language. The truth boundary between "reality"
//! and "an agent's representation of reality" should not depend on the agent's
//! good manners, so it is placed where the hardware enforces it.
//!
//! # Where the region lives
//!
//! `$XDG_RUNTIME_DIR/corescout/mirror.plane` when that exists, otherwise
//! `/dev/shm/corescout-mirror.plane`. Both are tmpfs, so the plane is shared
//! memory that happens to have a path: no daemon registry, no socket, no
//! discovery protocol. A consumer that knows the path can map it, and the
//! filesystem's own permissions decide who may.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::plane::layout::OFF_SEQ;
use corescout_core::error::{Error, Result};

/// Default path of the shared plane.
pub fn default_path() -> PathBuf {
    if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") {
        if !runtime.is_empty() {
            return PathBuf::from(runtime)
                .join("corescout")
                .join("mirror.plane");
        }
    }
    PathBuf::from("/dev/shm/corescout-mirror.plane")
}

/// How a [`PlaneMemory`] is backed.
enum Backing {
    /// A private allocation. Used by tests and by a mirror running without
    /// publishing. `Vec<u64>` rather than `Vec<u8>` so the region is 8-byte
    /// aligned and the seqlock's atomic access is well defined.
    ///
    /// The vector is never read through this field: it exists to own the
    /// allocation that `ptr` points into.
    Owned(#[allow(dead_code)] Vec<u64>),
    #[cfg(target_os = "linux")]
    Mapped {
        addr: *mut libc::c_void,
        len: usize,
        /// Held open so the mapping keeps the inode alive.
        _file: std::fs::File,
    },
}

/// A mapped or allocated plane region.
pub struct PlaneMemory {
    ptr: *mut u8,
    len: usize,
    writable: bool,
    backing: Backing,
}

// SAFETY: `PlaneMemory` owns its region exclusively. Cross-process sharing is
// handled by the seqlock protocol in the writer and reader, not by Rust's
// aliasing rules, which cannot describe another process's memory accesses
// anyway.
unsafe impl Send for PlaneMemory {}

impl std::fmt::Debug for PlaneMemory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlaneMemory")
            .field("len", &self.len)
            .field("writable", &self.writable)
            .field(
                "backing",
                &match &self.backing {
                    Backing::Owned(_) => "owned",
                    #[cfg(target_os = "linux")]
                    Backing::Mapped { .. } => "mapped",
                },
            )
            .finish()
    }
}

impl PlaneMemory {
    /// A private, writable region. Portable, and not shared with anyone.
    pub fn anonymous(len: usize) -> PlaneMemory {
        let words = len.div_ceil(8).max(1);
        let mut backing = vec![0u64; words];
        let ptr = backing.as_mut_ptr() as *mut u8;
        PlaneMemory {
            ptr,
            len,
            writable: true,
            backing: Backing::Owned(backing),
        }
    }

    /// Create or resize the shared region and map it writable.
    #[cfg(target_os = "linux")]
    pub fn create(path: &std::path::Path, len: usize) -> Result<PlaneMemory> {
        use std::os::unix::io::AsRawFd;

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
        }
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .map_err(|e| Error::io(path, e))?;
        file.set_len(len as u64).map_err(|e| Error::io(path, e))?;

        // SAFETY: mapping `len` bytes of a file we just sized to `len`.
        let addr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                file.as_raw_fd(),
                0,
            )
        };
        if addr == libc::MAP_FAILED {
            return Err(Error::Syscall {
                call: "mmap",
                errno: std::io::Error::last_os_error().raw_os_error().unwrap_or(0),
            });
        }
        Ok(PlaneMemory {
            ptr: addr as *mut u8,
            len,
            writable: true,
            backing: Backing::Mapped {
                addr,
                len,
                _file: file,
            },
        })
    }

    /// Map an existing region read-only.
    ///
    /// The `PROT_READ` here is the truth boundary. Do not be tempted to make it
    /// writable for convenience.
    #[cfg(target_os = "linux")]
    pub fn open_read_only(path: &std::path::Path) -> Result<PlaneMemory> {
        use std::os::unix::io::AsRawFd;

        let file = std::fs::File::open(path).map_err(|e| Error::io(path, e))?;
        let len = file.metadata().map_err(|e| Error::io(path, e))?.len() as usize;
        if len < crate::plane::layout::HEADER_BYTES {
            return Err(Error::invalid(format!(
                "{} is too small to be a self-state plane",
                path.display()
            )));
        }

        // SAFETY: mapping `len` bytes of a file of exactly that size.
        let addr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ,
                libc::MAP_SHARED,
                file.as_raw_fd(),
                0,
            )
        };
        if addr == libc::MAP_FAILED {
            return Err(Error::Syscall {
                call: "mmap",
                errno: std::io::Error::last_os_error().raw_os_error().unwrap_or(0),
            });
        }
        Ok(PlaneMemory {
            ptr: addr as *mut u8,
            len,
            writable: false,
            backing: Backing::Mapped {
                addr,
                len,
                _file: file,
            },
        })
    }

    #[cfg(not(target_os = "linux"))]
    pub fn create(_path: &std::path::Path, _len: usize) -> Result<PlaneMemory> {
        Err(Error::unsupported(
            "shared self-state plane (memory mapping is implemented for Linux only)",
        ))
    }

    #[cfg(not(target_os = "linux"))]
    pub fn open_read_only(_path: &std::path::Path) -> Result<PlaneMemory> {
        Err(Error::unsupported(
            "shared self-state plane (memory mapping is implemented for Linux only)",
        ))
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn is_writable(&self) -> bool {
        self.writable
    }

    /// The region as bytes.
    ///
    /// # A note on the memory model
    ///
    /// Another process may be writing these bytes concurrently. Strictly, that
    /// is a data race that Rust's model has no way to describe, because the
    /// other party is outside the model entirely. The seqlock is what makes it
    /// safe in practice: any read that overlapped a write is detected by the
    /// sequence check and discarded, so torn data is never acted on. This is the
    /// standard seqlock formulation, and the reason readers copy out of the
    /// region before interpreting anything.
    pub fn as_slice(&self) -> &[u8] {
        // SAFETY: `ptr` is valid for `len` bytes for the lifetime of `self`.
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }

    /// The region as mutable bytes.
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        debug_assert!(self.writable, "attempted to write a read-only plane");
        // SAFETY: as above, and `self` is borrowed mutably.
        unsafe { std::slice::from_raw_parts_mut(self.ptr, self.len) }
    }

    /// The seqlock counter, as an atomic.
    ///
    /// Odd means a write is in progress. This is the only field either side
    /// touches atomically; everything else is ordered by the acquire/release
    /// pair around it.
    pub fn seq(&self) -> &AtomicU64 {
        debug_assert!(self.len >= OFF_SEQ + 8);
        // SAFETY: the header is at least `OFF_SEQ + 8` bytes, the region is
        // 8-byte aligned (page-aligned when mapped, `Vec<u64>`-aligned when
        // owned), and `AtomicU64` has the same layout as `u64`.
        unsafe { &*(self.ptr.add(OFF_SEQ) as *const AtomicU64) }
    }

    /// Read the seqlock counter with acquire ordering.
    pub fn load_seq(&self) -> u64 {
        self.seq().load(Ordering::Acquire)
    }
}

impl Drop for PlaneMemory {
    fn drop(&mut self) {
        match &self.backing {
            Backing::Owned(_) => {}
            #[cfg(target_os = "linux")]
            Backing::Mapped { addr, len, .. } => {
                // SAFETY: unmapping exactly what we mapped.
                unsafe {
                    libc::munmap(*addr, *len);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_anonymous_region_is_zeroed_writable_and_aligned() {
        let mut memory = PlaneMemory::anonymous(1024);
        assert_eq!(memory.len(), 1024);
        assert!(memory.is_writable());
        assert!(memory.as_slice().iter().all(|b| *b == 0));
        assert_eq!(
            memory.as_mut_slice().as_ptr() as usize % 8,
            0,
            "the region must be 8-byte aligned for the seqlock atomic"
        );
    }

    #[test]
    fn writes_are_visible_through_the_read_view() {
        let mut memory = PlaneMemory::anonymous(256);
        memory.as_mut_slice()[7] = 0xAB;
        assert_eq!(memory.as_slice()[7], 0xAB);
    }

    #[test]
    fn the_seqlock_counter_is_addressable() {
        let memory = PlaneMemory::anonymous(256);
        assert_eq!(memory.load_seq(), 0);
        memory.seq().store(3, Ordering::Release);
        assert_eq!(memory.load_seq(), 3);
        // ...and it lives where the layout says it does.
        assert_eq!(crate::plane::layout::get_u64(memory.as_slice(), OFF_SEQ), 3);
    }

    #[test]
    fn default_path_follows_xdg_when_set() {
        std::env::set_var("XDG_RUNTIME_DIR", "/run/user/1000");
        let path = default_path();
        std::env::remove_var("XDG_RUNTIME_DIR");
        assert!(path.ends_with("mirror.plane"));
        assert!(path.to_string_lossy().contains("1000"));
    }

    #[test]
    #[cfg(not(target_os = "linux"))]
    fn shared_mapping_is_refused_with_a_clear_message_off_linux() {
        let err = PlaneMemory::create(std::path::Path::new("x"), 4096).unwrap_err();
        assert!(err.to_string().contains("Linux"));
    }
}
