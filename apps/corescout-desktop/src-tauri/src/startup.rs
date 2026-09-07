//! Starting when the user signs in.
//!
//! # Two mechanisms, because there are two kinds of install
//!
//! An installed build writes a per-user `Run` value. It needs no elevation, it
//! shows up in Task Manager under Startup where people expect to find it, and
//! it can be removed there without CoreScout's cooperation.
//!
//! A **packaged** build must not do that. A Store app declares a startup task
//! in its manifest, Windows lists it under Settings, and the user turns it on
//! there. Writing the registry from a packaged app is both rejected at
//! certification and wrong: it would put an entry in a place the user cannot
//! reconcile with what Settings tells them.
//!
//! So the packaged case reports that Windows owns this, and says where.
//!
//! # It is off until asked
//!
//! Nothing writes this on install, and the manifest declares the startup task
//! disabled. A product that adds itself to startup uninvited has made a
//! decision that was not its to make.

/// The value name under the Run key.
const NAME: &str = "CoreScout";

/// Whether CoreScout starts at sign-in, and whether that is ours to change.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Startup {
    /// Whether it is on.
    pub enabled: bool,
    /// Whether this application can change it.
    pub changeable: bool,
    /// What to tell the user, which differs between the two kinds of install.
    pub explain: String,
}

/// The current state.
pub fn state() -> Startup {
    if corescout_storage::packaged::is_packaged() {
        return Startup {
            // A packaged build cannot read its own startup task state without
            // WinRT, and guessing would be worse than saying so. Windows shows
            // the real answer in the one place that can change it.
            enabled: false,
            changeable: false,
            explain: "Windows manages this for installed apps. Settings \u{2192} Apps \u{2192} \
                      Startup has the switch, and it shows the real state."
                .into(),
        };
    }
    let enabled = is_enabled();
    Startup {
        enabled,
        changeable: cfg!(windows),
        explain: if enabled {
            "Your computer keeps learning from the moment you sign in. You can also remove this \
             in Task Manager, under Startup."
                .into()
        } else {
            "CoreScout only runs while you have it open. It will not learn anything in between."
                .into()
        },
    }
}

/// Whether CoreScout starts at sign-in.
#[cfg(windows)]
pub fn is_enabled() -> bool {
    !corescout_storage::packaged::is_packaged() && read().is_some()
}

/// Whether CoreScout starts at sign-in.
#[cfg(not(windows))]
pub fn is_enabled() -> bool {
    false
}

/// Turn it on or off.
#[cfg(windows)]
pub fn set_enabled(enabled: bool) -> std::io::Result<()> {
    if corescout_storage::packaged::is_packaged() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "Windows manages this for installed apps. Settings > Apps > Startup has the switch.",
        ));
    }
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
    let output = command()
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
    let status = command()
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
    let _ = command()
        .args(["delete", key(), "/v", NAME, "/f"])
        .status()?;
    Ok(())
}

/// `reg`, without a console window flashing up in front of the user.
#[cfg(windows)]
fn command() -> std::process::Command {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut command = std::process::Command::new("reg");
    command.creation_flags(CREATE_NO_WINDOW);
    command
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

    #[test]
    fn the_state_always_has_words_for_a_person() {
        let state = state();
        assert!(state.explain.len() > 30, "{state:?}");
        assert!(!state.explain.contains('_'), "{state:?}");
    }

    #[test]
    fn a_packaged_build_does_not_claim_to_own_this() {
        // A Store app writing the Run key fails certification, and puts an
        // entry somewhere the user cannot reconcile with what Settings says.
        if corescout_storage::packaged::is_packaged() {
            assert!(!state().changeable);
            assert!(set_enabled(true).is_err());
            assert!(state().explain.contains("Settings"));
        } else {
            // A test binary is never packaged, so this is the branch that runs
            // and it asserts the other half.
            assert!(state().changeable == cfg!(windows));
        }
    }

    #[cfg(not(windows))]
    #[test]
    fn elsewhere_it_says_it_is_unsupported_rather_than_pretending() {
        assert!(!is_enabled());
        assert!(set_enabled(true).is_err());
    }
}
