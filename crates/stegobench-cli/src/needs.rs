// Author:  Daniel Iwugo
// Comment: Christ is King
// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Daniel Iwugo
//! "What does this need from me?", answered the same way for every tool.
//!
//! THE PROBLEM THIS EXISTS TO REMOVE
//!
//! The registry describes three different kinds of thing. One detector is a
//! container image you pull. Another is a program you install and put on PATH.
//! A third is a SERVICE: the image names the subject, an adapter on this
//! machine posts to it, and until you export the address of your own instance
//! there is nothing to ask. A corpus is a fourth kind again: nothing to run at
//! all, a licence to read and gigabytes to fetch.
//!
//! A user should not have to know which of those they are holding. Before
//! this, the third kind was discovered by starting a run and watching the
//! first image fail. So every one of them is reduced here to the same two
//! facts: whether it is ready, and the list of lines to type if it is not.
//!
//! WHAT IS DELIBERATELY NOT HERE
//!
//! No value is ever printed. An endpoint and a licence token are named, never
//! filled in, for exactly the reason `Invoke::endpoint_env` carries a name and
//! not an address: a default address is scored against whatever answers on it,
//! so a result can name one detector while measuring another. The placeholder
//! in `export NAME=<...>` is angle brackets and a description, and a reader
//! who pastes it unchanged gets a shell error rather than a wrong number.
//!
//! Nothing here touches the network or the container runtime either. It reads
//! the entry and the availability answer that has already been computed, and
//! turns them into instructions.

use stegobench_core::corpus::{CorpusEntry, LicenceStatus};
use stegobench_core::registry::{Entry, Kind};
use stegobench_plugin::availability::{Availability, Presence};

/// Can the harness run this entry at all, as the entry is written?
///
/// Delegates to the entry, which owns the rule: it is a fact about the shape
/// of the TOML rather than about this machine. Kept as a function here so the
/// readers of it read one name.
pub fn can_be_driven(entry: &Entry) -> bool {
    entry.can_be_driven()
}

/// Why the harness cannot drive this entry, in one clause that follows the
/// entry's name.
///
/// Derived from which block is missing rather than written per entry, so a
/// new undrivable entry gets the sentence without anybody remembering to add
/// one, and so the two readers of this answer cannot word it differently.
pub fn undrivable_because(entry: &Entry) -> String {
    let block = match entry.kind {
        Kind::Detector => "invoke",
        Kind::Embedder => "roundtrip",
    };
    format!(
        "declares no {block} block, so nothing in its registry entry says \
         what command to launch and the host has no way to drive it"
    )
}

/// Whether the thing is usable right now, in five answers rather than two.
///
/// `Unknown` is load-bearing and is not a polite way of saying no. A service
/// whose address nobody has supplied is not missing, and a container runtime
/// that will not answer has told us nothing about the image. Both call for
/// something from the reader, and neither is a thing to install.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Readiness {
    /// Everything it needs is here. It may still be broken; that is what
    /// `doctor`'s self-test asks and this does not.
    Ready,
    /// There is something for the reader to do, and `steps` is the list.
    NeedsYou,
    /// This entry cannot run on this machine at all, so nothing to install
    /// would change it.
    CannotRunHere,
    /// The code may well be here; nothing in the entry says how to drive it.
    ///
    /// A JOURNEY FOUND THIS AS A CONTRADICTION BETWEEN THREE SURFACES.
    /// `aletheia-rich` has its image on the machine, so `describe` said
    /// "stegobench can run it" and `list` showed it among the detectors,
    /// while `plan` and `score` refused it for declaring no invoke block.
    /// Presence was standing in for drivability, and the reader was left to
    /// work out which of the three commands was lying.
    CannotBeDriven,
    /// Whether it is ready could not be established.
    Unknown,
}

impl Readiness {
    /// The word printed in the state column, fixed width across the four so a
    /// listing lines up.
    pub fn word(self) -> &'static str {
        match self {
            Readiness::Ready => "ready",
            Readiness::NeedsYou => "NEEDS YOU",
            Readiness::CannotRunHere => "n/a here",
            Readiness::CannotBeDriven => "undrivable",
            Readiness::Unknown => "unknown",
        }
    }

    /// The machine-readable spelling, which never changes with the prose.
    pub fn code(self) -> &'static str {
        match self {
            Readiness::Ready => "ready",
            Readiness::NeedsYou => "needs_you",
            Readiness::CannotRunHere => "cannot_run_here",
            Readiness::CannotBeDriven => "cannot_be_driven",
            Readiness::Unknown => "unknown",
        }
    }
}

