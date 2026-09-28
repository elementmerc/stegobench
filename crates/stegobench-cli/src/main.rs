// Author:  Daniel Iwugo
// Comment: Christ is King
// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Daniel Iwugo
//! The `stegobench` command.
//!
//! THE TWO RULES THIS FILE EXISTS TO HOLD
//! --------------------------------------
//! **Machine output on stdout, human output on stderr.** This is the opposite
//! of what most tools do and it is deliberate: it means `stegobench ... --json
//! | jq` works while progress still reaches the terminal. A tool that mixes
//! them forces every caller to choose between being readable and being usable.
//!
//! **Every subcommand takes `--json`.** Not most of them. A caller that has to
//! remember which commands speak JSON will parse the ones that do not, and
//! retrofitting the flag later is far more work than carrying it from the
//! first commit.
//!
//! Exit codes are a contract and live in `stegobench_core::exit`. Codes 3 and
//! 7 are refusals rather than failures: the tool is capable of the thing and is
//! declining, so a caller that cannot tell them from an error will retry them
//! forever.

use std::io::Write;
use std::path::{Path, PathBuf};

use clap::{CommandFactory, Parser};
use stegobench_cli::cli::{Cli, Command, ReportFormat};
use stegobench_cli::help_topics;
use stegobench_cli::needs;
use stegobench_cli::report;
use stegobench_cli::score;
use stegobench_core::registry::{Kind, Registry};
use stegobench_core::{exit, ManifestV1, Result1, RunV1};
use stegobench_plugin::availability::Presence;
use stegobench_plugin::{availability, selftest, Verified};

/// What a subcommand produced: a JSON value for stdout, and human text for stderr.
#[cfg_attr(test, derive(Debug))]
struct Output {
    json: serde_json::Value,
    human: String,
    code: i32,
    /// Whether the human text IS the payload and belongs on stdout even
    /// though the command did not exit zero.
    ///
    /// One command needs this. `report` prints a table AND exits non-zero
    /// when a file could not be read, because the table is worth having and
    /// the code says it is short. Without this flag the table would be
    /// diverted to stderr in exactly the case a reader most needs to see it.
    payload_on_stdout: bool,
}

impl Output {
    fn ok(json: serde_json::Value, human: impl Into<String>) -> Self {
        Output {
            json,
            human: human.into(),
            code: exit::OK,
            payload_on_stdout: false,
        }
    }

    /// A refusal or a failure. `code` says which, and the distinction is the
    /// whole reason the codes are enumerated.
    fn err(code: i32, human: impl Into<String>) -> Self {
        let human = human.into();
        Output {
            json: serde_json::json!({ "ok": false, "error": human }),
            human,
            code,
            payload_on_stdout: false,
        }
    }
}

const KNOWN_SCHEMAS: &[&str] = &["result-v1", "run-v1", "manifest-v1"];

fn schema_value(name: &str) -> Option<serde_json::Value> {
    let schema = match name {
        "result-v1" => serde_json::to_value(schemars::schema_for!(Result1)),
        "run-v1" => serde_json::to_value(schemars::schema_for!(RunV1)),
        "manifest-v1" => serde_json::to_value(schemars::schema_for!(ManifestV1)),
        _ => return None,
    };
    Some(schema.expect("a generated schema serialises"))
}

fn cmd_schema(name: &str) -> Output {
    if name == "all" {
        let map: serde_json::Map<String, serde_json::Value> = KNOWN_SCHEMAS
            .iter()
            .map(|n| (n.to_string(), schema_value(n).expect("known schema")))
            .collect();
        return Output::ok(
            serde_json::Value::Object(map),
            "all schemas written to stdout",
        );
    }
    match schema_value(name) {
        Some(v) => Output::ok(v, format!("{name} schema written to stdout")),
        None => Output::err(
            exit::USAGE,
            format!(
                "unknown schema {name:?}. Known schemas: {}, or `all`",
                KNOWN_SCHEMAS.join(", ")
            ),
        ),
    }
}

fn load_registry(dir: &Path) -> Result<Registry, Output> {
    Registry::load(dir).map_err(|e| {
        Output::err(
            exit::FAILURE,
            format!(
                "{e}\n\nLooked in {}. Point --registry or STEGOBENCH_REGISTRY \
                 at the directory holding the tool descriptions.",
                dir.display()
            ),
        )
    })
}

/// The human block for the registered corpora.
///
/// An empty registry prints a sentence saying so rather than nothing at all.
/// A blank listing under an exit code of zero reads as "checked, all fine",
/// and the honest statement is that nothing has been declared, which is a
/// different fact from there being no corpus in the world. `list` is a listing
/// rather than a check, so an empty one is not a failure: it is the count that
/// has to be visible, and it is, in both the text and the JSON.
fn corpora_block(reg: &Registry, dir: &Path) -> String {
    if reg.corpora.is_empty() {
        return format!(
            "No corpora are registered in {}/corpora. A corpus entry declares \
             where a dataset lives and what its terms permit; nothing here \
             means nothing has been declared.",
            dir.display()
        );
    }
    let mut text = reg
        .corpora
        .values()
        .map(|c| c.summary())
        .collect::<Vec<_>>()
        .join("\n");
    let publishable = reg
        .corpora
        .values()
        .filter(|c| c.licence.redistribution.allows_publishing())
        .count();
    text.push_str(&format!(
        "\n\n{} corpora, {publishable} of which may be republished. \
         `describe <id>` prints the terms in full; a corpus you may use is not \
         always one you may publish.",
        reg.corpora.len()
    ));
    text
}

fn cmd_list(dir: &Path, kind: &str) -> Output {
    let reg = match load_registry(dir) {
        Ok(r) => r,
        Err(o) => return o,
    };
    if kind == "corpora" {
        return Output::ok(
            serde_json::json!({
                "corpora": reg.corpora.values().collect::<Vec<_>>(),
                "count": reg.corpora.len(),
            }),
            corpora_block(&reg, dir),
        );
    }
    let wanted: Vec<_> = match kind {
        "detectors" => reg.of_kind(Kind::Detector),
        "embedders" => reg.of_kind(Kind::Embedder),
        "all" => reg.entries.values().collect(),
        other => {
            return Output::err(
                exit::USAGE,
                format!("unknown kind {other:?}. Known: detectors, embedders, corpora, all"),
            )
        }
    };
    let mut human = wanted
        .iter()
        .map(|e| e.summary())
        .collect::<Vec<_>>()
        .join("\n");
    if human.is_empty() {
        // Same standard as `corpora_block`: a bare line under an exit code of
        // zero reads as "checked, all fine". Say where it looked, and say
        // whether the registry is empty or merely has nothing of this kind,
        // because those two call for different actions.
        human = if reg.entries.is_empty() {
            format!(
                "No tools are registered under {}, so there is nothing to list \
                 as {kind:?}. A tool entry declares how to run a detector or an \
                 embedder; nothing here means nothing has been declared, which \
                 is not the same as nothing existing.",
                dir.display()
            )
        } else {
            format!(
                "{} tool(s) are registered under {}, but none of them is a {}. \
                 `list all` prints every one.",
                reg.entries.len(),
                dir.display(),
                kind.trim_end_matches('s')
            )
        };
    } else {
        // Said once, under the listing, rather than in every entry's
        // description. The two routes are the only structural choice in this
        // registry and they cost the reader different things.
        // THREE ROUTES, NOT TWO, AND THE LEGEND USED TO NAME TWO.
        //
        // It glossed `container` as "runs in a sandbox with no network", which
        // is true of a container and false of an entry that names an image and
        // sets `invoke.host`: StegaShield is a service, the image identifies
        // the subject, and what runs is an adapter on this machine that posts
        // to an instance over HTTP. A reader who took the legend at its word
        // would believe a run of it was sandboxed and offline. Both halves of
        // that are wrong, and the network one is the one that matters.
        human.push_str(
            "\n\ncontainer  runs in a sandbox with no network, pinned by image \
             digest, so two machines run identical bytes. Needs a container \
             runtime.\nlocal      runs a program you installed, pinned by the \
             hash of the file that ran. No sandbox, and the hash is particular \
             to your build.",
        );
        // Counted rather than asserted. A sentence saying "one of these is a
        // service" would be a claim about a registry that changes, and the
        // point of this listing is that it cannot go stale.
        let services = wanted
            .iter()
            .filter(|e| e.invoke.as_ref().is_some_and(|i| i.host))
            .map(|e| e.name.as_str())
            .collect::<Vec<_>>();
        if !services.is_empty() {
            human.push_str(&format!(
                "\n\n{} listed as `container` above {} in fact a SERVICE: {}. \
                 The image identifies the subject, an adapter on this machine \
                 reaches an instance you started, and the network is not \
                 merely available to it, it is required. Not sandboxed.",
                services.len(),
                if services.len() == 1 { "is" } else { "are" },
                services.join(", ")
            ));
        }
        human.push_str("\n\n`stegobench doctor` says what each one still needs from you.");
        let f = reg.footprint();
        human.push_str(&format!(
            "\n\n{} tools in {} images. Default image at most {} MB \
             (they share base layers, so the built image is smaller); \
             {:.1} GB more available on demand.",
            f.tools,
            f.unique_images,
            f.bundled_mb,
            f.on_demand_mb as f64 / 1024.0
        ));
    }
    let mut json = serde_json::json!({ "tools": wanted, "footprint": reg.footprint() });
    if kind == "all" {
        human.push_str(&format!("\n\n{}", corpora_block(&reg, dir)));
        json["corpora"] = serde_json::json!(reg.corpora.values().collect::<Vec<_>>());
    }
    Output::ok(json, human)
}

fn cmd_describe(dir: &Path, name: &str) -> Output {
    let reg = match load_registry(dir) {
        Ok(r) => r,
        Err(o) => return o,
    };
    // The needs block goes FIRST, above the entry, because it is the question
    // somebody typing `describe` is usually asking. The TOML below it is the
    // whole truth and is what they read second. One shape for every subject:
    // a container, a binary, a service and a corpus all answer here, so a
    // reader never has to know which of the four they are holding.
    if let Some(e) = reg.entries.get(name) {
        let needs = needs::of_tool(e, &availability::check(e));
        let mut json = serde_json::to_value(e).unwrap_or(serde_json::Value::Null);
        if let Some(map) = json.as_object_mut() {
            map.insert("needs".into(), needs.to_json());
        }
        return Output::ok(json, describe_block(&needs, e.name.as_str(), e, "tool"));
    }
    // One vocabulary: a corpus id is looked up in the same breath as a tool
    // name, because a user should not have to know which of the two a thing is
    // before they can ask about it.
    if let Some(c) = reg.corpora.get(name) {
        let needs = needs::of_corpus(c);
        let mut json = serde_json::to_value(c).unwrap_or(serde_json::Value::Null);
        if let Some(map) = json.as_object_mut() {
            map.insert("needs".into(), needs.to_json());
        }
        return Output::ok(json, describe_block(&needs, c.id.as_str(), c, "corpus"));
    }
    let known: Vec<_> = reg
        .entries
        .keys()
        .chain(reg.corpora.keys())
        .cloned()
        .collect();
    Output::err(
        exit::USAGE,
        format!(
            "nothing registered as {name:?}. Known tools and corpora: {}",
            known.join(", ")
        ),
    )
}

/// What `describe` prints: the needs block, then the entry in full.
///
/// One renderer for a tool and for a corpus, on purpose. They are different
/// types with different fields, and the thing a reader wants first is the same
/// for both: whether they can use it, and what to type if not.
fn describe_block<T: serde::Serialize + std::fmt::Debug>(
    needs: &needs::Needs,
    name: &str,
    entry: &T,
    kind: &str,
) -> String {
    let body = toml::to_string_pretty(entry).unwrap_or_else(|_| format!("{entry:#?}"));
    let steps = needs.block();
    if steps.is_empty() {
        return format!(
            "{name}  [{}]\nEverything this {kind} needs is here. Whether it \
             WORKS is what `stegobench doctor` asks and this does not.\n\n{body}",
            needs.readiness.word()
        );
    }
    format!(
        "{name}  [{}]\nWhat this {kind} needs from you:\n{steps}\n\n{body}",
        needs.readiness.word()
    )
}

