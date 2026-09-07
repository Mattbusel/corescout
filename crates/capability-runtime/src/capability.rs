//! What a capability is, and what makes one valid.
//!
//! # A capability is a promise with a receipt
//!
//! Anything can claim to be a better way of doing something. A capability
//! carries the evidence that produced it, the check that says whether it
//! worked, and the way back if it did not. A definition missing any of those
//! is refused by [`Capability::validate`] rather than stored, because the
//! moment a capability without a verification step exists, CoreScout can
//! report a success it never confirmed.
//!
//! # Generated definitions are validated, not trusted
//!
//! Capabilities are produced by CoreScout from observed procedures, which
//! means the thing constructing them is a program reading agent activity. That
//! is exactly the situation where a definition should be checked as if it came
//! from a stranger. [`Capability::validate`] rejects shell metacharacters,
//! commands that are not plain executables, and any path that tries to climb
//! out of where it was given.

use std::path::PathBuf;

use corescout_permissions::{Authority, Risk, Target};
use serde::{Deserialize, Serialize};

/// The longest a single step may run.
pub const MAX_TIMEOUT_MS: u64 = 15 * 60 * 1000;
/// The default, when a definition does not say.
pub const DEFAULT_TIMEOUT_MS: u64 = 5 * 60 * 1000;
/// The most steps a capability may have.
pub const MAX_STEPS: usize = 12;

/// Why a definition was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Invalid {
    /// It does nothing.
    NoSteps,
    /// It has more steps than a learned procedure ever legitimately needs.
    TooManySteps(usize),
    /// There is no way to tell whether it worked.
    NoVerification,
    /// It changes things and has no way back.
    NoRollback,
    /// A command contains characters that would be interpreted by a shell.
    ShellMetacharacters(String),
    /// A command is empty or is a path rather than a program name.
    NotAProgram(String),
    /// A command is a shell interpreter.
    ShellProgram(String),
    /// A path climbs out of where it was given.
    EscapingPath(PathBuf),
    /// A step would run for longer than any step may.
    TimeoutTooLong(u64),
    /// It has no name a person could read.
    Unnamed,
}

impl std::fmt::Display for Invalid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Invalid::NoSteps => write!(f, "a capability that does nothing is not a capability"),
            Invalid::TooManySteps(n) => {
                write!(f, "{n} steps is more than a learned procedure needs")
            }
            Invalid::NoVerification => write!(
                f,
                "there is no step that checks whether it worked, so success could never be more \
                 than a claim"
            ),
            Invalid::NoRollback => write!(
                f,
                "it changes something and has no way back, so it cannot be marked reversible"
            ),
            Invalid::ShellMetacharacters(command) => write!(
                f,
                "{command:?} contains characters a shell would interpret; steps run programs \
                 directly and never through a shell"
            ),
            Invalid::NotAProgram(command) => {
                write!(f, "{command:?} is not a plain program name")
            }
            Invalid::ShellProgram(command) => write!(
                f,
                "{command:?} is a shell; a capability names the program it wants to run, so that \
                 what runs is visible in the definition rather than inside a quoted string"
            ),
            Invalid::EscapingPath(path) => {
                write!(
                    f,
                    "{} climbs out of the folder it was given",
                    path.display()
                )
            }
            Invalid::TimeoutTooLong(ms) => write!(f, "{ms} ms is longer than any step may run"),
            Invalid::Unnamed => write!(f, "it has no name"),
        }
    }
}

impl std::error::Error for Invalid {}

/// One thing a capability does.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "op")]
pub enum Operation {
    /// Run a program. Never through a shell.
    Run {
        /// The program, by name. Resolved on the path.
        command: String,
        /// Arguments, passed as a vector so nothing is re-parsed.
        #[serde(default)]
        args: Vec<String>,
        /// Where to run it, relative to the workspace.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<PathBuf>,
        /// How long to allow.
        #[serde(default)]
        timeout_ms: Option<u64>,
        /// Text that must appear in the output for this to count as working.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expect: Option<String>,
    },
    /// Wait, for something that takes a moment to become true.
    Wait {
        /// How long.
        ms: u64,
        /// Why waiting is the right thing here.
        because: String,
    },
}

impl Operation {
    /// Run a program with arguments.
    pub fn run(command: impl Into<String>, args: &[&str]) -> Operation {
        Operation::Run {
            command: command.into(),
            args: args.iter().map(|a| a.to_string()).collect(),
            cwd: None,
            timeout_ms: None,
            expect: None,
        }
    }

