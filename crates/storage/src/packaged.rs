//! Whether this build is running from an MSIX package.
//!
//! # Why this matters, and it is not cosmetic
//!
//! A packaged app's writes under `%LOCALAPPDATA%` are redirected by Windows
//! into the package's own store. The environment variable still says
//! `C:\Users\you\AppData\Local`, the write succeeds, and the file is somewhere
//! else entirely.
//!
//! CoreScout's Privacy page names one folder and says that deleting it deletes
//! everything. If the product were shipped through the Store without this,
//! that page would name a folder the data is not in, and "delete everything"
//! would delete nothing. So the packaged case is detected and the real path is
//! used, which is also the path the page then shows.
//!
//! # Detecting it
//!
//! `GetCurrentPackageFamilyName` returns `APPMODEL_ERROR_NO_PACKAGE` when
//! there is no package. That is the documented way to ask, it needs no
//! dependency, and it is stable across every Windows this ships to.

/// The family name of the package this is running in, if it is running in one.
#[cfg(windows)]
pub fn family_name() -> Option<String> {
    // Declared here rather than taken from a crate: this is one call, and the
    // rest of this workspace already declares its own externs where a binding
    // would be the only reason for a dependency.
    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentPackageFamilyName(length: *mut u32, name: *mut u16) -> u32;
    }

    /// The process is not running in a package. Not an error.
    const APPMODEL_ERROR_NO_PACKAGE: u32 = 15700;
    const ERROR_INSUFFICIENT_BUFFER: u32 = 122;

    let mut length: u32 = 0;
    // The first call asks how long the name is, and is expected to fail.
    let probe = unsafe { GetCurrentPackageFamilyName(&mut length, std::ptr::null_mut()) };
    if probe == APPMODEL_ERROR_NO_PACKAGE || length == 0 {
        return None;
    }
    if probe != ERROR_INSUFFICIENT_BUFFER && probe != 0 {
        return None;
    }
    let mut buffer = vec![0u16; length as usize];
    let result = unsafe { GetCurrentPackageFamilyName(&mut length, buffer.as_mut_ptr()) };
    if result != 0 {
        return None;
    }
    // The length includes the terminating null, which is not part of the name.
    let end = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());
    Some(String::from_utf16_lossy(&buffer[..end]))
}

/// The family name of the package this is running in, if it is running in one.
#[cfg(not(windows))]
pub fn family_name() -> Option<String> {
    None
}

/// Whether this build is running from a package.
pub fn is_packaged() -> bool {
    family_name().is_some()
}

/// Where a packaged build's data actually lands.
///
/// `LocalCache\Local` is the package's own unvirtualised store: a write there
/// is a write there, and the path CoreScout shows is the path a user can open
/// in Explorer. Writing to `%LOCALAPPDATA%` directly from a packaged app would
/// succeed and end up somewhere the Privacy page does not name.
pub fn package_data_dir() -> Option<std::path::PathBuf> {
    let family = family_name()?;
    let local = std::env::var_os("LOCALAPPDATA")?;
    Some(
        std::path::PathBuf::from(local)
            .join("Packages")
            .join(family)
            .join("LocalCache")
            .join("Local")
            .join("CoreScout"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asking_whether_this_is_packaged_never_fails() {
        // Called on every start, on every Windows this ships to, and on the
        // platforms it does not ship to at all.
        let packaged = is_packaged();
        assert_eq!(packaged, family_name().is_some());
    }

    #[test]
    fn a_test_binary_is_not_a_packaged_application() {
        // If this ever reports true, the detection is wrong in the direction
        // that would send a developer's data into a package store.
        assert!(!is_packaged());
        assert_eq!(package_data_dir(), None);
    }

    #[cfg(not(windows))]
    #[test]
    fn elsewhere_there_is_no_such_thing_as_a_package() {
        assert_eq!(family_name(), None);
    }
}
