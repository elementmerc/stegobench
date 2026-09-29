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
//!
//! WHAT "HERE" MEANS, WHICH IS NOT ALWAYS THE SAME THING
//! ----------------------------------------------------
//! Presence asks about the program a run would actually launch, and for the
//! three routes that is three different programs: a container image, a binary
//! on PATH, or, for a tool that is a SERVICE, the adapter script that posts to
//! it. An entry setting `invoke.host` names an image, but nothing ever runs
//! that image here, so asking the container runtime about it answers a
//! question no run poses. It used to, and the cost was exact: `score` refused
//! to start against a service running on another host until the user pulled an
//! image nothing would use, and `doctor` skipped the self-test, which is the
//! only check that would have proved the service answers at all.
//!
//! For a service, then, "here" is a narrower claim than usual. The adapter is
//! here and can be run, and an address has been supplied. Whether anything is
//! listening at that address is the self-test's question, and nothing in this
//! file goes and asks it.

use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use stegobench_core::registry::{Entry, Invoke};

use crate::{binary_version, hash_file, which};

/// Whether the code a run would launch is on this machine.
///
/// Deliberately "the code a run would launch" rather than "the tool". For a
/// service the two differ: what runs here is the adapter, and the tool itself
/// is somewhere else entirely.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Presence {
    /// Here, and pinned to this exact version.
    Present { pin: String },
    /// Not here, with the reason a human can act on.
    Absent { reason: String },
    /// We could not find out, which is its own answer and must not be
    /// reported as either of the others.
    Unknown { reason: String },
    /// The entry says this tool does not run on this machine's operating
    /// system, so it is not missing and installing something will not help.
    ///
    /// A separate answer from Absent because the two ask different things of
    /// the reader. "Not installed" is an instruction; "cannot run here" is a
    /// fact about the machine, and reporting the second as the first sends
    /// somebody looking for a package that does not exist.
    Unsupported { reason: String },
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
            Presence::Unsupported { reason } => format!("n/a       {reason}"),
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

/// How long the container runtime gets to answer whether an image is here.
///
/// Generous, because a daemon that has just started can be slow, and this is a
/// ceiling for the one that never answers rather than a budget for the one
/// that is busy.
const DOCKER_TIMEOUT: Duration = Duration::from_secs(20);