    /// How long this step may take.
    pub fn timeout_ms(&self) -> u64 {
        match self {
            Operation::Run { timeout_ms, .. } => timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS),
            Operation::Wait { ms, .. } => *ms,
        }
    }

    /// A line for the interface.
    pub fn describe(&self) -> String {
        match self {
            Operation::Run { command, args, .. } => {
                if args.is_empty() {
                    command.clone()
                } else {
                    format!("{command} {}", args.join(" "))
                }
            }
            Operation::Wait { ms, because } => format!("wait {ms} ms ({because})"),
        }
    }

    /// What this step would touch.
    pub fn targets(&self) -> Vec<Target> {
        match self {
            Operation::Run { command, cwd, .. } => {
                let mut targets = vec![Target::Command(command.clone())];
                if let Some(cwd) = cwd {
                    targets.push(Target::Path(cwd.clone()));
                }
                targets
            }
            Operation::Wait { .. } => Vec::new(),
        }
    }
}

/// Where a capability came from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "source", content = "detail")]
pub enum Provenance {
    /// Learned from a procedure that accumulated randomised evidence.
    Learned {
        /// The procedure it came from.
        procedure: String,
        /// What the evidence says, in a sentence.
        because: String,
    },
    /// Written by the user.
    UserDefined,
    /// Shipped with CoreScout.
    BuiltIn,
}

/// A reusable, verified way of doing something.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Capability {
    /// Stable across restarts.
    pub id: String,
    /// What a person would call it.
    pub name: String,
    /// What it is for, in one sentence.
    pub purpose: String,
    /// Where it applies, if it is specific to one place.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    /// What has to be true before it makes sense to run this.
    #[serde(default)]
    pub preconditions: Vec<String>,
    /// What it does.
    pub steps: Vec<Operation>,
    /// How to tell whether it worked.
    ///
    /// Required. A capability without one can only ever report what it was
    /// told, which is the failure this whole product exists to catch.
    pub verification: Operation,
    /// How to put things back.
    #[serde(default)]
    pub rollback: Vec<Operation>,
    /// How much it could cost if it goes wrong.
    pub risk: Risk,
    /// Whether the rollback is expected to work.
    pub reversible: bool,
    /// Where it came from.
    pub provenance: Provenance,
    /// Times it has been run.
    pub uses: u64,
    /// Times it ran and verification confirmed it.
    pub successes: u64,
    /// When it was created, Unix milliseconds.
    pub created_ms: u64,
    /// When it was last run.
    #[serde(default)]
    pub last_used_ms: u64,
}

impl Capability {
    /// The share of runs that verification confirmed.
    ///
    /// `None` until it has been run, so a brand new capability does not
    /// display as perfectly reliable.
    pub fn reliability(&self) -> Option<f64> {
        (self.uses > 0).then(|| self.successes as f64 / self.uses as f64)
    }

    /// Everything this capability would touch.
    pub fn targets(&self) -> Vec<Target> {
        let mut targets: Vec<Target> = self
            .steps
            .iter()
            .chain(std::iter::once(&self.verification))
            .chain(self.rollback.iter())
            .flat_map(|operation| operation.targets())
            .collect();
        targets.dedup();
        targets
    }

    /// Whether an authority permits everything this would touch.
    pub fn is_within(&self, authority: &Authority) -> bool {
        authority.permits(&self.targets()).is_ok()
    }

    /// Check a definition before it is stored or run.
    ///
    /// Called on every definition CoreScout generates as well as every one a
    /// user writes, because the generator is a program reading agent activity
    /// and that is exactly when input should be treated as untrusted.
    pub fn validate(&self) -> Result<(), Invalid> {
        if self.name.trim().is_empty() {
            return Err(Invalid::Unnamed);
        }
        if self.steps.is_empty() {
            return Err(Invalid::NoSteps);
        }
        if self.steps.len() > MAX_STEPS {
            return Err(Invalid::TooManySteps(self.steps.len()));
        }
        if self.reversible && self.risk > Risk::Low && self.rollback.is_empty() {
            return Err(Invalid::NoRollback);
        }
        for operation in self
            .steps
            .iter()
            .chain(std::iter::once(&self.verification))
            .chain(self.rollback.iter())
        {
            validate_operation(operation)?;
        }
        Ok(())
    }
}

/// Characters a shell would act on. A step runs a program directly, so none of
/// these can do anything except signal that a definition was built by
/// splitting a shell command line, which is not how steps are constructed.
const METACHARACTERS: &[char] = &['&', '|', ';', '>', '<', '`', '$', '\n', '\r', '(', ')', '*'];

/// Programs that are shells.
///
/// Blocked because naming one puts the real command inside an argument, where
/// none of the checks above can see it. `cmd /c "build && del *"` passes every
/// other rule in this file. This is the rule that makes the others mean
/// something.
const SHELLS: &[&str] = &[
    "cmd",
    "cmd.exe",
    "powershell",
    "powershell.exe",
    "pwsh",
    "pwsh.exe",
    "sh",
    "bash",
    "zsh",
    "fish",
    "dash",
    "ksh",
    "wsl",
    "wsl.exe",
    "busybox",
    "conhost",
    "wscript",
    "cscript",
];

