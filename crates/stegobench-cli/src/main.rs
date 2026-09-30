// Author:  Daniel Iwugo
// Comment: Christ is King
// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Daniel Iwugo
//! The `stegobench` command.
//!
//! THE TWO RULES THIS FILE EXISTS TO HOLD
//! --------------------------------------
//! **Content on stdout, progress and refusals on stderr.** `--json` goes to
//! stdout so `stegobench ... --json | jq` works, and so does the human text
//! whenever that text IS the thing asked for: a listing, a table, a schema, a
//! help topic, `doctor`'s report. See [`human_is_content`]. Everything else,
//! the heartbeat of a long run and the reason a command refused, goes to
//! stderr, so a pipe carries the answer and nothing else.
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
// One rendering of an IO error for the whole binary, shared with `report`,
// whose own error type used to print the operating system's number.
use stegobench_cli::plain_io as plain;
use stegobench_cli::registry;
use stegobench_cli::registry::Resolved;
use stegobench_cli::report;
use stegobench_cli::score;
use stegobench_cli::STARTER_ID;
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

    /// The same, carrying a stable word a script can branch on.
    ///
    /// The exit code says how bad it was and the message says what happened in
    /// English, and neither is a thing to branch on: there are more failures
    /// than codes, and the prose is written to be read by a person and will be
    /// reworded when a person is confused by it. `metrics` has published one of
    /// these since it shipped and `score` did not, so automating against
    /// `score` meant matching English that nothing promised to keep.
    fn err_because(code: i32, reason: &str, human: impl Into<String>) -> Self {
        let mut out = Output::err(code, human);
        out.json["reason"] = serde_json::json!(reason);
        out
    }
}

/// A [`score::ScoreError`] as the command's answer: the stable word, the exit
/// code, and the prose with the flag that caused it named where one did.
fn score_refusal(e: &score::ScoreError, limit: Option<u64>) -> Output {
    let why = e.to_string();
    // `--limit` TAKES A PREFIX, AND A CORPUS PUTS ITS COVERS FIRST
    //
    // So the flag advertised for a smoke test is the one that reliably
    // produces an all-clean set and a refusal about the corpus. The refusal is
    // true and it names the wrong culprit: nothing is wrong with the corpus,
    // the command asked for a slice of it that cannot be measured. Naming the
    // flag here rather than widening the message in `score.rs` keeps the
    // corpus's own refusal about the corpus.
    let human = match (e, limit) {
        (score::ScoreError::OneSided { .. }, Some(n)) => format!(
            "{why}.\n--limit {n} takes the FIRST {n} item(s) in corpus order, \
             and a corpus lists its covers before its stego arms, so a small \
             limit reaches clean images only. Raise it past the covers, drop \
             it and score the whole corpus, or use --split test for a smaller \
             whole."
        ),
        _ => why,
    };
    Output::err_because(e.exit_code(), e.reason(), human)
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
    //
    // Over the whole listing rather than per group, so the two tables under
    // `list all` share one set of columns and read as one table with headings
    // rather than two tables that happen to be adjacent.
    let rows = wanted.iter().map(|e| e.cells()).collect::<Vec<_>>();
    let aligned = stegobench_core::table::align(&rows);
    let mut human = if kind == "all" {
        // HEADINGS, BECAUSE A BARE `stegobench list` HAD NONE.
        //
        // It printed thirteen tool rows, a footer, then five corpus rows,
        // with nothing saying that the first block was tools and the second
        // was data, or that six of the thirteen were embedders rather than
        // the detectors `list detectors` had just counted at seven.
        let mut out: Vec<String> = Vec::new();
        for (k, heading) in [(Kind::Detector, "DETECTORS"), (Kind::Embedder, "EMBEDDERS")] {
            let group: Vec<&String> = wanted
                .iter()
                .zip(&aligned)
                .filter(|(e, _)| e.kind == k)
                .map(|(_, line)| line)
                .collect();
            if group.is_empty() {
                continue;
            }
            if !out.is_empty() {
                out.push(String::new());
            }
            out.push(format!("{heading} ({})", group.len()));
            out.extend(group.into_iter().cloned());
        }
        out.join("\n")
    } else {
        aligned.join("\n")
    };
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
        human.push_str(&format!(
            "\n\nCORPORA ({})\n{}",
            reg.corpora.len(),
            corpora_block(reg, &corpora_from)
        ));
        json["corpora"] = serde_json::json!(reg.corpora.values().collect::<Vec<_>>());
    }
    human.push_str(&format!("\n\n{}", resolved.source.line()));
    Output::ok(json, human)
}

/// Hide one payload in one image with a registered embedder.
///
/// Deliberately one image. The pairing rule says a cover and its stego twin
/// must be written from the same source array through the same code path, and
/// a loop around this command satisfies neither: it reads the cover back off
/// disk each time and gives no way to state that the pair belongs together.
/// So this makes a demonstration or a fixture, and `generators/` makes a
/// corpus, and the help says so where somebody would otherwise find out by
/// publishing a number.
fn cmd_embed(
    resolved: &Resolved,
    embedder: &str,
    cover: &Path,
    payload: &Path,
    out: &Path,
    passphrase: Option<&str>,
    no_verify: bool,
) -> Output {
    let reg = &resolved.registry;
    let Some(entry) = reg.entries.get(embedder) else {
        return Output::err(
            exit::USAGE,
            format!(
                "no embedder called {embedder} is registered. `stegobench \
                 list embedders` shows the ones that are"
            ),
        );
    };
    if entry.kind != Kind::Embedder {
        return Output::err(
            exit::USAGE,
            format!(
                "{embedder} is registered as a detector, which answers \
                 questions about an image rather than hiding anything in \
                 one. `stegobench list embedders` shows what can embed"
            ),
        );
    }
    for (what, path) in [("cover", cover), ("payload", payload)] {
        if !path.is_file() {
            return Output::err(
                exit::PREFLIGHT_REFUSED,
                format!("the {what} {} is not a file here", path.display()),
            );
        }
    }

    match stegobench_plugin::embed::run(entry, cover, payload, out, passphrase, !no_verify) {
        Err(e) if e.contains("declares no way to embed") => Output::err(exit::PREFLIGHT_REFUSED, e),
        Err(e) if e.starts_with("could not stage") || e.starts_with("no scratch") => {
            Output::err(exit::PREFLIGHT_REFUSED, e)
        }
        Err(e) => Output::err(exit::PLUGIN_FAILED, e),
        Ok(done) => {
            let checked = match &done.recovered {
                Some(Ok(())) => "the payload was extracted again and matches byte for byte",
                Some(Err(_)) => "the payload did NOT survive",
                None => "the payload was NOT checked, so nothing here says it is really in there",
            };
            let json = serde_json::json!({
                "embedder": entry.name,
                "cover": cover.display().to_string(),
                "stego": done.stego.display().to_string(),
                "bytes": done.bytes,
                "verified": match &done.recovered {
                    Some(Ok(())) => serde_json::json!(true),
                    Some(Err(e)) => serde_json::json!({ "ok": false, "reason": e }),
                    None => serde_json::Value::Null,
                },
            });
            if let Some(Err(e)) = &done.recovered {
                let mut bad = Output::err(
                    exit::VERIFY_MISMATCH,
                    format!(
                        "{} wrote {} ({} bytes) and {e}.\nThe file is there \
                         and what is in it is not what you gave it, so it is \
                         not a stego image of that payload.",
                        entry.name,
                        done.stego.display(),
                        done.bytes
                    ),
                );
                bad.json = json;
                return bad;
            }
            Output::ok(
                json,
                format!(
                    "{} wrote {} ({} bytes), and {checked}.\nOne image is a \
                     demonstration. A corpus needs every pair written from \
                     one source through one code path: see `stegobench help \
                     pairing`.",
                    entry.name,
                    done.stego.display(),
                    done.bytes
                ),
            )
        }
    }
}

fn cmd_describe(resolved: &Resolved, name: &str, toml_only: bool) -> Output {
    let reg = &resolved.registry;
    // The needs block goes FIRST, above the facts, because it is the question
    // somebody typing `describe` is usually asking. One shape for every
    // subject: a container, a binary, a service and a corpus all answer here,
    // so a reader never has to know which of the four they are holding.
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
        if toml_only {
            return raw_toml(e, json, &e.name);
        }
        // Only the roots that will still be there tomorrow. The built-in
        // registry unpacks its adapters into a scratch directory this process
        // owns, and a command naming that path is a command that cannot be
        // pasted. `adapter_roots()` is the right answer for running something
        // now and the wrong one for printing something to keep.
        let durable: &[PathBuf] = match resolved.source.path() {
            Some(_) => resolved.adapter_roots(),
            None => &[],
        };
        return Output::ok(
            json,
            describe_block(&needs, e.name.as_str(), "tool", &tool_facts(e, durable)),
        );
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
        if toml_only {
            return raw_toml(c, json, &c.id);
        }
        return Output::ok(
            json,
            describe_block(&needs, c.id.as_str(), "corpus", &corpus_facts(c)),
        );
    }
    let known: Vec<_> = reg
        .entries
        .keys()
        .chain(reg.corpora.keys())
        .cloned()
        .collect();
    // A path here is the commonest way to reach this refusal with something
    // that is not a typo, because `score` and `verify` both take a directory
    // under a flag spelled almost the same way. Saying which of the two
    // vocabularies this command speaks costs one line and saves the reader
    // working it out from a list of names that does not contain theirs.
    let mut human = if Path::new(name).exists() {
        format!(
            "{name:?} is a path on this machine, and `describe` takes a \
             registered name or corpus id rather than a path. There is nothing \
             registered under that name.\nTo measure a directory of samples:\n  \
             stegobench score --corpus {name} --detector <name>\n"
        )
    } else {
        let close = near_registered_names(reg, name);
        let mut head = format!("nothing registered as {name:?}.");
        if !close.is_empty() {
            head.push_str(&format!(" Did you mean: {}?", close.join(", ")));
        }
        head.push('\n');
        head
    };
    human.push_str(&format!("Known tools and corpora: {}", known.join(", ")));
    Output::err(exit::USAGE, human)
}

/// The registered entry verbatim, and nothing else.
///
/// A serialisation failure is reported rather than papered over with the
/// `Debug` rendering the summary path once fell back to: this output exists to
/// be piped into a TOML parser, and handing that parser Rust's struct dump
/// under exit zero is the silent wrong answer the flag is least able to afford.
fn raw_toml<T: serde::Serialize>(entry: &T, json: serde_json::Value, name: &str) -> Output {
    match toml::to_string_pretty(entry) {
        Ok(text) => {
            let mut out = Output::ok(json, text);
            out.payload_on_stdout = true;
            out
        }
        Err(e) => Output::err(
            exit::FAILURE,
            format!(
                "the registered entry for {name} could not be written back as \
                 TOML: {e}. `stegobench describe {name}` prints the summary, \
                 and `--json` prints the same entry as JSON."
            ),
        ),
    }
}

/// What `describe` prints: the readiness line, the facts, then what is needed.
///
/// One renderer for a tool and for a corpus, on purpose. They are different
/// types with different fields, and the thing a reader wants first is the same
/// for both: whether they can use it, and what to type if not.
///
/// The whole entry USED TO BE PRINTED UNDERNEATH, as TOML. That put forty
/// lines of registry between the reader and the next command, most of it
/// fields only the harness reads, and a first-time user reported reading past
/// the answer looking for it. The entry is still one flag away, under `--toml`,
/// which prints it alone so a pipe gets a document rather than a document with
/// prose around it.
/// The word in the brackets beside the name.
///
/// `needs::Readiness` answers one question, "can the harness drive this", and
/// its bare words answered a different one for the reader. A first-time user
/// read `[ready]` on a containerised detector as "ready for me to run", went
/// looking for `zsteg` on their PATH, and found nothing; and read the honest
/// `[unknown]` on the corpus that ships inside the binary as a fault.
///
/// So a tool says who it is ready FOR, and a corpus says whose copy is being
/// talked about, which for the starter corpus is this binary's own.
fn readiness_word(needs: &needs::Needs, kind: &str, name: &str) -> String {
    if kind == "corpus" && needs.readiness == needs::Readiness::Unknown {
        return if name == STARTER_ID {
            "included in this binary".to_string()
        } else {
            "bring your own copy".to_string()
        };
    }
    if needs.readiness == needs::Readiness::Ready {
        return "stegobench can run it".to_string();
    }
    needs.readiness.word().to_string()
}

fn describe_block(
    needs: &needs::Needs,
    name: &str,
    kind: &str,
    facts: &[(&str, String)],
) -> String {
    let width = facts.iter().map(|(k, _)| k.len()).max().unwrap_or(0);
    let mut text = format!("{name}  [{}]\n", readiness_word(needs, kind, name));
    for (label, value) in facts {
        // A value of several lines keeps the column: the second line of a
        // command that wraps anyway must not read as the next field's value.
        let mut lines = value.lines();
        let first = lines.next().unwrap_or("");
        text.push_str(&format!("{label:<width$}  {first}\n", width = width));
        for line in lines {
            text.push_str(&format!("{:<width$}  {line}\n", "", width = width));
        }
    }
    let steps = needs.block();
    if steps.is_empty() {
        text.push_str(&format!(
            "\nNothing needed. `stegobench doctor` says whether this {kind} \
             works.\n"
        ));
    } else {
        text.push_str(&format!("\nNeeds from you:\n{steps}\n"));
    }
    text.push_str(&format!(
        "\nThe registered entry in full: stegobench describe {name} --toml"
    ));
    text
}

/// One shell word, quoted if the shell would otherwise take it apart.
///
/// A registry lives wherever the reader installed it, and a path with a space
/// in it pasted unquoted runs a different command rather than failing.
fn shell_word(word: &str) -> String {
    let safe = |c: char| c.is_ascii_alphanumeric() || "_@%+=:,./-{}".contains(c);
    if !word.is_empty() && word.chars().all(safe) {
        return word.to_string();
    }
    format!("'{}'", word.replace('\'', r"'\''"))
}

