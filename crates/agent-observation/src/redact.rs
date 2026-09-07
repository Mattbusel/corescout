//! Removing secrets before anything is written down.
//!
//! # Local-first is not the same as safe to keep
//!
//! Everything CoreScout stores stays on this machine, which is most of the
//! privacy question and not all of it. A command line containing an API token
//! written into a database that survives restarts is a credential at rest that
//! the user did not choose to store, sitting somewhere no credential scanner
//! is looking.
//!
//! So redaction happens at ingestion, before the value reaches storage, rather
//! than at display. A value that was never written cannot leak from a backup,
//! a support export, or a future feature nobody has written yet.
//!
//! # This is a net, not a proof
//!
//! Detecting secrets by shape catches the common cases and cannot catch all of
//! them. It is written to fail towards redacting too much: a fingerprint that
//! loses a value it did not need to is a small loss, and a token in the
//! database is not.

/// What replaces a redacted value.
pub const MASK: &str = "[redacted]";

/// Flags whose value is a secret.
const SECRET_FLAGS: &[&str] = &[
    "--token",
    "--password",
    "--passwd",
    "--api-key",
    "--apikey",
    "--secret",
    "--auth",
    "--access-token",
    "--private-key",
    "--credential",
    "--bearer",
];

/// Substrings that make an environment variable name a secret.
const SECRET_NAMES: &[&str] = &[
    "token",
    "secret",
    "password",
    "passwd",
    // Bare "key" is broad on purpose: it catches `--api-key`, `SSH_KEY` and
    // whatever the next tool calls its credential, at the cost of masking the
    // occasional harmless variable. That is the right direction to be wrong in.
    "key",
    "credential",
    "auth",
    "session",
];

/// Prefixes that identify a credential whatever it is attached to.
const SECRET_PREFIXES: &[&str] = &[
    "sk-",
    "sk_live_",
    "sk_test_",
    "pk_live_",
    "ghp_",
    "gho_",
    "ghu_",
    "ghs_",
    "github_pat_",
    "xoxb-",
    "xoxp-",
    "xapp-",
    "akia",
    "asia",
    "aiza",
    "eyj",
    "hf_",
    "glpat-",
    "npm_",
];

/// Scrub a command line or message.
pub fn scrub(text: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut mask_next = false;
    for token in text.split_whitespace() {
        if mask_next {
            mask_next = false;
            out.push(MASK.into());
            continue;
        }
        let lower = token.to_lowercase();
        if SECRET_FLAGS.contains(&lower.as_str()) {
            out.push(token.to_string());
            mask_next = true;
            continue;
        }
        out.push(scrub_token(token));
    }
    out.join(" ")
}

/// Scrub one token.
fn scrub_token(token: &str) -> String {
    // `--token=xyz` and `AWS_SECRET_ACCESS_KEY=xyz`.
    if let Some((name, value)) = token.split_once('=') {
        if !value.is_empty() && is_secret_name(name) {
            return format!("{name}={MASK}");
        }
        if !value.is_empty() && looks_like_a_secret(value) {
            return format!("{name}={MASK}");
        }
        return token.to_string();
    }
    // `Authorization: Bearer xyz` arrives as separate tokens; the value is
    // caught by shape.
    if looks_like_a_secret(token) {
        return MASK.into();
    }
    token.to_string()
}

/// Whether a name marks its value as a secret.
pub fn is_secret_name(name: &str) -> bool {
    let lower = name.trim_start_matches('-').to_lowercase();
    SECRET_NAMES.iter().any(|word| lower.contains(word))
}

/// Whether a value looks like a credential.
pub fn looks_like_a_secret(value: &str) -> bool {
    let trimmed = value.trim_matches(|c| c == '"' || c == '\'' || c == ',');
    let lower = trimmed.to_lowercase();
    if SECRET_PREFIXES
        .iter()
        .any(|prefix| lower.starts_with(prefix) && trimmed.len() >= prefix.len() + 8)
    {
        return true;
    }
    high_entropy(trimmed)
}