fn cmd_doctor(dir: &Path, fixtures: &Path, no_selftest: bool) -> Output {
    let reg = match load_registry(dir) {
        Ok(r) => r,
        Err(o) => return o,
    };

    let mut rows = Vec::new();
    let (mut missing, mut broken, mut passed, mut skipped, mut answered) = (0, 0, 0, 0, 0);
    // Counted apart from `missing` on purpose. A tool that cannot run on this
    // operating system is not a thing the reader failed to install, and a
    // count that lumps the two together reads as a job of work waiting.
    let mut unsupported = 0;
    // Present, absent and "nobody can say" are three answers, and the third
    // one is a real state rather than a soft no. See the match below.
    let mut undetermined = 0;

    for entry in reg.entries.values() {
        let mut check = availability::check(entry);
        // Only ask a tool to prove itself if its code is actually here. Running
        // a self-test against a missing image produces a failure that says
        // "broken" when the truth is "absent", and those need different fixes.
        let verdict = if no_selftest || !check.presence.is_present() {
            Verified::Skipped("not attempted".into())
        } else {
            selftest::run(entry, fixtures)
        };
        check.verified = match &verdict {
            Verified::Passed => Some(true),
            Verified::Failed(_) => Some(false),
            Verified::Skipped(_) | Verified::Answered(_) => None,
        };
        match &check.presence {
            Presence::Unsupported { .. } => unsupported += 1,
            // Counted apart from `missing`, because it is not one. A service
            // whose address nobody has supplied is not a thing the reader
            // failed to install, and the summary line used to say "13 not
            // installed" over a registry where one of the thirteen needed an
            // `export` rather than a `docker pull`. That sends somebody
            // looking for a package that does not exist.
            Presence::Unknown { .. } => undetermined += 1,
            p if !p.is_present() => missing += 1,
            _ => {}
        }
        match &verdict {
            Verified::Passed => passed += 1,
            Verified::Failed(_) => broken += 1,
            Verified::Answered(_) => answered += 1,
            Verified::Skipped(_) => skipped += 1,
        }
        let detail = match &verdict {
            Verified::Failed(why) => format!("{}  ({why})", check.summary()),
            Verified::Answered(why) => {
                format!(
                    "{}  ({why})",
                    check.summary().replace("not verified", "answering ")
                )
            }
            Verified::Skipped(why) if !no_selftest && check.presence.is_present() => {
                format!("{}  ({why})", check.summary())
            }
            _ => check.summary(),
        };
        // Computed from the availability answer already paid for, so nothing
        // here asks the container runtime a second time.
        let needs = needs::of_tool(entry, &check);
        rows.push((check, verdict, detail, needs));
    }

    // One line per tool, and under the ones that need something, the lines to
    // type. Indented under their own tool rather than gathered at the bottom,
    // so a reader scanning thirteen rows finds the instruction beside the
    // problem rather than having to match names up afterwards.
    let mut human: Vec<String> = Vec::new();
    for (_, _, detail, needs) in &rows {
        human.push(detail.clone());
        let block = needs.block();
        if !block.is_empty() {
            for line in block.lines() {
                human.push(format!("  {line}"));
            }
        }
    }
    let needing = rows
        .iter()
        .filter(|(_, _, _, n)| n.readiness == needs::Readiness::NeedsYou)
        .count();
    human.push(String::new());
    human.push(format!(
        "{} tool(s): {passed} verified, {answered} answering, {broken} broken, \
         {missing} not installed, {undetermined} undetermined, {skipped} not \
         checked.",
        rows.len()
    ));
    if undetermined > 0 {
        human.push(format!(
            "{undetermined} of those is something nobody here can answer yet, \
             not something to install: a service needs the address of your own \
             instance, and a container image cannot be looked for without a \
             runtime. Its line says which."
        ));
    }
    if unsupported > 0 {
        human.push(format!(
            "{unsupported} of those cannot run on {} at all, so nothing to \
             install would change it.",
            std::env::consts::OS
        ));
    }
    if answered > 0 {
        human.push(
            "An answering subject is installed and responding. Whether it \n\
             detects anything is what the benchmark measures, not what this \n\
             check decides."
                .into(),
        );
    }
    if skipped > 0 {
        // Never let "we did not look" read as "it is fine".
        human.push(
            "A tool that was not checked is not a tool that works. Each skipped \n\
             line says why."
                .into(),
        );
    }
    if needing > 0 {
        human.push(format!(
            "{needing} tool(s) need something from you, and the lines to type \
             are indented under each one. `stegobench describe <name>` prints \
             the same thing with the whole entry beside it."
        ));
    }

    let json = serde_json::json!({
        "checked": rows.len(),
        "needing_action": needing,
        "verified": passed,
        "broken": broken,
        "missing": missing,
        "unsupported_here": unsupported,
        "undetermined": undetermined,
        "not_checked": skipped,
        "answering": answered,
        "tools": rows.iter().map(|(c, v, d, n)| serde_json::json!({
            "name": c.name,
            "present": c.presence.is_present(),
            "verified": c.verified,
            "status": match v {
                Verified::Passed => "passed",
                Verified::Failed(_) => "failed",
                Verified::Answered(_) => "answering",
                Verified::Skipped(_) => "not_checked",
            },
            "detail": d,
            "missing_secrets": c.missing_secrets,
            // The same field name and the same shape as `describe`'s, so a
            // script reads one structure whichever command it called.
            "needs": n.to_json(),
        })).collect::<Vec<_>>(),
    });

    let mut out = Output::ok(json, human.join("\n"));
    // An undetermined tool counts, and it did not used to only because it was
    // being counted as missing. A machine where a service's address has never
    // been set is not a machine `doctor` should pronounce fit: nothing has
    // established that the tool can be reached at all.
    if missing > 0 || broken > 0 || undetermined > 0 {
        out.code = exit::ENVIRONMENT_UNFIT;
    }
    out
}

/// Which of the three published schemas a document is judged against.
///
/// Dispatched on the document's own `schema` field rather than guessed from
/// its shape, so a document that claims to be a `result-v1` but is missing a
/// required field fails with a reason instead of being silently tried against
/// the wrong type.
fn cmd_validate(file: &Path) -> Output {
    let text = match std::fs::read_to_string(file) {
        Ok(t) => t,
        Err(e) => {
            return Output::err(
                exit::FAILURE,
                format!("cannot read {}: {e}", file.display()),
            )
        }
    };
    let sniff: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => {
            return Output::err(
                exit::SCHEMA_INVALID,
                format!("{} is not valid JSON: {e}", file.display()),
            )
        }
    };
    let schema_field = sniff.get("schema").and_then(|v| v.as_str());

    macro_rules! validate_as {
        ($ty:ty, $label:literal) => {{
            let parsed: $ty = match serde_json::from_str(&text) {
                Ok(v) => v,
                Err(e) => {
                    return Output::err(
                        exit::SCHEMA_INVALID,
                        format!("{} is not a {} document: {e}", file.display(), $label),
                    )
                }
            };
            match parsed.validate() {
                Ok(()) => Output::ok(
                    serde_json::json!({ "ok": true, "schema": parsed.schema }),
                    format!("{} is a valid {} document", file.display(), $label),
                ),
                Err(problems) => {
                    let mut out = Output::err(
                        exit::SCHEMA_INVALID,
                        format!(
                            "{} is not valid:\n  {}",
                            file.display(),
                            problems.join("\n  ")
                        ),
                    );
                    out.json = serde_json::json!({ "ok": false, "problems": problems });
                    out
                }
            }
        }};
    }

    match schema_field {
        Some(s) if s.starts_with("stegobench/result-v") => validate_as!(Result1, "result-v1"),
        Some(s) if s.starts_with("stegobench/run-v") => validate_as!(RunV1, "run-v1"),
        Some(s) if s.starts_with("stegobench/manifest-v") => {
            validate_as!(ManifestV1, "manifest-v1")
        }
        Some(other) => Output::err(
            exit::SCHEMA_INVALID,
            format!(
                "{} declares schema {other:?}, which this version does not \
                 know. Known: {}",
                file.display(),
                KNOWN_SCHEMAS.join(", ")
            ),
        ),
        None => Output::err(
            exit::SCHEMA_INVALID,
            format!(
                "{} has no \"schema\" field, so it cannot be dispatched to a \
                 validator. Known: {}",
                file.display(),
                KNOWN_SCHEMAS.join(", ")
            ),
        ),
    }
}

/// Re-checks a result against the corpus it claims to have measured.
///
/// The claim a result makes about its corpus is the one thing a reader cannot
/// check by reading the document: every other field describes the run, and this
/// one describes bytes somewhere else. So it is recomputed here rather than
/// compared with itself.
///
/// The limit is stated rather than hidden. The digest names what the corpus's
/// own records declare about their images, so a match proves the document and
/// the corpus describe the same manifest. It does not prove the images match
/// their records: that would mean rehashing every file, which is a different
/// and much slower question, and the answer to it belongs to whoever packed
/// the release.
fn cmd_verify(file: &Path, corpus: &Path) -> Output {
    let text = match std::fs::read_to_string(file) {
        Ok(t) => t,
        Err(e) => {
            return Output::err(
                exit::FAILURE,
                format!("cannot read {}: {e}", file.display()),
            )
        }
    };
    let result: Result1 = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => {
            return Output::err(
                exit::SCHEMA_INVALID,
                format!(
                    "{} is not a result-v1 document: {e}. `stegobench validate \
                     {}` says what is wrong with it",
                    file.display(),
                    file.display()
                ),
            )
        }
    };

    if result.corpus.digest.is_empty() {
        return Output::err(
            exit::VERIFY_MISMATCH,
            format!(
                "{} names no corpus digest, so there is nothing to check it \
                 against. A result written over a corpus whose records state \
                 no digests of their own carries none, and cannot be verified \
                 by this route",
                file.display()
            ),
        );
    }

    let found = match score::corpus_digest(corpus) {
        Ok(Some(d)) => d,
        Ok(None) => {
            return Output::err(
                exit::VERIFY_MISMATCH,
                format!(
                    "the corpus at {} cannot be named: at least one of its \
                     records states no digest for its own image, so no digest \
                     over it would mean what {} claims",
                    corpus.display(),
                    file.display()
                ),
            )
        }
        Err(e) => return Output::err(exit::FAILURE, e.to_string()),
    };

    let claimed = &result.corpus.digest;
    let json = serde_json::json!({
        "ok": &found == claimed,
        "claimed": claimed,
        "found": found,
        "corpus": corpus.display().to_string(),
    });
    if &found == claimed {
        let mut out = Output::ok(
            json,
            format!(
                "{} was measured on the corpus at {}. Both name {claimed}",
                file.display(),
                corpus.display()
            ),
        );
        out.code = exit::OK;
        out
    } else {
        let mut out = Output::err(
            exit::VERIFY_MISMATCH,
            format!(
                "{} and the corpus at {} are not about each other.\n  \
                 the document claims {claimed}\n  the corpus is    {found}\n\
                 Whatever the two are called, the number in that document was \
                 not measured on these images",
                file.display(),
                corpus.display()
            ),
        );
        out.json = json;
        out
    }
}

fn cmd_completions(shell: clap_complete::Shell) -> Output {
    let mut cmd = Cli::command();
    let name = cmd.get_name().to_string();
    let mut buf = Vec::new();
    clap_complete::generate(shell, &mut cmd, name, &mut buf);
    // Not `unwrap_or_default()`: an empty string is a completion script that
    // silently does nothing, written to a shell's completion directory with an
    // exit code of zero. A generator that produced bytes we cannot read has to
    // say so.
    let script = match String::from_utf8(buf) {
        Ok(s) => s,
        Err(e) => {
            return Output::err(
                exit::FAILURE,
                format!(
                    "the {shell} completion generator produced {} bytes that \
                     are not valid UTF-8, so the script cannot be written. \
                     This is a bug in stegobench or clap_complete, not in \
                     your shell; please report it with this message: {e}",
                    e.as_bytes().len()
                ),
            )
        }
    };
    if script.trim().is_empty() {
        return Output::err(
            exit::FAILURE,
            format!(
                "the {shell} completion generator produced an empty script. \
                 Writing that to a completion directory would look like it \
                 worked and complete nothing, so it is refused instead"
            ),
        );
    }
    Output::ok(
        serde_json::json!({ "shell": shell.to_string(), "script": script }),
        script,
    )
}

fn cmd_help(topic: Option<&str>) -> Output {
    match topic {
        None => {
            let human = format!(
                "Known topics: {}\n\nExample:\n  stegobench help pairing",
                help_topics::TOPICS.join(", ")
            );
            Output::ok(serde_json::json!({ "topics": help_topics::TOPICS }), human)
        }
        Some(t) => match help_topics::text(t) {
            Some(text) => Output::ok(
                serde_json::json!({ "topic": t, "text": text }),
                text.to_string(),
            ),
            None => Output::err(
                exit::USAGE,
                format!(
                    "no help topic {t:?}. Known: {}",
                    help_topics::TOPICS.join(", ")
                ),
            ),
        },
    }
}

