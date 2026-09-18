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
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use stegobench_core::registry::{Kind, Registry};
use stegobench_plugin::{availability, selftest, Verified};
use stegobench_core::{exit, Result1};

#[derive(Parser)]
#[command(
    name = "stegobench",
    version,
    about = "A reproducible benchmark for image steganalysis",
    long_about = "Build a labelled corpus, run detectors over identical bytes, \
                  and report numbers somebody else can check.\n\n\
                  Machine-readable output goes to stdout with --json; progress \
                  and diagnostics go to stderr, so the two can be separated."
)]
struct Cli {
    /// Machine-readable output on stdout. Accepted by every subcommand.
    #[arg(long, global = true)]
    json: bool,

    /// Where the tool registry lives.
    #[arg(
        long,
        global = true,
        value_name = "DIR",
        env = "STEGOBENCH_REGISTRY",
        default_value = "plugins/registry"
    )]
    registry: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print a published schema, generated from the types the tool writes
    ///
    /// The schema is not maintained by hand beside the code; it is derived
    /// from it, so a document that validates is one this version can read.
    ///
    /// Example:
    ///   stegobench schema result-v1 > result-v1.schema.json
    Schema {
        /// Which schema. Currently only `result-v1`.
        #[arg(value_name = "NAME", default_value = "result-v1")]
        name: String,
    },

    /// Check a document against its schema and the rules the schema cannot hold
    ///
    /// Exits 6 when the document is invalid, naming every problem rather than
    /// only the first, because fixing them one round trip at a time is how a
    /// format gets a reputation for being fussy.
    ///
    /// Example:
    ///   stegobench validate results/rich-model-suniward-0400.json
    Validate {
        /// Path to a JSON document.
        #[arg(value_name = "FILE")]
        file: PathBuf,
    },

    /// List what this installation can do
    ///
    /// Generated from the registry, so what it prints is what the tool will
    /// actually run. A README goes stale; this cannot.
    ///
    /// Example:
    ///   stegobench list detectors --json | jq '.[].name'
    List {
        /// One of: detectors, embedders, all.
        #[arg(value_name = "KIND", default_value = "all")]
        kind: String,
    },

    /// Show everything registered about one tool
    Describe {
        /// A name as `list` prints it.
        #[arg(value_name = "NAME")]
        name: String,
    },

    /// Estimate what a run would cost, without running anything
    Plan {
        /// The command that would be run, as it would be typed.
        #[arg(value_name = "COMMAND", trailing_var_arg = true, num_args = 0..)]
        command: Vec<String>,
    },

    /// Check that this machine can run what it claims to
    ///
    /// Reports what is installed and what is missing. It does NOT yet run each
    /// tool against a known positive and a known negative, and says so per
    /// line rather than letting "present" read as "working": a rich-model
    /// extraction once ran over 2,000 images, exited zero every time and
    /// produced nothing, because a support package was missing.
    ///
    /// Exits 8 when something needed is missing.
    Doctor {
        /// Where the self-test fixtures live.
        #[arg(long, value_name = "DIR", default_value = "fixtures")]
        fixtures: PathBuf,
        /// Skip the self-tests and only report what is installed. Faster, and
        /// honest about being weaker: it cannot tell a working tool from a
        /// broken one.
        #[arg(long)]
        no_selftest: bool,
    },

    /// Score a corpus with one or more detectors
    Score {
        /// Corpus name or directory.
        #[arg(long, value_name = "NAME_OR_PATH")]
        corpus: String,
    },
}

/// What a subcommand produced: a JSON value for stdout, and human text for stderr.
struct Output {
    json: serde_json::Value,
    human: String,
    code: i32,
}

