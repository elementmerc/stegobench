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
use std::path::Path;

use clap::{CommandFactory, Parser};
use stegobench_cli::cli::{Cli, Command};
use stegobench_cli::help_topics;
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

fn cmd_list(dir: &Path, kind: &str) -> Output {
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

fn cmd_describe(dir: &Path, name: &str) -> Output {
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
    let script = String::from_utf8(buf).unwrap_or_default();
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
        Command::Doctor {
            fixtures,
            no_selftest,
        } => cmd_doctor(&cli.registry, fixtures, *no_selftest),
        Command::Score { .. } => not_yet("score", "needs the plugin host"),
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
            vec!["stegobench", "--json", "score", "--corpus", "x"],
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

    #[test]
    fn unbuilt_commands_refuse_clearly_rather_than_pretending() {
        let out = not_yet("score", "needs the plugin host");
        assert_eq!(out.code, exit::ENVIRONMENT_UNFIT);
        assert!(out.human.contains("not built yet"));
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

        // 3: pre-flight refusal. NOT YET REACHABLE: needs the governor
        // (`plan`/`score`), neither of which is built. See exit::PREFLIGHT_REFUSED.

        // 4: plugin failure. NOT YET REACHABLE: needs `score` running a real
        // plugin, which is not built. See exit::PLUGIN_FAILED.

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

        // 8: environment unfit. `not_yet` is the code path Command::Plan and
        // Command::Score actually use today. `cmd_doctor` reaches the same
        // code independently when a registered tool is missing or broken,
        // but is not additionally exercised here: it needs a real registry
        // directory relative to the process's working directory, which a
        // unit test cannot assume without constructing one, and doing that
        // honestly is worth its own test rather than a shortcut in this one.
        assert_eq!(
            not_yet("plan", "V10, needs the governor").code,
            exit::ENVIRONMENT_UNFIT
        );

        // 130: interrupted. NOT YET REACHABLE from a unit test: this is a
        // signal-handler exit path (SIGINT/SIGTERM), which needs a real
        // process and a real signal to drive, not a function call. No
        // signal handling exists in this binary yet to test in the first
        // place.
    }
}
