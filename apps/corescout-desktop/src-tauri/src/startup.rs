//! Starting when the user signs in.
//!
//! # Why a registry value rather than a scheduled task or a service
//!
//! A per-user `Run` value needs no elevation, is visible in Task Manager's
//! Startup tab where people expect to find it, and can be removed there
//! without CoreScout's cooperation. A Windows service would need an
//! administrator at install time and would be invisible in the place users
//! look, which is the wrong trade for something that is meant to be easy to
//! stop.
//!
//! # It is off until asked
//!
//! Nothing writes this value on install. A product that adds itself to
//! startup without being asked has made a decision that was not its to make.

/// The value name under the Run key.
const NAME: &str = "CoreScout";

/// Whether CoreScout starts at sign-in.
#[cfg(windows)]
pub fn is_enabled() -> bool {
    read().is_some()
}

/// Whether CoreScout starts at sign-in.
#[cfg(not(windows))]
pub fn is_enabled() -> bool {
    false
}

/// Turn it on or off.
#[cfg(windows)]
pub fn set_enabled(enabled: bool) -> std::io::Result<()> {
    if enabled {
        let exe = std::env::current_exe()?;
        write(&format!("\"{}\"", exe.display()))
    } else {
        remove()
    }
}

/// Turn it on or off.
#[cfg(not(windows))]
pub fn set_enabled(_enabled: bool) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "starting at sign-in is only implemented on Windows",
    ))
}

#[cfg(windows)]
fn key() -> &'static str {
    "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run"
}

/// Read the value, if it is there.
///
/// Shelling out to `reg` rather than taking a registry crate: this is three
/// calls in the whole application, and a dependency for them would be the only
/// reason the desktop shell needed one.
#[cfg(windows)]
fn read() -> Option<String> {
    let output = std::process::Command::new("reg")
        .args(["query", key(), "/v", NAME])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines()
        .find(|line| line.contains(NAME))
        .and_then(|line| line.split("REG_SZ").nth(1))
        .map(|value| value.trim().to_string())
}

#[cfg(windows)]
fn write(value: &str) -> std::io::Result<()> {
    let status = std::process::Command::new("reg")
        .args(["add", key(), "/v", NAME, "/t", "REG_SZ", "/d", value, "/f"])
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(
            "the startup entry could not be written",
        ))
    }
}

#[cfg(windows)]
fn remove() -> std::io::Result<()> {
    // Deleting something that is not there is a success, not a failure: the
    // user asked for it to be off and it is off.
    let _ = std::process::Command::new("reg")
        .args(["delete", key(), "/v", NAME, "/f"])
        .status()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reading_the_setting_never_panics_and_never_writes() {
        // Called on every render of the Settings page.
        let before = is_enabled();
        assert_eq!(is_enabled(), before, "reading must not change anything");
    }

    #[cfg(not(windows))]
    #[test]
    fn elsewhere_it_says_it_is_unsupported_rather_than_pretending() {
        assert!(!is_enabled());
        assert!(set_enabled(true).is_err());
    }
}
