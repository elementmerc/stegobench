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

use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};

use clap::{CommandFactory, Parser};
use stegobench_cli::cli::{Cli, Command, ReportFormat};
use stegobench_cli::fetch;
use stegobench_cli::fixtures;
use stegobench_cli::help_topics;
use stegobench_cli::metrics;
use stegobench_cli::needs;
use stegobench_cli::registry;
use stegobench_cli::registry::Resolved;
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

/// Finds the registry once, and hands it to the command that asked for it.
///
/// Every failure to find one is the same shape and the same exit code, so the
/// five commands that read a registry report it identically rather than each
/// inventing its own wording.
fn with_registry(cli: &Cli, f: impl FnOnce(&Resolved) -> Output) -> Output {
    match registry::resolve(cli.registry.as_deref()) {
        Ok(resolved) => f(&resolved),
        Err(e) => Output::err(e.exit_code(), e.to_string()),
    }
}

/// The human block for the registered corpora.
///
/// An empty registry prints a sentence saying so rather than nothing at all.
/// A blank listing under an exit code of zero reads as "checked, all fine",
/// and the honest statement is that nothing has been declared, which is a
/// different fact from there being no corpus in the world. `list` is a listing
/// rather than a check, so an empty one is not a failure: it is the count that
/// has to be visible, and it is, in both the text and the JSON.
fn corpora_block(reg: &Registry, where_from: &str) -> String {
    if reg.corpora.is_empty() {
        return format!("no corpora registered in {where_from}");
    }
    // Aligned over the rows actually being printed. A fixed pad here silently
    // stopped lining up the day a corpus with a longer id than the literal was
    // registered, and nothing failed to say so.
    let rows = reg.corpora.values().map(|c| c.cells()).collect::<Vec<_>>();
    let mut text = stegobench_core::table::align(&rows).join("\n");
    let publishable = reg
        .corpora
        .values()
        .filter(|c| c.licence.redistribution.allows_publishing())
        .count();
    // The use/publish split stays: it is the distinction the `redistribution`
    // field exists for, and a reader who misses it republishes something they
    // may only measure against.
    text.push_str(&format!(
        "\n\n{} corpora, {publishable} republishable. Using one is not \
         publishing it; `describe <id>` prints the terms.",
        reg.corpora.len()
    ));
    text
}

fn cmd_list(resolved: &Resolved, kind: &str) -> Output {
    let reg = &resolved.registry;
    // Said on every listing, not only in `doctor`. `list` is the first command
    // a person runs, and a machine with an installed registry and a checkout
    // has two answers to "what can this run"; which one answered is part of
    // the answer.
    let where_from = match resolved.source.path() {
        Some(p) => p.display().to_string(),
        None => "built in".to_string(),
    };
    let corpora_from = match resolved.source.path() {
        Some(p) => format!("{}/corpora", p.display()),
        None => where_from.clone(),
    };
    if kind == "corpora" {
        return Output::ok(
            serde_json::json!({
                "corpora": reg.corpora.values().collect::<Vec<_>>(),
                "count": reg.corpora.len(),
                "registry": resolved.to_json(),
            }),
            format!(
                "{}\n\n{}",
                corpora_block(reg, &corpora_from),
                resolved.source.line()
            ),
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
    // Same alignment rule as `corpora_block`, and for the same reason: the
    // widest name in the listing sets the column, not a literal in the source.
    let rows = wanted.iter().map(|e| e.cells()).collect::<Vec<_>>();
    let mut human = stegobench_core::table::align(&rows).join("\n");
    if human.is_empty() {
        // Same standard as `corpora_block`: a bare line under an exit code of
        // zero reads as "checked, all fine". Say where it looked, and say
        // whether the registry is empty or merely has nothing of this kind,
        // because those two call for different actions.
        human = if reg.entries.is_empty() {
            format!("no tools registered in {where_from}")
        } else {
            format!(
                "none of the {} tool(s) in {where_from} is a {}. `list all` \
                 prints every one.",
                reg.entries.len(),
                kind.trim_end_matches('s')
            )
        };
    } else {
        // Said once, under the listing, rather than in every entry's
        // description. TWO FACTS, NOT ONE: what pins the tool, and what it can
        // reach. The legend used to carry them as a single word and was wrong
        // about the third shape, where the image names the subject and an
        // adapter here does the running. The vocabulary matches
        // `provenance.plugins[].pinned_by` and `.isolation` in a result, so
        // the listing and the document say the same words.
        human.push_str(
            "\n\ncontainer  pinned by image digest, sandboxed, no network. \
             Needs a container runtime.\nlocal      a program you installed, \
             pinned by the hash of the file that ran. No sandbox, your \
             network.",
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
                "\n\nSERVICE, not sandboxed, and needs the network: {}. The \
                 image names the subject; an adapter here reaches an instance \
                 you started. Nothing checks that instance was built from that \
                 image, so a result of it reads `unpinned`.",
                services.join(", ")
            ));
        }
        human.push_str("\n\n`stegobench doctor` says what each one still needs.");
        // Over what was listed, not over the registry. Printed under seven
        // detectors, a thirteen tool total reads as the cost of the seven.
        let f = stegobench_core::registry::Registry::footprint_of(wanted.iter().copied());
        human.push_str(&format!(
            "\n\n{} tools in {} images. Up to {} MB bundled, {:.1} GB more on \
             demand.",
            f.tools,
            f.unique_images,
            f.bundled_mb,
            f.on_demand_mb as f64 / 1024.0
        ));
    }
    let mut json = serde_json::json!({
        "tools": wanted,
        "footprint": stegobench_core::registry::Registry::footprint_of(wanted.iter().copied()),
        "registry": resolved.to_json(),
    });
    if kind == "all" {
        human.push_str(&format!("\n\n{}", corpora_block(reg, &corpora_from)));
        json["corpora"] = serde_json::json!(reg.corpora.values().collect::<Vec<_>>());
    }
    human.push_str(&format!("\n\n{}", resolved.source.line()));
    Output::ok(json, human)
}

fn cmd_describe(resolved: &Resolved, name: &str) -> Output {
    let reg = &resolved.registry;
    // The needs block goes FIRST, above the entry, because it is the question
    // somebody typing `describe` is usually asking. The TOML below it is the
    // whole truth and is what they read second. One shape for every subject:
    // a container, a binary, a service and a corpus all answer here, so a
    // reader never has to know which of the four they are holding.
    if let Some(e) = reg.entries.get(name) {
        let needs = needs::of_tool(
            e,
            &availability::check(e, resolved.adapter_roots()),
            resolved.adapter_roots(),
        );
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
            "{name}  [{}]\nNothing needed. `stegobench doctor` says whether \
             this {kind} works.\n\n{body}",
            needs.readiness.word()
        );
    }
    format!(
        "{name}  [{}]\nNeeds from you:\n{steps}\n\n{body}",
        needs.readiness.word()
    )
}