/// What a run would cost, without running it.
///
/// It takes the command as you would type it, rather than its own flags, so
/// there is no second set of arguments to keep in step with `score`. The same
/// parser reads both, which means a plan cannot silently describe a different
/// run from the one that would happen.
fn cmd_plan(registry_dir: &Path, command: &[String]) -> Output {
    if command.is_empty() {
        return Output::err(
            exit::USAGE,
            "plan takes the command you would run, for example:\n  \
             stegobench plan score --corpus ./pentimento-nano --detector zsteg"
                .to_string(),
        );
    }
    let argv = std::iter::once("stegobench".to_string()).chain(command.iter().cloned());
    let parsed = match Cli::try_parse_from(argv) {
        Ok(c) => c,
        Err(e) => {
            return Output::err(
                exit::USAGE,
                format!("that is not a command this can plan:\n{e}"),
            )
        }
    };
    let Command::Score {
        corpus,
        detector,
        corpus_id,
        limit,
        timeout,
        ..
    } = &parsed.command
    else {
        return Output::err(
            exit::USAGE,
            "only `score` can be planned today. Nothing else here runs long \
             enough to be worth estimating."
                .to_string(),
        );
    };

    let reg = match load_registry(registry_dir) {
        Ok(r) => r,
        Err(o) => return o,
    };
    let entries = match resolve_detectors(&reg, detector) {
        Ok(e) => e,
        Err(o) => return o,
    };

    // Counted rather than guessed from the directory size. Walking the corpus
    // is the only way to know how many scorable samples it holds, and an
    // estimate built on a guess is the thing a plan exists to replace.
    let mut items: u64 = 0;
    for sample in match stegobench_core::samples::Samples::open(corpus) {
        Ok(s) => s,
        Err(e) => return Output::err(exit::FAILURE, e.to_string()),
    } {
        if let Err(e) = sample {
            return Output::err(exit::FAILURE, e.to_string());
        }
        items += 1;
        if limit.is_some_and(|n| items >= n) {
            break;
        }
    }

    // What the run would be WORTH, beside what it would cost. A plan that
    // reports six hours and omits that the result will be `custom` has
    // answered half the question somebody asks before committing six hours.
    let (configuration, why) = plan_configuration(&reg, corpus_id.as_deref(), limit.is_some());

    // One JSON line per answer, measured at roughly sixty bytes on the real
    // records this writes, and one records file per detector.
    let records_mb = (items as f64 * 60.0 * entries.len() as f64) / 1_048_576.0;

    let mut per_detector = Vec::new();
    let mut lines = Vec::new();
    // Summed only over the detectors that declare a rate. A total that
    // silently treated an unmeasured tool as free would be the plan lying
    // about the one thing it is for, so the count of unestimated ones is
    // carried beside the total rather than folded into it.
    let mut total_seconds = 0.0f64;
    let mut unestimated = 0usize;
    for entry in &entries {
        let per_image = entry.cost.seconds_per_image;
        let seconds = per_image.map(|s| s * items as f64);
        match seconds {
            Some(s) => total_seconds += s,
            None => unestimated += 1,
        }
        let duration = match seconds {
            Some(s) => format!("about {}", human_duration(s)),
            None => "unknown: it declares no seconds_per_image".to_string(),
        };
        lines.push(format!("{:<16} {duration}", entry.name));
        per_detector.push(serde_json::json!({
            "detector": entry.name,
            "seconds_per_image": per_image,
            "estimated_seconds": seconds,
        }));
    }

    let mut value = serde_json::Map::new();
    value.insert("items".into(), serde_json::json!(items));
    value.insert(
        "detectors".into(),
        serde_json::json!(entries.iter().map(|e| &e.name).collect::<Vec<_>>()),
    );
    value.insert("per_detector".into(), serde_json::json!(per_detector));
    value.insert("estimated_seconds".into(), serde_json::json!(total_seconds));
    value.insert(
        "unestimated_detectors".into(),
        serde_json::json!(unestimated),
    );
    value.insert("records_mb".into(), serde_json::json!(records_mb));
    value.insert("configuration".into(), serde_json::json!(configuration));
    value.insert(
        "worst_case_seconds".into(),
        serde_json::json!(items * timeout * entries.len() as u64),
    );

    let mut human = format!(
        "{items} item(s) to score with each of {} detector(s):\n",
        entries.len()
    );
    human.push_str(&lines.join("\n"));
    let estimated = entries.len() - unestimated;
    if estimated == 0 {
        // "Total: about 0 seconds" over a set where nothing could be estimated
        // is a number that reads as free and means nothing was measured. No
        // total is the honest output.
        human.push_str(
            "\n\nNo total: not one of these declares a seconds_per_image, so \
             nothing here can estimate how long the run takes. The worst case \
             below is the only bound there is.",
        );
    } else {
        human.push_str(&format!(
            "\n\nTotal: about {}, over the {estimated} detector(s) that \
             declare a rate.",
            human_duration(total_seconds),
        ));
        if unestimated > 0 {
            // Said plainly rather than left for somebody to work out from the
            // rows. A total that reads as the whole job when it covers five of
            // seven is worse than no total.
            human.push_str(&format!(
                " {unestimated} of them declare no rate, so the real total is \
                 larger by an amount nothing here can estimate."
            ));
        }
    }
    human.push_str(&format!(
        "\nThe corpus is walked once however many detectors are asked, so \
         adding one costs its own scoring pass and nothing else.\nRecords \
         files: about {records_mb:.1} MB in total. Worst case, if every item \
         hit the {timeout}s deadline for every detector: {}.\nEach result \
         would be {configuration}: {why}",
        human_duration((items * timeout * entries.len() as u64) as f64)
    ));

    Output::ok(serde_json::Value::Object(value), human)
}

/// Whether the run being planned would earn a `named` result, and why.
///
/// Answered from the registry alone, without hashing the corpus. A plan is
/// meant to be cheap, and the expensive half of the real check is comparing a
/// digest this has deliberately not computed. So a run this calls `named` is
/// one that COULD be named, and `score` still has to agree.
fn plan_configuration(
    reg: &Registry,
    corpus_id: Option<&str>,
    limited: bool,
) -> (&'static str, String) {
    let Some(id) = corpus_id else {
        return (
            "custom",
            "no --corpus-id was given, so there is no registered corpus to \
             check this directory against. A custom result is comparable \
             with itself rather than with anybody else's number"
                .into(),
        );
    };
    let Some(entry) = reg.corpora.get(id) else {
        return (
            "custom",
            format!("no corpus with id {id:?} is registered, so `score` would refuse this"),
        );
    };
    if entry
        .integrity
        .as_ref()
        .and_then(|i| i.records_sha256.as_deref())
        .is_none()
    {
        return (
            "custom",
            format!(
                "{id} is registered but declares no records digest, so there \
                 is nothing to check this directory against"
            ),
        );
    }
    if limited {
        return (
            "custom",
            "--limit scores part of the corpus, and a prefix of a tier is \
             not the tier"
                .into(),
        );
    }
    (
        "named",
        format!(
            "{id} declares a records digest. If this directory matches it \
             the result can be quoted beside anybody else's run over the \
             same corpus, and if it does not `score` refuses before \
             anything runs"
        ),
    )
}

/// Seconds as something a person can judge a decision against.
fn human_duration(seconds: f64) -> String {
    if seconds < 90.0 {
        return format!("{seconds:.0} seconds");
    }
    if seconds < 5_400.0 {
        return format!("{:.0} minutes", seconds / 60.0);
    }
    format!("{:.1} hours", seconds / 3_600.0)
}

/// What happened to one detector of a `score` command.
///
/// Three outcomes, kept apart because they call for different things from the
/// reader and because two of them must never be read as the third. A skipped
/// detector is not a measured one that scored badly; it is a number that does
/// not exist.
enum Outcome {
    Measured {
        result: Box<Result1>,
        tally: stegobench_plugin::runner::Tally,
        written: Option<PathBuf>,
    },
    /// Not available on this machine, so nothing ran. Not fatal to the others.
    Skipped { why: String },
    /// It was available and the run did not produce a usable result.
    Failed { why: String, code: i32 },
}

/// Which detectors a `--detector` list names, in a deterministic order.
///
/// `all` is the whole registry's detectors rather than every entry: an
/// embedder cannot be asked to tell two images apart, and quietly including
/// one so it could be refused a moment later would be a worse answer than not
/// including it. Named explicitly, an embedder is still refused, because
/// somebody who typed its name has asked a question and is owed the answer.
fn resolve_detectors<'a>(
    reg: &'a Registry,
    asked: &[String],
) -> Result<Vec<&'a stegobench_core::registry::Entry>, Output> {
    if asked.iter().any(|d| d == "all") {
        if asked.len() > 1 {
            return Err(Output::err(
                exit::USAGE,
                "`--detector all` already means every registered detector, so \
                 naming others beside it asks for two different things. Use \
                 `all` on its own, or list the ones you want."
                    .to_string(),
            ));
        }
        let mut all = reg.of_kind(Kind::Detector);
        // Sorted by name, so two runs of the same command write the same files
        // in the same order however the registry happened to be read.
        all.sort_by(|a, b| a.name.cmp(&b.name));
        if all.is_empty() {
            return Err(Output::err(
                exit::USAGE,
                "`--detector all` was asked for and no detectors are \
                 registered, so there is nothing to score with. \
                 `stegobench list detectors` says where it looked."
                    .to_string(),
            ));
        }
        return Ok(all);
    }

    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    for name in asked {
        let Some(entry) = reg.entries.get(name) else {
            return Err(Output::err(
                exit::USAGE,
                format!(
                    "no tool named {name:?} is registered. \
                     `stegobench list detectors` shows what is."
                ),
            ));
        };
        // Refused here rather than after the corpus is walked. An embedder
        // hides a payload; it cannot be asked to tell two images apart, and
        // somebody who typed its name has asked for something impossible
        // rather than something unavailable.
        if entry.kind == Kind::Embedder {
            return Err(Output::err(
                exit::USAGE,
                format!(
                    "{name} is registered as an embedder, and an embedder \
                     cannot be asked to tell two images apart. \
                     `stegobench list detectors` shows what can."
                ),
            ));
        }
        // A name given twice is one measurement, not two. Silently scoring it
        // twice would write the second run over the first and report two.
        if seen.insert(name.clone()) {
            out.push(entry);
        }
    }
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
fn cmd_score(
    registry_dir: &Path,
    corpus: &Path,
    detectors: &[String],
    corpus_id: Option<&str>,
    records: Option<&Path>,
    out: Option<&Path>,
    timeout: u64,
    limit: Option<u64>,
) -> Output {
    let reg = match load_registry(registry_dir) {
        Ok(r) => r,
        Err(o) => return o,
    };
    let entries = match resolve_detectors(&reg, detectors) {
        Ok(e) => e,
        Err(o) => return o,
    };
    let many = entries.len() > 1;

    // Resolved before anything runs, so a typo costs a usage error rather than
    // a corpus walk followed by one.
    let registered = match corpus_id {
        None => None,
        Some(id) => match reg.corpora.get(id) {
            Some(entry) => Some(entry),
            None => {
                return Output::err(
                    exit::USAGE,
                    format!(
                        "no corpus with id {id:?} is registered. \
                         `stegobench list corpora` shows what is."
                    ),
                )
            }
        },
    };

    // A destination that cannot hold several documents is a usage error, and
    // it is raised before anything runs rather than after an hour. The
    // DIRECTORY itself is not created until at least one detector can run, so
    // a command that refuses leaves nothing behind.
    if many {
        for (flag, path) in [("--out", out), ("--records", records)] {
            if path.is_some_and(|p| p.is_file()) {
                return Output::err(
                    exit::USAGE,
                    format!(
                        "{} is a file, and scoring {} detectors writes one \
                         {} each, so {flag} has to name a directory here.",
                        path.expect("checked").display(),
                        entries.len(),
                        if flag == "--out" {
                            "result document"
                        } else {
                            "records file"
                        }
                    ),
                );
            }
        }
    }

    // Progress goes to stderr, so `--json` on stdout stays machine readable
    // while a person can still watch a run that takes hours.
    let say = |line: &str| eprintln!("  {line}");

    // Availability is asked for EVERY detector before the corpus is touched,
    // and the answers are kept. A seven-detector command should say at the
    // start which of the seven it can actually run, not discover the fourth is
    // missing after three hours. Presence only: the self-test is `doctor`'s
    // job and costs a container pull.
    let mut runnable = Vec::new();
    let mut outcomes: Vec<(String, Outcome)> = Vec::new();
    for entry in &entries {
        match availability::check(entry).presence {
            Presence::Present { .. } => runnable.push(*entry),
            Presence::Unsupported { reason } => outcomes.push((
                entry.name.clone(),
                Outcome::Skipped {
                    why: format!("cannot run on this machine: {reason}"),
                },
            )),
            Presence::Absent { reason } => outcomes.push((
                entry.name.clone(),
                Outcome::Skipped {
                    why: format!("is registered but is not on this machine: {reason}"),
                },
            )),
            Presence::Unknown { reason } => outcomes.push((
                entry.name.clone(),
                Outcome::Skipped {
                    why: format!("whether it can run here could not be established: {reason}"),
                },
            )),
        }
    }

    // With one detector asked for and that one unavailable, nothing was
    // measured and the corpus is not worth walking. Kept as the refusal it has
    // always been, with the same exit code, rather than becoming a zero-result
    // "run" that happens to have skipped everything.
    if runnable.is_empty() {
        let why = outcomes
            .iter()
            .map(|(name, o)| match o {
                Outcome::Skipped { why } => format!("{name} {why}"),
                _ => format!("{name} was not run"),
            })
            .collect::<Vec<_>>()
            .join("\n  ");
        return Output::err(
            exit::PREFLIGHT_REFUSED,
            format!(
                "nothing was measured: not one of the {} detector(s) asked for \
                 is available here.\n  {why}\n`stegobench doctor` checks every \
                 registered tool at once and says what each one needs.",
                entries.len()
            ),
        );
    }
    for (name, outcome) in &outcomes {
        if let Outcome::Skipped { why } = outcome {
            say(&format!("SKIPPING {name}: {why}"));
        }
    }

    // Created only now that something is going to be written into it. A
    // refusal above leaves no empty directory beside the corpus.
    let out_dir = match resolve_out(corpus, out, many) {
        Ok(d) => d,
        Err(o) => return o,
    };

    // The expensive half, once. Every refusal it can raise is a fact about the
    // corpus, so it applies to all the detectors equally and there is nothing
    // partial to report: the whole command stops.
    say(&format!(
        "establishing what the corpus is, once, for {} detector(s)",
        runnable.len()
    ));
    let prepared = match score::prepare(corpus, registered, limit, say) {
        Ok(p) => p,
        Err(e) => return Output::err(e.exit_code(), e.to_string()),
    };
    say(&format!(
        "corpus established in {:.1}s: {} item(s) to score with each detector",
        prepared.preflight_seconds(),
        prepared.items()
    ));

    for (i, entry) in runnable.iter().enumerate() {
        let records_path = records_for(corpus, records, &out_dir, &entry.name, many);
        // Counted against what CAN run rather than against what was asked for,
        // so "1 of 2" beside a third skipped detector does not read as a
        // miscount. The ratio of asked to measured is the summary's job and it
        // says it in full at the end.
        say(&format!(
            "scoring with {} ({} of {} that can run here)",
            entry.name,
            i + 1,
            runnable.len()
        ));
        let outcome = run_one(
            entry,
            &prepared,
            &records_path,
            timeout,
            out,
            out_dir.as_deref(),
            many,
            say,
        );
        outcomes.push((entry.name.clone(), outcome));
    }

    summarise(&outcomes, &out_dir, many)
}

