// Author:  Daniel Iwugo
// Comment: Christ is King
// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Daniel Iwugo
//! Can this tool be run on this machine, and do we know that it works?
//!
//! THE DISTINCTION THIS FILE REFUSES TO BLUR
//! -----------------------------------------
//! **Present is not working.** An image can be pulled, a binary can be on
//! PATH, and the tool can still produce nothing: the rich-model extractor did
//! exactly that over 2,000 images, exiting zero the whole way, because Octave
//! Forge packages were missing and Aletheia catches that error per image.
//!
//! So [`Presence`] answers "is it here" and says plainly that it has not
//! answered "does it work". `doctor` prints both states separately and never
//! lets the first stand in for the second, because a machine reported healthy
//! by a check that cannot fail is worse than one nobody checked.

use std::process::Command;

use stegobench_core::registry::Entry;

use crate::{binary_version, hash_file, which};

/// Whether the tool's code is on this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Presence {
    /// Here, and pinned to this exact version.
    Present { pin: String },
    /// Not here, with the reason a human can act on.
    Absent { reason: String },
    /// We could not find out, which is its own answer and must not be
    /// reported as either of the others.
    Unknown { reason: String },
}

impl Presence {
    pub fn is_present(&self) -> bool {
        matches!(self, Presence::Present { .. })
    }
}

/// What `doctor` knows about one tool.
#[derive(Debug, Clone, PartialEq)]
pub struct Availability {
    pub name: String,
    pub presence: Presence,
    /// Whether a self-test has actually been run. **Not yet implemented**, and
    /// the field is here so that its absence is visible in the output rather
    /// than being quietly equated with health.
    pub verified: Option<bool>,
    /// Environment variables the tool needs that are not set. Names only.
    pub missing_secrets: Vec<String>,
}

impl Availability {
    /// The one line `doctor` prints.
    pub fn summary(&self) -> String {
        let state = match &self.presence {
            Presence::Present { pin } => {
                let short: String = pin.chars().take(23).collect();
                format!("present   {short}")
            }
            Presence::Absent { reason } => format!("MISSING   {reason}"),
            Presence::Unknown { reason } => format!("unknown   {reason}"),
        };
        let verified = match self.verified {
            Some(true) => "  verified",
            Some(false) => "  SELF TEST FAILED",
            None => "  not verified",
        };
        let secrets = if self.missing_secrets.is_empty() {
            String::new()
        } else {
            format!("  needs {}", self.missing_secrets.join(", "))
        };
        format!("{:<16} {state}{verified}{secrets}", self.name)
    }
}

/// Is a container image already on this machine?
///
/// Shells out rather than talking to the daemon socket: the socket is a
/// privileged interface and a benchmark has no business holding one open.
fn image_present(reference: &str) -> Presence {
    let out = Command::new("docker")
        .args(["image", "inspect", "--format", "{{.Id}}", reference])
        .output();
    match out {
        Ok(o) if o.status.success() => Presence::Present {
            pin: String::from_utf8_lossy(&o.stdout).trim().to_string(),
        },
        Ok(_) => Presence::Absent {
            reason: format!("not pulled. docker pull {reference}"),
        },
        Err(e) => Presence::Unknown {
            reason: format!("no container runtime: {e}"),
        },
    }
}

/// Is a binary on PATH, and what exactly is it?
///
/// The pin is the hash of the bytes that would run, not the version string.
/// Two builds can report the same version and differ, and the point of a pin
/// is to say which one produced a number.
fn binary_present(entry: &Entry, program: &str) -> Presence {
    let Some(path) = which(program) else {
        return Presence::Absent {
            reason: format!("{program} is not on PATH"),
        };
    };
    match hash_file(&path) {
        Ok(pin) => {
            let version = binary_version(entry).unwrap_or_else(|| "unknown version".into());
            Presence::Present {
                pin: format!("{version} {pin}"),
            }
        }
        Err(e) => Presence::Unknown {
            reason: format!("{} could not be read: {e}", path.display()),
        },
    }
}