fn validate_operation(operation: &Operation) -> Result<(), Invalid> {
    if operation.timeout_ms() > MAX_TIMEOUT_MS {
        return Err(Invalid::TimeoutTooLong(operation.timeout_ms()));
    }
    let Operation::Run {
        command, args, cwd, ..
    } = operation
    else {
        return Ok(());
    };
    let name = command.trim();
    if name.is_empty() {
        return Err(Invalid::NotAProgram(command.clone()));
    }
    if name.contains(METACHARACTERS) || args.iter().any(|a| a.contains(METACHARACTERS)) {
        return Err(Invalid::ShellMetacharacters(command.clone()));
    }
    // A bare program name, resolved on the path. Not an absolute path, which
    // would let a generated definition name any executable on the machine, and
    // not a relative path, which would let it name one inside a repository.
    if name.contains(['/', '\\']) || name.contains(':') {
        return Err(Invalid::NotAProgram(command.clone()));
    }
    if SHELLS.contains(&name.to_lowercase().as_str()) {
        return Err(Invalid::ShellProgram(command.clone()));
    }
    if let Some(cwd) = cwd {
        if cwd
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err(Invalid::EscapingPath(cwd.clone()));
        }
    }
    for arg in args {
        let path = PathBuf::from(arg);
        if path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err(Invalid::EscapingPath(path));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capability() -> Capability {
        Capability {
            id: "cap-1".into(),
            name: "Regenerate schema, then build".into(),
            purpose: "Builds here read a generated file that goes stale.".into(),
            workspace: Some("app".into()),
            preconditions: vec!["the schema generator exists".into()],
            steps: vec![
                Operation::run("cargo", &["run", "--bin", "gen-schema"]),
                Operation::run("cargo", &["build"]),
            ],
            verification: Operation::Run {
                command: "cargo".into(),
                args: vec!["build".into(), "--message-format".into(), "short".into()],
                cwd: None,
                timeout_ms: Some(60_000),
                expect: None,
            },
            rollback: Vec::new(),
            risk: Risk::Low,
            reversible: true,
            provenance: Provenance::Learned {
                procedure: "proc-1".into(),
                because: "43% to 7% across 29 randomised trials".into(),
            },
            uses: 0,
            successes: 0,
            created_ms: 1000,
            last_used_ms: 0,
        }
    }

    #[test]
    fn a_well_formed_capability_validates() {
        capability().validate().expect("should be valid");
    }

    #[test]
    fn a_brand_new_capability_is_not_perfectly_reliable() {
        // Zero of zero is not one hundred percent, and a card that says 100%
        // before anything has run is the worst thing this screen could print.
        assert_eq!(capability().reliability(), None);
    }

    #[test]
    fn a_capability_with_no_steps_is_refused() {
        let empty = Capability {
            steps: Vec::new(),
            ..capability()
        };
        assert_eq!(empty.validate(), Err(Invalid::NoSteps));
    }

    #[test]
    fn shell_metacharacters_are_refused_in_commands_and_arguments() {
        // Steps run programs directly, so these cannot do anything. Their
        // presence means a definition was built by splitting a shell line,
        // which is not how definitions are meant to be constructed, and the
        // next version of that mistake is one that does run through a shell.
        for bad in [
            Operation::run("cargo build && rm -rf x", &[]),
            Operation::run("cargo", &["build", "; curl evil"]),
            Operation::run("cargo", &["build", "$(whoami)"]),
            Operation::run("cargo", &["build", "> out.txt"]),
        ] {
            let definition = Capability {
                steps: vec![bad.clone()],
                ..capability()
            };
            assert!(
                matches!(definition.validate(), Err(Invalid::ShellMetacharacters(_))),
                "{bad:?} was accepted"
            );
        }
    }

    #[test]
    fn a_command_that_is_a_path_is_refused() {
        // A bare name resolves on the path. An absolute path lets a generated
        // definition name any executable on the machine; a relative one lets
        // it name something inside the repository it is working on.
        for bad in [
            "C:\\Windows\\System32\\cmd.exe",
            "/bin/sh",
            "./scripts/deploy.sh",
            "..\\evil.exe",
        ] {
            let definition = Capability {
                steps: vec![Operation::run(bad, &[])],
                ..capability()
            };
            assert!(
                matches!(definition.validate(), Err(Invalid::NotAProgram(_))),
                "{bad} was accepted"
            );
        }
    }

    #[test]
    fn a_path_that_climbs_out_is_refused() {
        let escaping = Capability {
            steps: vec![Operation::Run {
                command: "cargo".into(),
                args: vec!["build".into()],
                cwd: Some(PathBuf::from("..\\..\\Windows")),
                timeout_ms: None,
                expect: None,
            }],
            ..capability()
        };
        assert!(matches!(escaping.validate(), Err(Invalid::EscapingPath(_))));
    }

    #[test]
    fn an_argument_that_climbs_out_is_refused() {
        let escaping = Capability {
            steps: vec![Operation::run("cargo", &["build", "..\\..\\secrets"])],
            ..capability()
        };
        assert!(matches!(escaping.validate(), Err(Invalid::EscapingPath(_))));
    }

    #[test]
    fn a_step_cannot_run_forever() {
        let forever = Capability {
            steps: vec![Operation::Run {
                command: "cargo".into(),
                args: Vec::new(),
                cwd: None,
                timeout_ms: Some(MAX_TIMEOUT_MS + 1),
                expect: None,
            }],
            ..capability()
        };
        assert!(matches!(
            forever.validate(),
            Err(Invalid::TimeoutTooLong(_))
        ));
    }

    #[test]
    fn something_risky_calling_itself_reversible_needs_a_way_back() {
        let dishonest = Capability {
            risk: Risk::Moderate,
            reversible: true,
            rollback: Vec::new(),
            ..capability()
        };
        assert_eq!(dishonest.validate(), Err(Invalid::NoRollback));

        let honest = Capability {
            risk: Risk::Moderate,
            reversible: false,
            ..capability()
        };
        honest
            .validate()
            .expect("not claiming to be reversible is fine");
    }

    #[test]
    fn a_capability_cannot_have_an_unbounded_number_of_steps() {
        let sprawling = Capability {
            steps: (0..MAX_STEPS + 1)
                .map(|_| Operation::run("cargo", &["build"]))
                .collect(),
            ..capability()
        };
        assert!(matches!(
            sprawling.validate(),
            Err(Invalid::TooManySteps(_))
        ));
    }

    #[test]
    fn the_verification_step_is_validated_too() {
        // Otherwise the one step that is always present is the one nobody
        // checks.
        let sneaky = Capability {
            verification: Operation::run("cargo", &["build", "&& del *"]),
            ..capability()
        };
        assert!(matches!(
            sneaky.validate(),
            Err(Invalid::ShellMetacharacters(_))
        ));
    }

    #[test]
    fn a_shell_is_not_a_program_a_capability_may_name() {
        // The rule that makes the others mean something. `cmd /c "x && y"`
        // satisfies every other check in this file, because the interesting
        // half is inside a string none of them look at.
        for shell in ["cmd", "CMD.EXE", "powershell", "pwsh", "sh", "bash", "wsl"] {
            let definition = Capability {
                steps: vec![Operation::run(shell, &["-c", "anything"])],
                ..capability()
            };
            assert!(
                matches!(definition.validate(), Err(Invalid::ShellProgram(_))),
                "{shell} was accepted"
            );
        }
    }

    #[test]
    fn rollback_steps_are_validated_too() {
        let sneaky = Capability {
            rollback: vec![Operation::run("/bin/sh", &["-c", "x"])],
            ..capability()
        };
        assert!(matches!(sneaky.validate(), Err(Invalid::NotAProgram(_))));
    }

    #[test]
    fn a_capability_names_everything_it_would_touch() {
        let targets = capability().targets();
        assert!(targets.contains(&Target::Command("cargo".into())));
        assert!(!targets.is_empty());
    }

    #[test]
    fn a_capability_is_refused_by_an_authority_that_does_not_name_its_command() {
        let capability = capability();
        let mut authority = Authority::own_process_only();
        assert!(!capability.is_within(&authority));
        authority.grant_command("cargo");
        assert!(capability.is_within(&authority));
    }

    #[test]
    fn every_refusal_explains_itself_in_words() {
        let refusals = [
            Invalid::NoSteps,
            Invalid::TooManySteps(30),
            Invalid::NoVerification,
            Invalid::NoRollback,
            Invalid::ShellMetacharacters("x && y".into()),
            Invalid::NotAProgram("/bin/sh".into()),
            Invalid::ShellProgram("cmd".into()),
            Invalid::EscapingPath(PathBuf::from("..")),
            Invalid::TimeoutTooLong(99),
            Invalid::Unnamed,
        ];
        for refusal in refusals {
            assert!(refusal.to_string().len() > 12, "{refusal:?}");
        }
    }

    #[test]
    fn a_capability_survives_storage() {
        let capability = capability();
        let json = serde_json::to_string(&capability).expect("serialise");
        assert_eq!(
            serde_json::from_str::<Capability>(&json).expect("deserialise"),
            capability
        );
    }
}