/// Where the documents go, decided before any work happens.
///
/// `Ok(None)` means one detector writing to stdout or to the single file the
/// caller named. `Ok(Some(dir))` means a directory holding one document per
/// detector, created here so a failure to create it is a usage error rather
/// than something discovered after the first hour of scoring.
fn resolve_out(corpus: &Path, out: Option<&Path>, many: bool) -> Result<Option<PathBuf>, Output> {
    if !many {
        return Ok(None);
    }
    let dir = match out {
        Some(p) => p.to_path_buf(),
        // Beside the corpus and named after it, so two corpora scored on one
        // machine cannot write over each other's documents.
        None => {
            let mut name = corpus.file_name().unwrap_or_default().to_os_string();
            name.push(".results");
            corpus.with_file_name(name)
        }
    };
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return Err(Output::err(
            exit::FAILURE,
            format!("could not create {} for the results: {e}", dir.display()),
        ));
    }
    Ok(Some(dir))
}

/// Where one detector's per-item answers are kept.
///
/// THE NAME CARRIES THE DETECTOR, AND IT DID NOT USED TO
///
/// The default was `<corpus>.records.jsonl`, keyed on the corpus alone. Two
/// detectors over one corpus therefore shared a records file, and the second
/// one RESUMED from the first one's answers: it reported the first detector's
/// scores under its own name, with a resumed count that looked like a feature.
/// A records file is a record of what one subject said about one corpus, so
/// the name says both.
fn records_for(
    corpus: &Path,
    records: Option<&Path>,
    out_dir: &Option<PathBuf>,
    detector: &str,
    many: bool,
) -> PathBuf {
    let file = format!("{detector}.records.jsonl");
    match (records, many) {
        // One detector and an explicit path: exactly what the caller asked
        // for, because they named one run's file and there is one run.
        (Some(p), false) => p.to_path_buf(),
        // Several detectors: the caller named a directory to keep them in.
        (Some(p), true) => p.join(file),
        (None, true) => match out_dir {
            Some(dir) => dir.join(file),
            None => corpus.with_file_name(file),
        },
        (None, false) => {
            let mut name = corpus.file_name().unwrap_or_default().to_os_string();
            name.push(format!(".{file}"));
            corpus.with_file_name(name)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn run_one<P>(
    entry: &stegobench_core::registry::Entry,
    prepared: &score::Prepared,
    records: &Path,
    timeout: u64,
    out: Option<&Path>,
    out_dir: Option<&Path>,
    many: bool,
    say: P,
) -> Outcome
where
    P: Fn(&str),
{
    if let Some(parent) = records.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                return Outcome::Failed {
                    why: format!("could not create {} for the records: {e}", parent.display()),
                    code: exit::FAILURE,
                };
            }
        }
    }

    let (result, tally) = match score::score_one(
        entry,
        prepared,
        records,
        std::time::Duration::from_secs(timeout),
        |line: &str| say(line),
    ) {
        Ok(pair) => pair,
        Err(e) => {
            return Outcome::Failed {
                why: e.to_string(),
                code: e.exit_code(),
            }
        }
    };

    // Validated before it is written, not after. A document this refuses is
    // one no reader should have been handed in the first place.
    if let Err(problems) = result.validate() {
        return Outcome::Failed {
            why: format!(
                "the run finished but produced a result that does not \
                 validate, which is a bug in this harness rather than in the \
                 detector:\n  {}",
                problems.join("\n  ")
            ),
            code: exit::SCHEMA_INVALID,
        };
    }

    let body = match serde_json::to_string_pretty(&result) {
        Ok(b) => b,
        Err(e) => {
            return Outcome::Failed {
                why: format!("could not write the result: {e}"),
                code: exit::FAILURE,
            }
        }
    };

    // Written as this detector completes rather than at the end of the
    // command, so an interrupted seven-detector run keeps what it already
    // measured. That is the whole reason the loop is shaped this way.
    let destination = match (many, out_dir, out) {
        (true, Some(dir), _) => Some(dir.join(format!("{}.json", entry.name))),
        (false, _, Some(path)) => Some(path.to_path_buf()),
        _ => None,
    };
    if let Some(path) = &destination {
        if let Err(e) = std::fs::write(path, format!("{body}\n")) {
            return Outcome::Failed {
                why: format!("could not write the result to {}: {e}", path.display()),
                code: exit::FAILURE,
            };
        }
    }

    Outcome::Measured {
        result: Box::new(result),
        tally,
        written: destination,
    }
}

/// One line per detector and one line saying how many of them are real.
///
/// WHY THE COUNT IS ALWAYS PRINTED, EVEN WHEN IT IS `7 of 7`
///
/// The reader of a benchmark summary is deciding whether they have a baseline.
/// "AUC 0.83, 0.61, 0.55" over a registry of seven says nothing about the four
/// that are missing, and a reader who has to count the rows to find out is a
/// reader who will one day not bother. So the ratio leads, the skipped
/// detectors are named with their reasons, and the exit code carries the same
/// fact for anything that is not a person.
fn summarise(outcomes: &[(String, Outcome)], out_dir: &Option<PathBuf>, many: bool) -> Output {
    let measured = outcomes
        .iter()
        .filter(|(_, o)| matches!(o, Outcome::Measured { .. }))
        .count();
    let skipped = outcomes
        .iter()
        .filter(|(_, o)| matches!(o, Outcome::Skipped { .. }))
        .count();
    let failed = outcomes
        .iter()
        .filter(|(_, o)| matches!(o, Outcome::Failed { .. }))
        .count();

    let mut lines = Vec::new();
    for (name, outcome) in outcomes {
        match outcome {
            Outcome::Measured {
                result,
                tally,
                written,
            } => lines.push(format!(
                "{name:<16} AUC {:.4} over {} clean and {} stego image(s). \
                 {} scored, {} resumed, {} could not be answered.{}",
                result.metrics.auc,
                result.metrics.n_clean,
                result.metrics.n_stego,
                tally.scored,
                tally.resumed,
                result.metrics.n_error,
                match written {
                    Some(p) => format!(" Written to {}.", p.display()),
                    None => String::new(),
                }
            )),
            Outcome::Skipped { why } => {
                lines.push(format!("{name:<16} NOT MEASURED, skipped: {why}"))
            }
            Outcome::Failed { why, .. } => {
                lines.push(format!("{name:<16} NOT MEASURED, failed: {why}"))
            }
        }
    }

    lines.push(String::new());
    lines.push(format!(
        "{measured} of {} detector(s) measured.",
        outcomes.len()
    ));
    if skipped > 0 || failed > 0 {
        lines.push(format!(
            "{} detector(s) produced NO number at all ({skipped} skipped, \
             {failed} failed) and are named above. A table built from these \
             documents covers the {measured} that ran and nothing else.",
            skipped + failed
        ));
    }

    // A DIRECTORY IS NOT A RUN, AND A READER WILL TREAT IT AS ONE.
    //
    // A detector measured last week leaves its document behind. If it is
    // skipped this week, `stegobench report <dir>` still finds that document
    // and puts its number in the table, beside numbers from today, with
    // nothing in either the table or this summary saying so. The count above
    // would read "2 of 3" while the directory holds three. Nothing is deleted,
    // because a measurement is not this command's to throw away; it is named
    // instead, which is the half a reader cannot work out for themselves.
    if let Some(dir) = out_dir {
        // Everything in the directory that this run did not write, not only
        // the detectors it asked for and missed. A document from a detector
        // nobody named today is exactly as invisible in the resulting table,
        // and a run of five detectors into a directory that already held
        // seven produces a seven-row table under a "5 of 5 measured" summary.
        let written: std::collections::BTreeSet<&Path> = outcomes
            .iter()
            .filter_map(|(_, o)| match o {
                Outcome::Measured { written, .. } => written.as_deref(),
                _ => None,
            })
            .collect();
        let mut stale: Vec<String> = Vec::new();
        // A directory that cannot be read is not reported as clean: the
        // warning is skipped and the reason is carried instead, because
        // "nothing stale here" and "could not look" are different facts.
        match std::fs::read_dir(dir) {
            Ok(entries) => {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().is_some_and(|e| e == "json") && !written.contains(&*path) {
                        stale.push(path.display().to_string());
                    }
                }
                stale.sort();
            }
            Err(e) => lines.push(format!(
                "Could not list {} to check for documents left from earlier \
                 runs: {e}. A report over it may cover more than this run did.",
                dir.display()
            )),
        }
        if !stale.is_empty() {
            lines.push(format!(
                "WARNING: {} document(s) here were NOT measured by this run \
                 and are left over from an earlier one: {}. `stegobench report \
                 {}` will include them beside today's numbers, dated to when \
                 they were made. Move them aside if this run is meant to be \
                 the whole table.",
                stale.len(),
                stale.join(", "),
                dir.display()
            ));
        }
    }
    if many {
        if let Some(dir) = out_dir {
            lines.push(format!(
                "Next: stegobench report {} --format markdown",
                dir.display()
            ));
        }
    }

    // The code says the same thing the count says, for a caller that is not a
    // person. A failure outranks a skip because the two ask for different
    // things: a skip is something to install, a failure is something to
    // investigate, and reporting the louder one first is the honest order.
    let code = if failed > 0 {
        exit::PLUGIN_FAILED
    } else if skipped > 0 {
        exit::PREFLIGHT_REFUSED
    } else {
        exit::OK
    };

    let json = serde_json::json!({
        "ok": code == exit::OK,
        "requested": outcomes.len(),
        "measured": measured,
        "skipped": skipped,
        "failed": failed,
        "results_dir": out_dir.as_ref().map(|d| d.display().to_string()),
        "detectors": outcomes.iter().map(|(name, o)| match o {
            Outcome::Measured { result, tally, written } => serde_json::json!({
                "name": name,
                "status": "measured",
                "written": written.as_ref().map(|p| p.display().to_string()),
                "scored": tally.scored,
                "resumed": tally.resumed,
                "result": serde_json::to_value(result.as_ref())
                    .unwrap_or(serde_json::Value::Null),
            }),
            Outcome::Skipped { why } => serde_json::json!({
                "name": name, "status": "skipped", "reason": why,
            }),
            Outcome::Failed { why, code } => serde_json::json!({
                "name": name, "status": "failed", "reason": why, "exit_code": code,
            }),
        }).collect::<Vec<_>>(),
    });

    // ONE DETECTOR STILL BEHAVES EXACTLY AS IT ALWAYS DID.
    //
    // The whole multi-detector apparatus is the wrong shape for a command that
    // asked one question. A single run that worked puts the result document on
    // stdout, so `--json | jq` and `--json | stegobench validate -` keep
    // working, and a single run that failed returns its own exit code with its
    // own message rather than the summary's coarser one: an embedder refused
    // is a usage error and a one-sided corpus is a pre-flight refusal, and
    // flattening both to "a detector failed" would lose the distinction the
    // codes exist to carry.
    if !many {
        match outcomes.first() {
            Some((_, Outcome::Measured { result, .. })) => {
                let mut output = Output::ok(
                    serde_json::to_value(result.as_ref()).unwrap_or(serde_json::Value::Null),
                    lines.join("\n"),
                );
                output.code = code;
                return output;
            }
            Some((_, Outcome::Failed { why, code })) => return Output::err(*code, why.clone()),
            _ => {}
        }
    }

    let mut output = Output::ok(json, lines.join("\n"));
    output.code = code;
    output
}