/// The line that runs this tool on one image of the reader's own, exactly as
/// the harness would run it, with `{file}` left for them to fill in.
///
/// Built from the same pieces `stegobench_plugin::selftest` assembles, so the
/// sandbox flags, the mount, the entrypoint and the environment are the ones
/// a measured run really uses rather than a plausible set invented here. A
/// container reference carries its digest, so what this pastes is pinned.
///
/// `None` where no honest line exists: an entry with neither an image nor a
/// binary cannot be run at all, and an adapter that is not where the registry
/// says it is would paste a mount of a path that does not exist.
fn runnable_command(
    e: &stegobench_core::registry::Entry,
    adapter_roots: &[PathBuf],
) -> Option<String> {
    let invoke = e.invoke.as_ref();
    let argv = invoke.map(|i| i.argv.as_slice()).unwrap_or(&[]);

    // A service is reached by an adapter that runs here, so the image names
    // the subject and is not the thing to start.
    let host = invoke.is_some_and(|i| i.host);
    if host || e.image.is_none() {
        if let Some(bin) = e.binary.as_ref().filter(|b| !b.command.is_empty()) {
            let mut words: Vec<String> = bin.command.iter().map(|w| shell_word(w)).collect();
            words.extend(argv.iter().map(|a| shell_word(a)));
            return Some(words.join(" "));
        }
        let (Some(i), Some(rel)) = (invoke, invoke.and_then(|i| i.adapter.as_ref())) else {
            return None;
        };
        let adapter = stegobench_plugin::adapter::resolve(rel, adapter_roots).ok()?;
        let program = i
            .entrypoint
            .clone()
            .unwrap_or_else(|| stegobench_plugin::DEFAULT_HOST_ENTRYPOINT.to_string());
        let mut words = vec![shell_word(&program)];
        for arg in argv {
            words.push(shell_word(
                &arg.replace("{adapter}", &adapter.display().to_string()),
            ));
        }
        return Some(words.join(" "));
    }

    let image = e.image.as_ref()?;
    let i = invoke?;
    // The whole directory, writable, where the tool writes beside its input;
    // the one file, read only, otherwise. Same choice `selftest::run_one`
    // makes from the same flag.
    let mut words: Vec<String> = ["docker", "run", "--rm"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    if i.writable_workdir {
        words.push("--user".into());
        words.push("$(id -u):$(id -g)".into());
    }
    // The harness's own list, not a copy of it. What makes this command worth
    // printing is that it is the command the harness runs.
    for flag in stegobench_plugin::selftest::SANDBOX {
        words.push((*flag).into());
    }
    words.push("-v".into());
    words.push(if i.writable_workdir {
        "\"$PWD\":/work".into()
    } else {
        "\"$PWD/{file}\":/work/{file}:ro".into()
    });

    let mut adapter_inner = String::new();
    if let Some(rel) = &i.adapter {
        let abs = stegobench_plugin::adapter::resolve(rel, adapter_roots).ok()?;
        let base = abs.file_name()?.to_str()?;
        adapter_inner = format!("/adapter/{base}");
        words.push("-v".into());
        words.push(shell_word(&format!("{}:{adapter_inner}:ro", abs.display())));
    }
    for kv in &i.env {
        words.push("-e".into());
        words.push(shell_word(kv));
    }
    if let Some(ep) = &i.entrypoint {
        words.push("--entrypoint".into());
        words.push(shell_word(ep));
    }
    words.push(shell_word(&image.reference));
    for arg in argv {
        words.push(shell_word(
            &arg.replace("{file}", "/work/{file}")
                .replace("{adapter}", &adapter_inner),
        ));
    }
    Some(words.join(" "))
}

/// The fields of a tool entry a person reads, in the order they ask for them.
///
/// A subset rather than everything: the rest is one `--toml` away, and a
/// summary that reprints every field is the dump it replaced.
fn tool_facts(
    e: &stegobench_core::registry::Entry,
    adapter_roots: &[PathBuf],
) -> Vec<(&'static str, String)> {
    let mut facts = vec![(
        "Kind",
        match e.kind {
            Kind::Detector => "detector".to_string(),
            Kind::Embedder => "embedder".to_string(),
        },
    )];
    facts.push(("Licence", e.licence.clone()));
    let runs = match (&e.image, &e.binary) {
        (Some(i), _) => {
            let sandbox = if e.invoke.as_ref().is_some_and(|i| i.host) {
                "a service this only talks to, so its isolation is not ours to state"
            } else if i.needs_network {
                "a container, with network"
            } else {
                "a container, sandboxed with no network"
            };
            format!("{} ({sandbox})", i.reference)
        }
        (_, Some(b)) => format!(
            "{} (a program you installed, no sandbox, your network)",
            b.command.first().map(String::as_str).unwrap_or("a binary")
        ),
        _ => "nothing declared, so it cannot be run".to_string(),
    };
    facts.push(("Runs as", runs));
    // The command is in the summary rather than only in the entry because
    // `stegobench help scope` sends the reader who wants to examine their own
    // images here, to find the command that runs the tool directly. That
    // reader is the one least able to go looking for it under another flag.
    //
    // It used to be the raw `invoke.argv`, which for a container was an argv
    // that only means anything INSIDE the image (`zsteg -a {file}`, where
    // `zsteg` is on no host's PATH) and for a binary dropped the program name
    // the entry keeps in `binary.command` (`{file}`, alone). Both read as
    // something to paste and neither was.
    match runnable_command(e, adapter_roots) {
        Some(line) => {
            let containerised = e.image.is_some() && !e.invoke.as_ref().is_some_and(|i| i.host);
            let note = if !line.contains("{file}") {
                "It reads every image in the directory you run it from."
            } else if containerised {
                "{file} is your image's filename, and this runs from the directory it is in."
            } else {
                "{file} is the path to your image."
            };
            facts.push(("Run it", format!("{line}\n{note}")));
        }
        // The adapter is real and the command would be too, but the built-in
        // registry unpacks its adapters into a scratch directory that is
        // wiped when the command ends. A line naming that path pastes as a
        // file-not-found a minute later, which is worse than no line.
        None => {
            if let Some(adapter) = e.invoke.as_ref().and_then(|i| i.adapter.as_ref()) {
                facts.push((
                    "Run it",
                    format!(
                        "no command to paste: this one needs {adapter}, and \
                         the copy inside this binary lasts only as long as a \
                         run.\nWith a checkout of the repository: stegobench \
                         describe {} --registry plugins/registry",
                        e.name
                    ),
                ));
            }
        }
    }
    facts.push((
        "Answers",
        match e.emits.output {
            stegobench_core::registry::Output::Score => {
                if e.emits.higher_means_stego {
                    "a score, higher means more like stego".to_string()
                } else {
                    "a score, lower means more like stego".to_string()
                }
            }
            stegobench_core::registry::Output::Verdict => {
                "a verdict, yes or no, so it gives one point and not a curve".to_string()
            }
        },
    ));
    if !e.accepts.formats.is_empty() {
        facts.push(("Reads", e.accepts.formats.join(", ")));
    }
    if let Some(s) = e.cost.seconds_per_image {
        facts.push(("Costs", format!("about {s} seconds an image")));
    }
    if let Some(u) = &e.upstream {
        facts.push(("Upstream", u.clone()));
    }
    if let Some(n) = &e.notes {
        facts.push(("Notes", n.clone()));
    }
    facts
}

/// The fields of a corpus entry a person reads, in the order they ask for them.
///
/// The licence block is not abbreviated to its identifier. Whether the terms
/// were READ, and on what date, and whether a stego image derived from them may
/// be published, are the three questions the registry exists to answer, and
/// each one has a field precisely so a reader never has to infer it.
fn corpus_facts(c: &stegobench_core::corpus::CorpusEntry) -> Vec<(&'static str, String)> {
    use stegobench_core::corpus::{LicenceStatus, Redistribution};
    let mut facts = vec![("Name", c.name.clone()), ("What", c.description.clone())];
    if let Some(t) = &c.tier {
        facts.push(("Tier", t.clone()));
    }
    let licence = match c.licence.status {
        LicenceStatus::Verified => {
            let named = c.licence.spdx.clone().unwrap_or_else(|| "verified".into());
            let inferred = if c.licence.spdx_version_inferred {
                ", version inferred rather than stated"
            } else {
                ""
            };
            match &c.licence.verified_on {
                Some(day) => format!("{named}, verified_on {day}{inferred}"),
                None => format!("{named}, verified{inferred}"),
            }
        }
        LicenceStatus::Unverified => {
            "UNVERIFIED: nobody has established the terms, so treat it as granting nothing"
                .to_string()
        }
        LicenceStatus::NoneGranted => {
            "NO LICENCE: there is no grant, which is not the same as none being found".to_string()
        }
    };
    facts.push(("Licence", licence));
    if let Some(src) = &c.licence.source {
        facts.push(("Read from", src.clone()));
    }
    facts.push((
        "Republish",
        format!(
            "{}: {}",
            match c.licence.redistribution {
                Redistribution::Permitted => "permitted",
                Redistribution::Forbidden => "forbidden",
                Redistribution::Unknown => "unknown, which any gate here treats as no",
            },
            c.licence.redistribution_reason
        ),
    ));
    let size = match (c.properties.base_images, c.properties.total_images) {
        // IMAGES, NOT FILES. `total_images` is what the field holds and
        // "files" is what it used to print, so `describe` said 18 over a
        // corpus `fetch` correctly reports as 39 files: the same eighteen
        // images, their eighteen records, and three more beside them.
        (Some(b), Some(t)) => format!("{b} covers, {t} images in all"),
        (Some(b), None) => format!("{b} covers"),
        (None, Some(t)) => format!("{t} images"),
        (None, None) => match (&c.properties.size_mb, &c.properties.size_note) {
            (Some(mb), _) => format!("{mb} MB"),
            (None, Some(note)) => note.clone(),
            (None, None) => "size unestablished".to_string(),
        },
    };
    facts.push(("Size", size));
    if let Some(paired) = c.properties.paired {
        facts.push((
            "Paired",
            if paired {
                "yes, every stego image has its clean twin".to_string()
            } else {
                "NO, so it cannot answer the question this benchmark asks".to_string()
            },
        ));
    }
    if c.demonstration {
        facts.push((
            "Demonstration",
            "yes: enough to prove a detector runs, nowhere near enough to measure one".to_string(),
        ));
    }
    if let Some(cite) = &c.citation {
        facts.push(("Cite as", cite.clone()));
    }
    if let Some(n) = &c.notes {
        facts.push(("Notes", n.clone()));
    }
    facts
}

/// `fixtures` is `None` when the self-tests are not being run, which is the
/// only state in which no fixtures are resolved at all. Carrying the resolved
/// fixtures rather than just their directory is what lets the report say WHICH
/// ones answered: a stale `./fixtures` beside a checkout and the copy compiled
/// into the binary are different bytes, and a self-test result is about the
/// ones it actually read.
fn cmd_doctor(resolved: &Resolved, fixtures: Option<&fixtures::Fixtures>, strict: bool) -> Output {
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
    let mut present = 0;

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
            // Counted rather than left implicit, so the presence line adds up
            // to the total in front of it.
            _ => present += 1,
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
                    check.summary().replace("not verified", "responded ")
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
        rows.push((check, verdict, detail, needs, entry.kind));
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
    // Grouped under a heading naming the kind, because this screen and
    // `stegobench list detectors` are read one after the other and used to
    // disagree without explaining themselves: thirteen rows here, seven
    // there, and the word "embedder" never printed. A reader with no way to
    // tell which six were the difference reads it as something broken.
    let detectors = rows.iter().filter(|r| r.4 == Kind::Detector).count();
    let embedders = rows.len() - detectors;
    for (kind, heading, count) in [
        (Kind::Detector, "DETECTORS", detectors),
        (Kind::Embedder, "EMBEDDERS", embedders),
    ] {
        if count == 0 {
            continue;
        }
        human.push(String::new());
        human.push(format!("{heading} ({count})"));
        for (_, _, detail, needs, _) in rows.iter().filter(|r| r.4 == kind) {
            human.push(detail.clone());
            let block = needs.block();
            if !block.is_empty() {
                for line in block.lines() {
                    human.push(format!("  {line}"));
                }
            }
        }
    }
    let needing = rows
        .iter()
        .filter(|(_, _, _, n, _)| n.readiness == needs::Readiness::NeedsYou)
        .count();
    human.push(String::new());
    // TWO AXES, TWO LINES, AND EACH ONE SUMS TO THE TOTAL.
    //
    // These used to be one comma list: "13 tool(s): 6 verified, 0 answering,
    // 0 broken, 2 not installed, 1 undetermined, 7 not checked", which adds
    // up to 16. It was never wrong, it was two independent questions printed
    // as one list, and a reader adds a comma list up against the number in
    // front of it. Every bucket is printed even at zero, so both lines can
    // still be summed on a machine where one of them is empty.
    human.push(format!(
        "{} tool(s): {detectors} detector(s), {embedders} embedder(s).",
        rows.len()
    ));
    human.push(format!(
        "On this machine: {present} installed, {missing} not installed, \
         {undetermined} undetermined, {unsupported} cannot run here."
    ));
    human.push(format!(
        "Stegobench's own self-test: {passed} passed, {answered} responded, \
         {broken} failed, {skipped} not run. Passing says stegobench can \
         drive it, not that you can run it yourself."
    ));
    if undetermined > 0 {
        human.push(format!(
            "{undetermined} undetermined: nothing to install would settle it. \
             Its line says what it needs."
        ));
    }
    if unsupported > 0 {
        human.push(format!(
            "{unsupported} cannot run on {} at all, so nothing to install would change it.",
            std::env::consts::OS
        ));
    }
    if answered > 0 {
        human.push(
            "responded: it answered without settling either fixture, so nothing is proved.".into(),
        );
    }
    if skipped > 0 {
        // Never let "we did not look" read as "it is fine".
        human.push("not run is not the same as working; each line says why.".into());
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
        "detectors": detectors,
        "embedders": embedders,
        "needing_action": needing,
        "present": present,
        "verified": passed,
        "broken": broken,
        "missing": missing,
        "unsupported_here": unsupported,
        "undetermined": undetermined,
        "not_checked": skipped,
        "responded": answered,
        "tools": rows.iter().map(|(c, v, d, n, k)| serde_json::json!({
            "name": c.name,
            "kind": match k {
                Kind::Detector => "detector",
                Kind::Embedder => "embedder",
            },
            "present": c.presence.is_present(),
            "verified": c.verified,
            "status": match v {
                Verified::Passed => "passed",
                Verified::Failed(_) => "failed",
                Verified::Answered(_) => "responded",
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
    // The report IS the output, and it is the same report whether the verdict
    // is fit or unfit, so it goes to stdout under either exit code. Routing it
    // on the code instead sent `stegobench doctor > report.txt` to an empty
    // file on exactly the machine whose report was worth keeping.
    out.payload_on_stdout = true;
    // WHAT "UNFIT" MEANS, AND WHAT IT USED TO MEAN
    //
    // It used to mean any registered tool being absent, which no machine
    // will ever satisfy: thirteen tools are registered, most of them
    // containers, and a reader who has installed one detector and can
    // measure with it was told their machine was unfit and handed exit 8.
    // The front page recommends running this, so the first thing a newcomer
    // saw was a failure code for a working install, and anybody wiring it
    // into CI had to special case it.
    //
    // So the default now answers the question the command asks in its own
    // summary line, "can this machine run what it claims to": nothing usable
    // is unfit, and so is a tool that is installed and fails its own
    // self-test, because that one is not absent, it is lying. A tool you
    // simply have not installed is reported and is not a failure.
    //
    // `--strict` keeps the old meaning for the caller that wants it, which is
    // a release gate rather than a person at a terminal.
    // WITH `--no-selftest`, THE VERDICT IS ABOUT WHAT IS INSTALLED
    //
    // Fitness was read off the self-test results alone, and `--no-selftest`
    // runs none, so every machine came out with nothing usable: a box where
    // plain `doctor` exited 0 and four detectors had just scored was told it
    // was unfit and handed exit 8. The flag promises a faster report, not a
    // different answer, so what it cannot prove it does not judge, and the
    // question falls back to the one it can still answer.
    let strictly_short = strict && (missing > 0 || undetermined > 0);
    let usable = passed + answered;
    let unfit = if no_selftest {
        present == 0 || strictly_short
    } else {
        broken > 0 || usable == 0 || strictly_short
    };
    if unfit {
        out.code = exit::ENVIRONMENT_UNFIT;
    }
    if out.code == exit::ENVIRONMENT_UNFIT {
        let why = if !no_selftest && broken > 0 {
            format!("{broken} installed tool(s) failed their own self-test")
        } else if no_selftest && present == 0 {
            "no tool here is installed, so nothing could be measured".to_string()
        } else if !no_selftest && usable == 0 {
            "no tool here is usable, so nothing could be measured".to_string()
        } else {
            "--strict was given and something is missing or undetermined".to_string()
        };
        out.human.push_str(&format!("\n\nUNFIT: {why}."));
    } else if no_selftest {
        out.human.push_str(&format!(
            "\n\nFIT: {present} tool(s) installed. The self-tests were \
             skipped, so none of them is proved to work."
        ));
    } else if missing > 0 || undetermined > 0 {
        out.human.push_str(
            "\n\nFIT: what is not installed is listed, not counted against \
             you. `--strict` counts it.",
        );
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
/// Two checks, and the second is the one with teeth. The digest names what
/// the corpus's own records declare about their images, so a match proves the
/// document and the corpus describe the same manifest and nothing more: swap
/// a stego image for an easier one and leave its record alone, and every
/// digest still agrees while the number is now about different bytes. So
/// unless the caller passes `shallow`, every image is re-read and checked
/// against the digest its record states before this says the bytes are the
/// bytes.
fn cmd_verify(file: &Path, corpus: &Path, shallow: bool) -> Output {
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

    // Before anything about the corpus, because this asks whether the document
    // is the one that was written rather than whether the bytes it names are
    // the ones that were measured. A document edited after the fact cannot be
    // trusted to say which corpus to go and look at.
    if let Some(claimed) = result.content_digest.as_deref() {
        let actual = result.compute_content_digest();
        if claimed != actual {
            return Output::err(
                exit::VERIFY_MISMATCH,
                format!(
                    "{} does not match its own content digest. It declares\n  \
                     {claimed}\nand its contents come to\n  {actual}\nEvery \
                     field except the two that record WHEN the run happened is \
                     covered, so something in this document changed after it \
                     was written. Re-run the measurement rather than trusting \
                     the number in it",
                    file.display()
                ),
            );
        }
    }

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
        // The exit code is deliberately unchanged: `verify` has always
        // answered a corpus it could not read with a generic failure, and the
        // stable word is an addition beside it rather than a reclassification.
        Err(e) => return Output::err_because(exit::FAILURE, e.reason(), e.to_string()),
    };

    let claimed = &result.corpus.digest;
    let mut json = serde_json::json!({
        "ok": &found == claimed,
        "claimed": claimed,
        "found": found,
        "corpus": corpus.display().to_string(),
        "checked": if shallow { "records" } else { "bytes" },
    });
    if &found == claimed {
        // The records agree. That is necessary and it is not sufficient: the
        // digest is over what the records state, so images can be swapped
        // under records that still agree with each other. Unless the caller
        // asked for the cheap check, re-read the bytes before saying they are
        // the bytes.
        if !shallow {
            let (bad, read) = match score::rehash_corpus(corpus, |line| {
                eprintln!("verify: {line}");
            }) {
                Ok(v) => v,
                Err(e) => return Output::err_because(exit::FAILURE, e.reason(), e.to_string()),
            };
            if !bad.is_empty() {
                json["ok"] = serde_json::Value::Bool(false);
                json["mismatched"] = serde_json::json!(bad
                    .iter()
                    .map(|m| serde_json::json!({
                        "id": m.id,
                        "claimed": m.claimed,
                        "found": match &m.found {
                            Ok(f) => serde_json::json!(f),
                            Err(e) => serde_json::json!({ "error": e }),
                        },
                    }))
                    .collect::<Vec<_>>());
                let mut lines = String::new();
                for m in &bad {
                    let got = match &m.found {
                        Ok(f) => f.clone(),
                        Err(e) => format!("could not be read: {e}"),
                    };
                    lines.push_str(&format!(
                        "\n  {} claims {}\n    and is    {}",
                        m.id, m.claimed, got
                    ));
                }
                let mut out = Output::err(
                    exit::VERIFY_MISMATCH,
                    format!(
                        "the records at {} match the document, and the images \
                         do not match the records.{lines}\nThe corpus digest \
                         is computed from what the records state, so it still \
                         agrees; the bytes that were scored are not the bytes \
                         here now. That number cannot be attributed to this \
                         corpus.",
                        corpus.display()
                    ),
                );
                out.json = json;
                return out;
            }
            let mut out = Output::ok(
                json,
                format!(
                    "{} was measured on the corpus at {}. Both name {claimed}, \
                     and all {read} image(s) hash to what their records state",
                    file.display(),
                    corpus.display()
                ),
            );
            out.code = exit::OK;
            return out;
        }
        let mut out = Output::ok(
            json,
            format!(
                "{} names the same records as the corpus at {}. Both name \
                 {claimed}.\nThe images were NOT re-read, because --shallow \
                 was given, so this says the two describe the same list of \
                 records rather than the same bytes",
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
        out,
        records,
        split,
        trained_on,
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

    // The same refusals `score` gives, in the same order, because a plan that
    // accepts a command `score` would reject has answered a question nobody
    // can act on.
    if let Some(refused) = corpus_argument_problem(Some(reg), corpus) {
        return refused;
    }
    for (flag, path) in [("--out", out.as_deref()), ("--records", records.as_deref())] {
        if let Some(why) = output_inside_corpus(corpus, flag, path) {
            return Output::err_because(exit::USAGE, "output-inside-corpus", why);
        }
    }

    // The same teaching refusal `score` gives, for the same reason: somebody
    // asking what a run would cost over their own photographs is on the wrong
    // side of what this measures, and "no record beside DSC_0001" does not
    // tell them so.
    if let Some(why) = unlabelled_corpus(corpus) {
        return Output::err_because(exit::PREFLIGHT_REFUSED, "corpus-unlabelled", why);
    }

    // Asked before the corpus is walked, in the same position `score` asks
    // it, so a plan over nothing that can run refuses with the same word and
    // the same code rather than printing a table of zeroes.
    let blocked: Vec<Option<String>> = entries
        .iter()
        .map(|e| unavailable_reason(e, resolved.adapter_roots()))
        .collect();
    if blocked.iter().all(Option::is_some) {
        return Output::err_because(
            exit::PREFLIGHT_REFUSED,
            "nothing-available",
            nothing_available(&entries, &blocked),
        );
    }

    // THE SAME PRE-FLIGHT `score` RUNS, RATHER THAN A COUNT OF ITS OWN
    //
    // This used to walk the corpus itself and count scorable samples, which
    // answered "how many" and nothing else. Everything the walk could also
    // have said was left for `score` to say later, which is backwards for a
    // command whose whole job is telling you things before you spend the time:
    // a corpus of 120 dimension-mismatched pairs planned silently, and `score`
    // then found it instantly and statically from the same records.
    //
    // `registered` is deliberately `None`, so this does the record pass and
    // not the digest-and-rehash pass. Naming the corpus is `plan_configuration`'s
    // question and it answers it from the registry alone, which is what keeps
    // a plan cheaper than the run it describes.
    let mut notes: Vec<String> = Vec::new();
    let prepared = match score::prepare(
        corpus,
        None,
        *limit,
        trained_on.as_deref(),
        split.as_deref(),
        |line: &str| notes.push(line.to_string()),
    ) {
        Ok(p) => p,
        Err(e) => return score_refusal(&e, *limit),
    };
    let items = prepared.items();

    // What the run would be WORTH, beside what it would cost. A plan that
    // reports six hours and omits that the result will be `custom` has
    // answered half the question somebody asks before committing six hours.
    let (configuration, why) = plan_configuration(reg, corpus_id.as_deref(), limit.is_some());

    // WHAT `score` WOULD ACTUALLY RUN, NOT WHAT WAS ASKED FOR
    //
    // `plan ... --detector all` estimated seven detectors and `score` then
    // ran four, because this never asked the availability question `score`
    // asks. The three it could not run were not merely absent from the
    // total: they were absent from the screen, so the one command whose job
    // is to say what a run will cost overstated it and hid three blockers a
    // reader could have fixed before starting.
    let mut per_detector = Vec::new();
    let mut lines = Vec::new();
    // Summed only over the detectors that declare a rate. A total that
    // silently treated an unmeasured tool as free would be the plan lying
    // about the one thing it is for, so the count of unestimated ones is
    // carried beside the total rather than folded into it.
    let mut total_seconds = 0.0f64;
    let mut unestimated = 0usize;
    let mut unavailable = 0usize;
    let mut runnable = 0usize;
    for (entry, blocked) in entries.iter().zip(&blocked) {
        let per_image = entry.cost.seconds_per_image;
        // A rate over a detector that will not start is arithmetic about
        // nothing, so it is neither summed nor counted as unestimated.
        let seconds = match blocked {
            Some(_) => None,
            None => per_image.map(|s| s * items as f64),
        };
        match blocked {
            Some(_) => unavailable += 1,
            None => {
                runnable += 1;
                match seconds {
                    Some(s) => total_seconds += s,
                    None => unestimated += 1,
                }
            }
        }
        let duration = match (blocked, seconds) {
            (Some(why), _) => format!("NOT AVAILABLE: {why}"),
            (None, Some(s)) => format!("at least {}", human_duration(s)),
            (None, None) => "unknown: it declares no seconds_per_image".to_string(),
        };
        lines.push(format!("{:<16} {duration}", entry.name));
        per_detector.push(serde_json::json!({
            "detector": entry.name,
            "available": blocked.is_none(),
            "unavailable_reason": blocked,
            "seconds_per_image": per_image,
            "estimated_seconds": seconds,
        }));
    }

    // One JSON line per answer, measured at roughly sixty bytes on the real
    // records this writes, and one records file per detector that will run.
    let records_mb = (items as f64 * 60.0 * runnable as f64) / 1_048_576.0;
    let worst_case_seconds = items * timeout * runnable as u64;

    let mut value = serde_json::Map::new();
    value.insert("items".into(), serde_json::json!(items));
    value.insert(
        "detectors".into(),
        serde_json::json!(entries.iter().map(|e| &e.name).collect::<Vec<_>>()),
    );
    value.insert("per_detector".into(), serde_json::json!(per_detector));
    value.insert("runnable_detectors".into(), serde_json::json!(runnable));
    value.insert(
        "unavailable_detectors".into(),
        serde_json::json!(unavailable),
    );
    value.insert("estimated_seconds".into(), serde_json::json!(total_seconds));
    value.insert(
        "unestimated_detectors".into(),
        serde_json::json!(unestimated),
    );
    value.insert("records_mb".into(), serde_json::json!(records_mb));
    value.insert("configuration".into(), serde_json::json!(configuration));
    value.insert(
        "worst_case_seconds".into(),
        serde_json::json!(worst_case_seconds),
    );
    // So a caller cannot mistake the total for something this machine
    // measured. Nothing here times a detector, and nothing yet replaces a
    // declared rate with a measured one.
    value.insert("rate_source".into(), serde_json::json!("declared"));
    value.insert("estimate_is_a_lower_bound".into(), serde_json::json!(true));
    value.insert("corpus_notes".into(), serde_json::json!(notes));

    let mut human = format!("{items} item(s) to score with each of {runnable} detector(s):\n");
    human.push_str(&lines.join("\n"));
    let estimated = runnable - unestimated;
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
            "\n\nTotal          at least {}, over the {estimated} that declare \
             a rate",
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
        "\nRecords        about {records_mb:.1} MB\nWorst case     \
         {}\nConfiguration  {configuration}: {why}",
        human_duration(worst_case_seconds as f64)
    ));
    if unavailable > 0 {
        human.push_str(&format!(
            "\n\n{unavailable} of {} asked for cannot run here and are left \
             out of every figure above. `stegobench doctor` says what each \
             one needs.",
            entries.len()
        ));
    }

    // One line, because the reasoning behind it is on `plan --help` and a
    // reader who wants it can ask. What cannot be left out is the claim: a
    // total presented without the word "declared" reads as a measurement,
    // and saying "about" in front of a figure that is systematically low is
    // the plan being confidently wrong.
    if estimated > 0 {
        human.push_str(
            "\n\nRates are declared by each registry entry, not measured, and \
             exclude per-image start-up. Read the total as a floor. \
             `stegobench plan --help` says why.",
        );
    }
    if !notes.is_empty() {
        human.push_str("\n\nAbout this corpus:\n  ");
        human.push_str(&notes.join("\n  "));
    }

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
            "no --corpus-id, so nothing to check this directory against".into(),
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
    ///
    /// `reason` is the stable word, carried beside the prose so a single
    /// detector's failure can publish it the way `metrics` does.
    Failed {
        why: String,
        code: i32,
        reason: &'static str,
    },
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
            return Err(Output::err_because(
                exit::USAGE,
                "all-with-others",
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
            return Err(Output::err_because(
                exit::USAGE,
                "no-detectors-registered",
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
            return Err(Output::err_because(
                exit::USAGE,
                "unknown-detector",
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
            return Err(Output::err_because(
                exit::USAGE,
                "not-a-detector",
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

/// What `--corpus` was given, after BOTH of its plausible meanings were tried.
///
/// WHY ONE FLAG HAS TWO PLAUSIBLE MEANINGS
/// ---------------------------------------
/// This tool says the word "corpus" in two vocabularies. `fetch` and
/// `--corpus-id` take a registered ID, a name for a dataset somebody
/// publishes. `score --corpus` and `verify --corpus` take a PATH, the unpacked
/// bytes on this machine, because both of them read the bytes. A reader who
/// learns one of those first types it at the other, and the answer they used
/// to get was "there is no corpus at pentimento-core: nothing is there", which
/// is true, unhelpful, and says nothing about the vocabulary they are in.
///
/// So a value that is not a directory is looked up as an id before it is
/// refused, and the refusal says what was tried.
///
/// WHY EVERY ANSWER HERE IS EXIT 3 AND NOT EXIT 2
/// ----------------------------------------------
/// The two codes divide on a line this project has already drawn: 2 means the
/// command line was malformed, and 3 means what you named is not there or not
/// fit to use. `--corpus somewhere-that-is-not-there` is a well-formed command
/// naming a thing that does not exist, which is the second. Only the WORDING
/// changed here; the code is the one the contract test has asserted since
/// before the two vocabularies were noticed.
fn corpus_argument_problem(reg: Option<&Registry>, corpus: &Path) -> Option<Output> {
    if matches!(std::fs::metadata(corpus), Err(ref e) if e.kind() == std::io::ErrorKind::NotFound) {
        return Some(unresolved_corpus(reg, corpus));
    }
    corpus_path_problem(corpus)
        .map(|why| Output::err_because(exit::PREFLIGHT_REFUSED, "corpus-unusable", why))
}

/// A `--corpus` value that is not a directory, answered by what it might be.
fn unresolved_corpus(reg: Option<&Registry>, corpus: &Path) -> Output {
    let typed = corpus.to_string_lossy().into_owned();
    if reg.is_some_and(|r| r.corpora.contains_key(typed.as_str())) {
        return Output::err_because(
            exit::PREFLIGHT_REFUSED,
            "corpus-missing",
            format!(
                "there is no corpus at {typed}, and {typed} is a registered \
                 corpus id rather than a path: --corpus takes a directory of \
                 unpacked samples on this machine.\nGet the bytes first:\n  \
                 stegobench fetch {typed} \
                 --tier <tier>\nthen unpack them, point --corpus at the \
                 directory, and name the id under --corpus-id {typed} so the \
                 run is checked against the digest the registry declares.\n\
                 `stegobench describe {typed}` says how it is obtained when it \
                 declares no download route."
            ),
        );
    }
    let as_id = match reg {
        Some(r) => {
            let close = near_corpus_ids(r, &typed);
            if close.is_empty() {
                "none is registered under that name".to_string()
            } else {
                format!(
                    "none is registered under that name; close: {}",
                    close.join(", ")
                )
            }
        }
        // Fail loud rather than report an absence nobody established: with no
        // registry the id half of the question was not asked, and saying it
        // was would send the reader to check a spelling that may be right.
        None => "the registry could not be read, so this was not checked".to_string(),
    };
    Output::err_because(
        exit::PREFLIGHT_REFUSED,
        "corpus-missing",
        format!(
            "there is no corpus at {typed}, and it is not a registered corpus \
             id either. Both were tried:\n  as a directory  nothing is there\n  \
             as a corpus id  {as_id}\n`stegobench list corpora` names every \
             registered id. --corpus wants a directory of samples, an image \
             with a record beside it; `stegobench help scope` says what one \
             has to hold."
        ),
    )
}

/// A corpus id that is not in the registry, answered the same way everywhere.
///
/// `next` is the sentence that closes it, because the commands that take an id
/// fail for one reason and want different things done about it.
/// The per-arm breakdown, indented under the detector it belongs to.
///
/// Empty for a corpus of one arm, because `metrics.per_arm` is empty there
/// and a breakdown of one row is the headline printed twice.
///
/// Every arm is shown rather than a best-and-worst summary. A Core tier is 39
/// arms, which is about a screen, and the reader who is scoring one is not
/// doing it casually; the arm they care about is as likely to be in the
/// middle as at either end.
fn per_arm_lines(arms: &[stegobench_core::result::ArmMetrics]) -> Vec<String> {
    if arms.is_empty() {
        return Vec::new();
    }
    // Widest name, so the numbers line up in a column a reader can scan down
    // rather than hunting along ragged rows.
    let width = arms.iter().map(|a| a.arm.len()).max().unwrap_or(0);
    let mut out = vec![format!(
        "{:<16} by arm, each against the same {} clean image(s):",
        "",
        arms.first().map(|a| a.n_clean).unwrap_or(0)
    )];
    for a in arms {
        out.push(format!(
            "{:<18}{:<width$}  AUC {:.4}{}  {} stego",
            "",
            a.arm,
            a.auc,
            match a.auc_ci95 {
                Some([lo, hi]) => format!(" [{lo:.4}, {hi:.4}]"),
                None => String::new(),
            },
            a.n_stego,
            width = width
        ));
    }
    out
}

fn unregistered_corpus_id(reg: &Registry, id: &str, next: &str) -> Output {
    let mut human = format!("no corpus with id {id:?} is registered.");
    if Path::new(id).is_dir() {
        // The mirror image of `unresolved_corpus`, and the other half of the
        // same confusion: a directory handed to the flag that wants a name.
        human.push_str(&format!(
            " It is a directory on this machine, and an id names a dataset \
             rather than a path.\nThose bytes are already here, so score them \
             directly:\n  stegobench score --corpus {id} --detector <name>"
        ));
    } else {
        let close = near_corpus_ids(reg, id);
        if !close.is_empty() {
            human.push_str(&format!(" Did you mean: {}?", close.join(", ")));
        }
    }
    human.push('\n');
    human.push_str(next);
    // PREFLIGHT_REFUSED rather than USAGE, matching `--corpus` on a directory
    // that is not there. The two codes divide on whether the command line was
    // malformed or whether what it named is absent, and an id nobody has
    // registered is the second: the flag took the kind of value it asked for.
    // These two call sites answered USAGE until 2026-09-30 purely because
    // nothing had made them agree with the path case.
    Output::err_because(exit::PREFLIGHT_REFUSED, "unregistered-corpus-id", human)
}

/// Registered corpus ids close enough to what was typed to be worth naming.
///
/// Substring matches count however far apart the two strings are: `pentimento`
/// is fifteen edits from `pentimento-core` and is obviously the thing the
/// reader meant, which is the case an edit distance alone answers worst.
/// Sorted and capped so two runs print the same suggestions in the same order.
fn near_corpus_ids(reg: &Registry, typed: &str) -> Vec<String> {
    near_names(reg.corpora.keys().map(String::as_str), typed)
}

/// The same, over everything `describe` can look up.
fn near_registered_names(reg: &Registry, typed: &str) -> Vec<String> {
    near_names(
        reg.entries
            .keys()
            .chain(reg.corpora.keys())
            .map(String::as_str),
        typed,
    )
}

fn near_names<'a>(names: impl Iterator<Item = &'a str>, typed: &str) -> Vec<String> {
    let typed = typed.to_lowercase();
    let mut scored: Vec<(usize, String)> = names
        .filter_map(|name| {
            let lower = name.to_lowercase();
            let d = edit_distance(&typed, &lower);
            let close = d <= 3 || lower.contains(&typed) || typed.contains(&lower);
            close.then(|| (d, name.to_string()))
        })
        .collect();
    scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    scored.into_iter().take(3).map(|(_, name)| name).collect()
}

/// A path resolved far enough to be compared with another one, existing or not.
///
/// [`Path::canonicalize`] answers only for a path that is already there, and
/// the path this has to judge is one nothing has written yet. So the deepest
/// ancestor that DOES exist is canonicalised, which resolves every symbolic
/// link and every `..` in that half, and the components below it are rejoined
/// on the end.
///
/// `None` means the question could not be answered, which happens for a
/// relative path with no working directory and for one whose unwritten half
/// ends in `..`. A caller treats that as "cannot prove containment" and lets
/// the write fail on its own terms rather than refusing on a guess.
fn resolved_for_comparison(path: &Path) -> Option<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(path)
    };
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    let mut head = absolute.as_path();
    loop {
        if let Ok(base) = head.canonicalize() {
            let mut resolved = base;
            for component in tail.iter().rev() {
                resolved.push(component);
            }
            return Some(resolved);
        }
        let name = head.file_name()?;
        tail.push(name.to_os_string());
        head = head.parent()?;
    }
}

/// Whether an output path would be written INTO the corpus being scored.
///
/// WHY THIS IS A REFUSAL AND NOT A WARNING
/// ---------------------------------------
/// The corpus digest is computed over what the corpus holds. A result document
/// or a records file written inside it becomes part of it, so the next run
/// reads a different set, computes a different digest, and reports that the
/// corpus changed when nothing about the images did. The measurement before it
/// cannot be reproduced and nothing says why. `--out ./mycorpus/result.json`
/// is a natural thing to type and it quietly destroys the one property this
/// tool exists to protect, so it is refused at the boundary rather than
/// explained afterwards.
///
/// Both paths are resolved before they are compared, so a corpus reached
/// through a symbolic link and an `--out` reached through the real directory
/// are still recognised as the same place.
///
/// The caller raises this as a usage error, exit 2, deliberately: both paths
/// are well formed and both destinations exist or could be created, and it is
/// the COMBINATION of the two that is wrong. That is the line exit 2 draws.
/// Exit 3 is for a thing that is named and is not there, which is a different
/// fault and needs a different answer from a script.
fn output_inside_corpus(corpus: &Path, flag: &str, out: Option<&Path>) -> Option<String> {
    let out = out?;
    let corpus_real = resolved_for_comparison(corpus)?;
    let out_real = resolved_for_comparison(out)?;
    if !out_real.starts_with(&corpus_real) {
        return None;
    }
    let what = if out_real == corpus_real {
        format!("the corpus directory {} itself", corpus.display())
    } else {
        format!("inside the corpus at {}", corpus.display())
    };
    let mut why = format!(
        "{flag} {} is {what}.\nWriting there adds a file to the corpus, so the \
         next run reads a different set of bytes, computes a different corpus \
         digest, and reports that the corpus has changed when nothing about \
         the images did. The number measured now could not be reproduced \
         afterwards.\nPut it anywhere outside the corpus.",
        out.display()
    );
    if let Some(parent) = corpus_real.parent() {
        let name = out_real
            .file_name()
            .unwrap_or_else(|| std::ffi::OsStr::new("results"));
        why.push_str(&format!(
            " Beside it works: {flag} {}",
            parent.join(name).display()
        ));
    }
    Some(why)
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
    /// Which half of the train and test split to score, if only one.
    split: Option<&'a str>,
    records: Option<&'a Path>,
    out: Option<&'a Path>,
    timeout: u64,
    /// How many images to score at once. 1 is one at a time.
    jobs: usize,
    /// Keep what the detector printed for every image.
    keep_raw: bool,
    limit: Option<u64>,
}

fn cmd_score(resolved: &Resolved, req: ScoreRequest<'_>) -> Output {
    let ScoreRequest {
        corpus,
        detectors,
        corpus_id,
        trained_on,
        split,
        records,
        out,
        timeout,
        jobs,
        keep_raw,
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
    if let Some(refused) = corpus_argument_problem(Some(reg), corpus) {
        return refused;
    }

    // Before the corpus is walked and before any detector is probed, because
    // it costs two `canonicalize` calls and the alternative is discovering it
    // by way of a corpus that no longer matches its own digest.
    for (flag, path) in [("--out", out), ("--records", records)] {
        if let Some(why) = output_inside_corpus(corpus, flag, path) {
            return Output::err_because(exit::USAGE, "output-inside-corpus", why);
        }
    }

    // Asked BEFORE availability, because it is the more useful refusal and
    // because it does not depend on any detector. Somebody who points `score`
    // at their own folder of photographs is on the wrong side of the thing
    // this tool does, and telling them "docker pull ..." sends them to install
    // a container that will not answer their question either.
    if let Some(why) = unlabelled_corpus(corpus) {
        return Output::err_because(exit::PREFLIGHT_REFUSED, "corpus-unlabelled", why);
    }

    // Resolved before anything runs, so a typo costs a usage error rather than
    // a corpus walk followed by one.
    let registered = match corpus_id {
        None => None,
        Some(id) => match reg.corpora.get(id) {
            Some(entry) => Some(entry),
            None => {
                return unregistered_corpus_id(
                    reg,
                    id,
                    "`stegobench list corpora` shows what is. Leaving \
                     --corpus-id out scores the directory anyway, and marks \
                     the result `custom`.",
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
                return Output::err_because(
                    exit::USAGE,
                    "output-not-a-directory",
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
    let blocked: Vec<Option<String>> = entries
        .iter()
        .map(|e| unavailable_reason(e, resolved.adapter_roots()))
        .collect();
    let mut runnable = Vec::new();
    let mut outcomes: Vec<(String, Outcome)> = Vec::new();
    for (entry, why) in entries.iter().zip(&blocked) {
        match why {
            Some(why) => outcomes.push((entry.name.clone(), Outcome::Skipped { why: why.clone() })),
            None => runnable.push(*entry),
        }
    }

    // With one detector asked for and that one unavailable, nothing was
    // measured and the corpus is not worth walking. Kept as the refusal it has
    // always been, with the same exit code, rather than becoming a zero-result
    // "run" that happens to have skipped everything.
    if runnable.is_empty() {
        return Output::err_because(
            exit::PREFLIGHT_REFUSED,
            "nothing-available",
            format!(
                "nothing was measured: {}",
                nothing_available(&entries, &blocked)
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
    let prepared = match score::prepare(corpus, registered, limit, trained_on, split, say) {
        Ok(p) => p,
        Err(e) => return score_refusal(&e, limit),
    };
    say(&format!(
        "corpus established in {:.1}s: {} item(s) to score with each detector",
        prepared.preflight_seconds(),
        prepared.items()
    ));

    for (i, entry) in runnable.iter().enumerate() {
        let records_path = records_for(corpus, records, &out_dir, &entry.name, many, prepared.side);
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
            jobs,
            keep_raw,
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

/// Why a detector would not be run here, in the words `score` reports it in.
///
/// `None` means it would run. One function rather than one per command,
/// because `plan` answered this question by not asking it: a seven detector
/// plan estimated a run that `score` then did with four, overstating the job
/// and saying nothing at all about the three hard blockers. A pre-flight that
/// disagrees with the run it previews is worse than no pre-flight.
///
/// PRESENT IS NOT THE SAME AS DRIVABLE, AND `score` USED TO TREAT IT AS THE
/// SAME.
///
/// Availability answers whether the code is on this machine. An entry with no
/// invoke block passes that and still says nothing about what command to
/// launch, so there is nothing to run. `doctor` has reported this since it
/// shipped and `score` did not: the run announced "1 of 1 that can run here",
/// started the tool once per image, recorded "entry declares no invoke block"
/// against every one of them, and then refused with "the corpus holds 0 clean
/// and 0 stego image(s)" over a corpus holding six and twelve. A gap in this
/// project's own registry was reported as a fault in the user's corpus. It is
/// a skip.
///
/// Asked inside the Present arm rather than before the check, so a tool that
/// is neither installed nor drivable is still answered with the half the
/// reader can act on.
fn unavailable_reason(
    entry: &stegobench_core::registry::Entry,
    adapter_roots: &[PathBuf],
) -> Option<String> {
    match availability::check(entry, adapter_roots).presence {
        Presence::Present { .. } if entry.invoke.is_none() => Some(
            "declares no invoke block, so nothing in its registry entry says \
             what command to launch and the host has no way to drive it"
                .to_string(),
        ),
        Presence::Present { .. } => None,
        Presence::Unsupported { reason } => Some(format!("cannot run on this machine: {reason}")),
        Presence::Absent { reason } => Some(format!(
            "is registered but is not on this machine: {reason}"
        )),
        Presence::Unknown { reason } => Some(format!(
            "whether it can run here could not be established: {reason}"
        )),
    }
}

/// The refusal `score` and `plan` share when not one detector can run.
///
/// `blocked` is positional against `entries`, and every entry in it has a
/// reason by the time this is called.
fn nothing_available(
    entries: &[&stegobench_core::registry::Entry],
    blocked: &[Option<String>],
) -> String {
    let why = entries
        .iter()
        .zip(blocked)
        .filter_map(|(e, b)| b.as_ref().map(|w| format!("{} {w}", e.name)))
        .collect::<Vec<_>>()
        .join("\n  ");
    format!(
        "not one of the {} detector(s) asked for is available here.\n  \
         {why}\n`stegobench doctor` checks every registered tool at once and \
         says what each one needs.",
        entries.len()
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
    // NAMING NO DESTINATION USED TO MEAN THROWING THE DOCUMENT AWAY
    //
    // This returned `Ok(None)` for a single detector whatever `--out` said,
    // and the caller reads `None` as "write to the file the caller named".
    // With no file named there was nothing to write to, so the run printed
    // its AUC, wrote its records, exited 0 and discarded the versioned
    // document the whole tool exists to produce. A ten hour run over Core
    // lost the only artefact anybody could check, silently.
    //
    // So the default destination is decided by whether a path was given, not
    // by how many detectors were asked for. One detector and no `--out` now
    // lands in the same directory several detectors would.
    let dir = match out {
        // A path was named and it is not an existing directory, so it is the
        // file this one detector writes to. Still `None`; the caller owns it.
        Some(_) if !many => return Ok(None),
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
        return Err(Output::err_because(
            exit::FAILURE,
            "results-unwritable",
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
///
/// The split is the third part of that key, for the same reason. `--split
/// test` scores a subset in a different order, so a records file written for
/// the whole corpus does not line up with it; the resume check catches the
/// mismatch and refuses, which is correct but reads as "the corpus changed"
/// when nothing changed except which half was asked for. Keying the name on
/// the split means the two runs never meet in the first place.
fn records_for(
    corpus: &Path,
    records: Option<&Path>,
    out_dir: &Option<PathBuf>,
    detector: &str,
    many: bool,
    split: Option<score::Side>,
) -> PathBuf {
    let file = match split {
        Some(side) => format!("{detector}.{}.records.jsonl", side.as_str()),
        None => format!("{detector}.records.jsonl"),
    };
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
    jobs: usize,
    keep_raw: bool,
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
                    reason: "records-unwritable",
                };
            }
        }
    }

    let (result, tally) = match score::score_one(
        entry,
        prepared,
        records,
        score::How {
            timeout: std::time::Duration::from_secs(timeout),
            jobs,
            keep_raw,
            adapter_roots,
        },
        |line: &str| say(line),
    ) {
        Ok(pair) => pair,
        Err(e) => {
            return Outcome::Failed {
                why: e.to_string(),
                code: e.exit_code(),
                reason: e.reason(),
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
            reason: "result-invalid",
        };
    }

    let body = match serde_json::to_string_pretty(&result) {
        Ok(b) => b,
        Err(e) => {
            return Outcome::Failed {
                why: format!("could not write the result: {e}"),
                code: exit::FAILURE,
                reason: "result-unwritable",
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
                reason: "result-unwritable",
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
            } => {
                lines.push(format!(
                    "{name:<16} AUC {:.4}{}  {} clean / {} stego / {} unanswered{}{}",
                    result.metrics.auc,
                    // Printed beside the figure rather than under the table,
                    // because the interval is what stops two AUCs differing in
                    // the third decimal from being read as two different
                    // detectors, and a caveat a line away is a caveat nobody
                    // carries when they copy the number out.
                    match result.metrics.auc_ci95 {
                        Some([lo, hi]) => format!(" [{lo:.4}, {hi:.4}]"),
                        None => String::new(),
                    },
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
                ));
                // The pooled figure is the misleading one whenever a corpus
                // holds several arms: it lands between them and describes
                // none. Printed rather than left in the document, because a
                // number a reader has to open a file to qualify is a number
                // they will quote unqualified.
                lines.extend(per_arm_lines(&result.metrics.per_arm));
            }
            Outcome::Skipped { why } => {
                lines.push(format!("{name:<16} NOT MEASURED, skipped: {why}"))
            }
            Outcome::Failed { why, .. } => {
                lines.push(format!("{name:<16} NOT MEASURED, failed: {why}"))
            }
        }
    }

    // Adjacent to the figures rather than in the footer. A definition one
    // screen away from the number it defines is a definition nobody reads,
    // and neither "AUC" nor the bracketed pair beside it is expanded here.
    if measured > 0 {
        lines.push("AUC and the bracketed pair: `stegobench help results`.".into());
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
    // TWO "Next:" LINES ARE NONE.
    //
    // A run of the shipped starter corpus into a directory printed both
    // "Next: stegobench report ..." and "Next: stegobench describe
    // pentimento-core", one under the other, and a reader with two next steps
    // has no next step. They are gathered here and printed as an ordered
    // pair instead, nearest first.
    let mut next: Vec<String> = Vec::new();
    if many {
        if let Some(dir) = out_dir {
            next.push(format!(
                "stegobench report {} --format markdown",
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
                "{largest} images is a demonstration, not a measurement."
            ));
            next.push(offer.command().to_string());
        }
    }
    for (label, command) in ["Next: ", "Then: "].iter().zip(&next) {
        lines.push(format!("{label}{command}"));
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
            Outcome::Failed { why, code, .. } => serde_json::json!({
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
            Some((_, Outcome::Failed { why, code, reason })) => {
                return Output::err_because(*code, reason, why.clone())
            }
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

include!(concat!(env!("OUT_DIR"), "/embedded_starter.rs"));

/// Write the compiled-in starter corpus to `dest`, returning how many files.
///
/// Refuses a destination that already holds files rather than merging into
/// it: a half-overwritten corpus whose records describe images from two
/// different copies is worse than no corpus, and it is the kind of thing
/// nobody notices until a number comes out wrong.
fn write_embedded_starter(dest: &Path) -> Result<usize, String> {
    if let Ok(mut entries) = std::fs::read_dir(dest) {
        if entries.next().is_some() {
            return Err(format!(
                "{} already has something in it. Name an empty directory \
                 with --dest, or move that one aside: writing over half of a \
                 corpus leaves records describing images from two different \
                 copies, and nothing downstream can tell.",
                dest.display()
            ));
        }
    }
    for (rel, bytes) in EMBEDDED_STARTER {
        let path = dest.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("could not make {}: {e}", parent.display()))?;
        }
        std::fs::write(&path, bytes)
            .map_err(|e| format!("could not write {}: {e}", path.display()))?;
    }
    Ok(EMBEDDED_STARTER.len())
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
        return unregistered_corpus_id(
            &resolved.registry,
            id,
            "`stegobench list corpora` shows what is registered.",
        );
    };
    // A corpus carried inside this binary is obtained by writing it out, not
    // by downloading it, and this is the verb somebody reaches for either
    // way. Before the refusal, because the refusal is about a download route
    // and this corpus needs none: `describe` used to tell the reader to point
    // `score` at a path "in a checkout of this repository", which is false
    // for anybody who installed the binary, and that is everybody the starter
    // corpus exists for.
    if entry.id == STARTER_ID && !EMBEDDED_STARTER.is_empty() {
        let dest = dest
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from(STARTER_ID));
        return match write_embedded_starter(&dest) {
            Err(e) => Output::err(exit::FAILURE, e),
            Ok(n) => Output::ok(
                serde_json::json!({
                    "corpus": entry.id,
                    "dest": dest.display().to_string(),
                    "files": n,
                    "source": "compiled into this binary",
                }),
                format!(
                    "wrote {n} file(s) to {}, from the copy compiled into \
                     this binary rather than from a download.\n\nScore it \
                     with:\n  stegobench score --corpus {} --detector \
                     <name>\n\nEighteen images is a demonstration that the \
                     machinery works, not a measurement of anything.",
                    dest.display(),
                    dest.display()
                ),
            ),
        };
    }
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

/// The words somebody arriving with "is there something hidden in my photo"
/// types before they read anything.
///
/// They are answered with the same tailored refusal rather than left to clap's
/// did-you-mean, which suggested `schema` for `check` and nothing at all for
/// `scan`. They are all the same misunderstanding, so they all get the same
/// answer to it. `check` and `scan` are also hidden subcommands so that a
/// trailing path parses; the rest never reach clap's command tree and are
/// caught by `wrong_direction` instead.
const WRONG_DIRECTION: &[&str] = &[
    "analyse", "analyze", "check", "decode", "detect", "examine", "extract", "find", "inspect",
    "reveal", "run", "scan", "search", "test", "unhide",
];

/// Is this one of the words above, whatever case it was typed in?
fn wrong_direction(typed: &str) -> bool {
    let typed = typed.to_lowercase();
    WRONG_DIRECTION.contains(&typed.as_str())
}

/// Words that mean the right thing and are not the verb.
///
/// These are not the wrong direction: somebody typing them has understood
/// what the tool measures and has only guessed the wrong word for it. They
/// used to reach the generic "there is no `stegobench benchmark`" line, which
/// says what is absent and nothing about what is present, and "benchmark" is
/// the word in the product's own name and the likeliest first guess there is.
const RIGHT_IDEA: &[&str] = &[
    "assess",
    "bench",
    "benchmark",
    "benchmarks",
    "compare",
    "eval",
    "evaluate",
    "grade",
    "measure",
    "rank",
];

fn right_idea(typed: &str) -> bool {
    let typed = typed.to_lowercase();
    RIGHT_IDEA.contains(&typed.as_str())
}

/// The redirect those words get: the verb they meant, and the two commands.
fn cmd_right_idea(word: &str) -> Output {
    Output::err(
        exit::USAGE,
        format!(
            "there is no `stegobench {word}`. Measuring a detector against a \
             labelled corpus is `score`.\n\n\
             `stegobench plan score --corpus <dir> --detector <name>`  what \
             it would cost\n\
             `stegobench score --corpus <dir> --detector <name>`       the \
             measurement\n\
             `stegobench list detectors`                               what \
             is registered here"
        ),
    )
}

/// The refusal those words get.
///
/// The second clause names no verb of its own. "it does not run your own
/// images" is what a verb-substituting sentence produced for `run`, and a
/// refusal that reads as nonsense teaches the reader nothing.
fn cmd_wrong_direction(word: &str) -> Output {
    Output::err(
        exit::USAGE,
        format!(
            "there is no `stegobench {word}`. Stegobench measures DETECTORS \
             against labelled images, and it cannot tell you whether anything \
             is hidden in an image of yours.\n\n\
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
        Command::Verify {
            file,
            corpus,
            shallow,
        } => {
            // The registry is resolved leniently here and nowhere else in this
            // arm: `verify` compares a document against bytes and needs no
            // registry to do it, so a machine without one must still be able
            // to run it. It is read only so that a `--corpus` value naming a
            // registered id rather than a directory can be told apart from a
            // typo, and a registry that cannot be read costs that one sentence
            // rather than the command.
            let reg = registry::resolve(cli.registry.as_deref()).ok();
            match corpus_argument_problem(reg.as_ref().map(|r| &r.registry), corpus) {
                Some(refused) => refused,
                None => cmd_verify(file, corpus, *shallow),
            }
        }
        Command::List { kind } => with_registry(cli, |r| cmd_list(r, kind)),
        Command::Describe { name, toml } => with_registry(cli, |r| cmd_describe(r, name, *toml)),
        Command::Embed {
            embedder,
            cover,
            payload,
            out,
            passphrase,
            no_verify,
        } => with_registry(cli, |r| {
            cmd_embed(
                r,
                embedder,
                cover,
                payload,
                out,
                passphrase.as_deref(),
                *no_verify,
            )
        }),
        Command::Plan { command } => with_registry(cli, |r| cmd_plan(r, command)),
        Command::Doctor {
            fixtures,
            no_selftest,
            strict,
        } => with_registry(cli, |r| {
            if *no_selftest {
                // Nothing will read the fixtures, so nothing looks for them.
                // Unpacking the built-in copy here would be work done to
                // satisfy a parameter rather than a question.
                return cmd_doctor(r, None, *strict);
            }
            match fixtures::resolve(fixtures.as_deref()) {
                // Held for the whole call: for the built-in copy this owns the
                // scratch directory the images were unpacked into.
                Ok(found) => cmd_doctor(r, Some(&found), *strict),
                Err(e) => Output::err(exit::PREFLIGHT_REFUSED, e.to_string()),
            }
        }),
        Command::Score {
            corpus,
            detector,
            corpus_id,
            trained_on,
            split,
            records,
            out,
            timeout,
            jobs,
            keep_raw,
            limit,
        } => with_registry(cli, |r| {
            cmd_score(
                r,
                ScoreRequest {
                    corpus,
                    detectors: detector.as_slice(),
                    corpus_id: corpus_id.as_deref(),
                    trained_on: trained_on.as_deref(),
                    split: split.as_deref(),
                    records: records.as_deref(),
                    out: out.as_deref(),
                    timeout: *timeout,
                    jobs: *jobs,
                    keep_raw: *keep_raw,
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
            if wrong_direction(&typed) {
                eprintln!("{}", cmd_wrong_direction(&typed).human);
                std::process::exit(exit::USAGE);
            }
            if right_idea(&typed) {
                eprintln!("{}", cmd_right_idea(&typed).human);
                std::process::exit(exit::USAGE);
            }
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
        // `stegobench plan --corpus ...` is the natural first attempt at a
        // command that wraps another one, and clap answers it with "to pass
        // '--corpus' as a value, use '-- --corpus'", which would quote the
        // flag as a literal word and plan nothing at all.
        Err(ref e) if plan_without_its_verb(e).is_some() => {
            let rest = plan_without_its_verb(e).expect("matched");
            eprintln!(
                "`plan` takes the command you would run, whole, so it has to \
                 start with that command's own name."
            );
            eprintln!("\nYou typed:\n  stegobench plan {rest}");
            eprintln!("You probably meant:\n  stegobench plan score {rest}");
            eprintln!("\n`score` is the only command that can be planned today.");
            std::process::exit(exit::USAGE);
        }
        // `stegobench score ~/Pictures` is the commonest wrong first command
        // there is, because it is what the tool sounds like it does. clap
        // answers it with the required flags it did not get, which is true
        // and tells the one person who most needs telling nothing at all.
        // The splash screen is three lines about this exact
        // misunderstanding, so the refusal says the same thing.
        Err(e) if looks_like_a_folder_of_photos(&e) => {
            eprintln!(
                "a path was given to `score` without a flag, and this does \
                 not take a folder of images that way."
            );
            eprintln!(
                "\nIf you have a labelled corpus, name it with --corpus, and \
                 a detector with --detector."
            );
            eprintln!(
                "If you meant your own pictures, this is not the tool: it \
                 measures how good a detector is, using images whose answers \
                 are already known, and it cannot tell you whether something \
                 is hidden in yours."
            );
            eprintln!("\n`stegobench help scope` is the whole of why.");
            std::process::exit(exit::USAGE);
        }
        Err(ref e) if misleading_suggestion(e).is_some() => {
            let typed = misleading_suggestion(e).expect("matched");
            let command = std::env::args().nth(1).unwrap_or_default();
            eprintln!("there is no `{typed}` here.");
            if command.is_empty() || command.starts_with('-') {
                eprintln!("`stegobench --help` lists every flag.");
            } else {
                eprintln!("`stegobench {command} --help` lists every flag it takes.");
            }
            std::process::exit(exit::USAGE);
        }
        Err(e) => e.exit(),
    }
}

/// Did clap offer a flag that has nothing to do with what was typed?
///
/// `Some(typed)` is the unknown flag, and the caller answers without the
/// suggestion. clap's did-you-mean is looser than this tool's: `--threads`
/// drew `--records`, five edits away and about writing files rather than
/// about concurrency, which sends a reader down a road that does not go
/// where they were headed. There is no concurrency flag, and no suggestion at
/// all is a better answer than a confident wrong one.
///
/// A genuine near miss still gets clap's own message, which is the better one
/// for it: `--corpu` is one edit from `--corpus` and passes.
fn misleading_suggestion(e: &clap::Error) -> Option<String> {
    if e.kind() != clap::error::ErrorKind::UnknownArgument {
        return None;
    }
    let typed = e.get(clap::error::ContextKind::InvalidArg)?.to_string();
    let suggested = e.get(clap::error::ContextKind::SuggestedArg)?.to_string();
    // Every suggestion clap made has to be a poor one before this takes over,
    // so a list holding one good match is still clap's to answer.
    let bare = |s: &str| s.trim_matches(|c: char| !c.is_alphanumeric()).to_string();
    let typed_bare = bare(&typed);
    let any_close = suggested
        .split(|c: char| c == ',' || c.is_whitespace())
        .map(bare)
        .filter(|s| !s.is_empty())
        .any(|s| near_enough(&typed_bare, &s));
    if any_close {
        return None;
    }
    Some(typed)
}

/// The same rule [`nearest_command`] applies: at most two edits, and no more
/// than half the longer word.
fn near_enough(typed: &str, candidate: &str) -> bool {
    let d = edit_distance(typed, candidate);
    d <= 2 && d * 2 <= candidate.len().max(typed.len())
}

/// Did somebody type `stegobench plan --corpus ...` and mean
/// `stegobench plan score --corpus ...`?
///
/// `Some(rest)` is the arguments they gave after `plan`, quoted so the
/// suggestion can be pasted back. Matched on the arguments rather than on
/// clap's message, for the same reason as [`looks_like_a_folder_of_photos`].
///
/// Only a leading flag counts. A leading word is either a verb `plan` can
/// read or one it refuses by name, and both of those are better answers than
/// this one.
fn plan_without_its_verb(e: &clap::Error) -> Option<String> {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    plan_verb_hint(e.kind(), &argv)
}

/// The decision [`plan_without_its_verb`] makes, with the argv handed in so a
/// test can drive it without a process of its own.
fn plan_verb_hint(kind: clap::error::ErrorKind, argv: &[String]) -> Option<String> {
    if kind != clap::error::ErrorKind::UnknownArgument {
        return None;
    }
    if argv.first().map(String::as_str) != Some("plan") {
        return None;
    }
    let rest = &argv[1..];
    if !rest.first().is_some_and(|a| a.starts_with('-')) {
        return None;
    }
    Some(
        rest.iter()
            .map(|w| shell_word(w))
            .collect::<Vec<_>>()
            .join(" "),
    )
}

/// Did somebody type `stegobench score <path>` and mean "check these"?
///
/// Matched on the arguments rather than on clap's message, because the
/// message is prose and this has to keep working when it is reworded.
fn looks_like_a_folder_of_photos(e: &clap::Error) -> bool {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let Some(first) = argv.first() else {
        return false;
    };
    if first != "score" && first != "plan" {
        return false;
    }
    // A bare word that is not a flag and not the value of one. Only the
    // missing-argument and unexpected-argument failures, so a genuine typo in
    // a flag still gets clap's own answer, which is the better one for it.
    let kind = e.kind();
    if kind != clap::error::ErrorKind::MissingRequiredArgument
        && kind != clap::error::ErrorKind::UnknownArgument
    {
        return false;
    }
    let mut expecting_value = false;
    for arg in argv.iter().skip(1) {
        if expecting_value {
            expecting_value = false;
            continue;
        }
        if arg.starts_with('-') {
            expecting_value = !arg.contains('=');
            continue;
        }
        return true;
    }
    false
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
        .filter(|(_, name)| near_enough(&typed, name))
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

/// Whether a succeeding command's human text is the thing the caller asked
/// for, rather than a note about producing it.
///
/// A listing is content. `stegobench list detectors | grep zsteg` found
/// nothing and `stegobench describe zsteg > notes.txt` wrote an empty file,
/// because every one of these wrote its whole answer to stderr while
/// `--help` promised stdout carried the content.
///
/// `doctor` is not here and reaches stdout by its own route: it prints the
/// same report whether it ends in 0 or 8, so the decision cannot be made from
/// the exit code the way it is for everything else.
fn human_is_content(command: &Command) -> bool {
    match command {
        // An estimate and a set of figures are both the answer somebody ran
        // the command to get, so `plan score ... > estimate.txt` and
        // `metrics < scores.json | tee` work. Only reached on exit 0, so a
        // refusal from either still goes to stderr.
        Command::List { .. }
        | Command::Describe { .. }
        | Command::Help { .. }
        | Command::Plan { .. }
        | Command::Metrics { .. }
        | Command::Completions { .. } => true,
        // Named rather than caught by a wildcard, so adding a command is a
        // decision about which stream it writes to instead of a default.
        Command::Schema { .. }
        | Command::Validate { .. }
        | Command::Verify { .. }
        | Command::Embed { .. }
        | Command::Doctor { .. }
        | Command::Score { .. }
        | Command::Fetch { .. }
        | Command::Report { .. }
        | Command::Check { .. }
        | Command::Scan { .. } => false,
    }
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
        } else if cli.command.as_ref().is_some_and(human_is_content) {
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

    /// Naming no destination is not the same as asking for the document to
    /// be thrown away.
    ///
    /// One detector and no `--out` used to resolve to "nowhere", and the
    /// caller reads that as "write to the file the caller named", so with no
    /// file named the document was silently discarded under exit 0.
    #[test]
    fn a_run_that_names_no_destination_still_keeps_its_document() {
        let tmp = tempfile::tempdir().expect("a temporary directory");
        let corpus = tmp.path().join("starter");
        let expected = tmp.path().join("starter.results");

        for many in [false, true] {
            assert_eq!(
                resolve_out(&corpus, None, many).expect("a destination"),
                Some(expected.clone()),
                "a run of {} detector(s) with no --out lost its document",
                if many { "several" } else { "one" }
            );
        }

        // And naming a file for one detector still means that file, which is
        // the case the old early return existed to serve.
        let file = tmp.path().join("one.json");
        assert_eq!(
            resolve_out(&corpus, Some(&file), false).expect("a file"),
            None
        );
    }

    #[test]
    fn a_half_corpus_run_keeps_its_own_records_file() {
        // The same collision the detector name already fixed, one dimension
        // along. `--split test` scores a subset in a different order, so a
        // records file written for the whole corpus does not line up with it.
        // Sharing the name makes the resume check refuse with "the corpus has
        // changed", which is true of nothing: the corpus is identical and only
        // the question was narrowed.
        let corpus = PathBuf::from("/data/pentimento-core");
        let whole = records_for(&corpus, None, &None, "stegcore", false, None);
        let test = records_for(
            &corpus,
            None,
            &None,
            "stegcore",
            false,
            Some(score::Side::Test),
        );
        let train = records_for(
            &corpus,
            None,
            &None,
            "stegcore",
            false,
            Some(score::Side::Train),
        );
        assert_ne!(whole, test);
        assert_ne!(test, train);
        assert!(
            test.to_string_lossy()
                .ends_with("stegcore.test.records.jsonl"),
            "{}",
            test.display()
        );
    }

    #[test]
    fn a_records_path_the_caller_named_is_used_whatever_the_split() {
        // An explicit path is an instruction, not a suggestion. Decorating it
        // with the split would write somewhere the caller did not ask for and
        // leave them looking at an empty file.
        let corpus = PathBuf::from("/data/corpus");
        let asked = PathBuf::from("/tmp/mine.jsonl");
        let got = records_for(
            &corpus,
            Some(&asked),
            &None,
            "stegcore",
            false,
            Some(score::Side::Test),
        );
        assert_eq!(got, asked);
    }

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

    /// Measured 2026-09-30, in a first-time walkthrough: `score --corpus
    /// pentimento-core` answered "there is no corpus at pentimento-core:
    /// nothing is there". True, and it says nothing about the thing the reader
    /// got wrong, which is that the word "corpus" is spoken in two
    /// vocabularies here and they typed the other one.
    #[test]
    fn a_corpus_value_is_tried_as_an_id_before_it_is_refused_as_a_path() {
        let reg = Registry::load(&shipped_registry()).expect("the real registry loads");

        // Exit 3 throughout, not 2: the command line is well formed and the
        // thing it names is not there, which is the line this project's exit
        // codes already divide on.
        let as_id = corpus_argument_problem(Some(&reg), Path::new("pentimento-core"))
            .expect("a registered id is not a directory");
        assert_eq!(as_id.code, exit::PREFLIGHT_REFUSED);
        assert!(
            as_id.human.contains("registered corpus id")
                && as_id.human.contains("stegobench fetch pentimento-core"),
            "a registered id must be recognised as one: {}",
            as_id.human
        );

        let neither = corpus_argument_problem(Some(&reg), Path::new("pentimento"))
            .expect("a value that is nothing is a problem");
        assert_eq!(neither.code, exit::PREFLIGHT_REFUSED);
        assert!(
            neither.human.contains("as a directory") && neither.human.contains("as a corpus id"),
            "the refusal must say both meanings were tried: {}",
            neither.human
        );
        // The sentence the exit-code contract test reads is still here: the
        // wording grew, it did not move.
        assert!(neither.human.contains("no corpus at"), "{}", neither.human);
        assert!(
            neither.human.contains("pentimento-core"),
            "a near miss must be named: {}",
            neither.human
        );

        // A registry that could not be read says so rather than reporting an
        // absence it never established.
        let blind = corpus_argument_problem(None, Path::new("pentimento-core"))
            .expect("still not a directory");
        assert_eq!(blind.code, exit::PREFLIGHT_REFUSED);
        assert!(
            blind.human.contains("registry could not be read"),
            "{}",
            blind.human
        );

        // An existing directory is what the flag wants, and a path that is
        // there and the wrong shape stays the pre-flight refusal it was.
        let tmp = tempfile::tempdir().expect("tmp");
        assert!(corpus_argument_problem(Some(&reg), tmp.path()).is_none());
        let file = tmp.path().join("shard.tar");
        std::fs::write(&file, b"bytes").expect("written");
        let wrong_shape =
            corpus_argument_problem(Some(&reg), &file).expect("a file is not a corpus");
        assert_eq!(wrong_shape.code, exit::PREFLIGHT_REFUSED);
        assert!(
            wrong_shape.human.contains("is a file"),
            "{}",
            wrong_shape.human
        );
    }

    /// The mirror image: a directory handed to `fetch`, which takes an id.
    ///
    /// PREFLIGHT_REFUSED, not USAGE, and the same code `--corpus` gives for a
    /// directory that is not there. The two divide on whether the command
    /// line was malformed or whether what it named is absent, and an
    /// unregistered id is the second. These call sites disagreed with the
    /// path case until 2026-09-30 for no reason anybody had chosen.
    #[test]
    fn an_unregistered_corpus_id_names_the_close_ones_and_spots_a_directory() {
        let reg = Registry::load(&shipped_registry()).expect("the real registry loads");

        let typo =
            unregistered_corpus_id(&reg, "bossbas", "`stegobench list corpora` shows what is.");
        assert_eq!(typo.code, exit::PREFLIGHT_REFUSED);
        assert!(
            typo.human.contains("Did you mean") && typo.human.contains("bossbase"),
            "{}",
            typo.human
        );

        let tmp = tempfile::tempdir().expect("tmp");
        let as_path = unregistered_corpus_id(
            &reg,
            &tmp.path().display().to_string(),
            "`stegobench list corpora` shows what is.",
        );
        assert_eq!(as_path.code, exit::PREFLIGHT_REFUSED);
        assert!(
            as_path.human.contains("an id names a dataset")
                && as_path.human.contains("score --corpus"),
            "a directory handed to an id flag must be recognised: {}",
            as_path.human
        );
    }

    /// A result document or a records file written inside the corpus JOINS the
    /// corpus: the next run reads a different set of bytes, computes a
    /// different digest, and reports that the corpus changed when nothing
    /// about the images did. `--out ./mycorpus/result.json` is a natural thing
    /// to type, so it is refused at the boundary.
    #[test]
    fn an_output_path_inside_the_corpus_is_refused_and_one_outside_it_is_not() {
        let tmp = tempfile::tempdir().expect("tmp");
        let corpus = tmp.path().join("mycorpus");
        std::fs::create_dir_all(&corpus).expect("made");

        let inside = output_inside_corpus(&corpus, "--out", Some(&corpus.join("result.json")))
            .expect("a file in the corpus is refused");
        assert!(
            inside.contains("inside the corpus") && inside.contains("digest"),
            "the refusal must say what would have happened: {inside}"
        );
        assert!(
            inside.contains("Beside it works"),
            "the refusal must say where to put it instead: {inside}"
        );

        // The corpus directory itself, which is what `--out <dir>` means when
        // several detectors are named.
        let itself =
            output_inside_corpus(&corpus, "--out", Some(&corpus)).expect("the corpus itself");
        assert!(itself.contains("itself"), "{itself}");

        // A path that walks out and back in again resolves to the same place.
        let round_trip = corpus.join("..").join("mycorpus").join("result.json");
        assert!(
            output_inside_corpus(&corpus, "--out", Some(&round_trip)).is_some(),
            "a path that leaves the corpus and returns was not recognised"
        );

        // Directories below the corpus that nothing has created yet are still
        // inside it, and a missing parent must not panic on the way to saying
        // so.
        assert!(
            output_inside_corpus(
                &corpus,
                "--records",
                Some(&corpus.join("not-made-yet").join("deeper").join("r.jsonl"))
            )
            .is_some(),
            "an unwritten directory inside the corpus was not recognised"
        );

        // Outside is allowed, whether or not it exists yet.
        assert!(
            output_inside_corpus(&corpus, "--out", Some(&tmp.path().join("result.json"))).is_none()
        );
        assert!(output_inside_corpus(
            &corpus,
            "--out",
            Some(&tmp.path().join("nowhere").join("yet").join("result.json"))
        )
        .is_none());
        // A sibling whose name merely starts with the corpus's is a different
        // directory, and a textual prefix test would have called it inside.
        assert!(output_inside_corpus(
            &corpus,
            "--out",
            Some(&tmp.path().join("mycorpus-2/r.json"))
        )
        .is_none());
        assert!(output_inside_corpus(&corpus, "--out", None).is_none());
    }

    /// A corpus reached through a symbolic link and an `--out` reached through
    /// the real directory are the same place, and a comparison of the strings
    /// would not have said so.
    #[cfg(unix)]
    #[test]
    fn a_symbolic_link_does_not_hide_an_output_path_inside_the_corpus() {
        let tmp = tempfile::tempdir().expect("tmp");
        let real = tmp.path().join("real-corpus");
        std::fs::create_dir_all(&real).expect("made");
        let link = tmp.path().join("linked");
        std::os::unix::fs::symlink(&real, &link).expect("linked");

        assert!(
            output_inside_corpus(&link, "--out", Some(&real.join("result.json"))).is_some(),
            "the link hid an output path inside the corpus"
        );
        assert!(
            output_inside_corpus(&real, "--out", Some(&link.join("result.json"))).is_some(),
            "the link hid an output path inside the corpus"
        );
    }

    /// The two refusals above, reaching a user through the commands that can
    /// raise them, and raised before anything is scored rather than after.
    #[test]
    fn score_and_plan_both_refuse_before_they_touch_the_corpus() {
        let dir = tempfile::tempdir().expect("tmp");
        let reg = dir.path().join("registry");
        std::fs::create_dir_all(&reg).expect("made");
        std::fs::write(
            reg.join("ghost.toml"),
            "name = \"ghost\"\nkind = \"detector\"\nlicence = \"MIT\"\n\
             [binary]\ncommand = [\"definitely-not-installed-xyzzy\"]\n\
             version_args = [\"--version\"]\n\
             [selftest]\nmust_detect = \"a.png\"\nmust_clear = \"b.png\"\n",
        )
        .expect("written");
        let corpus = dir.path().join("mycorpus");
        std::fs::create_dir_all(&corpus).expect("made");
        let inside = corpus.join("result.json");

        let scored = cmd_score(
            &resolved_at(&reg),
            ScoreRequest {
                corpus: &corpus,
                detectors: &["ghost".to_string()],
                corpus_id: None,
                trained_on: None,
                split: None,
                records: None,
                out: Some(&inside),
                timeout: 5,
                jobs: 1,
                keep_raw: false,
                limit: None,
            },
        );
        assert_eq!(scored.code, exit::USAGE, "{}", scored.human);
        assert!(
            scored.human.contains("inside the corpus"),
            "{}",
            scored.human
        );
        assert!(
            !inside.exists(),
            "the refusal wrote into the corpus it was refusing to write into"
        );

        // `plan` is the command whose whole job is to answer before the run,
        // so a plan that approves a run `score` would refuse is the one
        // answer it must never give.
        let planned = cmd_plan(
            &resolved_at(&reg),
            &[
                "score".to_string(),
                "--corpus".to_string(),
                corpus.display().to_string(),
                "--detector".to_string(),
                "ghost".to_string(),
                "--records".to_string(),
                corpus.join("r.jsonl").display().to_string(),
            ],
        );
        assert_eq!(planned.code, exit::USAGE, "{}", planned.human);
        assert!(
            planned.human.contains("inside the corpus"),
            "{}",
            planned.human
        );
    }

    /// `score --corpus <a registered id>` is the mistake the two vocabularies
    /// invite, and it reaches the user through the command rather than only
    /// through the helper.
    #[test]
    fn score_answers_a_registered_id_in_the_path_flag_with_the_way_out() {
        let refused = cmd_score(
            &resolved_at(shipped_registry()),
            ScoreRequest {
                corpus: Path::new("pentimento-core"),
                detectors: &["zsteg".to_string()],
                corpus_id: None,
                trained_on: None,
                split: None,
                records: None,
                out: None,
                timeout: 5,
                jobs: 1,
                keep_raw: false,
                limit: None,
            },
        );
        assert_eq!(refused.code, exit::PREFLIGHT_REFUSED, "{}", refused.human);
        assert!(
            refused.human.contains("stegobench fetch pentimento-core"),
            "{}",
            refused.human
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

        let described = cmd_describe(&resolved_at(&dir), "reveal", false);
        assert_eq!(described.code, exit::OK);
        assert_eq!(described.json["licence"]["redistribution"], "permitted");
        // The summary carries the date the terms were read, in the summary's
        // own words rather than as the field name the TOML dump used to show.
        assert!(
            described.human.contains("verified_on"),
            "the summary drops the verification date: {}",
            described.human
        );
        assert!(
            described.human.contains("Republish"),
            "the summary drops the redistribution answer: {}",
            described.human
        );

        // And a tool still answers the same verb.
        assert_eq!(
            cmd_describe(&resolved_at(&dir), "steghide", false).code,
            exit::OK
        );
        // An unknown name is a usage error that names both kinds.
        let missing = cmd_describe(&resolved_at(&dir), "not-registered", false);
        assert_eq!(missing.code, exit::USAGE);
        assert!(missing.human.contains("reveal") && missing.human.contains("steghide"));
    }

    /// `describe` used to print the whole registry entry as TOML under its
    /// summary, which is forty lines of fields the harness reads and the
    /// reader does not, between them and the next command they have to type.
    ///
    /// The entry is still one flag away, and under that flag it is ALONE:
    /// anything else on the stream makes the output unpipeable, which is the
    /// only reason a raw form exists.
    #[test]
    fn describe_summarises_by_default_and_prints_the_entry_only_under_toml() {
        let dir = shipped_registry();
        for name in ["steghide", "reveal"] {
            let summary = cmd_describe(&resolved_at(&dir), name, false);
            assert_eq!(summary.code, exit::OK);
            assert!(
                !summary.human.contains("[licence]") && !summary.human.contains("[properties]"),
                "{name}'s summary still carries the raw TOML: {}",
                summary.human
            );
            assert!(
                summary.human.contains("--toml"),
                "{name}'s summary does not say where the whole entry went: {}",
                summary.human
            );

            let raw = cmd_describe(&resolved_at(&dir), name, true);
            assert_eq!(raw.code, exit::OK);
            assert!(
                raw.payload_on_stdout,
                "{name} --toml was diverted to stderr, so it cannot be piped"
            );
            let parsed: toml::Value = toml::from_str(&raw.human).unwrap_or_else(|e| {
                panic!("{name} --toml is not parseable TOML: {e}\n{}", raw.human)
            });
            assert!(
                parsed.get("name").is_some() || parsed.get("id").is_some(),
                "{name} --toml is missing the entry itself: {}",
                raw.human
            );
            assert!(
                !raw.human.contains("Needs from you") && !raw.human.contains("Nothing needed"),
                "{name} --toml carries the summary too: {}",
                raw.human
            );
        }

        // `stegobench help scope` promises that `describe` says where a tool
        // lives, what it costs and the command that runs it. That promise used
        // to be kept by the TOML dump, so the summary has to keep it now.
        let zsteg = cmd_describe(&resolved_at(&dir), "zsteg", false);
        assert_eq!(zsteg.code, exit::OK);
        // The arguments, not the program name. zsteg's image already has
        // `zsteg` as its entrypoint, so naming it again in the registry ran
        // it twice; asserting on the doubled form would hold that bug in
        // place.
        for expected in ["Runs as", "Costs", "Run it", "-a /work/{file}"] {
            assert!(
                zsteg.human.contains(expected),
                "the summary drops {expected:?}: {}",
                zsteg.human
            );
        }
    }

    /// The commonest way to reach `describe`'s refusal with something that is
    /// not a typo is to hand it the path that `score --corpus` wants. A list
    /// of registered names that does not contain theirs does not tell them so.
    #[test]
    fn describe_says_it_takes_a_name_when_it_is_handed_a_path() {
        let dir = shipped_registry();
        let tmp = tempfile::tempdir().expect("tmp");
        let refused = cmd_describe(&resolved_at(&dir), &tmp.path().display().to_string(), false);
        assert_eq!(refused.code, exit::USAGE);
        assert!(
            refused.human.contains("is a path on this machine")
                && refused.human.contains("score --corpus"),
            "{}",
            refused.human
        );

        // A near miss is still a near miss, and gets the suggestion instead.
        let typo = cmd_describe(&resolved_at(&dir), "steghid", false);
        assert_eq!(typo.code, exit::USAGE);
        assert!(
            typo.human.contains("Did you mean") && typo.human.contains("steghide"),
            "{}",
            typo.human
        );
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

    /// EVERY word on the list, not just the two that are hidden subcommands.
    ///
    /// `detect` is the likeliest first guess of all, and it fell through to
    /// the generic "there is no `stegobench detect`" while the tailored
    /// answer sat one match arm away. The whole list is walked here so that
    /// adding a word and forgetting to wire it up fails rather than ships.
    #[test]
    fn every_wrong_direction_word_reaches_the_tailored_answer() {
        assert!(
            WRONG_DIRECTION.windows(2).all(|w| w[0] < w[1]),
            "the list is meant to stay sorted so a reader can find a word in it"
        );
        for word in WRONG_DIRECTION {
            assert!(
                wrong_direction(word),
                "`{word}` is on the list and unmatched"
            );
            assert!(
                wrong_direction(&word.to_uppercase()),
                "`{word}` typed in capitals is the same misunderstanding"
            );
            let out = cmd_wrong_direction(word);
            assert_eq!(out.code, exit::USAGE, "`{word}` should be a usage error");
            for expected in ["measures DETECTORS", "help scope", "list detectors"] {
                assert!(
                    out.human.contains(expected),
                    "`{word}` does not say {expected:?}: {}",
                    out.human
                );
            }
            // Either clap parses it (the two hidden signposts) and `run`
            // routes it here, or clap rejects it as an unknown subcommand and
            // `parse_or_explain` catches it on the word. Anything else means
            // the word never reaches this answer at runtime.
            match Cli::try_parse_from(["stegobench", word, "./photo.png"]) {
                Ok(parsed) => assert!(
                    run(&parsed).human.contains("measures DETECTORS"),
                    "`{word}` parses but is not routed to the answer"
                ),
                Err(e) => assert_eq!(
                    e.kind(),
                    clap::error::ErrorKind::InvalidSubcommand,
                    "`{word}` fails in a way `parse_or_explain` does not catch"
                ),
            }
        }
    }

    /// The list must not swallow a real command or a plain typo.
    #[test]
    fn a_real_command_is_never_read_as_the_wrong_direction() {
        for word in [
            "list",
            "score",
            "help",
            "doctor",
            "verify",
            "validate",
            "report",
            "frobnicate",
            "",
        ] {
            assert!(!wrong_direction(word), "`{word}` was diverted");
        }
    }

    #[test]
    fn a_shell_word_is_quoted_only_when_the_shell_would_take_it_apart() {
        assert_eq!(shell_word("zsteg"), "zsteg");
        assert_eq!(shell_word("/work/{file}"), "/work/{file}");
        assert_eq!(
            shell_word("PYTHONPATH=/opt/aletheia"),
            "PYTHONPATH=/opt/aletheia"
        );
        assert_eq!(shell_word("/my pictures/a.png"), "'/my pictures/a.png'");
        assert_eq!(shell_word("it's.png"), r"'it'\''s.png'");
        assert_eq!(shell_word(""), "''");
    }

    fn parsed_entry(text: &str) -> stegobench_core::registry::Entry {
        toml::from_str(text).expect("the entry under test parses")
    }

    /// A containerised tool has no command anywhere a reader can paste, which
    /// is what sent a first-time user looking for `zsteg` on their PATH and
    /// then hand-writing a `docker run` out of `--toml`.
    #[test]
    fn a_containerised_tool_pastes_as_a_pinned_sandboxed_docker_run() {
        let e = parsed_entry(
            "name = \"zsteg\"\nkind = \"detector\"\nlicence = \"MIT\"\n\
             [image]\nreference = \"stegobench/zsteg@sha256:abc\"\n\
             [emits]\noutput = \"verdict\"\n\
             [invoke]\nargv = [\"zsteg\", \"-a\", \"{file}\"]\nparser = \"zsteg\"\n",
        );
        let line = runnable_command(&e, &[]).expect("a container has a command");
        // The digest, because an unpinned paste measures whatever the tag
        // points at today.
        assert!(line.contains("stegobench/zsteg@sha256:abc"), "{line}");
        // The same sandbox the harness uses, not a friendlier one.
        for flag in [
            "--rm",
            "--network=none",
            "--cap-drop=ALL",
            "--security-opt no-new-privileges",
            "--read-only",
            "--memory=2g",
        ] {
            assert!(line.contains(flag), "{flag} is missing from {line}");
        }
        assert!(
            line.contains("-v \"$PWD/{file}\":/work/{file}:ro"),
            "{line}"
        );
        assert!(line.ends_with("zsteg -a /work/{file}"), "{line}");
        assert!(
            !line.contains("--user"),
            "a read-only run needs no uid: {line}"
        );
    }

    /// StegExpose scans a directory and writes beside its input, so the
    /// harness gives it the whole directory and a uid. A paste that mounted
    /// one read-only file would print nothing and exit zero.
    #[test]
    fn a_tool_that_writes_beside_its_input_pastes_with_the_directory_mounted() {
        let e = parsed_entry(
            "name = \"stegexpose\"\nkind = \"detector\"\nlicence = \"GPL-3.0\"\n\
             [image]\nreference = \"stegobench/stegexpose@sha256:def\"\n\
             [emits]\noutput = \"score\"\nhigher_means_stego = true\n\
             [invoke]\nargv = [\"/work\", \"default\", \"0.2\"]\n\
             writable_workdir = true\nparser = \"stegexpose\"\n",
        );
        let line = runnable_command(&e, &[]).expect("a container has a command");
        assert!(line.contains("-v \"$PWD\":/work"), "{line}");
        assert!(line.contains("--user $(id -u):$(id -g)"), "{line}");
        assert!(
            !line.contains("{file}"),
            "nothing to substitute here: {line}"
        );
    }

    /// The old line printed `invoke.argv` alone, which for a binary entry is
    /// the arguments without the program: `{file}`, by itself.
    #[test]
    fn a_binary_tool_pastes_with_the_program_name_it_needs() {
        let e = parsed_entry(
            "name = \"stegcore\"\nkind = \"detector\"\nlicence = \"AGPL-3.0-or-later\"\n\
             [binary]\ncommand = [\"stegcore\", \"analyse\", \"--json\"]\n\
             [emits]\noutput = \"score\"\nhigher_means_stego = true\n\
             [invoke]\nargv = [\"{file}\"]\nparser = \"stegcore\"\n",
        );
        assert_eq!(
            runnable_command(&e, &[]).as_deref(),
            Some("stegcore analyse --json {file}")
        );
    }

    /// An adapter that is not where the registry says it is would paste a
    /// mount of a path that does not exist, which fails at the runtime rather
    /// than where the reader can see why.
    #[test]
    fn an_entry_with_nothing_runnable_pastes_no_command_at_all() {
        let missing_adapter = parsed_entry(
            "name = \"aletheia-rs\"\nkind = \"detector\"\nlicence = \"MIT\"\n\
             [image]\nreference = \"stegobench/aletheia@sha256:abc\"\n\
             [emits]\noutput = \"score\"\nhigher_means_stego = true\n\
             [invoke]\nadapter = \"nowhere/at/all.py\"\nentrypoint = \"python3\"\n\
             argv = [\"{adapter}\", \"{file}\"]\nparser = \"number\"\n",
        );
        assert_eq!(runnable_command(&missing_adapter, &[]), None);

        let nothing_declared = parsed_entry(
            "name = \"aletheia-rich\"\nkind = \"detector\"\nlicence = \"MIT\"\n\
             [image]\nreference = \"stegobench/aletheia-rich@sha256:abc\"\n\
             [emits]\noutput = \"score\"\nhigher_means_stego = true\n",
        );
        assert_eq!(runnable_command(&nothing_declared, &[]), None);
    }

    /// An adapter that IS there is mounted read only and named by the path it
    /// has inside the container, not the one it has here.
    #[test]
    fn an_adapter_is_mounted_and_then_referred_to_by_its_inside_path() {
        let tmp = tempfile::tempdir().unwrap();
        let adapters = tmp.path().join("plugins/adapters");
        std::fs::create_dir_all(&adapters).unwrap();
        std::fs::write(adapters.join("one.py"), "print(0)\n").unwrap();
        let e = parsed_entry(
            "name = \"aletheia-rs\"\nkind = \"detector\"\nlicence = \"MIT\"\n\
             [image]\nreference = \"stegobench/aletheia@sha256:abc\"\n\
             [emits]\noutput = \"score\"\nhigher_means_stego = true\n\
             [invoke]\nadapter = \"plugins/adapters/one.py\"\nentrypoint = \"python3\"\n\
             env = [\"PYTHONPATH=/opt/aletheia\"]\n\
             argv = [\"{adapter}\", \"{file}\", \"rs\"]\nparser = \"number\"\n",
        );
        let line = runnable_command(&e, &[tmp.path().to_path_buf()]).expect("resolvable");
        assert!(line.contains(":/adapter/one.py:ro"), "{line}");
        assert!(line.contains("-e PYTHONPATH=/opt/aletheia"), "{line}");
        assert!(line.contains("--entrypoint python3"), "{line}");
        assert!(line.ends_with("/adapter/one.py /work/{file} rs"), "{line}");
        assert!(
            !line.contains("{adapter}"),
            "the placeholder survived: {line}"
        );
    }

    /// The built-in registry unpacks its adapters into a scratch directory
    /// that is gone when the command ends, so `describe` must not print a
    /// command naming a path that will not be there when it is pasted.
    #[test]
    fn a_command_is_never_printed_against_an_adapter_that_will_not_survive() {
        let built_in = registry::Resolved::built_in().expect("the built-in registry loads");
        let out = cmd_describe(&built_in, "aletheia-rs", false);
        assert_eq!(out.code, exit::OK, "{}", out.human);
        let temp = std::env::temp_dir().display().to_string();
        assert!(
            !out.human.contains(&temp),
            "a scratch path was offered to be pasted: {}",
            out.human
        );
        assert!(
            out.human.contains("no command to paste"),
            "the reader is left with nothing and no reason: {}",
            out.human
        );
        assert!(
            out.human.contains("--registry plugins/registry"),
            "nothing says how to get the command: {}",
            out.human
        );
        // The same tool off a registry on disk does print one, so this is a
        // property of where the adapter lives rather than of the entry.
        let on_disk = cmd_describe(&resolved_at(shipped_registry()), "aletheia-rs", false);
        assert!(
            on_disk.human.contains("docker run --rm"),
            "a registry with its adapters beside it still cannot paste: {}",
            on_disk.human
        );
    }

    /// A service is reached by an adapter running HERE, so the image is the
    /// subject rather than the thing to start.
    #[test]
    fn a_service_pastes_the_adapter_that_runs_on_this_machine() {
        let tmp = tempfile::tempdir().unwrap();
        let adapters = tmp.path().join("plugins/adapters");
        std::fs::create_dir_all(&adapters).unwrap();
        std::fs::write(adapters.join("svc.py"), "print(0)\n").unwrap();
        let e = parsed_entry(
            "name = \"stegashield\"\nkind = \"detector\"\nlicence = \"proprietary\"\n\
             [image]\nreference = \"5iprojects/stegashield@sha256:abc\"\n\
             [emits]\noutput = \"score\"\nhigher_means_stego = true\n\
             [invoke]\nhost = true\nadapter = \"plugins/adapters/svc.py\"\n\
             entrypoint = \"python3\"\nargv = [\"{adapter}\", \"{file}\"]\nparser = \"number\"\n",
        );
        let line = runnable_command(&e, &[tmp.path().to_path_buf()]).expect("resolvable");
        assert!(line.starts_with("python3 "), "{line}");
        assert!(
            !line.contains("docker"),
            "a service is not started here: {line}"
        );
        assert!(line.ends_with("{file}"), "{line}");
    }

    /// Both status words were true about the harness and read as claims about
    /// the reader: `[ready]` sent one looking for a binary that only exists
    /// inside an image, and `[unknown]` on the corpus that ships in the binary
    /// read as a fault beside it.
    #[test]
    fn the_status_word_says_who_it_is_about() {
        let ready = needs::Needs {
            readiness: needs::Readiness::Ready,
            steps: Vec::new(),
        };
        assert_eq!(
            readiness_word(&ready, "tool", "zsteg"),
            "stegobench can run it"
        );

        let unknown = needs::Needs {
            readiness: needs::Readiness::Unknown,
            steps: Vec::new(),
        };
        assert_eq!(
            readiness_word(&unknown, "corpus", STARTER_ID),
            "included in this binary"
        );
        assert_eq!(
            readiness_word(&unknown, "corpus", "bossbase"),
            "bring your own copy"
        );
        // A tool whose readiness genuinely cannot be established keeps the
        // word that says so.
        assert_eq!(readiness_word(&unknown, "tool", "stegashield"), "unknown");

        let needs_you = needs::Needs {
            readiness: needs::Readiness::NeedsYou,
            steps: Vec::new(),
        };
        assert_eq!(readiness_word(&needs_you, "tool", "stegcore"), "NEEDS YOU");
    }

    /// Every whole number in a line, in the order it is printed.
    fn numbers_in(line: &str) -> Vec<u64> {
        line.split(|c: char| !c.is_ascii_digit())
            .filter(|s| !s.is_empty())
            .filter_map(|s| s.parse().ok())
            .collect()
    }

    /// The summary used to be one comma list over two independent questions:
    /// "13 tool(s): 6 verified, 0 answering, 0 broken, 2 not installed, 1
    /// undetermined, 7 not checked", which sums to 16 against the 13 in front
    /// of it. A reader adds a comma list up against the total beside it, so
    /// every list that can be summed has to sum.
    #[test]
    fn every_count_doctor_prints_adds_up_to_the_total_beside_it() {
        let out = cmd_doctor(&resolved_at(shipped_registry()), None, false);
        let total = out.json["checked"].as_u64().expect("a total");
        assert!(total > 1, "a registry of one proves nothing here");

        let mut summed = 0;
        for (head, from_second) in [
            ("tool(s):", true),
            ("On this machine:", false),
            ("Stegobench's own self-test:", false),
        ] {
            let line = out
                .human
                .lines()
                .find(|l| l.contains(head))
                .unwrap_or_else(|| panic!("no line for {head:?}: {}", out.human));
            let found = numbers_in(line);
            let (stated, parts) = if from_second {
                (found[0], &found[1..])
            } else {
                (total, &found[..])
            };
            assert_eq!(stated, total, "{head:?} states a different total: {line}");
            assert_eq!(
                parts.iter().sum::<u64>(),
                total,
                "{head:?} does not add up to {total}: {line}"
            );
            summed += 1;
        }
        assert_eq!(summed, 3, "a line went unchecked");
    }

    /// `doctor` listed thirteen tools, `list detectors` listed seven, and the
    /// word "embedder" appeared on neither screen. A reader with no way to
    /// tell which six were the difference reads it as something broken.
    #[test]
    fn doctor_and_the_listing_both_say_which_kind_each_tool_is() {
        let dir = shipped_registry();
        let doctor = cmd_doctor(&resolved_at(&dir), None, false);
        let listing = cmd_list(&resolved_at(&dir), "all");
        let detectors = doctor.json["detectors"].as_u64().expect("a count");
        let embedders = doctor.json["embedders"].as_u64().expect("a count");
        assert!(detectors > 0 && embedders > 0, "one kind is missing here");
        for screen in [&doctor.human, &listing.human] {
            for heading in [
                format!("DETECTORS ({detectors})"),
                format!("EMBEDDERS ({embedders})"),
            ] {
                assert!(screen.contains(&heading), "no {heading:?} in:\n{screen}");
            }
        }
        // The count under each heading is the count `list <kind>` gives, so
        // the two screens cannot drift apart.
        for (kind, count) in [("detectors", detectors), ("embedders", embedders)] {
            let one = cmd_list(&resolved_at(&dir), kind);
            assert_eq!(
                one.json["tools"].as_array().map(Vec::len),
                Some(count as usize),
                "`list {kind}` and the grouped screens disagree"
            );
        }
    }

    /// A bare `stegobench list` printed thirteen tool rows, a footer, then
    /// five corpus rows, with nothing saying which was which.
    #[test]
    fn a_bare_listing_says_where_the_tools_end_and_the_corpora_begin() {
        let dir = shipped_registry();
        let out = cmd_list(&resolved_at(&dir), "all");
        let corpora = out.json["corpora"].as_array().expect("corpora").len();
        assert!(
            out.human.contains(&format!("CORPORA ({corpora})")),
            "{}",
            out.human
        );
        let heading = out.human.find("CORPORA (").expect("the corpora heading");
        let tools = out.human.find("DETECTORS (").expect("the tools heading");
        assert!(tools < heading, "the corpora are listed before the tools");
    }

    /// The pasteable command is long and carries a note under it. A second
    /// line that started in column zero would read as the next field.
    #[test]
    fn a_fact_of_several_lines_keeps_the_column() {
        let block = describe_block(
            &needs::Needs {
                readiness: needs::Readiness::Ready,
                steps: Vec::new(),
            },
            "thing",
            "tool",
            &[
                ("Short", "one".to_string()),
                ("Much longer", "first\nsecond".to_string()),
            ],
        );
        let lines: Vec<&str> = block.lines().collect();
        let continuation = lines
            .iter()
            .find(|l| l.trim() == "second")
            .expect("the second line survived");
        let head = lines
            .iter()
            .find(|l| l.starts_with("Much longer"))
            .expect("the labelled line");
        assert_eq!(
            continuation.find("second"),
            head.find("first"),
            "the continuation does not line up: {continuation:?} against {head:?}"
        );
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
                jobs: 1,
                keep_raw: false,
                limit: None,
                split: None,
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

        let doctor = cmd_doctor(&resolved, None, false);
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

    #[test]
    fn the_starter_corpus_is_carried_in_the_binary_and_can_be_written_out() {
        // The first run for anybody who installed a binary. `describe` used
        // to tell them to look in a checkout they do not have.
        assert!(
            !EMBEDDED_STARTER.is_empty(),
            "no starter corpus was compiled in, so a first run needs a checkout"
        );
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("out");
        let n = write_embedded_starter(&dest).expect("written");
        assert_eq!(n, EMBEDDED_STARTER.len());

        // It has to be a corpus, not just files: every image needs the record
        // beside it or `score` will refuse what we just produced.
        let digest = score::corpus_digest(&dest)
            .expect("readable")
            .expect("every record states a digest");
        assert!(digest.starts_with("sha256:"), "got {digest}");
    }

    #[test]
    fn writing_the_starter_corpus_over_something_is_refused() {
        // Merging into a directory that already holds a corpus leaves records
        // describing images from two copies, and nothing downstream can tell.
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("out");
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(dest.join("something.txt"), b"mine").unwrap();
        let err = write_embedded_starter(&dest).expect_err("should have refused");
        assert!(err.contains("already has something in it"), "got {err}");
    }

    /// A corpus whose records state the digest their image actually has.
    ///
    /// `corpus_named` fabricates digests, which is what the record-level
    /// comparison needs and is exactly what the byte check exists to catch,
    /// so a test of that check cannot be built on it.
    fn corpus_honest(root: &Path, bodies: &[&[u8]]) {
        use sha2::{Digest, Sha256};
        std::fs::create_dir_all(root).unwrap();
        for (i, body) in bodies.iter().enumerate() {
            let role = if i == 0 { "clean" } else { "stego" };
            std::fs::write(root.join(format!("i{i}.png")), body).unwrap();
            let d = format!("{:x}", Sha256::digest(body));
            std::fs::write(
                root.join(format!("i{i}.json")),
                format!(r#"{{"role":"{role}","sha256":"{d}"}}"#),
            )
            .unwrap();
        }
    }

    #[test]
    fn a_document_edited_after_it_was_written_is_refused() {
        // The whole reason `content_digest` exists. Every field except the
        // two recording WHEN the run happened is covered, so a changed
        // number cannot survive the check, and a run edited to flatter a
        // detector stops being something a reader has to spot by eye.
        let dir = tempfile::tempdir().unwrap();
        let corpus = dir.path().join("corpus");
        corpus_honest(&corpus, &[b"cover bytes", b"stego bytes"]);
        let digest = score::corpus_digest(&corpus).unwrap().unwrap();
        let doc = dir.path().join("r.json");
        result_claiming(&doc, &digest);

        let mut sealed: Result1 =
            serde_json::from_str(&std::fs::read_to_string(&doc).unwrap()).unwrap();
        sealed.seal();
        // Improve the number after sealing, which is what a tamper looks like.
        sealed.metrics.auc = 0.99;
        std::fs::write(&doc, serde_json::to_string(&sealed).unwrap()).unwrap();

        let out = cmd_verify(&doc, &corpus, false);
        assert_eq!(out.code, exit::VERIFY_MISMATCH, "{}", out.human);
        assert!(
            out.human.contains("does not match its own content digest"),
            "{}",
            out.human
        );
    }

    #[test]
    fn a_document_whose_digest_matches_passes_the_check_it_used_to_skip() {
        let dir = tempfile::tempdir().unwrap();
        let corpus = dir.path().join("corpus");
        corpus_honest(&corpus, &[b"cover bytes", b"stego bytes"]);
        let digest = score::corpus_digest(&corpus).unwrap().unwrap();
        let doc = dir.path().join("r.json");
        result_claiming(&doc, &digest);

        let mut sealed: Result1 =
            serde_json::from_str(&std::fs::read_to_string(&doc).unwrap()).unwrap();
        sealed.seal();
        std::fs::write(&doc, serde_json::to_string(&sealed).unwrap()).unwrap();

        let out = cmd_verify(&doc, &corpus, false);
        assert_eq!(out.code, exit::OK, "{}", out.human);
    }

    #[test]
    fn a_document_carrying_no_content_digest_is_not_accused_of_a_mismatch() {
        // Absent means nobody offered one. A submitted or hand-written
        // document is allowed to say nothing here, and refusing it would
        // make the field mandatory by the back door.
        let dir = tempfile::tempdir().unwrap();
        let corpus = dir.path().join("corpus");
        corpus_honest(&corpus, &[b"cover bytes", b"stego bytes"]);
        let digest = score::corpus_digest(&corpus).unwrap().unwrap();
        let doc = dir.path().join("r.json");
        result_claiming(&doc, &digest);
        assert!(!std::fs::read_to_string(&doc)
            .unwrap()
            .contains("content_digest"));

        let out = cmd_verify(&doc, &corpus, false);
        assert_eq!(out.code, exit::OK, "{}", out.human);
    }

    #[test]
    fn the_per_arm_block_is_absent_for_a_corpus_of_one_arm() {
        assert!(per_arm_lines(&[]).is_empty());
    }

    #[test]
    fn the_per_arm_block_names_every_arm_and_omits_an_interval_it_does_not_have() {
        use stegobench_core::result::ArmMetrics;
        let arms = vec![
            ArmMetrics {
                arm: "lsb-0100".into(),
                auc: 0.5,
                auc_ci95: Some([0.1135, 0.8865]),
                n_clean: 6,
                n_stego: 6,
            },
            ArmMetrics {
                arm: "a-much-longer-arm".into(),
                auc: 0.6944,
                auc_ci95: None,
                n_clean: 6,
                n_stego: 6,
            },
        ];
        let text = per_arm_lines(&arms).join("\n");
        assert!(text.contains("lsb-0100"), "{text}");
        assert!(text.contains("a-much-longer-arm"), "{text}");
        assert!(text.contains("0.5000"), "{text}");
        assert!(text.contains("[0.1135, 0.8865]"), "{text}");
        assert!(
            !text.contains("0.6944 ["),
            "an interval was fabricated for the arm that had none: {text}"
        );
        assert!(
            text.contains("same 6 clean image(s)"),
            "the shared clean set should be stated once: {text}"
        );
    }

    #[test]
    fn verify_reads_the_images_and_says_so_when_they_match() {
        let dir = tempfile::tempdir().unwrap();
        let corpus = dir.path().join("corpus");
        corpus_honest(&corpus, &[b"cover bytes", b"stego bytes"]);
        let digest = score::corpus_digest(&corpus)
            .expect("readable")
            .expect("named");
        let doc = dir.path().join("r.json");
        result_claiming(&doc, &digest);

        let out = cmd_verify(&doc, &corpus, false);
        assert_eq!(out.code, exit::OK, "{}", out.human);
        assert_eq!(out.json["checked"], serde_json::json!("bytes"));
        assert!(
            out.human.contains("hash to what their records state"),
            "the answer does not say the bytes were read: {}",
            out.human
        );
    }

    #[test]
    fn verify_catches_an_image_swapped_under_a_record_that_still_agrees() {
        // The attack the byte check exists for, and the one the corpus digest
        // cannot see: replace a stego image with an easier one, leave its
        // record alone, and every digest in the corpus still agrees with
        // every other. Scored again it gives a better number, and before this
        // check `verify` called that number attributable to this corpus.
        let dir = tempfile::tempdir().unwrap();
        let corpus = dir.path().join("corpus");
        corpus_honest(&corpus, &[b"cover bytes", b"stego bytes"]);
        let digest = score::corpus_digest(&corpus)
            .expect("readable")
            .expect("named");
        let doc = dir.path().join("r.json");
        result_claiming(&doc, &digest);

        std::fs::write(corpus.join("i1.png"), b"a much easier image").unwrap();

        // The records are untouched, so the corpus still names itself the
        // same way. That is the point.
        let after = score::corpus_digest(&corpus)
            .expect("readable")
            .expect("named");
        assert_eq!(after, digest, "the swap changed the record level digest");

        let out = cmd_verify(&doc, &corpus, false);
        assert_eq!(out.code, exit::VERIFY_MISMATCH, "{}", out.human);
        assert_eq!(out.json["ok"], serde_json::json!(false));
        assert!(
            out.human.contains("the images do not match the records"),
            "the refusal does not name the fault: {}",
            out.human
        );

        // And the cheap check still passes, which is why it says what it
        // checked rather than claiming more.
        let shallow = cmd_verify(&doc, &corpus, true);
        assert_eq!(shallow.code, exit::OK, "{}", shallow.human);
        assert!(
            shallow.human.contains("NOT re-read"),
            "the shallow answer overclaims: {}",
            shallow.human
        );
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
        let out = cmd_verify(&doc, &corpus, true);
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

        let out = cmd_verify(&doc, &other, true);
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
        let out = cmd_verify(&doc, &corpus, true);
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
        let out = cmd_verify(&doc, &corpus, true);
        assert_eq!(out.code, exit::VERIFY_MISMATCH);
        assert!(out.human.contains("cannot be named"), "{}", out.human);
    }

    #[test]
    fn verify_sends_a_document_that_is_not_a_result_to_validate() {
        let dir = tempfile::tempdir().unwrap();
        let doc = dir.path().join("x.json");
        std::fs::write(&doc, r#"{"hello":"world"}"#).unwrap();
        let out = cmd_verify(&doc, dir.path(), true);
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
                jobs: 1,
                keep_raw: false,
                limit: None,
                split: None,
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

    /// "AUC" is never expanded on this screen and the bracketed pair beside
    /// it is never explained, so the screen has to say where both are
    /// defined. It used to point only at a report of a run that had just
    /// happened, which is a next step rather than an answer.
    #[cfg(unix)]
    #[test]
    fn the_score_summary_says_where_its_two_unexplained_figures_are_defined() {
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
                jobs: 1,
                keep_raw: false,
                limit: None,
                split: None,
            },
        );
        assert!(
            out.human.contains("AUC "),
            "nothing to explain: {}",
            out.human
        );
        assert!(
            out.human.contains("`stegobench help results`"),
            "the summary explains neither figure and points nowhere: {}",
            out.human
        );
        // Beside the figures, not at the bottom of the screen: a definition a
        // screen away from the number is one nobody reads.
        let figure = out.human.find("AUC ").expect("an AUC");
        let pointer = out
            .human
            .find("`stegobench help results`")
            .expect("a pointer");
        assert!(pointer > figure, "the pointer comes before the figure");
        assert!(
            out.human[figure..pointer].lines().count() <= 6,
            "the pointer is not beside the figures: {}",
            out.human
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
                jobs: 1,
                keep_raw: false,
                limit: None,
                split: None,
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
                jobs: 1,
                keep_raw: false,
                limit: None,
                split: None,
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
                jobs: 1,
                keep_raw: false,
                limit: None,
                split: None,
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
                jobs: 1,
                keep_raw: false,
                limit: None,
                split: None,
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
                jobs: 1,
                keep_raw: false,
                limit: None,
                split: None,
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
                jobs: 1,
                keep_raw: false,
                limit: None,
                split: None,
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
                jobs: 1,
                keep_raw: false,
                limit: None,
                split: None,
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
                jobs: 1,
                keep_raw: false,
                limit: None,
                split: None,
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
                jobs: 1,
                keep_raw: false,
                limit: None,
                split: None,
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
                jobs: 1,
                keep_raw: false,
                limit: None,
                split: None,
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
                jobs: 1,
                keep_raw: false,
                limit: None,
                split: None,
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
        let doctor = cmd_doctor(&resolved_at(&dir), None, false);
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
        let described = cmd_describe(&resolved_at(&dir), "stegashield", false);
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
        let corpus = cmd_describe(&resolved_at(&dir), "reveal", false);
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
    /// table at all, because a script or an agent trusts the table.
    ///
    /// Code 3 (pre-flight refusal) IS reachable and this test drives it,
    /// which it did not used to be. This comment claimed otherwise for
    /// longer than it was true, which is the dangerous way round: a reader
    /// trusts the prose and concludes a code they can see in the body is
    /// dead. Code 7 (licence refusal) is the one still unreachable, because
    /// no corpus-licence gate is built.
    ///
    /// Asserting the gap here means the day code 7 becomes reachable without
    /// a test acknowledging it, this test starts failing and says so, rather
    /// than staying quiet about a path nobody covered.
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
                        jobs: 1,
                        keep_raw: false,
                        limit: None,
                        split: None,
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

    /// A registry entry that is installed and that nothing here can drive.
    ///
    /// `aletheia-rich` is the shipped one: a real, registered detector with a
    /// self-test and no invoke block.
    #[cfg(unix)]
    fn registry_with_an_undrivable_detector(dir: &Path) -> PathBuf {
        let reg = registry_with_one_present_and_one_missing(dir);
        std::fs::write(
            reg.join("paperwork.toml"),
            "name = \"paperwork\"\nkind = \"detector\"\nlicence = \"MIT\"\n\
             [binary]\ncommand = [\"sh\"]\nversion_args = [\"-c\", \"echo v1\"]\n\
             [emits]\noutput = \"score\"\nhigher_means_stego = true\n\
             [selftest]\nmust_detect = \"a.png\"\nmust_clear = \"b.png\"\n",
        )
        .unwrap();
        reg
    }

    /// THE FAULT THIS EXISTS TO STOP: OUR REGISTRY, THEIR CORPUS, THEIR FAULT.
    ///
    /// An entry with no invoke block cannot be driven, `doctor` has always
    /// said so, and `score` did not ask. So it announced "1 of 1 that can run
    /// here", started the tool once per image, wrote "entry declares no invoke
    /// block" against every one, and refused with "the corpus holds 0 clean
    /// and 0 stego image(s)" over a corpus holding six and twelve.
    #[cfg(unix)]
    #[test]
    fn a_detector_with_no_invoke_block_is_skipped_before_the_corpus_is_walked() {
        let tmp = tempfile::tempdir().unwrap();
        let reg = registry_with_an_undrivable_detector(tmp.path());
        let corpus = tmp.path().join("corpus");
        scratch_corpus(&corpus);

        let out = cmd_score(
            &resolved_at(&reg),
            ScoreRequest {
                corpus: &corpus,
                detectors: &["paperwork".to_string()],
                corpus_id: None,
                trained_on: None,
                records: None,
                out: None,
                timeout: 5,
                jobs: 1,
                keep_raw: false,
                limit: None,
                split: None,
            },
        );

        assert_eq!(out.code, exit::PREFLIGHT_REFUSED, "{}", out.human);
        assert!(
            out.human.contains("invoke block"),
            "the refusal has to name the real reason: {}",
            out.human
        );
        assert!(
            !out.human.contains("0 clean"),
            "our registry's gap was reported as a fault in their corpus: {}",
            out.human
        );
        assert!(
            !corpus.join("paperwork.records.jsonl").exists(),
            "an entry that cannot be driven was still run over the corpus"
        );
    }

    /// The same entry, classified the same way whether it is asked for alone
    /// or beside a detector that works. It used to be a pre-flight refusal in
    /// one case and a plugin failure in the other, which is one fact answered
    /// with two exit codes.
    #[cfg(unix)]
    #[test]
    fn an_undrivable_detector_is_a_skip_beside_a_working_one_too() {
        let tmp = tempfile::tempdir().unwrap();
        let reg = registry_with_an_undrivable_detector(tmp.path());
        let corpus = tmp.path().join("corpus");
        scratch_corpus(&corpus);
        let out_dir = tmp.path().join("results");

        let out = cmd_score(
            &resolved_at(&reg),
            ScoreRequest {
                corpus: &corpus,
                detectors: &["paperwork".to_string(), "sizer".to_string()],
                corpus_id: None,
                trained_on: None,
                records: None,
                out: Some(&out_dir),
                timeout: 5,
                jobs: 1,
                keep_raw: false,
                limit: None,
                split: None,
            },
        );

        assert_eq!(out.code, exit::PREFLIGHT_REFUSED, "{}", out.human);
        assert!(
            out.human.contains("skipped: declares no invoke block"),
            "{}",
            out.human
        );
        assert_eq!(out.json["skipped"], serde_json::json!(1), "{}", out.json);
        assert_eq!(out.json["failed"], serde_json::json!(0), "{}", out.json);
        assert!(
            out_dir.join("sizer.json").exists(),
            "the detector that could run lost its work: {}",
            out.human
        );
    }

    /// `--limit` TAKES A PREFIX AND A CORPUS PUTS ITS COVERS FIRST.
    ///
    /// So the flag advertised for a smoke test reliably produces an all-clean
    /// set, and the refusal that followed talked only about the corpus. The
    /// corpus is fine; the slice asked for is not, and the message has to say
    /// which of the two it is.
    #[cfg(unix)]
    #[test]
    fn a_limit_that_reaches_only_covers_names_the_flag_rather_than_the_corpus() {
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
                jobs: 1,
                keep_raw: false,
                limit: Some(2),
                split: None,
            },
        );

        assert_eq!(out.code, exit::PREFLIGHT_REFUSED, "{}", out.human);
        assert!(out.human.contains("--limit 2"), "{}", out.human);
        assert!(
            out.human.contains("covers before its stego arms"),
            "{}",
            out.human
        );
        assert_eq!(out.json["reason"], serde_json::json!("one-sided"));
    }

    /// The same refusal without the flag says nothing about the flag, because
    /// then the corpus genuinely is the one-sided thing.
    #[cfg(unix)]
    #[test]
    fn a_one_sided_corpus_is_not_blamed_on_a_limit_nobody_gave() {
        let tmp = tempfile::tempdir().unwrap();
        let reg = registry_with_one_present_and_one_missing(tmp.path());
        let corpus = tmp.path().join("covers-only");
        std::fs::create_dir_all(&corpus).unwrap();
        for i in 0..3 {
            std::fs::write(corpus.join(format!("c{i}.png")), test_png(0)).unwrap();
            std::fs::write(
                corpus.join(format!("c{i}.json")),
                r#"{"role":"clean","sha256":"0"}"#,
            )
            .unwrap();
        }

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
                jobs: 1,
                keep_raw: false,
                limit: None,
                split: None,
            },
        );

        assert_eq!(out.code, exit::PREFLIGHT_REFUSED, "{}", out.human);
        assert!(!out.human.contains("--limit"), "{}", out.human);
        assert_eq!(out.json["reason"], serde_json::json!("one-sided"));
    }

    /// PLAN'S WHOLE JOB IS TELLING YOU THINGS BEFORE YOU SPEND THE TIME.
    ///
    /// A corpus whose stego images differ from their covers in more than the
    /// payload planned silently, and `score` then found it instantly and
    /// statically from the same records.
    #[cfg(unix)]
    #[test]
    fn plan_warns_about_a_confounded_corpus_before_anything_is_scored() {
        let tmp = tempfile::tempdir().unwrap();
        let reg = registry_with_one_present_and_one_missing(tmp.path());
        let corpus = tmp.path().join("confounded");
        std::fs::create_dir_all(&corpus).unwrap();
        // Wider than its cover, which is a second variable a measurement over
        // this corpus would pick up along with the payload.
        let mut wider = b"\x89PNG\r\n\x1a\n".to_vec();
        wider.extend_from_slice(&13u32.to_be_bytes());
        wider.extend_from_slice(b"IHDR");
        wider.extend_from_slice(&64u32.to_be_bytes());
        wider.extend_from_slice(&64u32.to_be_bytes());
        wider.extend_from_slice(&[8, 2, 0, 0, 0]);
        wider.extend_from_slice(&[0, 0, 0, 0]);
        for i in 0..3 {
            std::fs::write(corpus.join(format!("c{i}.png")), test_png(0)).unwrap();
            std::fs::write(
                corpus.join(format!("c{i}.json")),
                r#"{"role":"clean","sha256":"0"}"#,
            )
            .unwrap();
            std::fs::write(corpus.join(format!("s{i}.png")), &wider).unwrap();
            std::fs::write(
                corpus.join(format!("s{i}.json")),
                format!(r#"{{"role":"stego","source_png":"c{i}.png","sha256":"0"}}"#),
            )
            .unwrap();
        }

        let out = cmd_plan(
            &resolved_at(&reg),
            &[
                "score".into(),
                "--corpus".into(),
                corpus.display().to_string(),
                "--detector".into(),
                "sizer".into(),
            ],
        );

        assert_eq!(out.code, exit::OK, "{}", out.human);
        assert!(
            out.human.contains("differ from their cover in more than"),
            "plan said nothing about a corpus score would warn about: {}",
            out.human
        );
        assert!(
            out.human.contains("different sizes"),
            "the warning has to carry an example: {}",
            out.human
        );
        assert!(
            !out.json["corpus_notes"].as_array().unwrap().is_empty(),
            "{}",
            out.json
        );
    }

    /// A PLAN THAT SAYS "ABOUT" IN FRONT OF A GUESS IS CONFIDENTLY WRONG.
    ///
    /// Every total is arithmetic over a `seconds_per_image` somebody wrote
    /// into a registry entry, covering the work on an image and not the cost
    /// of starting the tool once per image. Measured 2026-09-30: an estimate
    /// of about 3 minutes against roughly 11 minutes on the same machine.
    #[cfg(unix)]
    #[test]
    fn plan_says_its_rate_is_declared_rather_than_measured() {
        let tmp = tempfile::tempdir().unwrap();
        let reg = registry_with_one_present_and_one_missing(tmp.path());
        let sizer = reg.join("sizer.toml");
        let with_cost = format!(
            "{}\n[cost]\nseconds_per_image = 0.5\n",
            std::fs::read_to_string(&sizer).unwrap()
        );
        std::fs::write(&sizer, with_cost).unwrap();
        let corpus = tmp.path().join("corpus");
        scratch_corpus(&corpus);

        let out = cmd_plan(
            &resolved_at(&reg),
            &[
                "score".into(),
                "--corpus".into(),
                corpus.display().to_string(),
                "--detector".into(),
                "sizer".into(),
            ],
        );

        assert_eq!(out.code, exit::OK, "{}", out.human);
        assert!(
            out.human.contains("Total          at least"),
            "{}",
            out.human
        );
        // The estimate still says it is declared rather than measured, and
        // still says the total is a floor. What moved into `plan --help` is
        // the REASONING for both; what stays here is the claim itself, which
        // is the part a reader has to see without asking for it.
        assert!(out.human.contains("declared"), "{}", out.human);
        assert!(
            out.human.contains("Read the total as a floor"),
            "{}",
            out.human
        );
        // And the worst-case figure is now bare, because the parenthetical
        // explaining it was longer than the line it qualified.
        assert!(out.human.contains("Worst case"), "{}", out.human);
        assert!(
            !out.human.contains("a ceiling, not a forecast"),
            "the explanation belongs in `plan --help`, not in every run: {}",
            out.human
        );
        assert_eq!(out.json["rate_source"], serde_json::json!("declared"));
        assert_eq!(
            out.json["estimate_is_a_lower_bound"],
            serde_json::json!(true)
        );
    }

    /// `plan` wraps a whole command, so the natural first attempt puts
    /// `score`'s flags straight after `plan`. clap answered that with "to
    /// pass '--corpus' as a value, use '-- --corpus'", which quotes the flag
    /// as a literal word and plans nothing at all.
    #[test]
    fn a_plan_command_starting_with_a_flag_is_answered_with_the_verb_it_needs() {
        let argv = |words: &[&str]| words.iter().map(|w| w.to_string()).collect::<Vec<_>>();
        let unknown = clap::error::ErrorKind::UnknownArgument;

        let hint = plan_verb_hint(unknown, &argv(&["plan", "--corpus", "./mine"]))
            .expect("the natural first attempt is not recognised");
        assert_eq!(hint, "--corpus ./mine");

        // A path with a space survives being pasted back.
        let quoted = plan_verb_hint(unknown, &argv(&["plan", "--corpus", "my corpus"]))
            .expect("a quoted value is not recognised");
        assert_eq!(quoted, "--corpus 'my corpus'");

        // A leading word is either a verb `plan` reads or one it refuses by
        // name, and both are better answers than this one.
        assert!(plan_verb_hint(unknown, &argv(&["plan", "score", "--corpus", "x"])).is_none());
        // Another command's unknown flag is not this.
        assert!(plan_verb_hint(unknown, &argv(&["score", "--nope"])).is_none());
        assert!(plan_verb_hint(unknown, &argv(&["plan"])).is_none());
        assert!(plan_verb_hint(unknown, &[]).is_none());
        // A different failure keeps clap's own answer, which is better for it.
        assert!(plan_verb_hint(
            clap::error::ErrorKind::MissingRequiredArgument,
            &argv(&["plan", "--corpus", "x"])
        )
        .is_none());
    }

    /// `metrics` has published a stable word beside every refusal since it
    /// shipped and `score` published prose. A caller branching on the second
    /// is matching English that nothing promises to keep.
    #[cfg(unix)]
    #[test]
    fn every_score_refusal_carries_a_word_a_script_can_branch_on() {
        let tmp = tempfile::tempdir().unwrap();
        let reg = registry_with_one_present_and_one_missing(tmp.path());
        let corpus = tmp.path().join("corpus");
        scratch_corpus(&corpus);
        let ask = |corpus: &Path, detectors: &[String]| {
            cmd_score(
                &resolved_at(&reg),
                ScoreRequest {
                    corpus,
                    detectors,
                    corpus_id: None,
                    trained_on: None,
                    records: None,
                    out: None,
                    timeout: 5,
                    jobs: 1,
                    keep_raw: false,
                    limit: None,
                    split: None,
                },
            )
        };

        let cases = [
            (
                ask(&tmp.path().join("nope"), &["sizer".to_string()]),
                "corpus-missing",
            ),
            (ask(&corpus, &["ghost".to_string()]), "nothing-available"),
            (ask(&corpus, &["nobody".to_string()]), "unknown-detector"),
            (
                ask(&corpus, &["all".to_string(), "sizer".to_string()]),
                "all-with-others",
            ),
        ];
        for (out, want) in cases {
            assert_ne!(out.code, exit::OK, "{}", out.human);
            assert_eq!(out.json["reason"], serde_json::json!(want), "{}", out.human);
            // The prose is still there beside the word, because a person
            // reads one and a script reads the other.
            assert!(out.json["error"].is_string(), "{}", out.json);
        }
    }

    /// The flagship integrity message, which rendered with runs of a dozen
    /// spaces inside it because the format string had been collapsed onto one
    /// line with its continuations baked in as literal whitespace.
    #[test]
    fn the_content_digest_refusal_wraps_like_every_other_message() {
        let dir = tempfile::tempdir().unwrap();
        let corpus = dir.path().join("corpus");
        corpus_honest(&corpus, &[b"cover bytes", b"stego bytes"]);
        let digest = score::corpus_digest(&corpus).unwrap().unwrap();
        let doc = dir.path().join("r.json");
        result_claiming(&doc, &digest);
        let mut sealed: Result1 =
            serde_json::from_str(&std::fs::read_to_string(&doc).unwrap()).unwrap();
        sealed.seal();
        sealed.metrics.auc = 0.99;
        std::fs::write(&doc, serde_json::to_string(&sealed).unwrap()).unwrap();

        let out = cmd_verify(&doc, &corpus, true);
        assert_eq!(out.code, exit::VERIFY_MISMATCH, "{}", out.human);
        assert!(
            !out.human.contains("   "),
            "the message still carries runs of whitespace:\n{}",
            out.human
        );
        assert!(
            out.human
                .contains("Every field except the two that record WHEN the run happened"),
            "the sentence did not survive rewrapping:\n{}",
            out.human
        );
    }

    /// `stegobench list detectors | grep zsteg` found nothing, and
    /// `stegobench doctor > report.txt` wrote an empty file, because a
    /// listing is content and every one of them was going to stderr.
    #[test]
    fn a_listing_is_content_and_a_refusal_is_not() {
        for command in [
            Command::List { kind: "all".into() },
            Command::Describe {
                name: "zsteg".into(),
                toml: false,
            },
            Command::Help { topic: None },
            Command::Completions {
                shell: clap_complete::Shell::Bash,
            },
            Command::Plan {
                command: vec!["score".into()],
            },
            Command::Metrics {
                file: None,
                at: vec![],
            },
        ] {
            assert!(human_is_content(&command), "not routed to stdout");
        }
        for command in [
            Command::Score {
                corpus: PathBuf::from("c"),
                detector: vec!["d".into()],
                split: None,
                corpus_id: None,
                trained_on: None,
                records: None,
                out: None,
                timeout: 60,
                jobs: 1,
                keep_raw: false,
                limit: None,
            },
            Command::Validate {
                file: PathBuf::from("f"),
            },
            Command::Fetch {
                corpus: "c".into(),
                tier: "nano".into(),
                dest: None,
                max_bytes: None,
                budget_minutes: 1,
            },
        ] {
            assert!(!human_is_content(&command), "progress reached stdout");
        }
    }

    /// `doctor` prints the same report whether it ends in 0 or 8, so the
    /// report is not routed on the exit code: redirecting it on the machine
    /// whose report was worth keeping wrote an empty file.
    #[test]
    fn doctors_report_goes_to_stdout_under_either_verdict() {
        let out = cmd_doctor(&resolved_at(shipped_registry()), None, false);
        assert!(out.payload_on_stdout, "{}", out.human);
    }

    /// `--no-selftest` promises a faster report, not a different verdict. It
    /// used to hand exit 8 to a machine where plain `doctor` exited 0, because
    /// fitness was read off self-test results the flag had just skipped.
    #[test]
    fn skipping_the_self_tests_judges_what_is_installed() {
        let out = cmd_doctor(&resolved_at(shipped_registry()), None, false);
        assert_eq!(
            out.json["verified"].as_u64(),
            Some(0),
            "this test only means something when nothing was self-tested"
        );
        let present = out.json["present"].as_u64().expect("a count");
        let expected = if present == 0 {
            exit::ENVIRONMENT_UNFIT
        } else {
            exit::OK
        };
        assert_eq!(out.code, expected, "{present} installed:\n{}", out.human);
        assert!(
            !out.human.contains("no tool here is usable"),
            "judged on a self-test it did not run:\n{}",
            out.human
        );
        if present > 0 {
            assert!(
                out.human.contains("The self-tests were skipped"),
                "the report claims more than it checked:\n{}",
                out.human
            );
        }
    }

    /// A human reading "FIT" does not need the number and a script reads the
    /// status. The documented contract stays under `--help`.
    #[test]
    fn the_verdict_line_carries_no_exit_code() {
        for strict in [false, true] {
            let out = cmd_doctor(&resolved_at(shipped_registry()), None, strict);
            assert!(
                out.human.contains("\nFIT:") || out.human.contains("\nUNFIT:"),
                "no verdict at all:\n{}",
                out.human
            );
            assert!(
                !out.human.contains("(exit "),
                "the verdict quotes its own exit code:\n{}",
                out.human
            );
        }
    }

    /// "0 answering" was undefined anywhere a reader would meet it.
    #[test]
    fn the_self_test_summary_uses_a_word_the_help_defines() {
        let out = cmd_doctor(&resolved_at(shipped_registry()), None, false);
        let line = out
            .human
            .lines()
            .find(|l| l.contains("Stegobench's own self-test:"))
            .expect("a summary line");
        assert!(line.contains("responded"), "{line}");
        assert!(!line.contains("answering"), "{line}");

        let mut doctor = Cli::command();
        doctor.build();
        let help = doctor
            .get_subcommands()
            .find(|s| s.get_name() == "doctor")
            .expect("doctor is a command")
            .get_long_about()
            .expect("doctor explains itself")
            .to_string();
        assert!(help.contains("responded"), "the word is defined nowhere");
        // The screen and the JSON say the same word, so a reader moving
        // between them is not looking at two vocabularies.
        assert_eq!(out.json["responded"], serde_json::json!(0));
    }

    /// `registry  plugins/registry` and `fixtures  fixtures`: the second is a
    /// value repeating its own key, which reads as a bug rather than as an
    /// answer to "which images were these".
    #[test]
    fn doctor_names_where_it_read_from_in_full() {
        let out = cmd_doctor(&resolved_at(shipped_registry()), None, false);
        let first = out.human.lines().next().expect("a registry line");
        let path = first
            .strip_prefix("registry  ")
            .expect("the registry line comes first");
        assert!(
            Path::new(path).is_absolute(),
            "a relative path is not somewhere a reader can go: {first}"
        );
    }

    /// `plan score --detector all` estimated seven detectors and `score` then
    /// ran four, because the plan never asked the availability question the
    /// run asks. It overstated the job and hid three blockers.
    #[cfg(unix)]
    #[test]
    fn a_plan_leaves_out_what_score_would_skip() {
        let tmp = tempfile::tempdir().unwrap();
        let reg = registry_with_one_present_and_one_missing(tmp.path());
        let corpus = tmp.path().join("corpus");
        scratch_corpus(&corpus);

        let out = cmd_plan(
            &resolved_at(&reg),
            &[
                "score".into(),
                "--corpus".into(),
                corpus.display().to_string(),
                "--detector".into(),
                "all".into(),
            ],
        );
        assert_eq!(out.code, exit::OK, "{}", out.human);
        assert_eq!(out.json["runnable_detectors"], serde_json::json!(1));
        assert_eq!(out.json["unavailable_detectors"], serde_json::json!(1));
        assert!(
            out.human.contains("to score with each of 1 detector(s)"),
            "the header counts what cannot run:\n{}",
            out.human
        );
        assert!(
            out.human.contains("ghost") && out.human.contains("NOT AVAILABLE"),
            "the blocker is not on the screen:\n{}",
            out.human
        );
        assert!(
            out.human.contains("cannot run here and are left out"),
            "nothing says the figures exclude it:\n{}",
            out.human
        );
        let ghost = out.json["per_detector"]
            .as_array()
            .expect("per detector")
            .iter()
            .find(|d| d["detector"] == "ghost")
            .expect("ghost is listed");
        assert_eq!(ghost["available"], serde_json::json!(false));
        assert!(ghost["estimated_seconds"].is_null(), "{ghost}");
    }

    /// A plan over nothing that can run refuses with the word and the code
    /// `score` refuses with, rather than printing a table of zeroes.
    #[cfg(unix)]
    #[test]
    fn a_plan_over_nothing_available_refuses_the_way_score_does() {
        let tmp = tempfile::tempdir().unwrap();
        let reg = registry_with_one_present_and_one_missing(tmp.path());
        let corpus = tmp.path().join("corpus");
        scratch_corpus(&corpus);

        let out = cmd_plan(
            &resolved_at(&reg),
            &[
                "score".into(),
                "--corpus".into(),
                corpus.display().to_string(),
                "--detector".into(),
                "ghost".into(),
            ],
        );
        assert_eq!(out.code, exit::PREFLIGHT_REFUSED, "{}", out.human);
        assert_eq!(out.json["reason"], serde_json::json!("nothing-available"));
    }

    /// Zero parsed, and meant "kill it before it can answer": every item
    /// recorded a timeout under a document that exited zero.
    #[test]
    fn a_timeout_of_zero_is_refused_rather_than_given_a_meaning() {
        let Err(e) = Cli::try_parse_from([
            "stegobench",
            "score",
            "--corpus",
            "c",
            "--detector",
            "d",
            "--timeout",
            "0",
        ]) else {
            panic!("0 has no honest meaning here")
        };
        let said = e.to_string();
        assert!(said.contains("kill the detector"), "{said}");
        assert!(said.contains("no value meaning no timeout"), "{said}");

        let ok = Cli::try_parse_from([
            "stegobench",
            "score",
            "--corpus",
            "c",
            "--detector",
            "d",
            "--timeout",
            "1",
        ]);
        assert!(ok.is_ok(), "one second is a real deadline");
    }

    /// clap offered `--records` for `--threads`: five edits apart, and about
    /// writing files rather than about concurrency.
    #[test]
    fn an_unrelated_flag_is_not_offered_as_a_correction() {
        let Err(e) = Cli::try_parse_from([
            "stegobench",
            "score",
            "--corpus",
            "c",
            "--detector",
            "d",
            "--threads",
            "4",
        ]) else {
            panic!("there is no --threads")
        };
        assert_eq!(
            misleading_suggestion(&e).as_deref(),
            Some("--threads"),
            "clap's suggestion was let through: {e}"
        );

        // A genuine near miss still gets clap's own answer, which is better.
        let Err(e) =
            Cli::try_parse_from(["stegobench", "score", "--corpu", "c", "--detector", "d"])
        else {
            panic!("--corpu is a typo")
        };
        assert_eq!(misleading_suggestion(&e), None, "{e}");
    }

    /// "benchmark" is the word in the product's own name and the likeliest
    /// first guess there is. It used to reach the generic unknown-command
    /// line, which says what is absent and nothing about what is present.
    #[test]
    fn the_words_that_mean_score_are_redirected_to_score() {
        for word in ["benchmark", "eval", "evaluate", "Benchmark"] {
            assert!(right_idea(word), "{word} gets the generic message");
            assert!(
                !wrong_direction(word),
                "{word} is not the wrong direction, only the wrong verb"
            );
        }
        let out = cmd_right_idea("benchmark");
        assert_eq!(out.code, exit::USAGE);
        assert!(
            out.human.contains("stegobench score --corpus"),
            "{}",
            out.human
        );
        assert!(
            !out.human.contains("cannot tell you whether"),
            "a reader who typed benchmark has understood the tool:\n{}",
            out.human
        );
    }

    /// `describe` said "18 files in all" over a corpus `fetch` correctly
    /// reports as 39 files. The field holds images.
    #[test]
    fn the_starter_corpus_counts_the_same_thing_on_every_screen() {
        let reg = Registry::load(&shipped_registry()).expect("the real registry loads");
        let starter = reg.corpora.get(STARTER_ID).expect("the starter is listed");
        let facts = corpus_facts(starter);
        let size = facts
            .iter()
            .find(|(k, _)| *k == "Size")
            .map(|(_, v)| v.clone())
            .expect("a size fact");
        assert_eq!(size, "6 covers, 18 images in all", "{size}");
        assert_eq!(
            EMBEDDED_STARTER.len(),
            39,
            "the count the quickstart quotes for `fetch` has moved"
        );
    }

    /// TWO "Next:" LINES ARE NONE.
    ///
    /// A run of the shipped starter corpus into a directory closed with both
    /// "Next: stegobench report ..." and "Next: stegobench describe
    /// pentimento-core", one under the other.
    #[test]
    fn a_run_closes_with_one_next_step() {
        let doc: Result1 = serde_json::from_value(serde_json::json!({
            "schema": stegobench_core::result::RESULT_SCHEMA_ID,
            "subject": {"name": "x", "version": "sha256:a", "kind": "detector"},
            "corpus": {"name": "c", "source": "supplied", "digest": "sha256:a", "pairs": 2},
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
        }))
        .expect("the fixture document parses");

        let dir = tempfile::tempdir().unwrap();
        let reg = Registry::load(&shipped_registry()).expect("the real registry loads");
        let offer = fetch::offer(&reg).expect("the registry offers a real corpus");
        let outcomes = vec![
            (
                "one".to_string(),
                Outcome::Measured {
                    result: Box::new(doc.clone()),
                    tally: stegobench_plugin::runner::Tally::default(),
                    written: Some(dir.path().join("one.json")),
                },
            ),
            (
                "two".to_string(),
                Outcome::Measured {
                    result: Box::new(doc),
                    tally: stegobench_plugin::runner::Tally::default(),
                    written: Some(dir.path().join("two.json")),
                },
            ),
        ];
        let out = summarise(
            &outcomes,
            &Some(dir.path().to_path_buf()),
            true,
            Some(&offer),
        );
        let nexts = out.human.lines().filter(|l| l.starts_with("Next:")).count();
        assert_eq!(nexts, 1, "two next steps is none:\n{}", out.human);
        assert!(
            out.human.contains("Then: "),
            "the second suggestion was dropped rather than ordered:\n{}",
            out.human
        );
        assert!(
            out.human.contains("is a demonstration, not a measurement."),
            "{}",
            out.human
        );
    }
}