/// Whether a string is long and mixed enough to be a random credential.
///
/// The thresholds matter: a real commit message and a real file path have to
/// come through untouched, so the bar is a long unbroken run with at least
/// three character classes, which prose and paths do not produce.
fn high_entropy(value: &str) -> bool {
    if value.len() < 24 {
        return false;
    }
    if value.contains(['/', '\\', ' ']) {
        return false;
    }
    let mut lower = false;
    let mut upper = false;
    let mut digit = false;
    let mut other = false;
    for c in value.chars() {
        match c {
            'a'..='z' => lower = true,
            'A'..='Z' => upper = true,
            '0'..='9' => digit = true,
            '-' | '_' | '.' | '+' | '=' | ':' => {}
            _ => other = true,
        }
    }
    if other {
        return false;
    }
    let classes = u8::from(lower) + u8::from(upper) + u8::from(digit);
    classes >= 3
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flag_takes_its_value_with_it() {
        assert_eq!(
            scrub("curl --token abc123 https://example.com"),
            format!("curl --token {MASK} https://example.com")
        );
    }

    #[test]
    fn an_attached_value_is_caught_too() {
        assert_eq!(
            scrub("deploy --api-key=abc123"),
            format!("deploy --api-key={MASK}")
        );
    }

    #[test]
    fn an_environment_assignment_with_a_secret_name_is_masked() {
        assert_eq!(
            scrub("AWS_SECRET_ACCESS_KEY=wJalrXUtnFEMI env"),
            format!("AWS_SECRET_ACCESS_KEY={MASK} env")
        );
        assert_eq!(
            scrub("GITHUB_TOKEN=short npm publish"),
            format!("GITHUB_TOKEN={MASK} npm publish")
        );
    }

    #[test]
    fn known_credential_prefixes_are_caught_wherever_they_appear() {
        for token in [
            "sk-abcdefghijklmnop",
            "ghp_abcdefghijklmnopqrst",
            "xoxb-1234567890-abcdef",
            "glpat-abcdefghijklmnop",
        ] {
            let scrubbed = scrub(&format!("echo {token}"));
            assert_eq!(scrubbed, format!("echo {MASK}"), "{token} survived");
        }
    }

    #[test]
    fn a_long_random_looking_value_is_masked_by_shape() {
        let secret = "aB3dE7gH9jK2mN5pQ8rS1tU4vW6x";
        assert!(looks_like_a_secret(secret));
        assert_eq!(scrub(&format!("run {secret}")), format!("run {MASK}"));
    }

    #[test]
    fn ordinary_commands_come_through_untouched() {
        // The cost of over-redaction is a fingerprint that loses a subcommand,
        // and the whole learning layer is built on those.
        for command in [
            "cargo build --release",
            "git commit -m fixed-the-thing",
            "npm run build",
            "C:\\Program Files\\nodejs\\npm.cmd install",
            "docker compose up -d",
            "pytest tests/test_api.py::test_health",
        ] {
            assert_eq!(scrub(command), command, "{command} was mangled");
        }
    }

    #[test]
    fn a_long_file_path_is_not_a_secret() {
        let path = "C:\\Users\\someone\\Projects\\application\\src\\main.rs";
        assert!(!looks_like_a_secret(path));
        assert_eq!(scrub(path), path);
    }

    #[test]
    fn a_git_hash_is_not_treated_as_a_credential() {
        // Hex only, so it has two character classes rather than three.
        let hash = "a1b2c3d4e5f60718293a4b5c6d7e8f9012345678";
        assert!(!looks_like_a_secret(hash), "a commit hash is not a secret");
    }

    #[test]
    fn a_semantic_version_is_not_a_secret() {
        assert!(!looks_like_a_secret("1.2.3-alpha.4+build.567"));
    }

    #[test]
    fn a_short_value_is_left_alone() {
        assert!(!looks_like_a_secret("aB3dE7"));
        assert_eq!(scrub("echo aB3dE7"), "echo aB3dE7");
    }

    #[test]
    fn secret_names_are_recognised_with_or_without_dashes() {
        assert!(is_secret_name("--token"));
        assert!(is_secret_name("GITHUB_TOKEN"));
        assert!(is_secret_name("apiKey"));
        assert!(!is_secret_name("--target-dir"));
        assert!(!is_secret_name("PATH"));
    }

    #[test]
    fn redaction_is_idempotent() {
        // Scrubbing a stored value again must not mangle the mask itself.
        let once = scrub("curl --token abc123");
        assert_eq!(scrub(&once), once);
    }
}