/// Turns result documents into a table, and refuses to turn them into a
/// misleading one.
///
/// WHY A SKIPPED FILE STILL PRINTS A TABLE, AND STILL EXITS NON-ZERO
///
/// Two failures were available here and both are real. Refusing the whole
/// report over one bad file makes the command useless against a directory
/// somebody else assembled; dropping the file quietly produces a table that
/// looks complete and is not, which is the worse of the two by a distance.
/// So it does both halves of the honest thing: the table prints, the skipped
/// files are named at the TOP of it with their reasons, and the exit code
/// says the table is short, so a script cannot treat it as whole.
fn cmd_report(paths: &[PathBuf], format: ReportFormat, out: Option<&Path>) -> Output {
    let report = match report::build(paths) {
        Ok(r) => r,
        Err(e) => return Output::err(e.exit_code(), e.to_string()),
    };
    let body = report::render(&report, format);
    let json = report::to_json(&report);

    let human = match out {
        None => body,
        Some(path) => {
            if let Err(e) = report::write_atomically(path, &body) {
                return Output::err(e.exit_code(), e.to_string());
            }
            format!(
                "{} result(s) in {} table(s) written to {}. {}",
                report.read,
                report.groups.len(),
                path.display(),
                if report.skipped.is_empty() {
                    "Every file found was readable.".to_string()
                } else {
                    format!(
                        "{} file(s) could not be read and are named at the top \
                         of it, so the tables are incomplete.",
                        report.skipped.len()
                    )
                }
            )
        }
    };

    let mut output = Output::ok(json, human);
    output.code = report.exit_code();
    // The table goes to stdout when it IS the output. When `--out` took it,
    // the human text is a confirmation line and belongs on stderr with every
    // other progress message.
    output.payload_on_stdout = out.is_none();
    output
}

fn run(cli: &Cli) -> Output {
    match &cli.command {
        Command::Schema { name } => cmd_schema(name),
        Command::Validate { file } => cmd_validate(file),
        Command::Verify { file, corpus } => cmd_verify(file, corpus),
        Command::List { kind } => cmd_list(&cli.registry, kind),
        Command::Describe { name } => cmd_describe(&cli.registry, name),
        Command::Plan { command } => cmd_plan(&cli.registry, command),
        Command::Doctor {
            fixtures,
            no_selftest,
        } => cmd_doctor(&cli.registry, fixtures, *no_selftest),
        Command::Score {
            corpus,
            detector,
            corpus_id,
            records,
            out,
            timeout,
            limit,
        } => cmd_score(
            &cli.registry,
            corpus,
            detector.as_slice(),
            corpus_id.as_deref(),
            records.as_deref(),
            out.as_deref(),
            *timeout,
            *limit,
        ),
        Command::Report { paths, format, out } => cmd_report(paths, *format, out.as_deref()),
        Command::Completions { shell } => cmd_completions(*shell),
        Command::Help { topic } => cmd_help(topic.as_deref()),
    }
}