/// One thing to do, and what doing it gets you.
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    /// The line to type, exactly as typed. `None` where no command exists,
    /// which is honest rather than a placeholder somebody would paste.
    pub run: Option<String>,
    /// One sentence. What this step is for.
    pub why: String,
}

impl Step {
    fn run(command: impl Into<String>, why: impl Into<String>) -> Self {
        Step {
            run: Some(command.into()),
            why: why.into(),
        }
    }

    fn just(why: impl Into<String>) -> Self {
        Step {
            run: None,
            why: why.into(),
        }
    }
}

/// The answer, in the one shape every subject gets.
#[derive(Debug, Clone, PartialEq)]
pub struct Needs {
    pub readiness: Readiness,
    /// Ordered so the first line is the first thing to do.
    pub steps: Vec<Step>,
}

impl Needs {
    /// The block `describe` prints and `doctor` indents, or nothing at all
    /// when there is nothing to do.
    ///
    /// Returns an empty string for a ready subject on purpose. A line saying
    /// "no action needed" under every one of thirteen tools is thirteen lines
    /// of noise around the two that matter.
    pub fn block(&self) -> String {
        if self.steps.is_empty() {
            return String::new();
        }
        let mut out = String::new();
        for step in &self.steps {
            match &step.run {
                Some(cmd) => out.push_str(&format!("  {cmd}\n      {}\n", step.why)),
                None => out.push_str(&format!("  {}\n", step.why)),
            }
        }
        out.pop();
        out
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "readiness": self.readiness.code(),
            "steps": self.steps.iter().map(|s| serde_json::json!({
                "run": s.run,
                "why": s.why,
            })).collect::<Vec<_>>(),
        })
    }
}

/// Does a run of this entry happen inside a container?
///
/// The same question `score` asks, and it has to be asked the same way: an
/// entry can name an image AND set `invoke.host`, in which case the image
/// identifies the subject and an adapter on this machine is what runs. Telling
/// a user to pull that image would be sending them to fetch something no run
/// will ever execute.
/// Does the IMAGE name the bytes that actually run?
///
/// An entry can name an image and still not run it: `invoke.host` means an
/// adapter runs here against an instance somebody else started, and the image
/// then names the subject rather than the executor. The distinction decides
/// whether the digest pins the run, so `pinning` and the registry reach check
/// both turn on it.
pub fn runs_in_container(entry: &Entry) -> bool {
    entry.image.is_some() && !entry.invoke.as_ref().is_some_and(|i| i.host)
}

/// What a tool needs from the reader, given what is already known about it.
///
/// Takes the availability answer rather than recomputing it, because
/// `doctor` has already paid for it and asking the container runtime twice per
/// tool would double the slowest part of that command.
pub fn of_tool(
    entry: &Entry,
    availability: &Availability,
    adapter_roots: &[std::path::PathBuf],
) -> Needs {
    if let Presence::Unsupported { reason } = &availability.presence {
        return Needs {
            readiness: Readiness::CannotRunHere,
            steps: vec![Step::just(format!(
                "{reason}. Nothing to install will change it."
            ))],
        };
    }

    // Asked before anything about this machine, because the answer is a
    // property of the entry: pulling the image would not make it drivable, so
    // telling the reader to pull it would be sending them to fetch something
    // no run here will ever execute.
    if !can_be_driven(entry) {
        return Needs {
            readiness: Readiness::CannotBeDriven,
            steps: vec![Step::just(format!(
                "This entry {}. Nothing you install will change that.",
                undrivable_because(entry)
            ))],
        };
    }

    let mut steps = Vec::new();

    // Derived from the entry's own structure rather than by reading the
    // presence reason's prose. A check that parses its own error messages
    // breaks silently the day somebody improves the wording.
    if !availability.presence.is_present() {
        steps.extend(route_steps(entry, &availability.presence, adapter_roots));
    }

    // Asked whatever the presence answer was. A service can be perfectly
    // present, adapter and all, and still be missing the licence token the
    // container checks at start up, and a reader finds that out in the
    // vendor's own error message otherwise.
    for name in &availability.missing_secrets {
        steps.push(Step::run(
            format!("export {name}=<your value>"),
            format!("{name} is not set. The registry never records a value"),
        ));
    }

    let readiness = if !steps.is_empty() {
        Readiness::NeedsYou
    } else if availability.presence.is_present() {
        Readiness::Ready
    } else {
        Readiness::Unknown
    };

    Needs { readiness, steps }
}