/// Is a container image already on this machine?
///
/// Shells out rather than talking to the daemon socket: the socket is a
/// privileged interface and a benchmark has no business holding one open.
fn image_present(reference: &str) -> Presence {
    let mut cmd = Command::new("docker");
    cmd.args(["image", "inspect", "--format", "{{.Id}}", reference]);
    // Bounded, because a container daemon that has wedged answers nothing and
    // never closes the pipe either, and `Command::output()` would wait on that
    // for as long as the machine stays up. This runs inside `list`, `describe`
    // and `doctor`, so an unbounded wait here is the whole tool hanging on a
    // question it only asked in passing.
    let out = crate::exec::captured(cmd, "docker image inspect", DOCKER_TIMEOUT);
    match out {
        Ok(o) if o.status.success() => Presence::Present {
            pin: String::from_utf8_lossy(&o.stdout).trim().to_string(),
        },
        Ok(_) => Presence::Absent {
            reason: format!("not pulled. docker pull {reference}"),
        },
        // "Unknown" rather than "Absent", and it always was: not being able to
        // ask whether an image is here is a different answer from its not
        // being here, and only one of the two is fixed by pulling. The reason
        // is the runner's own sentence, which distinguishes a runtime that is
        // not installed from one that stopped answering.
        Err(e) => Presence::Unknown {
            reason: format!("could not ask the container runtime: {e}"),
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
///
/// `adapter_roots` are the trees a relative `invoke.adapter` is resolved
/// against, and are empty for a caller that has no registry directory to name.
/// See [`crate::adapter`] for why the entry does not carry the answer itself.
pub fn check(entry: &Entry, adapter_roots: &[PathBuf]) -> Availability {
    let presence = match unsupported_here(entry) {
        Some(p) => p,
        None => present_here(entry, adapter_roots),
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

/// Does this entry rule out the machine we are on?
///
/// Only an entry whose program runs on THIS machine can be ruled out this way,
/// which is a locally installed binary or a host adapter. A container is a
/// Linux image wherever it runs, and on macOS and Windows the container runtime
/// supplies the Linux to run it in, so a platform list on an image entry would
/// describe the image's contents rather than where it can be used. A host
/// adapter is the opposite case: the image may well be Linux and irrelevant,
/// because the script runs here, so a platform list on one is a statement about
/// this machine and is honoured. An entry that says nothing is not ruled out:
/// unstated is a question nobody answered, not a claim that it runs everywhere.
fn unsupported_here(entry: &Entry) -> Option<Presence> {
    let runs_on_this_machine =
        entry.binary.is_some() || entry.invoke.as_ref().is_some_and(|i| i.host);
    if entry.platforms.is_empty() || !runs_on_this_machine {
        return None;
    }
    let here = std::env::consts::OS;
    if entry.platforms.iter().any(|p| p == here) {
        return None;
    }
    Some(Presence::Unsupported {
        reason: format!(
            "this entry runs on {} and this machine is {here}, so nothing to \
             install would make it available",
            entry.platforms.join(" and ")
        ),
    })
}

/// Can the adapter for a SERVICE entry run on this machine?
///
/// `invoke.host = true` says the thing to run is here rather than inside the
/// image, and [`crate::selftest::read_one`] means it literally: it launches
/// `invoke.entrypoint` with the adapter script and never touches the image at
/// all. So presence asks about exactly that program. Asking whether the image
/// is pulled would answer a question no run ever poses, and it is the question
/// this check used to answer: the image arm matched first, so a service
/// running on another host, which is the normal arrangement, was reported
/// missing and `score` refused to start until the user pulled an image nothing
/// would run.
///
/// **The image reference stays required, and still means what it meant.** It
/// is what a result names as the subject, pinned by digest so somebody else
/// can fetch the identical bytes. It is a fact about the artefact, not about
/// this machine, and the two were being conflated.
///
/// NOTHING HERE TOUCHES THE NETWORK, ON PURPOSE
/// --------------------------------------------
/// A probe of the service would answer the question this module refuses to
/// answer, which is whether the tool WORKS, and it would answer it badly: the
/// strongest thing a bare connection can report is that something accepted it
/// on that port. The self-test already asks the real question, by posting a
/// fixture and reading the number back, and because presence no longer waits
/// on a pulled image `doctor` now actually reaches it. A probe would also send
/// a request to somebody else's service every time a user typed `doctor`.
///
/// So the two situations the reader has to tell apart are told apart by the
/// two columns that already exist: an unset endpoint is a presence answer, and
/// a service that is down is a self-test failure carrying the adapter's own
/// message about the address it could not reach.
fn host_adapter_present(invoke: &Invoke, adapter_roots: &[PathBuf]) -> Presence {
    // Refused by `Entry::validate`, so this is only reachable for an entry
    // built in memory. It is still an entry fault rather than a missing
    // install, which is why it is Unknown and not Absent.
    let Some(rel) = &invoke.adapter else {
        return Presence::Unknown {
            reason: "invoke.host is set but the entry declares no adapter, so \
                     it names no program to run"
                .into(),
        };
    };

    // Resolved exactly as `run_host_adapter` resolves it, against the same
    // roots, so presence cannot say yes to a path a run would then fail to
    // open. The message names where it looked, which is the whole of the
    // difference between a two second fix and a bug report.
    let adapter = match crate::adapter::resolve(rel, adapter_roots) {
        Ok(p) => p,
        Err(why) => {
            return Presence::Absent {
                reason: why.to_string(),
            }
        }
    };

    let program = invoke
        .entrypoint
        .clone()
        .unwrap_or_else(|| crate::DEFAULT_HOST_ENTRYPOINT.into());
    if which(&program).is_none() {
        return Presence::Absent {
            reason: format!("{program} is not on PATH, and it is what runs the adapter"),
        };
    }

    // Asked last, because the first two are faults in the installation and
    // this one is a thing the reader has not done yet.
    if let Some(name) = &invoke.endpoint_env {
        // Trimmed, because the adapter trims. A variable holding a space is
        // unset as far as `stegashield_one.py` is concerned, and a presence
        // check that disagreed with the thing it is predicting would send a
        // corpus run off to fail on its first image.
        let supplied =
            std::env::var_os(name).is_some_and(|v| !v.to_string_lossy().trim().is_empty());
        if !supplied {
            // Unknown rather than Absent, and the distinction is the honest
            // one: the adapter is here and runnable, and without an address
            // there is no instance to ask, so whether this tool can run here
            // is a question nobody has supplied the information to answer.
            // Reporting it as Absent would send the reader looking for
            // something to install.
            //
            // One line, because `doctor` prints one line per tool over a whole
            // registry and a paragraph here pushes the other twelve off the
            // screen. The reasoning lives in docs/guide/http-detector.md.
            return Presence::Unknown {
                reason: format!(
                    "{name} is not set to an address, so there is no instance \
                     to ask. Start your own and export it; there is no default"
                ),
            };
        }
    }

    // The adapter's bytes, not the image's digest, and labelled so the two
    // cannot be read for each other. This is what decides how the question is
    // asked on this machine; the digest identifying the subject is in the
    // entry and `describe` prints it.
    match hash_file(&adapter) {
        Ok(pin) => Presence::Present {
            pin: format!("adapter {pin}"),
        },
        Err(e) => Presence::Unknown {
            reason: format!("{} could not be read: {e}", adapter.display()),
        },
    }
}

fn present_here(entry: &Entry, adapter_roots: &[PathBuf]) -> Presence {
    // A host adapter wins over both routes, and it has to win in exactly the
    // order `read_one` dispatches: presence is only worth anything if it asks
    // about the program a run would actually launch.
    if let Some(invoke) = &entry.invoke {
        if invoke.host {
            return host_adapter_present(invoke, adapter_roots);
        }
    }
    match (&entry.image, &entry.binary) {
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// These tests are about PRESENCE, not about where an adapter lives, so
    /// they ask with no roots. That falls back to the directory the test runs
    /// in, which is exactly what the signature without roots used to do, so
    /// every expectation below still means what it meant. Adapter resolution
    /// has its own tests in `crate::adapter`.
    fn check(entry: &Entry) -> Availability {
        super::check(entry, &[])
    }

    fn entry(toml_text: &str) -> Entry {
        toml::from_str(toml_text).expect("parses")
    }

    const SELFTEST: &str = "\n[selftest]\nmust_detect = \"a.png\"\nmust_clear = \"b.png\"\n";

    #[test]
    fn a_tool_that_cannot_run_here_is_not_reported_as_missing() {
        // "Not installed" is an instruction and "cannot run on this operating
        // system" is a fact about the machine. Reporting the second as the
        // first sends somebody looking for a package that does not exist for
        // them, which is the whole reason this is a separate answer.
        let elsewhere = match std::env::consts::OS {
            "linux" => "windows",
            _ => "linux",
        };
        let e = entry(&format!(
            "name = \"x\"\nkind = \"detector\"\nlicence = \"X\"\n\
             platforms = [\"{elsewhere}\"]\n\
             [binary]\ncommand = [\"definitely-not-installed\"]\n{SELFTEST}"
        ));
        let got = check(&e);
        match &got.presence {
            Presence::Unsupported { reason } => {
                assert!(reason.contains(elsewhere), "{reason}");
                assert!(reason.contains(std::env::consts::OS), "{reason}");
            }
            other => panic!("reported as {other:?} rather than unsupported"),
        }
        assert!(!got.presence.is_present());
    }

    #[test]
    fn a_tool_listing_this_platform_is_checked_normally() {
        let e = entry(&format!(
            "name = \"sh\"\nkind = \"detector\"\nlicence = \"X\"\n\
             platforms = [\"{}\"]\n\
             [binary]\ncommand = [\"sh\"]\nversion_args = [\"--version\"]\n{SELFTEST}",
            std::env::consts::OS
        ));
        assert!(
            !matches!(check(&e).presence, Presence::Unsupported { .. }),
            "a tool that names this platform was ruled out on it"
        );
    }

    #[test]
    fn a_platform_list_does_not_rule_out_a_container() {
        // A container is a Linux image wherever it runs, and on macOS and
        // Windows the runtime supplies the Linux to run it in. A platform list
        // on an image entry would describe the image's contents rather than
        // where it can be used, so it must not make the tool unavailable.
        let e = entry(
            "name = \"x\"\nkind = \"detector\"\nlicence = \"X\"\n\
             platforms = [\"plan9\"]\n\
             [image]\nreference = \"r@sha256:abc\"\nsize_mb = 1\nbundled = true\n",
        );
        assert!(!matches!(check(&e).presence, Presence::Unsupported { .. }));
    }

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

    /// Everything a service entry needs on this machine, with the adapter and
    /// the interpreter both real so only the thing under test can fail.
    ///
    /// `sh` stands in for `python3`: what is being checked is that presence
    /// asks about the entrypoint the runner would launch, and an interpreter
    /// that is on every Unix machine keeps the test from depending on which
    /// Pythons happen to be installed.
    struct Service {
        _dir: tempfile::TempDir,
        adapter: std::path::PathBuf,
    }

    impl Service {
        fn new() -> Self {
            let dir = tempfile::tempdir().expect("a temporary directory");
            let adapter = dir.path().join("service_one.sh");
            std::fs::write(&adapter, b"#!/bin/sh\necho 0.9\n").expect("the adapter is written");
            Service { _dir: dir, adapter }
        }

        /// A host entry pointing at that adapter.
        ///
        /// `top` and `invoke_extra` are separate because TOML assigns a bare
        /// key to the table above it, so a `platforms` line written after
        /// `[invoke]` becomes an invoke field that nothing reads, and the test
        /// asserting on it passes for the wrong reason. `registry.rs`'s own
        /// helper carries the same warning for the same reason.
        fn entry(&self, top: &str, invoke_extra: &str) -> Entry {
            let e = entry(&format!(
                "name = \"svc\"\nkind = \"detector\"\nlicence = \"X\"\n{top}\
                 [image]\nreference = \"r@sha256:abc\"\nsize_mb = 1250\nbundled = false\n\
                 [invoke]\nhost = true\nentrypoint = \"sh\"\n\
                 adapter = \"{}\"\nargv = [\"{{adapter}}\", \"{{file}}\"]\n\
                 parser = \"number\"\n{invoke_extra}{SELFTEST}",
                self.adapter.display()
            ));
            assert!(e.invoke.as_ref().expect("an invoke block").host);
            e
        }

        /// The same entry with an endpoint variable declared.
        fn with_endpoint(&self, var: &str) -> Entry {
            let e = self.entry("", &format!("endpoint_env = \"{var}\"\n"));
            assert_eq!(
                e.invoke.as_ref().unwrap().endpoint_env.as_deref(),
                Some(var),
                "the fixture did not actually declare the variable"
            );
            e
        }
    }

    /// The bug this file was opened for. A service entry was answered by
    /// asking whether a container image was pulled, when the adapter never
    /// runs that image and the instance being scored is normally on another
    /// host entirely. `score` then refused with a pre-flight error and
    /// `doctor` skipped the one check that would have proved the service
    /// answers.
    #[test]
    fn a_service_entry_is_answered_by_its_adapter_and_not_by_its_image() {
        let svc = Service::new();
        let e = svc.entry("", "");
        // The image in that entry is a reference nothing has ever pulled, so
        // the old answer was a refusal and the new one must not be.
        assert!(
            !image_present(&e.image.as_ref().unwrap().reference).is_present(),
            "the fixture's image turned out to be pulled, so this proves nothing"
        );
        let got = check(&e);
        assert!(got.presence.is_present(), "got {:?}", got.presence);
    }

    #[test]
    fn a_ready_service_is_pinned_by_the_adapters_bytes_and_says_so() {
        let svc = Service::new();
        let a = check(&svc.entry("", "")).presence;
        let Presence::Present { pin } = &a else {
            panic!("expected Present, got {a:?}");
        };
        assert!(
            pin.starts_with("adapter sha256:"),
            "the pin must say what it pinned, or it reads as the image digest: {pin}"
        );
        assert_eq!(
            pin,
            &format!("adapter {}", hash_file(&svc.adapter).expect("hashes")),
            "the pin should be the bytes that actually run here"
        );
        // Truncated to 23 characters by `summary`, so the label has to survive
        // the truncation or the line is worse than no line.
        assert!(check(&svc.entry("", ""))
            .summary()
            .contains("adapter sha256"));
    }

    /// The other half of the situation the reader has to be told apart from a
    /// service that is down. Nobody has said where the instance is, so there
    /// is nothing to ask, and that is Unknown rather than a missing install:
    /// there is no package to go and fetch.
    #[test]
    fn a_service_whose_endpoint_variable_is_unset_says_which_variable_and_why() {
        const VAR: &str = "STEGOBENCH_TEST_UNSET_ENDPOINT_A";
        std::env::remove_var(VAR);
        let svc = Service::new();
        let e = svc.with_endpoint(VAR);
        match check(&e).presence {
            Presence::Unknown { reason } => {
                assert!(reason.contains(VAR), "the variable is not named: {reason}");
                assert!(
                    reason.contains("no default"),
                    "it should say why there is no default: {reason}"
                );
            }
            other => panic!("expected Unknown naming the variable, got {other:?}"),
        }
    }

    /// An exported but empty variable is the same situation as an unset one:
    /// `export FOO=` leaves the first behind, and a copied line with a stray
    /// space leaves the second. The adapter trims before deciding, so this has
    /// to agree with it or presence stops predicting what a run will do.
    #[test]
    fn an_empty_or_blank_endpoint_variable_counts_as_unset() {
        for (i, blank) in ["", "   ", "\t\n"].iter().enumerate() {
            let var = format!("STEGOBENCH_TEST_BLANK_ENDPOINT_{i}");
            std::env::set_var(&var, blank);
            let svc = Service::new();
            let got = check(&svc.with_endpoint(&var)).presence;
            std::env::remove_var(&var);
            assert!(
                matches!(got, Presence::Unknown { .. }),
                "{blank:?} was taken for an address: {got:?}"
            );
        }
    }

    /// Present, because the adapter can run and it has been told where to ask.
    /// Whether anything answers there is the self-test's question, and this
    /// module must not pretend to have asked it: no request is made, so a
    /// machine with no route to that address reaches the same answer, in the
    /// same time, as one sitting next to the service.
    #[test]
    fn a_service_with_an_endpoint_set_is_present_without_anything_being_contacted() {
        const VAR: &str = "STEGOBENCH_TEST_SET_ENDPOINT";
        // An address on a documentation range that is not routed anywhere, so
        // if this ever did reach out the test would stall rather than pass.
        std::env::set_var(VAR, "http://198.51.100.7:3000/api/analyze");
        let svc = Service::new();
        let started = std::time::Instant::now();
        let got = check(&svc.with_endpoint(VAR));
        std::env::remove_var(VAR);
        assert!(got.presence.is_present(), "got {:?}", got.presence);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(1),
            "a presence check that takes a second is one that went and asked"
        );
        assert_eq!(
            got.verified, None,
            "present must never be allowed to stand in for working"
        );
        assert!(got.summary().contains("not verified"));
    }

    /// The value of the endpoint is read only to see whether there is one. It
    /// is not a secret, but it is one team's internal address and it has no
    /// business in output somebody pastes into a bug report.
    #[test]
    fn the_endpoints_value_never_reaches_the_output() {
        const VAR: &str = "STEGOBENCH_TEST_ENDPOINT_VALUE";
        const SENTINEL: &str = "http://internal-host-do-not-print.invalid:3000/api/analyze";
        std::env::set_var(VAR, SENTINEL);
        let svc = Service::new();
        let got = check(&svc.with_endpoint(VAR));
        std::env::remove_var(VAR);
        assert!(
            !format!("{got:?}").contains("internal-host-do-not-print"),
            "the address leaked into the output: {got:?}"
        );
    }

    /// The message a stranger meets. It used to end "run from the root of the
    /// clone", which is no advice at all to somebody who installed a package,
    /// and that reader is exactly who the built-in registry exists for.
    #[test]
    fn a_service_whose_adapter_is_missing_says_where_it_looked_and_not_which_clone() {
        let root = tempfile::tempdir().expect("tmp");
        let svc = Service::new();
        let mut e = svc.entry("", "");
        e.invoke.as_mut().unwrap().adapter = Some("plugins/adapters/not-here-xyzzy.py".into());
        match super::check(&e, &[root.path().to_path_buf()]).presence {
            Presence::Absent { reason } => {
                assert!(reason.contains("not-here-xyzzy.py"), "{reason}");
                assert!(
                    reason.contains(&root.path().display().to_string()),
                    "it does not say where it looked: {reason}"
                );
                assert!(
                    !reason.contains("clone"),
                    "a packaged install is still being sent to a clone: {reason}"
                );
            }
            other => panic!("expected Absent, got {other:?}"),
        }
    }

    /// The other half: an adapter that IS under the root is found, so the
    /// refusal above is a real finding rather than this path never working.
    #[test]
    fn a_service_whose_adapter_is_under_the_root_is_present() {
        let root = tempfile::tempdir().expect("tmp");
        let adapter = root.path().join("plugins").join("adapters").join("one.py");
        std::fs::create_dir_all(adapter.parent().expect("parent")).expect("dirs");
        std::fs::write(&adapter, b"#!/usr/bin/env python3\n").expect("written");

        let svc = Service::new();
        let mut e = svc.entry("", "");
        e.invoke.as_mut().unwrap().adapter = Some("plugins/adapters/one.py".into());
        // Nothing found it from the working directory, which is what the old
        // resolution had to rely on.
        assert!(matches!(
            check(&e).presence,
            Presence::Absent { .. } | Presence::Unknown { .. }
        ));
        match super::check(&e, &[root.path().to_path_buf()]).presence {
            Presence::Present { pin } => assert!(pin.starts_with("adapter "), "{pin}"),
            other => panic!("expected Present, got {other:?}"),
        }
    }

    #[test]
    fn a_service_whose_interpreter_is_not_installed_names_the_interpreter() {
        let svc = Service::new();
        let mut e = svc.entry("", "");
        e.invoke.as_mut().unwrap().entrypoint = Some("definitely-not-a-runtime-xyzzy".into());
        match check(&e).presence {
            Presence::Absent { reason } => {
                assert!(
                    reason.contains("definitely-not-a-runtime-xyzzy"),
                    "{reason}"
                );
                assert!(reason.contains("not on PATH"), "{reason}");
            }
            other => panic!("expected Absent, got {other:?}"),
        }
    }

    /// `Entry::validate` refuses this, so it can only arrive from an entry
    /// built in memory. It is an entry fault rather than a missing install,
    /// and reporting it as Absent would send somebody looking for a package.
    #[test]
    fn a_host_entry_with_no_adapter_at_all_is_unknown() {
        let svc = Service::new();
        let mut e = svc.entry("", "");
        e.invoke.as_mut().unwrap().adapter = None;
        match check(&e).presence {
            Presence::Unknown { reason } => assert!(reason.contains("no adapter"), "{reason}"),
            other => panic!("expected Unknown, got {other:?}"),
        }
    }

    /// A host adapter runs on THIS machine, so unlike a container a platform
    /// list on one is a statement about this machine and has to be honoured.
    #[test]
    fn a_platform_list_does_rule_out_a_host_adapter_even_though_it_names_an_image() {
        let elsewhere = match std::env::consts::OS {
            "linux" => "windows",
            _ => "linux",
        };
        let svc = Service::new();
        let e = svc.entry(&format!("platforms = [\"{elsewhere}\"]\n"), "");
        match check(&e).presence {
            Presence::Unsupported { reason } => assert!(reason.contains(elsewhere), "{reason}"),
            other => panic!("expected Unsupported, got {other:?}"),
        }
    }

    #[test]
    fn a_host_adapter_listing_this_platform_is_checked_normally() {
        let svc = Service::new();
        let e = svc.entry(&format!("platforms = [\"{}\"]\n", std::env::consts::OS), "");
        assert!(check(&e).presence.is_present(), "{:?}", check(&e).presence);
    }

    /// The regression guard. An ordinary container entry is still answered by
    /// asking the container runtime, and an ordinary binary entry by looking
    /// on PATH; neither goes anywhere near the host-adapter path.
    #[test]
    fn a_container_entry_without_host_is_unaffected() {
        let e = entry(
            "name = \"x\"\nkind = \"detector\"\nlicence = \"X\"\n\
             [image]\nreference = \"definitely-not-pulled-xyzzy@sha256:abc\"\n\
             size_mb = 1\nbundled = true\n\
             [invoke]\nadapter = \"plugins/adapters/x.py\"\nargv = [\"{file}\"]\n\
             parser = \"number\"\n",
        );
        // Either Absent (a runtime said no) or Unknown (there is no runtime).
        // Never Present, and never the adapter path, which would have found
        // that adapter missing and said so in its own words.
        match check(&e).presence {
            Presence::Absent { reason } => assert!(reason.contains("docker pull"), "{reason}"),
            Presence::Unknown { reason } => {
                assert!(reason.contains("no container runtime"), "{reason}")
            }
            other => panic!("a container entry took another route: {other:?}"),
        }
    }

    #[test]
    fn a_binary_entry_without_host_is_unaffected() {
        let e = entry(&format!(
            "name = \"sh\"\nkind = \"detector\"\nlicence = \"X\"\n\
             [binary]\ncommand = [\"sh\"]\nversion_args = [\"--version\"]\n\
             [invoke]\nargv = [\"{{file}}\"]\nparser = \"number\"\n{SELFTEST}"
        ));
        let a = check(&e).presence;
        assert!(a.is_present(), "got {a:?}");
        if let Presence::Present { pin } = &a {
            assert!(
                !pin.starts_with("adapter "),
                "a binary entry was pinned as an adapter: {pin}"
            );
        }
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
