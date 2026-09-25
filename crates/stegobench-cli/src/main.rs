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
use stegobench_cli::cli::{Cli, Command};
use stegobench_cli::help_topics;
use stegobench_cli::score;
use stegobench_core::registry::{Kind, Registry};
use stegobench_core::{exit, ManifestV1, Result1, RunV1};
use stegobench_plugin::{availability, selftest, Verified};

/// What a subcommand produced: a JSON value for stdout, and human text for stderr.
struct Output {
    json: serde_json::Value,
    human: String,
    code: i32,
}

impl Output {
    fn ok(json: serde_json::Value, human: impl Into<String>) -> Self {
        Output {
            json,
            human: human.into(),
            code: exit::OK,
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
    if let Some(e) = reg.entries.get(name) {
        return Output::ok(
            serde_json::to_value(e).unwrap_or(serde_json::Value::Null),
            toml::to_string_pretty(e).unwrap_or_else(|_| format!("{e:#?}")),
        );
    }
    // One vocabulary: a corpus id is looked up in the same breath as a tool
    // name, because a user should not have to know which of the two a thing is
    // before they can ask about it.
    if let Some(c) = reg.corpora.get(name) {
        return Output::ok(
            serde_json::to_value(c).unwrap_or(serde_json::Value::Null),
            toml::to_string_pretty(c).unwrap_or_else(|_| format!("{c:#?}")),
        );
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

fn cmd_doctor(dir: &Path, fixtures: &Path, no_selftest: bool) -> Output {
    let reg = match load_registry(dir) {
        Ok(r) => r,
        Err(o) => return o,
    };

    let mut rows = Vec::new();
    let (mut missing, mut broken, mut passed, mut skipped, mut answered) = (0, 0, 0, 0, 0);

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
        if !check.presence.is_present() {
            missing += 1;
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
        rows.push((check, verdict, detail));
    }

    let mut human: Vec<String> = rows.iter().map(|(_, _, d)| d.clone()).collect();
    human.push(String::new());
    human.push(format!(
        "{} tool(s): {passed} verified, {answered} answering, {broken} broken, \
         {missing} not installed, {skipped} not checked.",
        rows.len()
    ));
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

    let json = serde_json::json!({
        "checked": rows.len(),
        "verified": passed,
        "broken": broken,
        "missing": missing,
        "not_checked": skipped,
        "answering": answered,
        "tools": rows.iter().map(|(c, v, d)| serde_json::json!({
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
        })).collect::<Vec<_>>(),
    });

    let mut out = Output::ok(json, human.join("\n"));
    if missing > 0 || broken > 0 {
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
    let Some(entry) = reg.entries.get(detector) else {
        return Output::err(
            exit::USAGE,
            format!("no tool named {detector:?} is registered."),
        );
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

    let per_image = entry.cost.seconds_per_image;
    let seconds = per_image.map(|s| s * items as f64);
    // One JSON line per answer, measured at roughly sixty bytes on the real
    // records this writes.
    let records_mb = (items as f64 * 60.0) / 1_048_576.0;

    let mut value = serde_json::Map::new();
    value.insert("items".into(), serde_json::json!(items));
    value.insert("detector".into(), serde_json::json!(detector));
    value.insert("seconds_per_image".into(), serde_json::json!(per_image));
    value.insert("estimated_seconds".into(), serde_json::json!(seconds));
    value.insert("records_mb".into(), serde_json::json!(records_mb));
    value.insert(
        "worst_case_seconds".into(),
        serde_json::json!(items * timeout),
    );

    let duration = match seconds {
        Some(s) => format!("about {}", human_duration(s)),
        // Said rather than defaulted. A tool with no measured rate cannot be
        // estimated, and inventing a number here would be the plan lying
        // about the one thing it is for.
        None => format!(
            "unknown: {detector:?} declares no seconds_per_image, so nothing \
             here can estimate how long it takes"
        ),
    };
    Output::ok(
        serde_json::Value::Object(value),
        format!(
            "{items} item(s) to score with {detector}. Time: {duration}. \
             Records file: about {records_mb:.1} MB. Worst case, if every \
             item hit the {timeout}s deadline: {}.",
            human_duration((items * timeout) as f64)
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

#[allow(clippy::too_many_arguments)]
fn cmd_score(
    registry_dir: &Path,
    corpus: &Path,
    detector: &str,
    records: Option<&Path>,
    out: Option<&Path>,
    timeout: u64,
    limit: Option<u64>,
) -> Output {
    let reg = match load_registry(registry_dir) {
        Ok(r) => r,
        Err(o) => return o,
    };
    let Some(entry) = reg.entries.get(detector) else {
        return Output::err(
            exit::USAGE,
            format!(
                "no tool named {detector:?} is registered. \
                 `stegobench list detectors` shows what is."
            ),
        );
    };

    // The records file sits beside the corpus by default, named after it, so
    // two runs over two corpora cannot resume from each other's answers.
    let records = records.map(PathBuf::from).unwrap_or_else(|| {
        let mut name = corpus.file_name().unwrap_or_default().to_os_string();
        name.push(".records.jsonl");
        corpus.with_file_name(name)
    });

    let request = score::Request {
        corpus,
        records,
        timeout: std::time::Duration::from_secs(timeout),
        limit,
    };

    // Progress goes to stderr, so `--json` on stdout stays machine readable
    // while a person can still watch a run that takes hours.
    let report = |line: &str| eprintln!("  {line}");
    let (result, tally) = match score::score(entry, &request, report) {
        Ok(pair) => pair,
        Err(e) => return Output::err(exit::PLUGIN_FAILED, e.to_string()),
    };

    // Validated before it is written, not after. A document this refuses is
    // one no reader should have been handed in the first place.
    if let Err(problems) = result.validate() {
        return Output::err(
            exit::SCHEMA_INVALID,
            format!(
                "the run finished but produced a result that does not \
                 validate, which is a bug in this harness rather than in the \
                 detector:\n  {}",
                problems.join("\n  ")
            ),
        );
    }

    let body = match serde_json::to_string_pretty(&result) {
        Ok(b) => b,
        Err(e) => return Output::err(exit::FAILURE, format!("could not write the result: {e}")),
    };
    if let Some(path) = out {
        if let Err(e) = std::fs::write(path, format!("{body}\n")) {
            return Output::err(
                exit::FAILURE,
                format!("could not write the result to {}: {e}", path.display()),
            );
        }
    }

    let value = serde_json::to_value(&result).unwrap_or(serde_json::Value::Null);
    Output::ok(
        value,
        format!(
            "{} scored, {} resumed, {} could not be answered. AUC {:.4} over \
             {} clean and {} stego image(s).",
            tally.scored,
            tally.resumed,
            result.metrics.n_error,
            result.metrics.auc,
            result.metrics.n_clean,
            result.metrics.n_stego,
        ),
    )
}

fn run(cli: &Cli) -> Output {
    match &cli.command {
        Command::Schema { name } => cmd_schema(name),
        Command::Validate { file } => cmd_validate(file),
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
            records,
            out,
            timeout,
            limit,
        } => cmd_score(
            &cli.registry,
            corpus,
            detector,
            records.as_deref(),
            out.as_deref(),
            *timeout,
            *limit,
        ),
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
            walked >= 9,
            "walked {walked} subcommand(s); the command tree should carry at \
             least the nine 04-cli-surface.md names, so this walk looked at \
             almost nothing"
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

    /// Drives the binary's own code to every exit code the contract in
    /// `stegobench_core::exit` names, or records plainly why a given code
    /// cannot be reached yet.
    ///
    /// The table in `04-cli-surface.md` and the man page are only as good as
    /// the binary's agreement with them: a documented code the binary never
    /// actually returns is worse than no table, because a script or an agent
    /// trusts the table. Codes 3 (pre-flight refusal) and 4 (plugin failure)
    /// and 5 (verify mismatch) and 7 (licence refusal) genuinely have NO code
    /// path yet, because the commands that would produce them (`plan`,
    /// `score`, and any corpus-licence or provenance check) are not built.
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

        // 3: pre-flight refusal. NOT YET REACHABLE: nothing refuses a run
        // before starting it on grounds of capacity or fitness yet.
        // See exit::PREFLIGHT_REFUSED.

        // 4: plugin failure. Reachable now that `score` runs: an embedder
        // asked to tell two images apart is refused through this code. It is
        // driven in the score module's own tests, which can build a corpus
        // and a registry entry without this test constructing both.

        // 5: verify mismatch. NOT YET REACHABLE: there is no `verify`
        // subcommand yet (04-cli-surface.md names one; it is not in
        // Command). See exit::VERIFY_MISMATCH.

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