fn main() {
    let cli = Cli::parse();
    let out = run(&cli);

    if cli.json {
        let mut stdout = std::io::stdout().lock();
        let _ = serde_json::to_writer_pretty(&mut stdout, &out.json);
        let _ = writeln!(stdout);
    } else if out.payload_on_stdout {
        let mut stdout = std::io::stdout().lock();
        let _ = writeln!(stdout, "{}", out.human);
    } else if out.code == exit::OK {
        // Human mode still puts the payload on stdout when the payload IS the
        // point, so redirecting to a file works without remembering a flag.
        // `schema` writes structured JSON even in human mode, because the
        // schema itself is JSON; `completions` and `help` write the plain
        // text a shell or a reader wants, not a JSON wrapper around it.
        if matches!(cli.command, Command::Schema { .. }) {
            let mut stdout = std::io::stdout().lock();
            let _ = serde_json::to_writer_pretty(&mut stdout, &out.json);
            let _ = writeln!(stdout);
        } else if matches!(
            cli.command,
            Command::Completions { .. } | Command::Help { .. }
        ) {
            let mut stdout = std::io::stdout().lock();
            let _ = writeln!(stdout, "{}", out.human);
        } else {
            eprintln!("{}", out.human);
        }
    } else {
        eprintln!("{}", out.human);
    }

    std::process::exit(out.code);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Every code `stegobench_core::exit` publishes, which is what the help
    /// text, `llms.txt` and the man page all print as the contract.
    const CONTRACT_EXIT_CODES: &[i32] = &[
        exit::OK,
        exit::FAILURE,
        exit::USAGE,
        exit::PREFLIGHT_REFUSED,
        exit::PLUGIN_FAILED,
        exit::VERIFY_MISMATCH,
        exit::SCHEMA_INVALID,
        exit::LICENCE_REFUSED,
        exit::ENVIRONMENT_UNFIT,
        exit::INTERRUPTED,
    ];

    #[test]
    fn the_command_tree_is_well_formed() {
        Cli::command().debug_assert();
    }

    /// `build.rs` renders one man page per subcommand from this same tree, and
    /// a subcommand's page can only name `--json` and `--registry` once clap
    /// has propagated the globals into it. Pinned here because the failure is
    /// silent: the pages still generate, they just quietly stop describing two
    /// flags the binary accepts, which is the drift the generation exists to
    /// prevent.
    #[test]
    fn the_globals_reach_every_subcommand_so_the_man_pages_can_name_them() {
        let mut root = Cli::command();
        root.build();
        let mut seen = 0;
        for sub in root.get_subcommands() {
            let longs: Vec<&str> = sub.get_arguments().filter_map(|a| a.get_long()).collect();
            for global in ["json", "registry"] {
                assert!(
                    longs.contains(&global),
                    "`{}` does not carry --{global}, so its man page cannot \
                     name a flag the binary accepts",
                    sub.get_name()
                );
            }
            seen += 1;
        }
        assert!(seen >= 9, "checked only {seen} subcommand(s)");
    }

    #[test]
    fn json_is_accepted_by_every_subcommand() {
        // The rule in the module docstring, asserted rather than trusted.
        // A subcommand added later without --json fails here.
        for args in [
            vec!["stegobench", "--json", "schema", "result-v1"],
            vec!["stegobench", "--json", "validate", "x.json"],
            vec!["stegobench", "--json", "list", "detectors"],
            vec!["stegobench", "--json", "describe", "steghide"],
            vec!["stegobench", "--json", "plan", "score"],
            vec!["stegobench", "--json", "doctor"],
            vec!["stegobench", "--json", "doctor", "--no-selftest"],
            vec![
                "stegobench",
                "--json",
                "score",
                "--corpus",
                "x",
                "--detector",
                "y",
            ],
            vec!["stegobench", "--json", "completions", "bash"],
            vec!["stegobench", "--json", "help", "pairing"],
            vec!["stegobench", "--json", "help"],
        ] {
            assert!(Cli::try_parse_from(&args).is_ok(), "rejected: {args:?}");
        }
    }

    /// L: walks the actual command tree from `Cli::command()` rather than a
    /// hand-kept list, so a subcommand added later without `--json` fails
    /// here even if nobody remembered to update a fixture list by hand. Each
    /// subcommand's required positionals are filled with a placeholder value
    /// picked from what the argument declares (or `bash` for the one enum
    /// value, `completions`' shell), since the parse succeeding is what is
    /// under test, not what the placeholder does downstream.
    #[test]
    fn every_subcommand_in_the_tree_accepts_json_and_the_run_output_is_parseable() {
        let root = Cli::command();
        // A walker over an empty tree passes every assertion inside the loop
        // by never reaching one, which is the same fault as a check that
        // reports clean because it could not look. The tree is therefore
        // required to hold at least the commands the docs name.
        let walked = root.get_subcommands().count();
        assert!(
            walked >= 10,
            "walked {walked} subcommand(s); the command tree carries schema, \
             validate, verify, list, describe, doctor, plan, score, \
             completions and help, so this walk looked at almost nothing"
        );
        for sub in root.get_subcommands() {
            let name = sub.get_name().to_string();
            let mut argv = vec!["stegobench".to_string(), "--json".to_string(), name.clone()];
            for arg in sub.get_arguments() {
                if !arg.is_required_set() {
                    continue;
                }
                let placeholder = if name == "completions" {
                    "bash".to_string()
                } else {
                    "x".to_string()
                };
                if let Some(long) = arg.get_long() {
                    argv.push(format!("--{long}"));
                }
                argv.push(placeholder);
            }
            let parsed = Cli::try_parse_from(&argv);
            assert!(parsed.is_ok(), "{name:?} rejected --json: {argv:?}");

            // And the command actually runs to a well-formed, parseable JSON
            // payload rather than merely parsing its flags. `score`, `plan`
            // and any future not-yet-built command still emit a JSON error
            // object, which is exactly the point: --json is honoured on the
            // refusal path too.
            //
            // Asserting `to_string(&out.json).is_ok()` would be a check that
            // cannot fail: `out.json` is already a `serde_json::Value`, whose
            // object keys are `String`s, so serialising one always succeeds.
            // The bytes are therefore rendered and re-parsed, and the result
            // is required to be the documented envelope (an object) carrying
            // a code the exit contract actually names.
            let cli = parsed.unwrap();
            let out = run(&cli);
            let rendered =
                serde_json::to_string(&out.json).expect("a Value always renders to text");
            let reparsed: serde_json::Value = serde_json::from_str(&rendered)
                .unwrap_or_else(|e| panic!("{name:?} produced JSON that will not re-parse: {e}"));
            assert!(
                reparsed.is_object(),
                "{name:?} emitted {reparsed} on --json, but every subcommand's \
                 machine output is a JSON object"
            );
            assert!(
                CONTRACT_EXIT_CODES.contains(&out.code),
                "{name:?} exited {} with --json, which is not in the exit code \
                 contract the help text and the man page publish",
                out.code
            );
            assert!(
                !out.human.trim().is_empty(),
                "{name:?} produced no human text, so a failure would print \
                 nothing to stderr"
            );
        }
    }

    /// The registry this repository actually ships, not a fixture.
    fn shipped_registry() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugins/registry")
    }

    /// Every corpus file in the repository is loaded and validated.
    ///
    /// The count is asserted first and deliberately. A loop over an empty
    /// directory passes every assertion inside it by never reaching one, which
    /// is the same fault as a check that reports clean because it could not
    /// look: this file would go green on a registry with the corpora deleted.
    #[test]
    fn every_shipped_corpus_validates_and_there_is_at_least_one_to_validate() {
        let reg = Registry::load(&shipped_registry()).expect("the real registry loads");
        assert!(
            !reg.corpora.is_empty(),
            "no corpora were loaded from {}, so this test verified nothing. A \
             count of zero here is a failure, not a pass",
            shipped_registry().display()
        );
        for (id, corpus) in &reg.corpora {
            assert_eq!(corpus.validate(), Ok(()), "{id} is registered but invalid");
            assert_eq!(id, &corpus.id, "keyed under a name that is not its id");
        }
    }

    /// Every result document this repository ships must satisfy this
    /// repository's own validator.
    ///
    /// It did not, and nothing noticed. Two fields were added to `result-v1`
    /// in earlier work and the twenty-four documents under `results/` were
    /// left behind, so a reader following the README's own instruction to
    /// validate a document would have been told the project's own published
    /// measurements are not valid documents. A benchmark whose sample output
    /// its own tool refuses has undermined the point of publishing it.
    ///
    /// The same failure will happen again on the next schema change; the
    /// difference is that it will happen here rather than in front of a
    /// reader.
    #[test]
    fn every_result_this_repository_publishes_validates() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("results/v1");
        let mut checked = 0;
        for entry in std::fs::read_dir(&dir).expect("results/v1 exists") {
            let path = entry.expect("readable").path();
            if path.extension().is_none_or(|e| e != "json") {
                continue;
            }
            let out = cmd_validate(&path);
            assert_eq!(
                out.code,
                exit::OK,
                "{} does not validate:\n{}",
                path.display(),
                out.human
            );
            checked += 1;
        }
        // A walk over an empty directory passes every assertion by never
        // reaching one, which is the same fault as a check reporting clean
        // because it could not look.
        assert!(
            checked >= 20,
            "checked {checked} document(s) in {}; this repository publishes \
             two dozen, so the walk found almost nothing",
            dir.display()
        );
    }

    /// One registry to a user: the same two verbs reach a corpus and a tool.
    #[test]
    fn list_corpora_and_describe_reach_a_corpus_the_way_they_reach_a_tool() {
        let dir = shipped_registry();
        let listed = cmd_list(&dir, "corpora");
        assert_eq!(listed.code, exit::OK);
        assert!(
            listed.human.contains("reveal") && listed.human.contains("republish:"),
            "got: {}",
            listed.human
        );
        assert!(
            listed.json["count"].as_u64().is_some_and(|n| n > 0),
            "the machine output must carry the count, not only the rows"
        );

        let described = cmd_describe(&dir, "reveal");
        assert_eq!(described.code, exit::OK);
        assert_eq!(described.json["licence"]["redistribution"], "permitted");
        assert!(described.human.contains("verified_on"));

        // And a tool still answers the same verb.
        assert_eq!(cmd_describe(&dir, "steghide").code, exit::OK);
        // An unknown name is a usage error that names both kinds.
        let missing = cmd_describe(&dir, "not-registered");
        assert_eq!(missing.code, exit::USAGE);
        assert!(missing.human.contains("reveal") && missing.human.contains("steghide"));
    }

    /// `list all` is the whole registry, so leaving corpora out of it would
    /// make the two halves reachable only by knowing which is which.
    #[test]
    fn list_all_carries_the_corpora_as_well_as_the_tools() {
        let out = cmd_list(&shipped_registry(), "all");
        assert_eq!(out.code, exit::OK);
        assert!(out.json["tools"].as_array().is_some_and(|a| !a.is_empty()));
        assert!(out.json["corpora"]
            .as_array()
            .is_some_and(|a| !a.is_empty()));
        assert!(
            out.human.contains("may be republished"),
            "got: {}",
            out.human
        );
    }

    /// The tools half had the fault the corpora half was written to avoid: it
    /// printed `nothing registered under "detectors"` and exited zero, which
    /// says neither where it looked nor whether anything is registered at all.
    #[test]
    fn an_empty_tools_listing_says_where_it_looked() {
        let dir = tempfile::tempdir().unwrap();
        let out = cmd_list(dir.path(), "detectors");
        assert_eq!(out.code, exit::OK);
        assert!(
            out.human.contains("nothing has been declared")
                && out.human.contains(&dir.path().display().to_string()),
            "got: {}",
            out.human
        );
    }

    /// A registry with tools but none of this kind is a different fact from an
    /// empty registry, and calls for a different next step.
    #[test]
    fn a_kind_with_no_tools_is_distinguished_from_an_empty_registry() {
        // Built here rather than read from the shipped registry, which has
        // both kinds: a test that only checks this when the registry happens
        // to be one-sided is a test that can pass without looking.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("only-a-detector.toml"),
            format!(
                "name = \"solo\"\nkind = \"detector\"\nlicence = \"MIT\"\n\
                 [image]\nreference = \"ghcr.io/x/y@sha256:{}\"\n\
                 [selftest]\nmust_detect = \"a.png\"\nmust_clear = \"b.png\"\n",
                "5".repeat(64)
            ),
        )
        .unwrap();

        let out = cmd_list(dir.path(), "embedders");
        assert_eq!(out.code, exit::OK);
        assert!(out.json["tools"].as_array().is_some_and(|a| a.is_empty()));
        assert!(
            out.human.contains("1 tool(s) are registered")
                && out.human.contains("none of them is a embedder"),
            "got: {}",
            out.human
        );
        assert!(
            !out.human.contains("nothing has been declared"),
            "a registry with a tool in it was reported as empty: {}",
            out.human
        );
    }

    /// An empty listing under exit zero reads as "checked, all fine". It has
    /// to say that nothing is declared, which is a different fact.
    #[test]
    fn an_empty_corpora_listing_says_so_rather_than_printing_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let out = cmd_list(dir.path(), "corpora");
        assert_eq!(out.code, exit::OK);
        assert_eq!(out.json["count"], 0);
        assert!(
            out.human.contains("nothing has been declared"),
            "an empty registry printed: {:?}",
            out.human
        );
    }

    #[test]
    fn an_unknown_list_kind_names_corpora_among_the_known_ones() {
        let out = cmd_list(&shipped_registry(), "corpuses");
        assert_eq!(out.code, exit::USAGE);
        assert!(out.human.contains("corpora"), "got: {}", out.human);
    }

    #[test]
    fn every_shell_gets_a_script_with_the_command_names_in_it() {
        for shell in [
            clap_complete::Shell::Bash,
            clap_complete::Shell::Zsh,
            clap_complete::Shell::Fish,
            clap_complete::Shell::PowerShell,
            clap_complete::Shell::Elvish,
        ] {
            let out = cmd_completions(shell);
            assert_eq!(out.code, exit::OK, "{shell} completions failed");
            assert!(
                out.human.contains("doctor") && out.human.contains("validate"),
                "{shell} script does not mention the subcommands it completes"
            );
        }
    }

    #[test]
    fn schema_generates_and_names_the_format() {
        let out = cmd_schema("result-v1");
        assert_eq!(out.code, exit::OK);
        let text = serde_json::to_string(&out.json).unwrap();
        assert!(
            text.contains("Result1"),
            "schema should describe the result type"
        );
        assert!(
            text.contains("n_error"),
            "the required honesty field must be in the schema"
        );
    }

    #[test]
    fn an_unknown_schema_name_is_a_usage_error_not_a_crash() {
        assert_eq!(cmd_schema("result-v9").code, exit::USAGE);
    }

    #[test]
    fn validating_a_missing_file_fails_without_panicking() {
        let out = cmd_validate(&PathBuf::from("/definitely/not/here.json"));
        assert_eq!(out.code, exit::FAILURE);
    }

    #[test]
    fn a_non_result_document_exits_schema_invalid() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.json");
        std::fs::write(&p, r#"{"hello":"world"}"#).unwrap();
        assert_eq!(cmd_validate(&p).code, exit::SCHEMA_INVALID);
    }

    /// A corpus of `n` samples whose records state the digests given.
    fn corpus_named(root: &Path, digests: &[&str]) {
        std::fs::create_dir_all(root).unwrap();
        for (i, d) in digests.iter().enumerate() {
            let role = if i == 0 { "clean" } else { "stego" };
            std::fs::write(root.join(format!("i{i}.png")), b"\x89PNG\r\n\x1a\n").unwrap();
            std::fs::write(
                root.join(format!("i{i}.json")),
                format!(r#"{{"role":"{role}","sha256":"{d}"}}"#),
            )
            .unwrap();
        }
    }

    /// A result-v1 document claiming it was measured on `digest`.
    fn result_claiming(path: &Path, digest: &str) {
        let doc = serde_json::json!({
            "schema": stegobench_core::result::RESULT_SCHEMA_ID,
            "subject": {"name": "x", "version": "sha256:a", "kind": "detector"},
            "corpus": {"name": "c", "source": "supplied", "digest": digest, "pairs": 2},
            "arm": {"embedder": "wow", "domain": "spatial", "format": "png"},
            "metrics": {"auc": 0.9, "tpr_at_fpr": {}, "n_clean": 1, "n_stego": 1, "n_error": 0},
            "provenance": {
                "plugins": [{"name": "x", "image": "sha256:a", "determinism": "nondeterministic", "route": "local"}],
                "harness_version": "0.1.0",
                "started_utc": "2026-09-25T00:00:00Z",
                "elapsed_seconds": 1.0,
                "network_reachable": false
            },
            "declarations": {
                "split_discipline": "not-applicable",
                "pairing": "unverified",
                "configuration": "custom",
                "self_reported": false
            }
        });
        std::fs::write(path, serde_json::to_string(&doc).unwrap()).unwrap();
    }

    #[test]
    fn verify_agrees_when_the_document_and_the_corpus_are_the_same_corpus() {
        let dir = tempfile::tempdir().unwrap();
        let corpus = dir.path().join("corpus");
        corpus_named(&corpus, &["aa", "bb"]);
        let digest = score::corpus_digest(&corpus)
            .expect("readable")
            .expect("named");
        let doc = dir.path().join("r.json");
        result_claiming(&doc, &digest);
        let out = cmd_verify(&doc, &corpus);
        assert_eq!(out.code, exit::OK, "{}", out.human);
        assert_eq!(out.json["ok"], serde_json::json!(true));
    }

    #[test]
    fn verify_refuses_a_corpus_the_document_was_not_measured_on() {
        // The whole job. Two directories can have the same name, the same file
        // count and different contents, and the number in a result is about
        // exactly one of them.
        let dir = tempfile::tempdir().unwrap();
        let measured = dir.path().join("measured");
        let other = dir.path().join("other");
        corpus_named(&measured, &["aa", "bb"]);
        corpus_named(&other, &["aa", "bc"]);
        let digest = score::corpus_digest(&measured)
            .expect("readable")
            .expect("named");
        let doc = dir.path().join("r.json");
        result_claiming(&doc, &digest);

        let out = cmd_verify(&doc, &other);
        assert_eq!(out.code, exit::VERIFY_MISMATCH);
        assert!(out.human.contains("not about each other"), "{}", out.human);
        assert_eq!(out.json["ok"], serde_json::json!(false));
    }

    #[test]
    fn verify_says_so_when_there_is_nothing_to_check() {
        // A result over a corpus that cannot be named carries an empty digest,
        // and comparing an empty string with an empty string would pass while
        // proving nothing at all.
        let dir = tempfile::tempdir().unwrap();
        let corpus = dir.path().join("corpus");
        corpus_named(&corpus, &["aa", "bb"]);
        let doc = dir.path().join("r.json");
        result_claiming(&doc, "");
        let out = cmd_verify(&doc, &corpus);
        assert_eq!(out.code, exit::VERIFY_MISMATCH);
        assert!(
            out.human.contains("names no corpus digest"),
            "{}",
            out.human
        );
    }

    #[test]
    fn verify_refuses_a_corpus_that_cannot_be_named() {
        let dir = tempfile::tempdir().unwrap();
        let corpus = dir.path().join("corpus");
        std::fs::create_dir_all(&corpus).unwrap();
        std::fs::write(corpus.join("a.png"), b"\x89PNG\r\n\x1a\n").unwrap();
        std::fs::write(corpus.join("a.json"), r#"{"role":"clean"}"#).unwrap();
        let doc = dir.path().join("r.json");
        result_claiming(&doc, "sha256:whatever");
        let out = cmd_verify(&doc, &corpus);
        assert_eq!(out.code, exit::VERIFY_MISMATCH);
        assert!(out.human.contains("cannot be named"), "{}", out.human);
    }

    #[test]
    fn verify_sends_a_document_that_is_not_a_result_to_validate() {
        let dir = tempfile::tempdir().unwrap();
        let doc = dir.path().join("x.json");
        std::fs::write(&doc, r#"{"hello":"world"}"#).unwrap();
        let out = cmd_verify(&doc, dir.path());
        assert_eq!(out.code, exit::SCHEMA_INVALID);
        assert!(out.human.contains("stegobench validate"), "{}", out.human);
    }

    /// The table is the payload, so it has to reach stdout even when the
    /// command exits non-zero. It exits non-zero precisely when a file was
    /// dropped, which is the case a reader most needs the table in front of
    /// them for.
    #[test]
    fn a_report_with_a_dropped_file_still_puts_the_table_on_stdout() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../results/v1/rich-suniward-suniward-0400-aletheia-rs.json"),
            dir.path().join("good.json"),
        )
        .unwrap();
        std::fs::write(dir.path().join("bad.json"), "{nope").unwrap();

        let out = cmd_report(&[dir.path().to_path_buf()], ReportFormat::Text, None);
        assert_eq!(out.code, exit::SCHEMA_INVALID);
        assert!(out.payload_on_stdout, "the table was diverted to stderr");
        assert!(out.human.contains("THIS REPORT IS INCOMPLETE"));
        assert!(out.human.contains("bad.json"));
        assert!(out.human.contains("rich-suniward"));
        assert_eq!(out.json["complete"], serde_json::json!(false));
    }

    /// With --out the payload is the file, so the human text is a
    /// confirmation and belongs on stderr with the rest of the progress.
    #[test]
    fn writing_a_report_to_a_file_confirms_on_stderr_and_says_if_it_is_short() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../results/v1/rich-suniward-suniward-0400-aletheia-rs.json"),
            dir.path().join("good.json"),
        )
        .unwrap();
        let target = dir.path().join("out").join("report.md");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();

        let out = cmd_report(
            &[dir.path().join("good.json")],
            ReportFormat::Markdown,
            Some(&target),
        );
        assert_eq!(out.code, exit::OK);
        assert!(!out.payload_on_stdout);
        assert!(out.human.contains("Every file found was readable"));
        let written = std::fs::read_to_string(&target).unwrap();
        assert!(written.starts_with("# Steganalysis results"));
        assert!(written.contains("rich-suniward"));

        // And the short case says so in the confirmation, so a person
        // watching stderr does not have to open the file to find out.
        std::fs::write(dir.path().join("bad.json"), "{").unwrap();
        let out = cmd_report(
            &[dir.path().to_path_buf()],
            ReportFormat::Markdown,
            Some(&target),
        );
        assert_eq!(out.code, exit::SCHEMA_INVALID);
        assert!(out.human.contains("tables are incomplete"), "{}", out.human);
    }

    #[test]
    fn a_report_over_nothing_is_a_usage_error_rather_than_an_empty_table() {
        let dir = tempfile::tempdir().unwrap();
        let out = cmd_report(&[dir.path().to_path_buf()], ReportFormat::Text, None);
        assert_eq!(out.code, exit::USAGE);
        assert!(out.human.contains("no table to print"), "{}", out.human);
    }

    #[test]
    fn a_report_to_an_unwritable_destination_fails_rather_than_reporting_success() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../results/v1/rich-suniward-suniward-0400-aletheia-rs.json"),
            dir.path().join("good.json"),
        )
        .unwrap();
        let out = cmd_report(
            &[dir.path().to_path_buf()],
            ReportFormat::Csv,
            Some(&dir.path().join("no-such-dir").join("r.csv")),
        );
        assert_eq!(out.code, exit::FAILURE);
        assert!(out.human.contains("could not write the report"));
    }

    /// A registry holding one detector that works and one that is not here.
    ///
    /// Both are needed in the same registry, because the behaviour under test
    /// is what happens to the good one when the other is missing, and a test
    /// with only one of them cannot see it.
    #[cfg(unix)]
    fn registry_with_one_present_and_one_missing(dir: &Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let reg = dir.join("registry");
        std::fs::create_dir_all(&reg).unwrap();

        let script = dir.join("size.sh");
        std::fs::write(
            &script,
            "#!/bin/sh\n[ \"$1\" = \"--version\" ] && { echo v1; exit 0; }\n\
             wc -c < \"$1\" | tr -d ' ' | awk '{print $1/1000}'\n",
        )
        .unwrap();
        let mut perms = std::fs::metadata(&script).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script, perms).unwrap();

        std::fs::write(
            reg.join("sizer.toml"),
            format!(
                "name = \"sizer\"\nkind = \"detector\"\nlicence = \"MIT\"\n\
                 [binary]\ncommand = [{:?}]\nversion_args = [\"--version\"]\n\
                 [emits]\noutput = \"score\"\nhigher_means_stego = true\n\
                 [invoke]\nargv = [\"{{file}}\"]\nparser = \"number\"\n\
                 [selftest]\nmust_detect = \"a.png\"\nmust_clear = \"b.png\"\n",
                script.display().to_string()
            ),
        )
        .unwrap();
        std::fs::write(
            reg.join("ghost.toml"),
            "name = \"ghost\"\nkind = \"detector\"\nlicence = \"MIT\"\n\
             [binary]\ncommand = [\"definitely-not-installed-xyzzy\"]\n\
             version_args = [\"--version\"]\n\
             [selftest]\nmust_detect = \"a.png\"\nmust_clear = \"b.png\"\n",
        )
        .unwrap();
        reg
    }

    /// A PNG header with `padding` bytes after it, so a stego image is the
    /// same shape as its cover and only larger.
    fn test_png(padding: usize) -> Vec<u8> {
        let mut v = b"\x89PNG\r\n\x1a\n".to_vec();
        v.extend_from_slice(&13u32.to_be_bytes());
        v.extend_from_slice(b"IHDR");
        v.extend_from_slice(&32u32.to_be_bytes());
        v.extend_from_slice(&32u32.to_be_bytes());
        v.extend_from_slice(&[8, 2, 0, 0, 0]);
        v.extend_from_slice(&[0, 0, 0, 0]);
        v.extend_from_slice(&vec![0u8; padding]);
        v
    }

    fn scratch_corpus(root: &Path) {
        std::fs::create_dir_all(root).unwrap();
        for i in 0..3 {
            std::fs::write(root.join(format!("c{i}.png")), test_png(0)).unwrap();
            std::fs::write(
                root.join(format!("c{i}.json")),
                r#"{"role":"clean","sha256":"0"}"#,
            )
            .unwrap();
            std::fs::write(root.join(format!("s{i}.png")), test_png(64)).unwrap();
            std::fs::write(
                root.join(format!("s{i}.json")),
                format!(r#"{{"role":"stego","source_png":"c{i}.png","sha256":"0"}}"#),
            )
            .unwrap();
        }
    }

    /// THE FAILURE THIS WHOLE COMMAND HAS TO AVOID.
    ///
    /// A baseline over several detectors where one is missing must not exit
    /// zero, because a zero is read as "every detector I asked for was
    /// measured" and a script will read it that way forever. The other
    /// detector's work still has to survive.
    #[cfg(unix)]
    #[test]
    fn one_missing_detector_does_not_lose_the_others_and_never_exits_zero() {
        let tmp = tempfile::tempdir().unwrap();
        let reg = registry_with_one_present_and_one_missing(tmp.path());
        let corpus = tmp.path().join("corpus");
        scratch_corpus(&corpus);
        let out_dir = tmp.path().join("results");

        let out = cmd_score(
            &reg,
            &corpus,
            &["ghost".to_string(), "sizer".to_string()],
            None,
            None,
            Some(&out_dir),
            5,
            None,
        );

        assert_eq!(
            out.code,
            exit::PREFLIGHT_REFUSED,
            "a run that measured one of two detectors exited {}: {}",
            out.code,
            out.human
        );
        assert_ne!(out.code, exit::OK, "{}", out.human);

        // The good one's work survived and is on disk.
        let written = out_dir.join("sizer.json");
        assert!(written.exists(), "{}", out.human);
        assert_eq!(cmd_validate(&written).code, exit::OK);

        // And the summary says so in words a person reads, not only in a code.
        assert!(
            out.human.contains("1 of 2 detector(s) measured"),
            "{}",
            out.human
        );
        assert!(out.human.contains("NOT MEASURED"), "{}", out.human);
        assert_eq!(out.json["measured"], serde_json::json!(1));
        assert_eq!(out.json["skipped"], serde_json::json!(1));
        assert_eq!(out.json["requested"], serde_json::json!(2));
        assert_eq!(out.json["ok"], serde_json::json!(false));
    }

    /// A directory is not a run, and a reader will treat it as one.
    ///
    /// A detector measured yesterday leaves its document behind. Skipped
    /// today, that document is still in the directory and `report` will put
    /// its number in the table. Nothing is deleted, because a measurement is
    /// not this command's to throw away, but the summary has to say it.
    #[cfg(unix)]
    #[test]
    fn a_document_left_from_an_earlier_run_of_a_now_skipped_detector_is_named() {
        let tmp = tempfile::tempdir().unwrap();
        let reg = registry_with_one_present_and_one_missing(tmp.path());
        let corpus = tmp.path().join("corpus");
        scratch_corpus(&corpus);
        let out_dir = tmp.path().join("results");
        std::fs::create_dir_all(&out_dir).unwrap();
        // Stand in for last week's run of a detector that is gone today.
        std::fs::write(out_dir.join("ghost.json"), "{}").unwrap();

        let out = cmd_score(
            &reg,
            &corpus,
            &["ghost".to_string(), "sizer".to_string()],
            None,
            None,
            Some(&out_dir),
            5,
            None,
        );
        assert!(
            out.human.contains("NOT measured by this run"),
            "a stale document went unmentioned: {}",
            out.human
        );
        assert!(out.human.contains("ghost.json"), "{}", out.human);
        assert!(
            out_dir.join("ghost.json").exists(),
            "the stale document was deleted rather than named"
        );
    }

    /// The same hazard from the other direction, which the narrow version of
    /// the check missed entirely: a document from a detector nobody named
    /// today is exactly as invisible in the resulting table.
    #[cfg(unix)]
    #[test]
    fn a_document_from_a_detector_this_run_never_asked_for_is_named_too() {
        let tmp = tempfile::tempdir().unwrap();
        let reg = registry_with_one_present_and_one_missing(tmp.path());
        let corpus = tmp.path().join("corpus");
        scratch_corpus(&corpus);
        let out_dir = tmp.path().join("results");
        std::fs::create_dir_all(&out_dir).unwrap();
        std::fs::write(out_dir.join("somebody-elses-tool.json"), "{}").unwrap();

        let out = cmd_score(
            &reg,
            &corpus,
            &["sizer".to_string(), "ghost".to_string()],
            None,
            None,
            Some(&out_dir),
            5,
            None,
        );
        assert!(
            out.human.contains("somebody-elses-tool.json"),
            "a document nobody asked about today went unmentioned: {}",
            out.human
        );
    }

    /// And a clean directory says nothing, so the warning stays worth reading.
    #[cfg(unix)]
    #[test]
    fn a_run_that_wrote_every_document_in_its_directory_warns_about_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let reg = registry_with_one_present_and_one_missing(tmp.path());
        std::fs::copy(reg.join("sizer.toml"), reg.join("sizer2.toml")).unwrap();
        let text = std::fs::read_to_string(reg.join("sizer2.toml"))
            .unwrap()
            .replace("name = \"sizer\"", "name = \"sizer2\"");
        std::fs::write(reg.join("sizer2.toml"), text).unwrap();
        let corpus = tmp.path().join("corpus");
        scratch_corpus(&corpus);
        let out_dir = tmp.path().join("results");

        let out = cmd_score(
            &reg,
            &corpus,
            &["sizer".to_string(), "sizer2".to_string()],
            None,
            None,
            Some(&out_dir),
            5,
            None,
        );
        assert_eq!(out.code, exit::OK, "{}", out.human);
        assert!(
            !out.human.contains("NOT measured by this run"),
            "warned about documents it had just written itself: {}",
            out.human
        );
    }

    /// Two detectors sharing one records file was a live bug: the second
    /// resumed from the first one's answers and reported them as its own.
    #[cfg(unix)]
    #[test]
    fn two_detectors_over_one_corpus_do_not_resume_from_each_others_answers() {
        let tmp = tempfile::tempdir().unwrap();
        let reg = registry_with_one_present_and_one_missing(tmp.path());
        // A second working detector, so two of them actually run.
        std::fs::copy(reg.join("sizer.toml"), reg.join("sizer2.toml")).unwrap();
        let text = std::fs::read_to_string(reg.join("sizer2.toml"))
            .unwrap()
            .replace("name = \"sizer\"", "name = \"sizer2\"");
        std::fs::write(reg.join("sizer2.toml"), text).unwrap();

        let corpus = tmp.path().join("corpus");
        scratch_corpus(&corpus);
        let out_dir = tmp.path().join("results");

        let out = cmd_score(
            &reg,
            &corpus,
            &["sizer".to_string(), "sizer2".to_string()],
            None,
            None,
            Some(&out_dir),
            5,
            None,
        );
        assert_eq!(out.code, exit::OK, "{}", out.human);

        // Each scored all six items itself. A resumed count above zero here
        // would mean one of them read the other's file.
        for d in out.json["detectors"].as_array().expect("rows") {
            assert_eq!(d["scored"], serde_json::json!(6), "{d}");
            assert_eq!(
                d["resumed"],
                serde_json::json!(0),
                "{} resumed from another detector's records: {d}",
                d["name"]
            );
        }
        assert!(out_dir.join("sizer.records.jsonl").exists());
        assert!(out_dir.join("sizer2.records.jsonl").exists());
    }

    /// A resumed multi-detector run picks up per detector, so an interrupted
    /// baseline does not re-score what it already measured.
    #[cfg(unix)]
    #[test]
    fn a_second_run_of_the_same_command_resumes_each_detector_separately() {
        let tmp = tempfile::tempdir().unwrap();
        let reg = registry_with_one_present_and_one_missing(tmp.path());
        let corpus = tmp.path().join("corpus");
        scratch_corpus(&corpus);
        let out_dir = tmp.path().join("results");
        let args = ["sizer".to_string()];

        let first = cmd_score(&reg, &corpus, &args, None, None, Some(&out_dir), 5, None);
        assert_eq!(first.code, exit::OK, "{}", first.human);

        let second = cmd_score(&reg, &corpus, &args, None, None, Some(&out_dir), 5, None);
        assert_eq!(second.code, exit::OK, "{}", second.human);
        assert!(
            second.human.contains("6 resumed"),
            "the second run re-scored instead of resuming: {}",
            second.human
        );
    }

    /// With no `--out` and several detectors the documents still land
    /// somewhere findable, and the command says where rather than leaving the
    /// reader to guess.
    #[cfg(unix)]
    #[test]
    fn several_detectors_with_no_out_write_beside_the_corpus_and_say_where() {
        let tmp = tempfile::tempdir().unwrap();
        let reg = registry_with_one_present_and_one_missing(tmp.path());
        let corpus = tmp.path().join("corpus");
        scratch_corpus(&corpus);

        let out = cmd_score(
            &reg,
            &corpus,
            &["sizer".to_string(), "ghost".to_string()],
            None,
            None,
            None,
            5,
            None,
        );
        let expected = tmp.path().join("corpus.results");
        assert!(expected.join("sizer.json").exists(), "{}", out.human);
        assert!(
            out.human.contains(&expected.display().to_string()),
            "the command did not say where it wrote: {}",
            out.human
        );
        assert!(out.human.contains("stegobench report"), "{}", out.human);
    }

    /// One detector still behaves exactly as it always did: the document on
    /// stdout, nothing invented around it.
    #[cfg(unix)]
    #[test]
    fn a_single_detector_still_puts_the_result_document_on_stdout() {
        let tmp = tempfile::tempdir().unwrap();
        let reg = registry_with_one_present_and_one_missing(tmp.path());
        let corpus = tmp.path().join("corpus");
        scratch_corpus(&corpus);

        let out = cmd_score(
            &reg,
            &corpus,
            &["sizer".to_string()],
            None,
            None,
            None,
            5,
            None,
        );
        assert_eq!(out.code, exit::OK, "{}", out.human);
        assert_eq!(
            out.json["schema"],
            serde_json::json!(stegobench_core::result::RESULT_SCHEMA_ID),
            "a single run stopped emitting the result document itself"
        );
        assert!(out.json["metrics"]["auc"].is_number());
    }

    /// Asking for every detector when none of them is here is not a run that
    /// measured nothing; it is a refusal, and it keeps the exit code it has
    /// always had.
    #[cfg(unix)]
    #[test]
    fn a_command_where_no_detector_is_available_refuses_rather_than_reporting_zero() {
        let tmp = tempfile::tempdir().unwrap();
        let reg = tmp.path().join("registry");
        std::fs::create_dir_all(&reg).unwrap();
        std::fs::write(
            reg.join("ghost.toml"),
            "name = \"ghost\"\nkind = \"detector\"\nlicence = \"MIT\"\n\
             [binary]\ncommand = [\"definitely-not-installed-xyzzy\"]\n\
             version_args = [\"--version\"]\n\
             [selftest]\nmust_detect = \"a.png\"\nmust_clear = \"b.png\"\n",
        )
        .unwrap();
        let corpus = tmp.path().join("corpus");
        scratch_corpus(&corpus);

        let out = cmd_score(
            &reg,
            &corpus,
            &["all".to_string()],
            None,
            None,
            None,
            5,
            None,
        );
        assert_eq!(out.code, exit::PREFLIGHT_REFUSED, "{}", out.human);
        assert!(out.human.contains("nothing was measured"), "{}", out.human);
        assert!(!corpus.with_file_name("corpus.results").exists());
    }

    #[test]
    fn all_beside_a_named_detector_is_a_usage_error_rather_than_a_guess() {
        let reg = Registry::load(&shipped_registry()).expect("loads");
        let err = resolve_detectors(&reg, &["all".into(), "zsteg".into()]).expect_err("refused");
        assert_eq!(err.code, exit::USAGE);
        assert!(err
            .human
            .contains("already means every registered detector"));
    }

    #[test]
    fn all_is_every_detector_and_never_an_embedder() {
        let reg = Registry::load(&shipped_registry()).expect("loads");
        let picked = resolve_detectors(&reg, &["all".into()]).expect("resolved");
        assert!(picked.len() >= 5, "resolved only {}", picked.len());
        assert!(picked.iter().all(|e| e.kind == Kind::Detector));
        // Sorted, so two runs of the same command write the same files in the
        // same order.
        let names: Vec<_> = picked.iter().map(|e| e.name.clone()).collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted);
    }

    #[test]
    fn naming_an_embedder_is_refused_before_the_corpus_is_touched() {
        let reg = Registry::load(&shipped_registry()).expect("loads");
        let embedder = reg
            .of_kind(Kind::Embedder)
            .first()
            .map(|e| e.name.clone())
            .expect("the shipped registry has embedders");
        let err = resolve_detectors(&reg, std::slice::from_ref(&embedder)).expect_err("refused");
        assert_eq!(err.code, exit::USAGE);
        assert!(err
            .human
            .contains("cannot be asked to tell two images apart"));
    }

    #[test]
    fn the_same_detector_named_twice_is_one_measurement() {
        let reg = Registry::load(&shipped_registry()).expect("loads");
        let picked = resolve_detectors(&reg, &["zsteg".into(), "zsteg".into()]).expect("resolved");
        assert_eq!(picked.len(), 1);
    }

    /// `doctor` and `describe` answer the same question the same way,
    /// whichever kind of thing is asked about. The service is the case that
    /// forces this: it is the one a user could only discover by running it.
    #[test]
    fn doctor_and_describe_both_say_what_a_tool_needs_and_agree() {
        let dir = shipped_registry();
        let doctor = cmd_doctor(&dir, Path::new("fixtures"), true);
        assert!(
            doctor.json["needing_action"].is_number(),
            "doctor does not report how many tools need something"
        );
        for tool in doctor.json["tools"].as_array().expect("rows") {
            assert!(
                tool["needs"]["readiness"].is_string(),
                "{} has no readiness: {tool}",
                tool["name"]
            );
            assert!(tool["needs"]["steps"].is_array());
        }

        // And `describe` carries the identical structure for the same tool.
        let described = cmd_describe(&dir, "stegashield");
        assert_eq!(described.code, exit::OK);
        assert!(described.json["needs"]["readiness"].is_string());
        assert!(
            described.human.contains("What this tool needs from you")
                || described
                    .human
                    .contains("Everything this tool needs is here"),
            "{}",
            described.human
        );

        // A corpus answers in the same shape, which is the whole point: a
        // reader should not have to know which kind of thing they typed.
        let corpus = cmd_describe(&dir, "reveal");
        assert_eq!(corpus.code, exit::OK);
        assert!(corpus.json["needs"]["steps"].is_array());
        assert!(
            corpus.human.contains("What this corpus needs from you"),
            "{}",
            corpus.human
        );
    }

    /// The legend under `list` called every image entry a sandbox with no
    /// network, which is false for a service and false in the direction that
    /// matters most.
    #[test]
    fn the_list_legend_does_not_call_a_service_a_sandbox() {
        let out = cmd_list(&shipped_registry(), "detectors");
        assert_eq!(out.code, exit::OK);
        assert!(
            out.human.contains("in fact a SERVICE"),
            "the legend still presents every container entry as sandboxed: {}",
            out.human
        );
        assert!(out.human.contains("stegashield"), "{}", out.human);
    }

    /// Drives the binary's own code to every exit code the contract in
    /// `stegobench_core::exit` names, or records plainly why a given code
    /// cannot be reached yet.
    ///
    /// The man page is only as good as the binary's agreement with it: a
    /// documented code the binary never actually returns is worse than no
    /// table at all, because a script or an agent trusts the table. Codes 3
    /// (pre-flight refusal) and 7 (licence refusal) genuinely have NO code
    /// path yet, because nothing refuses a run on grounds of capacity and no
    /// corpus-licence gate is built.
    /// That is a true statement about this release, not a gap in the test:
    /// asserting it here means the day one of those codes becomes reachable
    /// without a test acknowledging it, this test starts failing to mention
    /// it rather than silently staying quiet about a codepath nobody wrote a
    /// test for.
    #[test]
    fn the_exit_code_contract_is_driven_or_explicitly_not_yet_reachable() {
        // 0: success.
        assert_eq!(cmd_schema("result-v1").code, exit::OK);

        // 1: generic failure.
        assert_eq!(
            cmd_validate(&PathBuf::from("/definitely/not/here.json")).code,
            exit::FAILURE
        );

        // 2: usage error.
        assert_eq!(cmd_schema("not-a-real-schema").code, exit::USAGE);
        assert_eq!(cmd_help(Some("not-a-real-topic")).code, exit::USAGE);

        // 3: pre-flight refusal. Reachable: `score` asks whether the detector
        // is on this machine before it walks the corpus, and refuses rather
        // than spending the walk to find out.
        {
            let dir = tempfile::tempdir().unwrap();
            let reg = dir.path().join("detectors");
            std::fs::create_dir_all(&reg).unwrap();
            std::fs::write(
                reg.join("ghost.toml"),
                "name = \"ghost\"\nkind = \"detector\"\nlicence = \"MIT\"\n\
                 [binary]\ncommand = [\"definitely-not-installed-xyzzy\"]\n\
                 version_args = [\"--version\"]\n\
                 [selftest]\nmust_detect = \"a.png\"\nmust_clear = \"b.png\"\n",
            )
            .unwrap();
            let out = cmd_score(
                dir.path(),
                &dir.path().join("no-such-corpus"),
                &["ghost".to_string()],
                None,
                None,
                None,
                5,
                None,
            );
            assert_eq!(out.code, exit::PREFLIGHT_REFUSED, "{}", out.human);
            // Proof it refused on the tool rather than on the missing corpus.
            assert!(out.human.contains("not on this machine"), "{}", out.human);
        }

        // 4: plugin failure. Reachable now that `score` runs: an embedder
        // asked to tell two images apart is refused through this code. It is
        // driven in the score module's own tests, which can build a corpus
        // and a registry entry without this test constructing both.

        // 5: verify mismatch. Reachable: `verify` recomputes a corpus digest
        // and compares it with the one a result claims. Driven below, against
        // a corpus the document was not measured on.

        // 6: schema invalid.
        {
            let dir = tempfile::tempdir().unwrap();
            let p = dir.path().join("x.json");
            std::fs::write(&p, r#"{"hello":"world"}"#).unwrap();
            assert_eq!(cmd_validate(&p).code, exit::SCHEMA_INVALID);
        }

        // 7: licence refusal. NOT YET REACHABLE: no corpus-licence gate
        // exists yet. See exit::LICENCE_REFUSED.

        // 8: environment unfit. NO LONGER REACHABLE FROM A STUB: every
        // command in the tree is built, so the `not_yet` helper that used to
        // return this code is gone rather than kept as scaffolding nothing
        // stands on. `cmd_doctor` still reaches it when a registered tool is
        // missing or broken, and that wants its own test with a real registry
        // directory rather than a shortcut here.

        // 130: interrupted. NOT YET REACHABLE from a unit test: this is a
        // signal-handler exit path (SIGINT/SIGTERM), which needs a real
        // process and a real signal to drive, not a function call. No
        // signal handling exists in this binary yet to test in the first
        // place.
    }
}