/// `fixtures` is `None` when the self-tests are not being run, which is the
/// only state in which no fixtures are resolved at all. Carrying the resolved
/// fixtures rather than just their directory is what lets the report say WHICH
/// ones answered: a stale `./fixtures` beside a checkout and the copy compiled
/// into the binary are different bytes, and a self-test result is about the
/// ones it actually read.
fn cmd_doctor(resolved: &Resolved, fixtures: Option<&fixtures::Fixtures>) -> Output {
    let reg = &resolved.registry;
    let no_selftest = fixtures.is_none();

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
        let mut check = availability::check(entry, resolved.adapter_roots());
        // Only ask a tool to prove itself if its code is actually here. Running
        // a self-test against a missing image produces a failure that says
        // "broken" when the truth is "absent", and those need different fixes.
        let verdict = match fixtures {
            Some(f) if check.presence.is_present() => {
                selftest::run(entry, f.dir(), resolved.adapter_roots())
            }
            _ => Verified::Skipped("not attempted".into()),
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
        let needs = needs::of_tool(entry, &check, resolved.adapter_roots());
        rows.push((check, verdict, detail, needs));
    }

    // One line per tool, and under the ones that need something, the lines to
    // type. Indented under their own tool rather than gathered at the bottom,
    // so a reader scanning thirteen rows finds the instruction beside the
    // problem rather than having to match names up afterwards.
    let mut human: Vec<String> = Vec::new();
    // First line, before any tool. Every row below is a fact about the
    // registry that answered, and a machine can easily hold two: a checkout,
    // an installed copy, and the one compiled into the binary. A reader
    // looking at a row they did not expect needs to know which one it came
    // from before they start looking for the tool.
    human.push(resolved.source.line());
    // And the same question about the fixtures, for the same reason: a
    // self-test verdict is about the bytes it read, and there is more than one
    // place those can come from.
    human.push(match fixtures {
        Some(f) => f.line(),
        None => "fixtures  not read, because the self-tests were not run".to_string(),
    });
    human.push(String::new());
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
            "{undetermined} undetermined: nothing to install would settle it. \
             Its line says what it needs."
        ));
    }
    if unsupported > 0 {
        human.push(format!(
            "{unsupported} cannot run on {} at all.",
            std::env::consts::OS
        ));
    }
    if answered > 0 {
        human.push("answering: installed and responding, not proved accurate.".into());
    }
    if skipped > 0 {
        // Never let "we did not look" read as "it is fine".
        human.push("not checked is not the same as working; each line says why.".into());
    }
    if needing > 0 {
        human.push(format!(
            "{needing} need something from you, indented under each."
        ));
    }

    let json = serde_json::json!({
        "registry": resolved.to_json(),
        "fixtures": fixtures.map(|f| serde_json::json!({
            "source": f.source().tag(),
            "path": f.dir().display().to_string(),
        })),
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
                 against. A corpus whose records state no digests produces a \
                 result that cannot be verified this way",
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
                    "the corpus at {} cannot be named: at least one record \
                     states no digest for its own image",
                    corpus.display()
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
                 That number was not measured on these images.",
                file.display(),
                corpus.display()
            ),
        );
        out.json = json;
        out
    }
}