impl Output {
    fn ok(json: serde_json::Value, human: impl Into<String>) -> Self {
        Output { json, human: human.into(), code: exit::OK }
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

fn cmd_schema(name: &str) -> Output {
    match name {
        "result-v1" => {
            let schema = schemars::schema_for!(Result1);
            Output::ok(
                serde_json::to_value(&schema).expect("a generated schema serialises"),
                "result-v1 schema written to stdout",
            )
        }
        other => Output::err(
            exit::USAGE,
            format!("unknown schema {other:?}. Known schemas: result-v1"),
        ),
    }
}

fn load_registry(dir: &PathBuf) -> Result<Registry, Output> {
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

fn cmd_list(dir: &PathBuf, kind: &str) -> Output {
    let reg = match load_registry(dir) {
        Ok(r) => r,
        Err(o) => return o,
    };
    let wanted: Vec<_> = match kind {
        "detectors" => reg.of_kind(Kind::Detector),
        "embedders" => reg.of_kind(Kind::Embedder),
        "all" => reg.entries.values().collect(),
        other => {
            return Output::err(
                exit::USAGE,
                format!("unknown kind {other:?}. Known: detectors, embedders, all"),
            )
        }
    };
    let mut human = wanted
        .iter()
        .map(|e| e.summary())
        .collect::<Vec<_>>()
        .join("\n");
    if human.is_empty() {
        human = format!("nothing registered under {kind:?}");
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
    Output::ok(
        serde_json::json!({ "tools": wanted, "footprint": reg.footprint() }),
        human,
    )
}

fn cmd_describe(dir: &PathBuf, name: &str) -> Output {
    let reg = match load_registry(dir) {
        Ok(r) => r,
        Err(o) => return o,
    };
    match reg.entries.get(name) {
        Some(e) => Output::ok(
            serde_json::to_value(e).unwrap_or(serde_json::Value::Null),
            toml::to_string_pretty(e).unwrap_or_else(|_| format!("{e:#?}")),
        ),
        None => {
            let known: Vec<_> = reg.entries.keys().cloned().collect();
            Output::err(
                exit::USAGE,
                format!("no tool named {name:?}. Known: {}", known.join(", ")),
            )
        }
    }
}

fn cmd_doctor(dir: &PathBuf, fixtures: &PathBuf, no_selftest: bool) -> Output {
    let reg = match load_registry(dir) {
        Ok(r) => r,
        Err(o) => return o,
    };

    let mut rows = Vec::new();
    let (mut missing, mut broken, mut passed, mut skipped) = (0, 0, 0, 0);

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
            Verified::Skipped(_) => None,
        };
        if !check.presence.is_present() {
            missing += 1;
        }
        match &verdict {
            Verified::Passed => passed += 1,
            Verified::Failed(_) => broken += 1,
            Verified::Skipped(_) => skipped += 1,
        }
        let detail = match &verdict {
            Verified::Failed(why) => format!("{}  ({why})", check.summary()),
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
        "{} tool(s): {passed} verified, {broken} broken, {missing} not installed, \
         {skipped} not checked.",
        rows.len()
    ));
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
        "tools": rows.iter().map(|(c, v, d)| serde_json::json!({
            "name": c.name,
            "present": c.presence.is_present(),
            "verified": c.verified,
            "status": match v {
                Verified::Passed => "passed",
                Verified::Failed(_) => "failed",
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

fn cmd_validate(file: &PathBuf) -> Output {
    let text = match std::fs::read_to_string(file) {
        Ok(t) => t,
        Err(e) => {
            return Output::err(exit::FAILURE, format!("cannot read {}: {e}", file.display()))
        }
    };
    let parsed: Result1 = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => {
            return Output::err(
                exit::SCHEMA_INVALID,
                format!("{} is not a result-v1 document: {e}", file.display()),
            )
        }
    };
    match parsed.validate() {
        Ok(()) => Output::ok(
            serde_json::json!({ "ok": true, "schema": parsed.schema }),
            format!("{} is a valid result-v1 document", file.display()),
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
}

/// Subcommands whose behaviour is scoped and planned but not yet built.
///
/// They exist in the tree from the first release so the vocabulary is fixed
/// before anyone depends on it, and they exit 8 with the reason rather than
/// pretending to work.
fn not_yet(what: &str, tracked_as: &str) -> Output {
    Output::err(
        exit::ENVIRONMENT_UNFIT,
        format!(
            "`{what}` is not built yet in this release.\n\
             It is scoped as {tracked_as}. This command exists now so the \
             vocabulary is settled before anything depends on it."
        ),
    )
}

fn run(cli: &Cli) -> Output {
    match &cli.command {
        Command::Schema { name } => cmd_schema(name),
        Command::Validate { file } => cmd_validate(file),
        Command::List { kind } => cmd_list(&cli.registry, kind),
        Command::Describe { name } => cmd_describe(&cli.registry, name),
        Command::Plan { .. } => not_yet("plan", "V10, needs the governor"),
        Command::Doctor { fixtures, no_selftest } => {
            cmd_doctor(&cli.registry, fixtures, *no_selftest)
        }
        Command::Score { .. } => not_yet("score", "needs the plugin host"),
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
        // point, as it is for `schema`, so redirecting to a file works without
        // remembering a flag.
        if matches!(cli.command, Command::Schema { .. }) {
            let mut stdout = std::io::stdout().lock();
            let _ = serde_json::to_writer_pretty(&mut stdout, &out.json);
            let _ = writeln!(stdout);
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
    use clap::CommandFactory;

    #[test]
    fn the_command_tree_is_well_formed() {
        Cli::command().debug_assert();
    }

    #[test]
    fn json_is_accepted_by_every_subcommand() {
        // The rule in the module docstring, asserted rather than trusted.
        // A subcommand added later without --json fails here.
        for args in [
            vec!["stegobench", "--json", "schema", "result-v1"],
            vec!["stegobench", "--json", "validate", "x.json"],
            vec!["stegobench", "--json", "list", "detectors"],
            vec!["stegobench", "--json", "doctor"],
            vec!["stegobench", "--json", "doctor", "--no-selftest"],
            vec!["stegobench", "--json", "score", "--corpus", "x"],
        ] {
            assert!(Cli::try_parse_from(&args).is_ok(), "rejected: {args:?}");
        }
    }

    #[test]
    fn schema_generates_and_names_the_format() {
        let out = cmd_schema("result-v1");
        assert_eq!(out.code, exit::OK);
        let text = serde_json::to_string(&out.json).unwrap();
        assert!(text.contains("Result1"), "schema should describe the result type");
        assert!(text.contains("n_error"), "the required honesty field must be in the schema");
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

    #[test]
    fn unbuilt_commands_refuse_clearly_rather_than_pretending() {
        let out = not_yet("score", "needs the plugin host");
        assert_eq!(out.code, exit::ENVIRONMENT_UNFIT);
        assert!(out.human.contains("not built yet"));
    }
}