/// The steps particular to how this entry is run.
fn route_steps(
    entry: &Entry,
    presence: &Presence,
    adapter_roots: &[std::path::PathBuf],
) -> Vec<Step> {
    let mut steps = Vec::new();

    if runs_in_container(entry) {
        let Some(image) = &entry.image else {
            return vec![Step::just(
                "this entry declares neither an image nor a binary. That is a \
                 fault in the registry entry, not something to install.",
            )];
        };
        if matches!(presence, Presence::Unknown { .. }) {
            steps.push(Step::just(
                "No container runtime answered. Install Docker (or a drop-in \
                 replacement) and start the daemon.",
            ));
        }
        let size = match image.size_mb {
            Some(mb) => format!("about {mb} MB, "),
            None => String::new(),
        };
        steps.push(Step::run(
            format!("docker pull {}", image.reference),
            format!("{size}pinned by digest"),
        ));
        return steps;
    }

    // A host adapter: the program that runs is here, and the subject may be
    // somewhere else entirely.
    if let Some(invoke) = &entry.invoke {
        if invoke.host {
            if let Some(declared) = &invoke.adapter {
                // The refusal names where it looked. Telling a reader to run
                // from the root of a clone, which is what this used to say, is
                // no advice at all to the one who installed a package.
                if let Err(why) = stegobench_plugin::adapter::resolve(declared, adapter_roots) {
                    steps.push(Step::just(why.to_string()));
                }
            }
            let program = invoke
                .entrypoint
                .clone()
                .unwrap_or_else(|| stegobench_plugin::DEFAULT_HOST_ENTRYPOINT.into());
            if stegobench_plugin::which(&program).is_none() {
                steps.push(Step::just(format!(
                    "{program} is not on PATH, and it runs the adapter. \
                     Install it."
                )));
            }
            if let Some(name) = &invoke.endpoint_env {
                let set =
                    std::env::var_os(name).is_some_and(|v| !v.to_string_lossy().trim().is_empty());
                if !set {
                    steps.push(Step::run(
                        format!("export {name}=<the address of your own instance>"),
                        format!(
                            "{} is a service, scored by posting to an instance \
                             you started. No default: one would be scored \
                             against whatever answered. `stegobench describe \
                             {}` has the shape",
                            entry.name, entry.name
                        ),
                    ));
                }
            }
            return steps;
        }
    }

    // A locally installed program.
    let program = entry
        .binary
        .as_ref()
        .and_then(|b| b.command.first())
        .cloned();
    match program {
        Some(program) => {
            let where_from = match &entry.upstream {
                Some(url) => format!(" Upstream: {url}"),
                None => String::new(),
            };
            steps.push(Step::just(format!(
                "{program} is not on PATH. Install it until `which {program}` \
                 finds it.{where_from}"
            )));
        }
        None => steps.push(Step::just(
            "this entry declares neither an image nor a runnable command. \
             That is a fault in the registry entry, not something to install.",
        )),
    }
    steps
}