/// Checks one registry entry against this machine.
///
/// Reads the environment only to ask whether a named variable is set. It never
/// reads a secret's value, and no value reaches the returned structure, which
/// is what keeps a token out of `doctor --json` output that somebody will paste
/// into a bug report.
pub fn check(entry: &Entry) -> Availability {
    let presence = match (&entry.image, &entry.binary) {
        (Some(img), _) => image_present(&img.reference),
        (_, Some(bin)) => match bin.command.first() {
            Some(program) => binary_present(entry, program),
            None => Presence::Unknown {
                reason: "entry declares an empty command".into(),
            },
        },
        _ => Presence::Unknown {
            reason: "entry declares neither an image nor a binary".into(),
        },
    };

    let missing_secrets = entry
        .secrets
        .iter()
        .filter(|name| std::env::var_os(name).is_none())
        .cloned()
        .collect();

    Availability {
        name: entry.name.clone(),
        presence,
        verified: None,
        missing_secrets,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(toml_text: &str) -> Entry {
        toml::from_str(toml_text).expect("parses")
    }

    const SELFTEST: &str = "\n[selftest]\nmust_detect = \"a.png\"\nmust_clear = \"b.png\"\n";

    #[test]
    fn a_binary_on_path_is_present_and_pinned_by_its_hash() {
        let e = entry(&format!(
            "name = \"sh\"\nkind = \"detector\"\nlicence = \"X\"\n\
             [binary]\ncommand = [\"sh\"]\nversion_args = [\"--version\"]\n{SELFTEST}"
        ));
        let a = check(&e);
        assert!(a.presence.is_present(), "got {:?}", a.presence);
        if let Presence::Present { pin } = &a.presence {
            assert!(pin.contains("sha256:"), "pin should hash the bytes: {pin}");
        }
    }

    #[test]
    fn a_missing_binary_says_what_is_missing() {
        let e = entry(&format!(
            "name = \"nope\"\nkind = \"detector\"\nlicence = \"X\"\n\
             [binary]\ncommand = [\"definitely-not-real-xyzzy\"]\nversion_args = [\"-v\"]\n{SELFTEST}"
        ));
        match check(&e).presence {
            Presence::Absent { reason } => assert!(reason.contains("not on PATH")),
            other => panic!("expected Absent, got {other:?}"),
        }
    }

    #[test]
    fn presence_never_claims_the_tool_was_verified() {
        // The distinction this module exists for. Being installed is not
        // being correct, and doctor must not let one stand for the other.
        let e = entry(&format!(
            "name = \"sh\"\nkind = \"detector\"\nlicence = \"X\"\n\
             [binary]\ncommand = [\"sh\"]\nversion_args = [\"--version\"]\n{SELFTEST}"
        ));
        let a = check(&e);
        assert_eq!(a.verified, None);
        assert!(a.summary().contains("not verified"));
    }

    #[test]
    fn an_unset_secret_is_reported_by_name_only() {
        let e = entry(&format!(
            "name = \"x\"\nkind = \"detector\"\nlicence = \"X\"\n\
             secrets = [\"STEGOBENCH_TEST_UNSET_VAR\"]\n\
             [binary]\ncommand = [\"sh\"]\nversion_args = [\"--version\"]\n{SELFTEST}"
        ));
        let a = check(&e);
        assert_eq!(a.missing_secrets, ["STEGOBENCH_TEST_UNSET_VAR"]);
        // The name appears; nothing else about it can, because the value was
        // never read.
        assert!(a.summary().contains("STEGOBENCH_TEST_UNSET_VAR"));
    }

    #[test]
    fn a_set_secret_is_not_reported_as_missing_and_its_value_never_appears() {
        std::env::set_var("STEGOBENCH_TEST_SET_VAR", "swordfish-do-not-print");
        let e = entry(&format!(
            "name = \"x\"\nkind = \"detector\"\nlicence = \"X\"\n\
             secrets = [\"STEGOBENCH_TEST_SET_VAR\"]\n\
             [binary]\ncommand = [\"sh\"]\nversion_args = [\"--version\"]\n{SELFTEST}"
        ));
        let a = check(&e);
        assert!(a.missing_secrets.is_empty());
        assert!(
            !format!("{a:?}").contains("swordfish"),
            "a secret value must never reach the output"
        );
        std::env::remove_var("STEGOBENCH_TEST_SET_VAR");
    }

    /// The StegaShield token-leak test named in the v0.1 shortlist (13-v0.1-
    /// scope.md section 2): "a test asserts that: run the plugin with a known
    /// token value and grep every artefact the run produced for it."
    ///
    /// This exercises the REAL registered entry (`plugins/registry/detectors/
    /// stegashield.toml`), not a synthetic one, on both paths the item asks
    /// for:
    ///
    /// - **Absent token**: `check()` must report `STEGASHIELD_LICENCE` as
    ///   missing rather than silently proceeding or panicking.
    /// - **Present token**: given a sentinel value, that value must not
    ///   appear anywhere in `Availability`'s Debug output, its `summary()`
    ///   line, or the JSON `stegobench describe stegashield` would emit
    ///   (`Entry` only ever carries the variable's NAME in `secrets`, never a
    ///   value, so this also guards against a future change to `Entry`
    ///   accidentally starting to carry one).
    ///
    /// **What this does NOT test, and why.** `score`/`run` do not exist yet
    /// (see `main.rs`'s `not_yet`), so there is no code path that actually
    /// starts the StegaShield container, and therefore nothing yet writes a
    /// log line, a manifest row or a `result-v1` document from a real
    /// invocation for a real run to grep. The full "run it and grep every
    /// artefact" version of this test has to wait for that command to exist;
    /// this test is written so it keeps passing unchanged once it does, and
    /// so the absent-token path (the one that must fail clearly) is checked
    /// today rather than left untested until then. No real
    /// `STEGASHIELD_LICENCE` token was available to this session and none
    /// was requested, per the instruction that the token is user-supplied
    /// and never hardcoded; the sentinel here stands in for it.
    #[test]
    fn the_stegashield_licence_token_never_leaks_on_either_path() {
        let registry_dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/registry");
        let reg = stegobench_core::registry::Registry::load(&registry_dir)
            .expect("the real registry loads");
        let stegashield = reg
            .entries
            .get("stegashield")
            .expect("stegashield is registered");
        assert_eq!(stegashield.secret_names(), ["STEGASHIELD_LICENCE"]);

        // Absent-token path: must be reported missing, not silently ignored.
        std::env::remove_var("STEGASHIELD_LICENCE");
        let absent = check(stegashield);
        assert_eq!(absent.missing_secrets, ["STEGASHIELD_LICENCE"]);

        // Present-token path: a sentinel must never surface anywhere.
        const SENTINEL: &str = "sk-test-sentinel-do-not-print-9f31c2";
        std::env::set_var("STEGASHIELD_LICENCE", SENTINEL);
        let present = check(stegashield);
        std::env::remove_var("STEGASHIELD_LICENCE");

        assert!(present.missing_secrets.is_empty());
        let debug_repr = format!("{present:?}");
        assert!(
            !debug_repr.contains(SENTINEL),
            "the token leaked into Availability's Debug output: {debug_repr}"
        );
        assert!(
            !present.summary().contains(SENTINEL),
            "the token leaked into the doctor summary line"
        );
        let describe_json = serde_json::to_string(stegashield).expect("Entry serialises");
        assert!(
            !describe_json.contains(SENTINEL),
            "the token leaked into what `stegobench describe stegashield` would print"
        );
    }

    #[test]
    fn an_entry_with_neither_image_nor_binary_is_unknown_not_absent() {
        let mut e = entry(&format!(
            "name = \"x\"\nkind = \"detector\"\nlicence = \"X\"\n\
             [binary]\ncommand = [\"sh\"]\nversion_args = [\"-v\"]\n{SELFTEST}"
        ));
        e.binary = None;
        assert!(matches!(check(&e).presence, Presence::Unknown { .. }));
    }
}