/// The metrics over one set of scores and labels.
///
/// The refusal carries a stable `reason` word beside the message. A caller
/// driving this from another language has to tell "one class only", which is a
/// fact about its corpus, from "that file is not JSON", which is its own bug,
/// and matching on English prose to do it is how a caller stops noticing the
/// difference the first time the wording improves.
fn cmd_metrics(file: Option<&Path>, at: &[String]) -> Output {
    // `-` is the conventional spelling for standard input, and a file actually
    // called `-` is not worth the ambiguity.
    let file = file.filter(|p| p.as_os_str() != "-");
    match metrics::run(file, at) {
        Ok(report) => Output::ok(report.to_json(), report.human()),
        Err(e) => {
            let mut out = Output::err(e.exit_code(), e.to_string());
            out.json = serde_json::json!({
                "ok": false,
                "reason": e.reason(),
                "error": e.to_string(),
            });
            out
        }
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
                     are not valid UTF-8. This is a bug in stegobench; please \
                     report it with this message: {e}",
                    e.as_bytes().len()
                ),
            )
        }
    };
    if script.trim().is_empty() {
        return Output::err(
            exit::FAILURE,
            format!(
                "the {shell} completion generator produced an empty script, \
                 which would complete nothing. Refused rather than written"
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
fn cmd_plan(resolved: &Resolved, command: &[String]) -> Output {
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
    let Some(Command::Score {
        corpus,
        detector,
        corpus_id,
        limit,
        timeout,
        ..
    }) = &parsed.command
    else {
        return Output::err(
            exit::USAGE,
            "only `score` can be planned today.".to_string(),
        );
    };

    let reg = &resolved.registry;
    let entries = match resolve_detectors(reg, detector) {
        Ok(e) => e,
        Err(o) => return o,
    };

    // The same teaching refusal `score` gives, for the same reason: somebody
    // asking what a run would cost over their own photographs is on the wrong
    // side of what this measures, and "no record beside DSC_0001" does not
    // tell them so.
    if let Some(why) = unlabelled_corpus(corpus) {
        return Output::err(exit::PREFLIGHT_REFUSED, why);
    }

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
    let (configuration, why) = plan_configuration(reg, corpus_id.as_deref(), limit.is_some());

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
            "\n\nNo total: none of these declares a seconds_per_image. The \
             worst case below is the only bound there is.",
        );
    } else {
        human.push_str(&format!(
            "\n\nTotal          about {}, over the {estimated} that declare a \
             rate",
            human_duration(total_seconds),
        ));
        if unestimated > 0 {
            // A total that reads as the whole job when it covers five of seven
            // is worse than no total.
            human.push_str(&format!(
                "\n               {unestimated} declare none, so the real \
                 total is larger"
            ));
        }
    }
    human.push_str(&format!(
        "\nRecords        about {records_mb:.1} MB\nWorst case     {} (every \
         item hitting the {timeout}s deadline)\nConfiguration  {configuration}: \
         {why}",
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
            "no --corpus-id, so nothing to check this directory against. \
             Comparable with itself, not with anybody else's number"
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
            format!("{id} declares no records digest, so there is nothing to check against"),
        );
    }
    if limited {
        return (
            "custom",
            "--limit scores part of the corpus, and a prefix of a tier is not the tier".into(),
        );
    }
    (
        "named",
        format!(
            "{id} declares a records digest. `score` checks this directory \
             against it and refuses if it disagrees"
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
                "`--detector all` already means every registered detector. Use \
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
                "no detectors are registered, so `--detector all` has nothing \
                 to score with. `stegobench list detectors` says where it \
                 looked."
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
                    "{name} is an embedder: it hides payloads, it cannot tell \
                     two images apart. `stegobench list detectors` shows what \
                     can."
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
/// Image file extensions, lowercased, that a person is likely to have a folder
/// of. Not the set the scorer supports: this is a heuristic for recognising
/// "somebody's pictures", so it is deliberately wider.
const LOOKS_LIKE_AN_IMAGE: &[&str] = &[
    "png", "jpg", "jpeg", "tif", "tiff", "bmp", "webp", "gif", "pgm", "ppm", "heic",
];

/// How many directory entries the shape check is allowed to look at.
///
/// It runs before a command that may take hours, so it has to be cheap and
/// bounded rather than correct on a pathological tree. Enough to be sure about
/// a folder of holiday photographs, and it gives up quietly on anything
/// larger, which then meets the ordinary corpus errors as before.
const SHAPE_CHECK_ENTRIES: usize = 4096;

/// Why this path cannot be a corpus, in the words a user can act on.
///
/// `None` means the path is a directory this process can list, which is all
/// that is checked here: whether what is inside it is a corpus is the loader's
/// question and it answers it far better than a pre-flight could.
fn corpus_path_problem(corpus: &Path) -> Option<String> {
    let shown = corpus.display();
    match std::fs::metadata(corpus) {
        Ok(meta) if meta.is_dir() => match std::fs::read_dir(corpus) {
            Ok(_) => None,
            Err(e) => Some(format!(
                "{shown} cannot be listed: {}.\n\
                 Check the permissions on it, or point at a copy you can read.",
                plain(&e)
            )),
        },
        Ok(_) => Some(format!(
            "{shown} is a file, and a corpus is a directory.\n\
             Point at an unpacked corpus rather than at a shard or a file \
             beside one. `stegobench help scope` says what a corpus has to \
             hold."
        )),
        Err(e) => Some(format!(
            "there is no corpus at {shown}: {}.\n\
             Check the path. `stegobench list corpora` names the ones this \
             registry knows, and `stegobench help scope` says what a corpus \
             has to hold.",
            plain(&e)
        )),
    }
}

/// An IO error as a sentence, without the operating system's error number.
///
/// `os error 2` names nothing a reader can act on and reads as a crash rather
/// than an answer, which is the whole of why it never reaches a user here.
fn plain(e: &std::io::Error) -> String {
    match e.kind() {
        std::io::ErrorKind::NotFound => "nothing is there".to_string(),
        std::io::ErrorKind::PermissionDenied => "permission was refused".to_string(),
        _ => {
            let text = e.to_string();
            match text.split_once(" (os error") {
                Some((head, _)) => head.to_string(),
                None => text,
            }
        }
    }
}

/// Whether this looks like a folder of unlabelled images rather than a corpus.
///
/// WHY THE REFUSAL TEACHES RATHER THAN REPORTS
/// -------------------------------------------
/// Most people who reach for a steganalysis tool want to know whether THEIR
/// images are hiding something. That is the opposite direction from what this
/// measures, and the moment somebody points `score` at a folder of their own
/// files is the moment that distinction is worth the most: they have already
/// installed the thing and typed a real command, so a message that only says
/// "no records found" costs them another twenty minutes before they work out
/// they are in the wrong place.
///
/// Returns `None` whenever it cannot be sure, which includes an empty
/// directory, a directory it could not read, and a tree wider than the bound.
/// Those cases meet the ordinary corpus errors, which is the safe direction to
/// be wrong in: a corpus wrongly accused of being a photo album would be a
/// refusal nobody could work around.
fn unlabelled_corpus(corpus: &Path) -> Option<String> {
    let mut images = 0usize;
    let mut seen = 0usize;
    let mut stack = vec![(corpus.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        let read = std::fs::read_dir(&dir).ok()?;
        for item in read.flatten() {
            seen += 1;
            if seen > SHAPE_CHECK_ENTRIES {
                return None;
            }
            let path = item.path();
            let Ok(meta) = std::fs::metadata(&path) else {
                return None;
            };
            if meta.is_dir() {
                if depth < 2 {
                    stack.push((path, depth + 1));
                }
                continue;
            }
            match path
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .as_deref()
            {
                // One record anywhere is enough to stop guessing. Whether the
                // records are the RIGHT ones is the corpus loader's job, and
                // it says so far better than a heuristic could.
                Some("json" | "jsonl") => return None,
                Some(ext) if LOOKS_LIKE_AN_IMAGE.contains(&ext) => images += 1,
                _ => {}
            }
        }
    }
    if images == 0 {
        return None;
    }
    Some(format!(
        "{} holds {images} image(s) and no records saying which of them hides \
         anything, so there is nothing to be right or wrong about.\n\n\
         Stegobench measures DETECTORS against labelled images; it does not \
         examine your own.\n\
         `stegobench help scope`      the difference, and where to go instead\n\
         `stegobench list detectors`  what is registered here\n\
         `stegobench help pairing`    what a corpus has to carry first",
        corpus.display()
    ))
}

/// Everything `score` was asked for, past the registry it resolves against.
///
/// Bundled rather than passed one at a time because the list outgrew what a
/// reader can hold in order: `records` and `out` are both `Option<&Path>` and
/// sit next to each other, so a transposition at a call site would have written
/// the per-image scores where the result documents go and compiled cleanly.
struct ScoreRequest<'a> {
    corpus: &'a Path,
    detectors: &'a [String],
    corpus_id: Option<&'a str>,
    /// The corpus the detector was trained on, as the user declared it.
    trained_on: Option<&'a str>,
    records: Option<&'a Path>,
    out: Option<&'a Path>,
    timeout: u64,
    limit: Option<u64>,
}

fn cmd_score(resolved: &Resolved, req: ScoreRequest<'_>) -> Output {
    let ScoreRequest {
        corpus,
        detectors,
        corpus_id,
        trained_on,
        records,
        out,
        timeout,
        limit,
    } = req;
    let reg = &resolved.registry;
    let entries = match resolve_detectors(reg, detectors) {
        Ok(e) => e,
        Err(o) => return o,
    };
    let many = entries.len() > 1;

    // Asked before the shape check, because that check reads the directory and
    // answers "nothing to say" for a path it cannot open, so a corpus that is
    // not there fell through to the loader and arrived as
    // "cannot read /nope: No such file or directory (os error 2)" under exit 1.
    // Three faults: a raw operating system error in front of a user, FAILURE
    // where the contract has a refusal, and a message that does not say what to
    // do. A path that is not a readable directory is a refusal, and it is the
    // cheapest one there is.
    if let Some(why) = corpus_path_problem(corpus) {
        return Output::err(exit::PREFLIGHT_REFUSED, why);
    }

    // Asked BEFORE availability, because it is the more useful refusal and
    // because it does not depend on any detector. Somebody who points `score`
    // at their own folder of photographs is on the wrong side of the thing
    // this tool does, and telling them "docker pull ..." sends them to install
    // a container that will not answer their question either.
    if let Some(why) = unlabelled_corpus(corpus) {
        return Output::err(exit::PREFLIGHT_REFUSED, why);
    }

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
                        "{} is a file. Scoring {} detectors writes one {} \
                         each, so {flag} has to name a directory.",
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
        match availability::check(entry, resolved.adapter_roots()).presence {
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
    let prepared = match score::prepare(corpus, registered, limit, trained_on, say) {
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
            resolved.adapter_roots(),
            say,
        );
        outcomes.push((entry.name.clone(), outcome));
    }

    summarise(
        &outcomes,
        &out_dir,
        many,
        fetch::offer(&resolved.registry).as_ref(),
    )
}

/// Where the documents go, decided before any work happens.
///
/// `Ok(None)` means one detector writing to stdout or to the single file the
/// caller named. `Ok(Some(dir))` means a directory holding one document per
/// detector, created here so a failure to create it is a usage error rather
/// than something discovered after the first hour of scoring.
fn resolve_out(corpus: &Path, out: Option<&Path>, many: bool) -> Result<Option<PathBuf>, Output> {
    // `--out` USED TO MEAN DIFFERENT THINGS ON DIFFERENT DAYS
    //
    // It was read as a directory when several detectors were named and as a
    // file when one was, so the same path worked and then did not depending on
    // how many detectors the caller asked for. Measured 2026-09-29: one run
    // wrote /tmp/multi/zsteg.json and /tmp/multi/stegexpose.json happily, and
    // the next, differing only in naming a single detector, scored the whole
    // corpus and then died with
    //
    //     could not write the result to /tmp/multi: Is a directory (os error 21)
    //
    // Three faults in one line: a raw operating system error in front of a
    // user, a generic exit code where the contract has a specific one, and all
    // of it discovered AFTER the work rather than before it.
    //
    // A path that is already a directory is now treated as one whatever the
    // detector count, which is the reading that never surprises, and the check
    // happens here, before a single image is scored. The other half of the
    // question, several detectors aimed at an existing FILE, is refused by
    // `cmd_score` before any detector is even probed for, which is earlier
    // than this and therefore where it belongs.
    if let Some(dir) = out.filter(|p| p.is_dir()) {
        return Ok(Some(dir.to_path_buf()));
    }
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
    match (records, many, out_dir) {
        // One detector and an explicit path: exactly what the caller asked
        // for, because they named one run's file and there is one run.
        (Some(p), false, _) => p.to_path_buf(),
        // Several detectors: the caller named a directory to keep them in.
        (Some(p), true, _) => p.join(file),
        // Whatever the detector count, records go where the documents go, so
        // everything one run produced is in the place the caller named.
        (None, _, Some(dir)) => dir.join(file),
        (None, true, None) => corpus.with_file_name(file),
        (None, false, None) => {
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
    adapter_roots: &[PathBuf],
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
        adapter_roots,
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
    // A directory wins over the detector count: `resolve_out` has already
    // decided, once, before anything ran, so this is not the place that guesses.
    let destination = match (out_dir, out) {
        (Some(dir), _) => Some(dir.join(format!("{}.json", entry.name))),
        (None, Some(path)) => Some(path.to_path_buf()),
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
fn summarise(
    outcomes: &[(String, Outcome)],
    out_dir: &Option<PathBuf>,
    many: bool,
    offer: Option<&fetch::Offer>,
) -> Output {
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
                "{name:<16} AUC {:.4}  {} clean / {} stego / {} unanswered{}{}",
                result.metrics.auc,
                result.metrics.n_clean,
                result.metrics.n_stego,
                result.metrics.n_error,
                if tally.resumed > 0 {
                    format!("  {} resumed", tally.resumed)
                } else {
                    String::new()
                },
                match written {
                    Some(p) => format!("  {}", p.display()),
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
            "{} produced NO number ({skipped} skipped, {failed} failed). A \
             report over these documents covers only the {measured} that ran.",
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
                "could not list {} to check for older documents: {e}. A report \
                 over it may cover more than this run did.",
                dir.display()
            )),
        }
        if !stale.is_empty() {
            lines.push(format!(
                "WARNING: {} document(s) here are from an earlier run, not \
                 this one: {}. `stegobench report {}` will include them \
                 beside today's numbers. Move them aside if this run is meant \
                 to be the whole table.",
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

    // WHY THIS DOES NOT PRINT ON EVERY RUN.
    //
    // An offer a reader sees after every measurement is an advertisement, and
    // an advertisement is something people learn to skip. This fires only where
    // it is the useful next line: a run small enough that the number cannot be
    // quoted, which is what scoring the shipped starter corpus looks like. A
    // real tier is thousands of images and never reaches it.
    let largest = outcomes
        .iter()
        .filter_map(|(_, o)| match o {
            Outcome::Measured { result, .. } => {
                Some(result.metrics.n_clean + result.metrics.n_stego)
            }
            _ => None,
        })
        .max()
        .unwrap_or(0);
    if let Some(offer) = offer {
        if largest > 0 && largest < fetch::SMALL_RUN_IMAGES {
            lines.push(format!(
                "{largest} images is a demonstration, not a measurement. \
                 Next: {}",
                offer.command()
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

/// Where a fetch lands: the flag, then the environment, then the default.
///
/// Split out from `cmd_fetch` with the environment's answer handed in so the
/// precedence, and the treatment of an empty variable, are testable without
/// `set_var`. Setting a variable is process-global and this suite runs its
/// tests on several threads at once.
fn fetch_dest(named: Option<&Path>, from_env: Option<&str>) -> PathBuf {
    named
        .map(Path::to_path_buf)
        .or_else(|| from_env.filter(|v| !v.is_empty()).map(PathBuf::from))
        .unwrap_or_else(fetch::default_dest)
}

/// Download one tier of one corpus.
///
/// THE ORDER HERE IS THE POINT.
///
/// The corpus is resolved, then the refusal is decided, and only then is a
/// transport looked for. A corpus this tool may not fetch is refused
/// identically on a machine with `curl` and on a machine without one, and
/// neither answer is ever "install curl" to somebody whose real answer is that
/// the terms say no.
fn cmd_fetch(
    resolved: &Resolved,
    id: &str,
    tier: &str,
    dest: Option<&Path>,
    max_bytes: Option<u64>,
    budget_minutes: u64,
) -> Output {
    let Some(entry) = resolved.registry.corpora.get(id) else {
        return Output::err(
            exit::USAGE,
            format!(
                "no corpus with id {id:?} is registered. \
                 `stegobench list corpora` shows what is."
            ),
        );
    };
    if let Some(refused) = fetch::refusal(entry) {
        return Output::err(refused.exit_code(), refused.to_string());
    }

    // Asked before a directory is created, so a mistyped tier leaves nothing
    // behind. The message is the core's own rather than a second copy of it.
    if entry.route(tier).is_none() {
        let e = fetch::Error::Failed(stegobench_core::fetch::FetchError::UnknownTier {
            id: entry.id.clone(),
            tier: tier.to_string(),
            known: entry.download.iter().map(|r| r.tier.clone()).collect(),
        });
        return Output::err(e.exit_code(), e.to_string());
    }

    let limits = fetch::limits(max_bytes, budget_minutes);
    let dest = fetch_dest(
        dest,
        registry::non_empty("STEGOBENCH_CORPUS_DIR").as_deref(),
    );
    // A pre-flight rather than a discovery an hour in: an unwritable
    // destination is the same failure whether it is found now or after 48 GB.
    if let Err(e) = std::fs::create_dir_all(&dest) {
        return Output::err(
            exit::FAILURE,
            format!(
                "cannot use {} to keep the download: {e}. Name a writable \
                 directory with --dest.",
                dest.display()
            ),
        );
    }
    let transport = match fetch::Curl::find(limits) {
        Ok(t) => t,
        Err(e) => return Output::err(e.exit_code(), e.to_string()),
    };
    let store = stegobench_core::fetch::FileStore::new(&dest);

    let mut stderr = std::io::stderr();
    let interactive = stderr.is_terminal();
    let mut renderer = fetch::Renderer::new(&mut stderr, interactive);
    let outcome = fetch::run(entry, tier, &transport, &store, limits, &mut renderer);
    let progress_error = renderer.write_error().map(|e| e.to_string());
    drop(renderer);

    match outcome {
        Ok(fetched) => {
            let (json, human) = fetch::describe_success(entry, tier, &fetched, progress_error);
            Output::ok(json, human)
        }
        Err(e) => Output::err(e.exit_code(), e.to_string()),
    }
}

/// What a bare `stegobench` prints.
///
/// It used to print the whole help, which is three screens answering a
/// question nobody typing one word has asked yet. A person who types the bare
/// name wants to know what this is and what to type next, and both fit in a
/// few lines. Everything else is one `--help` away and nothing was removed.
fn cmd_orientation(offer: Option<fetch::Offer>) -> Output {
    let mut human = "stegobench measures how good a steganography detector is, by \
         running it over images whose answers are already known.\n\
         It does NOT examine your own images (`stegobench help scope`).\n\n  \
         stegobench list detectors    what this installation can run\n  \
         stegobench doctor            what is installed, and what it needs\n  \
         stegobench help              the reasoning, one topic at a time\n  \
         stegobench --help            every command and flag"
        .to_string();
    let mut next = vec![
        "stegobench list detectors".to_string(),
        "stegobench doctor".to_string(),
        "stegobench help scope".to_string(),
    ];
    // ONE line, and only the one that runs. A reader with nothing to score
    // needs somewhere to get images, and the offer is generated from the
    // registry entry so it can never name a route that entry has not declared.
    if let Some(offer) = &offer {
        human.push_str(&format!("\n\n  {}", offer.line()));
        next.push(offer.run.clone());
    }
    let mut out = Output::ok(
        serde_json::json!({
            "ok": false,
            "error": "no command given",
            "next": next,
        }),
        human,
    );
    // Nothing was done, so this is not a success. Exit 2 is what the contract
    // calls a usage error and what clap returns for the same case, and the
    // text still goes to stdout because it IS the answer to what was asked.
    out.code = exit::USAGE;
    out.payload_on_stdout = true;
    out
}

/// The refusal for `check` and `scan`, the two words somebody looking for an
/// image examiner guesses.
///
/// They are routed rather than left to clap's did-you-mean, which suggested
/// `schema` for `check` and nothing at all for `scan`. Both guesses are about
/// the same misunderstanding, so both get the same answer to it.
fn cmd_wrong_direction(word: &str) -> Output {
    Output::err(
        exit::USAGE,
        format!(
            "there is no `stegobench {word}`. Stegobench measures DETECTORS \
             against labelled images; it does not {word} your own images.\n\n\
             `stegobench help scope`      the difference, and where to go \
             instead\n\
             `stegobench list detectors`  what is registered here\n\
             `stegobench score --corpus <labelled corpus> --detector <name>`"
        ),
    )
}

fn run(cli: &Cli) -> Output {
    let Some(command) = &cli.command else {
        // Fail-open: a registry that will not load is a problem for the
        // commands that need one, and refusing to print three lines of
        // orientation over it would answer the wrong question.
        let offer = registry::resolve(cli.registry.as_deref())
            .ok()
            .and_then(|r| fetch::offer(&r.registry));
        return cmd_orientation(offer);
    };
    match command {
        Command::Schema { name } => cmd_schema(name),
        Command::Validate { file } => cmd_validate(file),
        Command::Verify { file, corpus } => cmd_verify(file, corpus),
        Command::List { kind } => with_registry(cli, |r| cmd_list(r, kind)),
        Command::Describe { name } => with_registry(cli, |r| cmd_describe(r, name)),
        Command::Plan { command } => with_registry(cli, |r| cmd_plan(r, command)),
        Command::Doctor {
            fixtures,
            no_selftest,
        } => with_registry(cli, |r| {
            if *no_selftest {
                // Nothing will read the fixtures, so nothing looks for them.
                // Unpacking the built-in copy here would be work done to
                // satisfy a parameter rather than a question.
                return cmd_doctor(r, None);
            }
            match fixtures::resolve(fixtures.as_deref()) {
                // Held for the whole call: for the built-in copy this owns the
                // scratch directory the images were unpacked into.
                Ok(found) => cmd_doctor(r, Some(&found)),
                Err(e) => Output::err(exit::PREFLIGHT_REFUSED, e.to_string()),
            }
        }),
        Command::Score {
            corpus,
            detector,
            corpus_id,
            trained_on,
            records,
            out,
            timeout,
            limit,
        } => with_registry(cli, |r| {
            cmd_score(
                r,
                ScoreRequest {
                    corpus,
                    detectors: detector.as_slice(),
                    corpus_id: corpus_id.as_deref(),
                    trained_on: trained_on.as_deref(),
                    records: records.as_deref(),
                    out: out.as_deref(),
                    timeout: *timeout,
                    limit: *limit,
                },
            )
        }),
        Command::Fetch {
            corpus,
            tier,
            dest,
            max_bytes,
            budget_minutes,
        } => with_registry(cli, |r| {
            cmd_fetch(
                r,
                corpus,
                tier,
                dest.as_deref(),
                *max_bytes,
                *budget_minutes,
            )
        }),
        Command::Metrics { file, at } => cmd_metrics(file.as_deref(), at),
        Command::Report { paths, format, out } => cmd_report(paths, *format, out.as_deref()),
        Command::Completions { shell } => cmd_completions(*shell),
        Command::Help { topic } => cmd_help(topic.as_deref()),
        Command::Check { .. } => cmd_wrong_direction("check"),
        Command::Scan { .. } => cmd_wrong_direction("scan"),
    }
}

/// Parses, and on an unknown subcommand names the closest real one.
///
/// clap's own did-you-mean only fires when its threshold is met, so `scan`
/// produced a usage error naming nothing at all. This always names the nearest
/// command and always names where the full list is, and falls back to clap's
/// own message untouched when the offending word cannot be recovered from the
/// error, because a worse message is better than no message.
fn parse_or_explain() -> Cli {
    match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) if e.kind() == clap::error::ErrorKind::InvalidSubcommand => {
            let typed = e
                .get(clap::error::ContextKind::InvalidSubcommand)
                .map(|v| v.to_string());
            let Some(typed) = typed else {
                e.exit();
            };
            let nearest = nearest_command(&typed);
            eprintln!("there is no `stegobench {typed}`.");
            if let Some(nearest) = nearest {
                eprintln!("The closest command is `{nearest}`.");
            }
            eprintln!(
                "`stegobench` on its own says what the tool is for, and \
                 `stegobench --help` lists every command."
            );
            std::process::exit(exit::USAGE);
        }
        Err(e) => e.exit(),
    }
}

/// The visible command whose name is closest to what was typed.
///
/// Hidden commands are skipped deliberately: `check` and `scan` are signposts
/// rather than features, and suggesting one as the nearest match to some third
/// word would send a reader to a refusal.
fn nearest_command(typed: &str) -> Option<String> {
    let mut cmd = Cli::command();
    cmd.build();
    let typed = typed.to_lowercase();
    cmd.get_subcommands()
        .filter(|s| !s.is_hide_set())
        .map(|s| s.get_name().to_string())
        .map(|name| {
            let d = edit_distance(&typed, &name);
            (d, name)
        })
        // Half the word's length, so a genuinely unrelated word gets no
        // suggestion rather than a confident wrong one. Two edits at most, AND
        // no more than half the word: `chek` is three edits from `schema`,
        // which clap was happy to offer and which is a worse answer than
        // silence, because it sends somebody to a command about JSON schemas.
        .filter(|(d, name)| *d <= 2 && *d * 2 <= name.len().max(typed.len()))
        .min_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)))
        .map(|(_, name)| name)
}

/// Levenshtein distance, two rows at a time.
///
/// Bounded by the product of two command names, both of which are at most a
/// dozen characters, so there is nothing here to cap.
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            cur[j + 1] = (prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

fn main() {
    let cli = parse_or_explain();
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
        if matches!(cli.command, Some(Command::Schema { .. })) {
            let mut stdout = std::io::stdout().lock();
            let _ = serde_json::to_writer_pretty(&mut stdout, &out.json);
            let _ = writeln!(stdout);
        } else if matches!(
            cli.command,
            Some(Command::Completions { .. } | Command::Help { .. })
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

    /// Measured 2026-09-29, before the pre-flight existed: `score --corpus
    /// /nope/not-here` walked past the shape check, reached the loader, and
    /// printed "cannot read /nope/not-here: No such file or directory (os
    /// error 2)" with exit 1. A mistyped path is the commonest mistake there
    /// is and it deserves a sentence, not an errno under the code that means
    /// "something went wrong, try again".
    #[test]
    fn a_corpus_path_that_is_not_a_readable_directory_is_refused_in_plain_words() {
        let missing = corpus_path_problem(Path::new("/nope/not-here-either"))
            .expect("a path that is not there is a problem");
        assert!(
            missing.contains("no corpus at"),
            "says what is wrong: {missing}"
        );
        assert!(
            !missing.contains("os error"),
            "an operating system error number reached the user: {missing}"
        );

        let tmp = tempfile::tempdir().expect("tmp");
        let file = tmp.path().join("one.png");
        std::fs::write(&file, b"not really a png").expect("written");
        let is_file = corpus_path_problem(&file).expect("a file is a problem");
        assert!(
            is_file.contains("is a file"),
            "says what is wrong: {is_file}"
        );

        assert!(
            corpus_path_problem(tmp.path()).is_none(),
            "a readable directory is not a problem, and if it were this test \
             would be passing for the wrong reason"
        );
    }

    /// An environment variable bound with clap's `env` is read before any of
    /// our code runs, and clap treats a variable that is set but empty as a
    /// flag supplied without its value. `STEGOBENCH_REGISTRY=` in a shell
    /// profile therefore made EVERY command refuse with "a value is required
    /// for '--registry <DIR>'", including `--help`. Every variable is now
    /// read by the resolver that wants it, through `non_empty`, which treats
    /// empty as unset. This walks the real tree so a later argument cannot
    /// reintroduce the binding quietly.
    /// The third of the three variables, and the only one whose resolver is
    /// here rather than in a module of its own. Asserted through the seam,
    /// because setting a variable is process-global and this suite runs its
    /// tests on several threads at once.
    #[test]
    fn the_corpus_directory_comes_from_the_flag_then_the_environment_then_the_default() {
        let named = Path::new("/tmp/named-by-the-flag");
        assert_eq!(
            fetch_dest(Some(named), Some("/tmp/named-by-the-variable")),
            named,
            "the environment overrode an explicit --dest"
        );
        assert_eq!(
            fetch_dest(None, Some("/tmp/named-by-the-variable")),
            Path::new("/tmp/named-by-the-variable"),
            "the variable was not read"
        );
        // An empty value is not an answer, so the default answers instead.
        assert_eq!(fetch_dest(None, Some("")), fetch::default_dest());
        assert_eq!(fetch_dest(None, None), fetch::default_dest());
    }

    #[test]
    fn no_argument_reads_its_environment_variable_through_clap() {
        fn walk(cmd: &clap::Command, found: &mut Vec<String>) {
            for arg in cmd.get_arguments() {
                if let Some(var) = arg.get_env() {
                    found.push(format!(
                        "{} --{} reads {}",
                        cmd.get_name(),
                        arg.get_id(),
                        var.to_string_lossy()
                    ));
                }
            }
            for sub in cmd.get_subcommands() {
                walk(sub, found);
            }
        }
        let root = Cli::command();
        let mut found = Vec::new();
        walk(&root, &mut found);
        assert!(
            found.is_empty(),
            "an empty value in one of these variables would refuse every \
             command: {}",
            found.join(", ")
        );
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

    /// A registry loaded from exactly the directory a test names.
    ///
    /// Deliberately the explicit route rather than `registry::resolve`, so a
    /// test never picks up whichever registry happens to be installed on the
    /// machine running it.
    fn resolved_at(dir: impl AsRef<Path>) -> Resolved {
        Resolved::from_dir(dir.as_ref()).expect("the registry under test loads")
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
    /// in earlier work and every document under `results/` was left
    /// behind, so a reader following the README's own instruction to
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
        let listed = cmd_list(&resolved_at(&dir), "corpora");
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

        let described = cmd_describe(&resolved_at(&dir), "reveal");
        assert_eq!(described.code, exit::OK);
        assert_eq!(described.json["licence"]["redistribution"], "permitted");
        assert!(described.human.contains("verified_on"));

        // And a tool still answers the same verb.
        assert_eq!(cmd_describe(&resolved_at(&dir), "steghide").code, exit::OK);
        // An unknown name is a usage error that names both kinds.
        let missing = cmd_describe(&resolved_at(&dir), "not-registered");
        assert_eq!(missing.code, exit::USAGE);
        assert!(missing.human.contains("reveal") && missing.human.contains("steghide"));
    }

    /// Every listed row's columns start in the same place, including the
    /// longest name's.
    ///
    /// This is the regression the fixed `{:<16}` pad could not survive:
    /// `stegobench-starter` is eighteen characters, so its row overflowed the
    /// column and every cell after it on that line sat two places right of
    /// everybody else's. A test that only looked for the text somewhere in the
    /// output passed throughout.
    #[test]
    fn a_listing_lines_its_columns_up_however_long_the_longest_name_is() {
        let dir = shipped_registry();
        for kind in ["corpora", "detectors", "embedders"] {
            let out = cmd_list(&resolved_at(&dir), kind);
            assert_eq!(out.code, exit::OK, "{kind}");
            // The rows, which are the lines before the first blank one: the
            // legend and the footnotes below it are prose, not table.
            let rows: Vec<&str> = out
                .human
                .lines()
                .take_while(|l| !l.trim().is_empty())
                .collect();
            assert!(rows.len() > 1, "{kind}: nothing to align");
            // Byte offsets throughout, which is the same as character offsets
            // here because every registered name is ASCII. `table::align` is
            // the place that has to care about the difference.
            let longest = rows
                .iter()
                .map(|r| r.split_whitespace().next().unwrap_or("").len())
                .max()
                .expect("rows");
            for row in &rows {
                let name = row.split_whitespace().next().expect("a name");
                // Where the second column begins: past the name, past its
                // padding, at the first character that is not a space.
                let second = row[name.len()..]
                    .find(|c: char| c != ' ')
                    .map(|i| i + name.len());
                assert_eq!(
                    second,
                    Some(longest + 1),
                    "{kind}: {row:?} does not start its second column where the others do"
                );
            }
            assert!(
                rows.iter().any(|r| r.len() > longest + 1),
                "{kind}: every row is a bare name, so this proved nothing"
            );
        }
    }

    /// `list all` is the whole registry, so leaving corpora out of it would
    /// make the two halves reachable only by knowing which is which.
    #[test]
    fn list_all_carries_the_corpora_as_well_as_the_tools() {
        let out = cmd_list(&resolved_at(shipped_registry()), "all");
        assert_eq!(out.code, exit::OK);
        assert!(out.json["tools"].as_array().is_some_and(|a| !a.is_empty()));
        assert!(out.json["corpora"]
            .as_array()
            .is_some_and(|a| !a.is_empty()));
        assert!(out.human.contains("republishable"), "got: {}", out.human);
    }

    /// The tools half had the fault the corpora half was written to avoid: it
    /// printed `nothing registered under "detectors"` and exited zero, which
    /// says neither where it looked nor whether anything is registered at all.
    #[test]
    fn an_empty_tools_listing_says_where_it_looked() {
        let dir = tempfile::tempdir().unwrap();
        let out = cmd_list(&resolved_at(dir.path()), "detectors");
        assert_eq!(out.code, exit::OK);
        assert!(
            out.human.contains("no tools registered")
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

        let out = cmd_list(&resolved_at(dir.path()), "embedders");
        assert_eq!(out.code, exit::OK);
        assert!(out.json["tools"].as_array().is_some_and(|a| a.is_empty()));
        assert!(
            out.human.contains("none of the 1 tool(s)") && out.human.contains("is a embedder"),
            "got: {}",
            out.human
        );
        assert!(
            !out.human.contains("no tools registered"),
            "a registry with a tool in it was reported as empty: {}",
            out.human
        );
    }

    /// An empty listing under exit zero reads as "checked, all fine". It has
    /// to say that nothing is declared, which is a different fact.
    #[test]
    fn an_empty_corpora_listing_says_so_rather_than_printing_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let out = cmd_list(&resolved_at(dir.path()), "corpora");
        assert_eq!(out.code, exit::OK);
        assert_eq!(out.json["count"], 0);
        assert!(
            out.human.contains("no corpora registered"),
            "an empty registry printed: {:?}",
            out.human
        );
    }

    /// The defect the analysis called the worst one: the first command a
    /// person types must work from anywhere. Pinned at the parser, because
    /// the fix was removing a default value and a default value is one line to
    /// put back.
    #[test]
    fn the_registry_flag_carries_no_relative_default_value() {
        let mut cmd = Cli::command();
        cmd.build();
        let arg = cmd
            .get_arguments()
            .find(|a| a.get_long() == Some("registry"))
            .expect("--registry exists");
        assert!(
            arg.get_default_values().is_empty(),
            "--registry has a default value again, so an installed binary run \
             outside a checkout resolves it against the current directory and \
             every command that reads a registry fails"
        );
    }

    /// A bare invocation used to print three screens. Somebody who types one
    /// word is asking what this is, and that answer is short.
    #[test]
    fn a_bare_invocation_points_somewhere_rather_than_printing_everything() {
        let out = cmd_orientation(None);
        assert_eq!(out.code, exit::USAGE);
        assert!(
            out.payload_on_stdout,
            "the orientation text is the answer to what was asked, so it \
             belongs on stdout"
        );
        assert!(
            out.human.contains("stegobench list detectors"),
            "no obvious next command: {}",
            out.human
        );
        assert!(
            out.human.contains("help scope"),
            "the bare invocation does not say what the tool is not for: {}",
            out.human
        );
        assert!(
            out.human.lines().count() <= 16,
            "the orientation is {} lines, which is the whole help again",
            out.human.lines().count()
        );
    }

    /// `check` and `scan` are the two words rungs 2 and 3 guess, and clap
    /// suggested `schema` for one and nothing for the other. Both are the same
    /// misunderstanding and both get the same answer.
    #[test]
    fn check_and_scan_explain_the_direction_rather_than_reporting_a_typo() {
        for word in ["check", "scan"] {
            let parsed = Cli::try_parse_from(["stegobench", word, "./images"])
                .unwrap_or_else(|e| panic!("`{word}` is routed rather than rejected: {e}"));
            let out = run(&parsed);
            assert_eq!(out.code, exit::USAGE, "`{word}` should be a usage error");
            assert!(
                out.human.contains("help scope"),
                "`{word}` does not point at the scope topic: {}",
                out.human
            );
            assert!(
                out.human.contains("measures DETECTORS"),
                "`{word}` does not say what the tool measures: {}",
                out.human
            );
        }
    }

    /// Neither of the two signposts may be offered as the nearest match to
    /// some third word: that would send a reader to a refusal.
    #[test]
    fn the_nearest_command_is_named_for_a_near_miss_and_withheld_for_a_stranger() {
        assert_eq!(nearest_command("lst").as_deref(), Some("list"));
        assert_eq!(nearest_command("doctr").as_deref(), Some("doctor"));
        assert_eq!(nearest_command("REPORT").as_deref(), Some("report"));
        assert_eq!(nearest_command("frobnicate"), None);
        for word in ["chek", "scn", "sca"] {
            let got = nearest_command(word);
            assert!(
                !matches!(got.as_deref(), Some("check" | "scan")),
                "{word} was pointed at a hidden signpost: {got:?}"
            );
        }
        assert_eq!(
            nearest_command("chek"),
            None,
            "`chek` was pointed at some third command, which is what clap did \
             when it answered it with `schema`"
        );
    }

    #[test]
    fn the_edit_distance_is_the_ordinary_one() {
        assert_eq!(edit_distance("", ""), 0);
        assert_eq!(edit_distance("", "list"), 4);
        assert_eq!(edit_distance("list", ""), 4);
        assert_eq!(edit_distance("list", "list"), 0);
        assert_eq!(edit_distance("lst", "list"), 1);
        assert_eq!(edit_distance("kitten", "sitting"), 3);
    }

    /// The reorder the ladder asked for, pinned so it cannot quietly revert:
    /// the commands come before the machine contract, and the machine contract
    /// is still there.
    #[test]
    fn the_long_help_names_the_commands_before_the_exit_codes_and_keeps_both() {
        let mut cmd = Cli::command();
        let mut buf: Vec<u8> = Vec::new();
        cmd.write_long_help(&mut buf).expect("the help renders");
        let help = String::from_utf8(buf).expect("help is text");
        let commands = help.find("Commands:").expect("the command list is there");
        let codes = help
            .find("EXIT CODES")
            .expect("the exit codes are still there");
        assert!(
            commands < codes,
            "the exit codes come before the list of commands again"
        );
        for code in CONTRACT_EXIT_CODES {
            assert!(
                help.contains(&format!("{code}")),
                "exit code {code} is no longer documented anywhere in the help"
            );
        }
        assert!(
            help.contains("stdout") && help.contains("stderr"),
            "the stream contract was dropped rather than moved"
        );
        assert!(
            help.to_lowercase()
                .contains("does not examine your own images"),
            "the long help does not separate measuring a detector from \
             examining your own images"
        );
    }

    #[test]
    fn scope_is_a_help_topic_and_says_what_this_is_not_for() {
        let out = cmd_help(Some("scope"));
        assert_eq!(out.code, exit::OK);
        assert!(out.human.contains("IS SOMETHING HIDDEN IN THESE PICTURES"));
        assert!(out.human.contains("stegobench list detectors"));
        let listed = cmd_help(None);
        assert!(
            listed.human.contains("scope"),
            "the topic list does not offer scope: {}",
            listed.human
        );
    }

    /// The heuristic behind the teaching refusal, including both directions it
    /// must not fire in.
    #[test]
    fn a_folder_of_photographs_is_recognised_and_a_corpus_is_not() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            unlabelled_corpus(dir.path()),
            None,
            "an empty directory is not a photo album"
        );
        std::fs::write(dir.path().join("DSC_0001.JPG"), b"x").unwrap();
        std::fs::write(dir.path().join("DSC_0002.png"), b"x").unwrap();
        let why = unlabelled_corpus(dir.path()).expect("two images and no records");
        assert!(why.contains("help scope"), "got: {why}");
        assert!(why.contains("2 image(s)"), "got: {why}");

        std::fs::write(dir.path().join("DSC_0001.json"), b"{}").unwrap();
        assert_eq!(
            unlabelled_corpus(dir.path()),
            None,
            "one record is enough to stop guessing and let the corpus loader speak"
        );

        assert_eq!(
            unlabelled_corpus(&dir.path().join("not-there")),
            None,
            "a missing directory is the corpus loader's error to report, not this one's"
        );
    }

    /// The refusal has to reach the person BEFORE the availability check,
    /// because "docker pull ..." sends them to install something that will not
    /// answer their question either.
    #[test]
    fn score_refuses_a_folder_of_unlabelled_images_before_it_talks_about_detectors() {
        let dir = tempfile::tempdir().unwrap();
        let corpus = dir.path().join("holiday-photos");
        std::fs::create_dir_all(&corpus).unwrap();
        std::fs::write(corpus.join("a.png"), b"x").unwrap();
        let out = cmd_score(
            &resolved_at(shipped_registry()),
            ScoreRequest {
                corpus: &corpus,
                detectors: &["all".to_string()],
                corpus_id: None,
                trained_on: None,
                records: None,
                out: None,
                timeout: 5,
                limit: None,
            },
        );
        assert_eq!(out.code, exit::PREFLIGHT_REFUSED);
        assert!(out.human.contains("help scope"), "got: {}", out.human);
        assert!(
            !out.human.contains("docker pull"),
            "the refusal sent somebody to install a container: {}",
            out.human
        );
    }

    /// `plan` is the other command somebody on the wrong side of this points
    /// at their own pictures, and it used to answer "DSC_0001 has an image but
    /// no record beside it", which does not tell them what is wrong.
    #[test]
    fn plan_refuses_a_folder_of_unlabelled_images_the_same_way_score_does() {
        let dir = tempfile::tempdir().unwrap();
        let corpus = dir.path().join("holiday-photos");
        std::fs::create_dir_all(&corpus).unwrap();
        std::fs::write(corpus.join("a.png"), b"x").unwrap();
        let out = cmd_plan(
            &resolved_at(shipped_registry()),
            &[
                "score".to_string(),
                "--corpus".to_string(),
                corpus.display().to_string(),
                "--detector".to_string(),
                "zsteg".to_string(),
            ],
        );
        assert_eq!(out.code, exit::PREFLIGHT_REFUSED);
        assert!(out.human.contains("help scope"), "got: {}", out.human);
    }

    /// A machine can hold a checkout, an installed copy and the built-in one.
    /// Which answered is part of the answer.
    #[test]
    fn list_and_doctor_both_name_the_registry_that_answered() {
        let resolved = resolved_at(shipped_registry());
        let listed = cmd_list(&resolved, "detectors");
        assert!(
            listed.human.contains("registry  "),
            "list does not name the registry: {}",
            listed.human
        );
        assert_eq!(listed.json["registry"]["source"], "explicit");
        assert_eq!(
            listed.json["registry"]["path"],
            shipped_registry().display().to_string()
        );

        let doctor = cmd_doctor(&resolved, None);
        assert!(
            doctor.human.starts_with("registry  "),
            "doctor does not open by naming the registry: {}",
            doctor.human.lines().next().unwrap_or_default()
        );
        assert_eq!(doctor.json["registry"]["source"], "explicit");
        assert!(doctor.json["registry"]["tools"].as_u64().unwrap() > 0);
    }

    #[test]
    fn an_unknown_list_kind_names_corpora_among_the_known_ones() {
        let out = cmd_list(&resolved_at(shipped_registry()), "corpuses");
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
                "plugins": [{"name": "x", "image": "sha256:a", "determinism": "nondeterministic", "pinned_by": "executable-hash", "isolation": "host"}],
                "harness_version": "0.1.0",
                "started_utc": "2026-09-25T00:00:00Z",
                "elapsed_seconds": 1.0,
                "network_reachable": true
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
                .join("../../results/v1/round3-q95-structural-0000-aletheia-rs.json"),
            dir.path().join("good.json"),
        )
        .unwrap();
        std::fs::write(dir.path().join("bad.json"), "{nope").unwrap();

        let out = cmd_report(&[dir.path().to_path_buf()], ReportFormat::Text, None);
        assert_eq!(out.code, exit::SCHEMA_INVALID);
        assert!(out.payload_on_stdout, "the table was diverted to stderr");
        assert!(out.human.contains("THIS REPORT IS INCOMPLETE"));
        assert!(out.human.contains("bad.json"));
        assert!(out.human.contains("round3-q95"));
        assert_eq!(out.json["complete"], serde_json::json!(false));
    }

    /// With --out the payload is the file, so the human text is a
    /// confirmation and belongs on stderr with the rest of the progress.
    #[test]
    fn writing_a_report_to_a_file_confirms_on_stderr_and_says_if_it_is_short() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../results/v1/round3-q95-structural-0000-aletheia-rs.json"),
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
        assert!(written.contains("round3-q95"));

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
                .join("../../results/v1/round3-q95-structural-0000-aletheia-rs.json"),
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
    #[cfg(unix)]
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

    #[cfg(unix)]
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
    fn one_detector_writing_into_a_directory_puts_its_document_inside_it() {
        // `--out` used to mean a directory when several detectors were named
        // and a file when one was, so the same path worked and then did not
        // depending on how many detectors were asked for. Measured 2026-09-29:
        // a run wrote /tmp/multi/zsteg.json and /tmp/multi/stegexpose.json,
        // and the next run, differing only in naming one detector, scored the
        // whole corpus and then died with "Is a directory (os error 21)".
        let tmp = tempfile::tempdir().unwrap();
        let reg = registry_with_one_present_and_one_missing(tmp.path());
        let corpus = tmp.path().join("corpus");
        scratch_corpus(&corpus);
        let out_dir = tmp.path().join("results");
        std::fs::create_dir_all(&out_dir).unwrap();

        let out = cmd_score(
            &resolved_at(&reg),
            ScoreRequest {
                corpus: &corpus,
                detectors: &["sizer".to_string()],
                corpus_id: None,
                trained_on: None,
                records: None,
                out: Some(&out_dir),
                timeout: 5,
                limit: None,
            },
        );

        assert_eq!(out.code, exit::OK, "{}", out.human);
        let written = out_dir.join("sizer.json");
        assert!(
            written.exists(),
            "one detector did not write into the directory it was given: {}",
            out.human
        );
        assert_eq!(cmd_validate(&written).code, exit::OK);
        // The per-item answers keep it company rather than landing beside the
        // corpus, so one run's output is in one place.
        assert!(
            out_dir.join("sizer.records.jsonl").exists(),
            "the records did not follow the document into the directory"
        );
    }

    #[cfg(unix)]
    #[test]
    fn several_detectors_aimed_at_a_file_are_refused_before_anything_is_scored() {
        let tmp = tempfile::tempdir().unwrap();
        let reg = registry_with_one_present_and_one_missing(tmp.path());
        let corpus = tmp.path().join("corpus");
        scratch_corpus(&corpus);
        let target = tmp.path().join("one-file.json");
        std::fs::write(&target, "{}").unwrap();

        let out = cmd_score(
            &resolved_at(&reg),
            ScoreRequest {
                corpus: &corpus,
                detectors: &["ghost".to_string(), "sizer".to_string()],
                corpus_id: None,
                trained_on: None,
                records: None,
                out: Some(&target),
                timeout: 5,
                limit: None,
            },
        );

        assert_eq!(out.code, exit::USAGE, "{}", out.human);
        // The exact refusal, so this cannot pass on some other message that
        // happens to carry the word "directory".
        let expected = "is a file. Scoring 2 detectors writes one result \
                        document each, so --out has to name a directory.";
        assert!(out.human.contains(expected), "{}", out.human);
        // Refused BEFORE the work: the file it was pointed at is untouched.
        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            "{}",
            "the run wrote over the file it should have refused"
        );
    }

    #[cfg(unix)]
    #[test]
    fn one_missing_detector_does_not_lose_the_others_and_never_exits_zero() {
        let tmp = tempfile::tempdir().unwrap();
        let reg = registry_with_one_present_and_one_missing(tmp.path());
        let corpus = tmp.path().join("corpus");
        scratch_corpus(&corpus);
        let out_dir = tmp.path().join("results");

        let out = cmd_score(
            &resolved_at(&reg),
            ScoreRequest {
                corpus: &corpus,
                detectors: &["ghost".to_string(), "sizer".to_string()],
                corpus_id: None,
                trained_on: None,
                records: None,
                out: Some(&out_dir),
                timeout: 5,
                limit: None,
            },
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
            &resolved_at(&reg),
            ScoreRequest {
                corpus: &corpus,
                detectors: &["ghost".to_string(), "sizer".to_string()],
                corpus_id: None,
                trained_on: None,
                records: None,
                out: Some(&out_dir),
                timeout: 5,
                limit: None,
            },
        );
        assert!(
            out.human.contains("are from an earlier run"),
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
            &resolved_at(&reg),
            ScoreRequest {
                corpus: &corpus,
                detectors: &["sizer".to_string(), "ghost".to_string()],
                corpus_id: None,
                trained_on: None,
                records: None,
                out: Some(&out_dir),
                timeout: 5,
                limit: None,
            },
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
            &resolved_at(&reg),
            ScoreRequest {
                corpus: &corpus,
                detectors: &["sizer".to_string(), "sizer2".to_string()],
                corpus_id: None,
                trained_on: None,
                records: None,
                out: Some(&out_dir),
                timeout: 5,
                limit: None,
            },
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
            &resolved_at(&reg),
            ScoreRequest {
                corpus: &corpus,
                detectors: &["sizer".to_string(), "sizer2".to_string()],
                corpus_id: None,
                trained_on: None,
                records: None,
                out: Some(&out_dir),
                timeout: 5,
                limit: None,
            },
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

        let first = cmd_score(
            &resolved_at(&reg),
            ScoreRequest {
                corpus: &corpus,
                detectors: &args,
                corpus_id: None,
                trained_on: None,
                records: None,
                out: Some(&out_dir),
                timeout: 5,
                limit: None,
            },
        );
        assert_eq!(first.code, exit::OK, "{}", first.human);

        let second = cmd_score(
            &resolved_at(&reg),
            ScoreRequest {
                corpus: &corpus,
                detectors: &args,
                corpus_id: None,
                trained_on: None,
                records: None,
                out: Some(&out_dir),
                timeout: 5,
                limit: None,
            },
        );
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
            &resolved_at(&reg),
            ScoreRequest {
                corpus: &corpus,
                detectors: &["sizer".to_string(), "ghost".to_string()],
                corpus_id: None,
                trained_on: None,
                records: None,
                out: None,
                timeout: 5,
                limit: None,
            },
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
            &resolved_at(&reg),
            ScoreRequest {
                corpus: &corpus,
                detectors: &["sizer".to_string()],
                corpus_id: None,
                trained_on: None,
                records: None,
                out: None,
                timeout: 5,
                limit: None,
            },
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
            &resolved_at(&reg),
            ScoreRequest {
                corpus: &corpus,
                detectors: &["all".to_string()],
                corpus_id: None,
                trained_on: None,
                records: None,
                out: None,
                timeout: 5,
                limit: None,
            },
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
        assert!(err.human.contains("it cannot tell two images apart"));
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
        let doctor = cmd_doctor(&resolved_at(&dir), None);
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
        let described = cmd_describe(&resolved_at(&dir), "stegashield");
        assert_eq!(described.code, exit::OK);
        assert!(described.json["needs"]["readiness"].is_string());
        assert!(
            described.human.contains("Needs from you")
                || described.human.contains("Nothing needed"),
            "{}",
            described.human
        );

        // A corpus answers in the same shape, which is the whole point: a
        // reader should not have to know which kind of thing they typed.
        let corpus = cmd_describe(&resolved_at(&dir), "reveal");
        assert_eq!(corpus.code, exit::OK);
        assert!(corpus.json["needs"]["steps"].is_array());
        assert!(corpus.human.contains("Needs from you"), "{}", corpus.human);
    }

    /// The legend under `list` called every image entry a sandbox with no
    /// network, which is false for a service and false in the direction that
    /// matters most.
    #[test]
    fn the_list_legend_does_not_call_a_service_a_sandbox() {
        let out = cmd_list(&resolved_at(shipped_registry()), "detectors");
        assert_eq!(out.code, exit::OK);
        assert!(
            out.human.contains("SERVICE, not sandboxed"),
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

        // 3: pre-flight refusal, which `score` reaches two ways. A corpus
        // path that is not a readable directory is answered first, because a
        // stat costs nothing and a detector probe can pull a container. Then,
        // for a corpus that is there, whether the detector is on this machine,
        // refused rather than discovered after the walk.
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
            let ask = |corpus: &Path| {
                cmd_score(
                    &resolved_at(dir.path()),
                    ScoreRequest {
                        corpus,
                        detectors: &["ghost".to_string()],
                        corpus_id: None,
                        trained_on: None,
                        records: None,
                        out: None,
                        timeout: 5,
                        limit: None,
                    },
                )
            };

            let missing = ask(&dir.path().join("no-such-corpus"));
            assert_eq!(missing.code, exit::PREFLIGHT_REFUSED, "{}", missing.human);
            assert!(
                missing.human.contains("no corpus at"),
                "a path that is not there should be answered as itself: {}",
                missing.human
            );

            // The same request over a corpus that IS there, so the tool check
            // is what answers and this arm cannot pass on the path alone.
            let empty = dir.path().join("empty-corpus");
            std::fs::create_dir_all(&empty).unwrap();
            let absent = ask(&empty);
            assert_eq!(absent.code, exit::PREFLIGHT_REFUSED, "{}", absent.human);
            assert!(
                absent.human.contains("not on this machine"),
                "{}",
                absent.human
            );
        }

        // 4: plugin failure. Reachable now that `score` runs: a detector that
        // broke, or a run whose answers could not be turned into an AUC, is
        // reported through this code. It is driven in the score module's own
        // tests, which can build a corpus and a registry entry without this
        // test constructing both.

        // 5: verify mismatch. Reachable: `verify` recomputes a corpus digest
        // and compares it with the one a result claims. Driven by the
        // `verify_refuses_*` tests above, against a corpus the document was
        // not measured on and against a document naming no digest at all.

        // 6: schema invalid.
        {
            let dir = tempfile::tempdir().unwrap();
            let p = dir.path().join("x.json");
            std::fs::write(&p, r#"{"hello":"world"}"#).unwrap();
            assert_eq!(cmd_validate(&p).code, exit::SCHEMA_INVALID);
        }

        // 7: licence refusal. Reachable: `fetch` refuses a corpus that may
        // not be redistributed before it looks for a transport at all. Driven
        // in the fetch module's own tests, which hand it a refusing transport
        // and prove the licence answered first.

        // 8: environment unfit. NO LONGER REACHABLE FROM A STUB: every
        // command in the tree is built, so the `not_yet` helper that used to
        // return this code is gone rather than kept as scaffolding nothing
        // stands on. Two real paths reach it: `fetch` when no transport is
        // installed, and `cmd_doctor` when a registered tool is missing or
        // broken. Doctor's wants its own test with a real registry directory
        // rather than a shortcut here.

        // 130: interrupted. NOT YET REACHABLE from a unit test: this is a
        // signal-handler exit path (SIGINT/SIGTERM), which needs a real
        // process and a real signal to drive, not a function call. No
        // signal handling exists in this binary yet to test in the first
        // place.
    }
}