/// What a corpus needs from the reader.
///
/// The same shape as a tool's, and the readiness is honestly `Unknown` for
/// every corpus: a corpus entry describes a dataset and names no directory, so
/// nothing here can tell whether you already have it. Saying `ready` would be
/// the harness vouching for bytes it has never seen.
pub fn of_corpus(corpus: &CorpusEntry) -> Needs {
    let mut steps = Vec::new();

    if corpus.obtain.requires_acceptance {
        steps.push(Step::just(format!(
            "Accept {}'s terms yourself first. No script here can do it for \
             you.",
            corpus.id
        )));
    }
    if let Some(instructions) = &corpus.obtain.instructions {
        steps.push(Step::just(format!("How to obtain it: {instructions}")));
    }
    match (&corpus.obtain.doi, &corpus.obtain.url) {
        (Some(doi), _) => steps.push(Step::just(format!("Download it, citing {doi}"))),
        (None, Some(url)) => steps.push(Step::just(format!("Download it from {url}"))),
        // The starter corpus travels inside the binary, so "yours to arrange"
        // is true of every other routeless entry and false of the one entry
        // a first run depends on.
        (None, None) if corpus.id == crate::STARTER_ID => steps.push(Step::just(
            "No download: this one is compiled into the binary. \
             `stegobench fetch stegobench-starter --tier nano` writes it out.",
        )),
        (None, None) => steps.push(Step::just(
            "This entry records no download route; obtaining it is yours to \
             arrange.",
        )),
    }

    match corpus.licence.status {
        LicenceStatus::Verified => {}
        LicenceStatus::Unverified => steps.push(Step::just(format!(
            "{}'s terms are UNVERIFIED here. Read them before publishing \
             anything derived from it; an unknown is not a yes.",
            corpus.id
        ))),
        LicenceStatus::NoneGranted => steps.push(Step::just(format!(
            "{} grants NO terms. Measuring against it privately may be \
             possible; publishing anything derived from it is not.",
            corpus.id
        ))),
    }

    let declares_digest = corpus
        .integrity
        .as_ref()
        .is_some_and(|i| i.records_sha256.is_some());
    if declares_digest {
        steps.push(Step::run(
            format!(
                "stegobench score --corpus <your copy> --corpus-id {} \
                 --detector <name>",
                corpus.id
            ),
            "checks your copy against the declared digest first, so the \
             result can be quoted beside anybody else's over the same corpus",
        ));
    } else {
        steps.push(Step::just(format!(
            "{} declares no records digest, so a run over your copy is \
             `custom`: comparable with itself alone.",
            corpus.id
        )));
    }

    Needs {
        readiness: Readiness::Unknown,
        steps,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(toml_text: &str) -> Entry {
        toml::from_str(toml_text).expect("parses")
    }

    const SELFTEST: &str = "\n[selftest]\nmust_detect = \"a.png\"\nmust_clear = \"b.png\"\n";

    /// The block that makes a fixture detector drivable, which three of these
    /// fixtures used to leave out while asserting the entry was ready. A
    /// detector with no `[invoke]` is one nothing here can run, so those
    /// three were modelling an entry the registry would refuse to drive.
    const INVOKE: &str = "\n[invoke]\nargv = [\"{file}\"]\nparser = \"number\"\n";

    fn availability(entry: &Entry) -> Availability {
        stegobench_plugin::availability::check(entry, &[])
    }

    /// These tests are about the SHAPE of the needs block, not about where an
    /// adapter lives, so they ask with no roots. That falls back to the
    /// directory the test runs in, exactly as the signature without roots did.
    /// Adapter resolution has its own tests in `stegobench_plugin::adapter`.
    fn of_tool(entry: &Entry, availability: &Availability) -> Needs {
        super::of_tool(entry, availability, &[])
    }

    /// The headline claim of this module, asserted rather than described: a
    /// service, a container and a binary all answer in the same two fields.
    #[test]
    fn three_different_kinds_of_tool_answer_in_one_shape() {
        let digest = "5".repeat(64);
        let container = entry(&format!(
            "name = \"c\"\nkind = \"detector\"\nlicence = \"MIT\"\n\
             [image]\nreference = \"ghcr.io/x/y@sha256:{digest}\"\nsize_mb = 42\n{INVOKE}{SELFTEST}"
        ));
        let binary = entry(&format!(
            "name = \"b\"\nkind = \"detector\"\nlicence = \"MIT\"\n\
             upstream = \"https://example.invalid/b\"\n\
             [binary]\ncommand = [\"definitely-not-installed-xyzzy\"]\n\
             version_args = [\"--version\"]\n{INVOKE}{SELFTEST}"
        ));
        let service = entry(&format!(
            "name = \"s\"\nkind = \"detector\"\nlicence = \"MIT\"\n\
             [image]\nreference = \"ghcr.io/x/s@sha256:{digest}\"\n\
             [invoke]\nhost = true\nadapter = \"plugins/adapters/stegashield_one.py\"\n\
             entrypoint = \"python3\"\nargv = [\"{{adapter}}\", \"{{file}}\"]\n\
             endpoint_env = \"STEGOBENCH_TEST_ENDPOINT_UNSET\"\nparser = \"number\"\n{SELFTEST}"
        ));

        for e in [&container, &binary, &service] {
            let needs = of_tool(e, &availability(e));
            // Every one of them has something to say, and says it the same
            // way. The point of the module is that a reader never has to know
            // which of the three they are looking at.
            assert!(
                !needs.steps.is_empty(),
                "{} offered no next step at all",
                e.name
            );
            assert!(!needs.block().is_empty(), "{} rendered nothing", e.name);
            assert!(needs.to_json()["readiness"].is_string());
        }
    }

    #[test]
    fn a_container_is_told_to_pull_the_digest_and_not_a_tag() {
        let digest = "5".repeat(64);
        let e = entry(&format!(
            "name = \"c\"\nkind = \"detector\"\nlicence = \"MIT\"\n\
             [image]\nreference = \"ghcr.io/x/y@sha256:{digest}\"\nsize_mb = 42\n\
             {INVOKE}{SELFTEST}"
        ));
        // Built by hand rather than probed, so the test says the same thing on
        // a machine with the image pulled and on one without a runtime at all.
        let av = Availability {
            name: e.name.clone(),
            presence: Presence::Absent {
                reason: "not pulled".into(),
            },
            verified: None,
            missing_secrets: vec![],
        };
        let needs = of_tool(&e, &av);
        assert_eq!(needs.readiness, Readiness::NeedsYou);
        let line = needs.steps[0].run.as_deref().expect("a command to type");
        assert!(line.starts_with("docker pull "), "{line}");
        assert!(
            line.contains(&digest),
            "the pull must name the digest: {line}"
        );
        assert!(
            needs.steps[0].why.contains("42 MB"),
            "{}",
            needs.steps[0].why
        );
    }

    /// The case the whole module was written for. A service entry names an
    /// image, and telling the reader to pull it would send them to fetch bytes
    /// no run will execute.
    #[test]
    fn a_service_is_never_told_to_pull_the_image_that_only_names_it() {
        let digest = "5".repeat(64);
        let e = entry(&format!(
            "name = \"s\"\nkind = \"detector\"\nlicence = \"MIT\"\n\
             [image]\nreference = \"ghcr.io/x/s@sha256:{digest}\"\n\
             [invoke]\nhost = true\nadapter = \"plugins/adapters/stegashield_one.py\"\n\
             entrypoint = \"python3\"\nargv = [\"{{adapter}}\", \"{{file}}\"]\n\
             endpoint_env = \"STEGOBENCH_TEST_ENDPOINT_UNSET\"\nparser = \"number\"\n{SELFTEST}"
        ));
        let needs = of_tool(&e, &availability(&e));
        let rendered = needs.block();
        assert!(
            !rendered.contains("docker pull"),
            "a service was told to pull the image that only names it:\n{rendered}"
        );
        assert!(
            rendered.contains("export STEGOBENCH_TEST_ENDPOINT_UNSET="),
            "{rendered}"
        );
        assert_eq!(needs.readiness, Readiness::NeedsYou);
    }

    /// No value, ever. An address written into output somebody pastes into a
    /// bug report is the failure `endpoint_env` carries a name to prevent.
    #[test]
    fn an_endpoint_is_named_and_never_filled_in() {
        let key = "STEGOBENCH_TEST_ENDPOINT_UNSET";
        let digest = "5".repeat(64);
        let e = entry(&format!(
            "name = \"s\"\nkind = \"detector\"\nlicence = \"MIT\"\n\
             [image]\nreference = \"ghcr.io/x/s@sha256:{digest}\"\n\
             [invoke]\nhost = true\nadapter = \"plugins/adapters/stegashield_one.py\"\n\
             entrypoint = \"python3\"\nargv = [\"{{adapter}}\", \"{{file}}\"]\n\
             endpoint_env = \"{key}\"\nparser = \"number\"\n{SELFTEST}"
        ));
        let rendered = of_tool(&e, &availability(&e)).block();
        assert!(
            !rendered.contains("http://") && !rendered.contains("https://"),
            "an address reached the output:\n{rendered}"
        );
        assert!(
            rendered.contains("<the address of your own instance>"),
            "{rendered}"
        );
    }

    #[test]
    fn a_missing_secret_is_a_step_whatever_the_presence_answer_was() {
        let e = entry(&format!(
            "name = \"x\"\nkind = \"detector\"\nlicence = \"MIT\"\n\
             secrets = [\"STEGOBENCH_TEST_TOKEN_UNSET\"]\n\
             [binary]\ncommand = [\"sh\"]\nversion_args = [\"--version\"]\n{INVOKE}{SELFTEST}"
        ));
        let av = Availability {
            name: e.name.clone(),
            presence: Presence::Present { pin: "x".into() },
            verified: None,
            missing_secrets: vec!["STEGOBENCH_TEST_TOKEN_UNSET".into()],
        };
        let needs = of_tool(&e, &av);
        assert_eq!(
            needs.readiness,
            Readiness::NeedsYou,
            "a present tool with an unset token was reported ready"
        );
        assert!(needs
            .block()
            .contains("export STEGOBENCH_TEST_TOKEN_UNSET="));
        assert!(!needs.block().contains("docker pull"));
    }

    #[test]
    fn a_tool_with_everything_in_place_says_so_and_prints_nothing() {
        let e = entry(&format!(
            "name = \"x\"\nkind = \"detector\"\nlicence = \"MIT\"\n\
             [binary]\ncommand = [\"sh\"]\nversion_args = [\"--version\"]\n{INVOKE}{SELFTEST}"
        ));
        let av = Availability {
            name: e.name.clone(),
            presence: Presence::Present { pin: "x".into() },
            verified: None,
            missing_secrets: vec![],
        };
        let needs = of_tool(&e, &av);
        assert_eq!(needs.readiness, Readiness::Ready);
        assert_eq!(needs.block(), "");
    }

    /// A tool that cannot run here is not a job of work waiting, and the
    /// output must not offer a command that would not help.
    #[test]
    fn a_tool_that_cannot_run_here_offers_nothing_to_type() {
        let e = entry(&format!(
            "name = \"x\"\nkind = \"detector\"\nlicence = \"MIT\"\n\
             [binary]\ncommand = [\"sh\"]\n{SELFTEST}"
        ));
        let av = Availability {
            name: e.name.clone(),
            presence: Presence::Unsupported {
                reason: "this entry runs on windows and this machine is linux".into(),
            },
            verified: None,
            missing_secrets: vec![],
        };
        let needs = of_tool(&e, &av);
        assert_eq!(needs.readiness, Readiness::CannotRunHere);
        assert!(needs.steps.iter().all(|s| s.run.is_none()));
    }

    /// The registry this repository actually ships, walked in full: every
    /// entry must produce an answer, and an entry that needs something must
    /// offer a step rather than an empty list.
    #[test]
    fn every_shipped_entry_answers_the_question() {
        let dir =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugins/registry");
        let reg = stegobench_core::registry::Registry::load(&dir).expect("the real registry loads");
        assert!(
            reg.entries.len() >= 10,
            "walked {} entries; this registry ships thirteen",
            reg.entries.len()
        );
        for e in reg.entries.values() {
            let needs = of_tool(e, &availability(e));
            if needs.readiness == Readiness::NeedsYou {
                assert!(
                    !needs.steps.is_empty(),
                    "{} says it needs something and names nothing",
                    e.name
                );
            }
        }
        assert!(!reg.corpora.is_empty());
        for c in reg.corpora.values() {
            let needs = of_corpus(c);
            assert!(!needs.steps.is_empty(), "{} offered no next step", c.id);
            assert!(!needs.block().is_empty());
        }
    }

    #[test]
    fn a_corpus_that_needs_terms_accepting_says_so_before_it_says_download() {
        let dir =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugins/registry");
        let reg = stegobench_core::registry::Registry::load(&dir).expect("loads");
        let mut saw_acceptance = false;
        for c in reg.corpora.values() {
            if !c.obtain.requires_acceptance {
                continue;
            }
            saw_acceptance = true;
            let block = of_corpus(c).block();
            let accept = block.find("Accept").expect("the acceptance line");
            let download = block.find("Download").or_else(|| block.find("Download it"));
            if let Some(download) = download {
                assert!(
                    accept < download,
                    "{} tells the reader to download before it tells them to \
                     accept the terms:\n{block}",
                    c.id
                );
            }
        }
        assert!(
            saw_acceptance,
            "no shipped corpus requires acceptance, so this test verified \
             nothing. If that is now true of the registry, delete it rather \
             than leaving a check that cannot fail"
        );
    }
}
