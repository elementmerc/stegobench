// Author:  Daniel Iwugo
// Comment: Christ is King
// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Daniel Iwugo
//
// The command tree, kept in its own file (rather than in main.rs) so
// build.rs can textually include it and generate man pages from the exact
// same definition the binary parses against. A hand-written man page drifts
// from the tool it describes; this one cannot, because there is only one
// definition and both the binary and the man pages are generated from it.
//
// This file is compiled twice: once as a module of the `stegobench_cli` lib
// target (used by the binary), and once `include!`d directly into build.rs
// (see build.rs for why). It therefore avoids anything that only makes sense
// in one of those two contexts, including its own `use std::path::PathBuf`,
// which build.rs already imports; `PathBuf` is referred to by its full path
// below instead so the two copies never fight over the same import.

use clap::{Parser, Subcommand, ValueEnum};

/// How `report` renders a table.
///
/// Defined here rather than beside the renderer because `build.rs` includes
/// this file and nothing else, so a type the command tree mentions has to be
/// reachable from it.
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum ReportFormat {
    /// Aligned columns for a terminal.
    Text,
    /// A Markdown table, to paste into an evaluation document.
    Markdown,
    /// Every recorded field, one column each, for a spreadsheet or a script.
    Csv,
}

/// EXIT CODES, part of the CLI's contract (see `stegobench_core::exit`):
///   0 success · 1 generic failure · 2 usage error · 3 pre-flight refusal ·
///   4 plugin failure · 5 verification mismatch · 6 schema invalid ·
///   7 licence refusal · 8 environment unfit · 130 interrupted.
/// Codes 3 and 7 are refusals, not failures: the tool is capable of the thing
/// and declines. A caller that cannot tell a refusal from an error will retry
/// it forever, so the two are never merged and never reused for anything else.
#[derive(Parser)]
#[command(
    name = "stegobench",
    version,
    // clap's built-in `help` subcommand (`stegobench help <subcommand>`,
    // printing that subcommand's --help) would otherwise collide with our
    // own `help` subcommand (`stegobench help <topic>`, conceptual material
    // like `pairing` and `splits`). `-h`/`--help` on any command still work;
    // this only removes the implicit bare `help` word clap adds on top of
    // them.
    disable_help_subcommand = true,
    // WHAT GOES FIRST, AND WHY IT CHANGED
    //
    // This help used to open with the stdout/stderr contract and ten lines of
    // exit codes, and reach the list of commands after them. That ordering is
    // right for the person writing a script around the tool and backwards for
    // the person deciding whether the tool is the one they want: the first
    // screen answered a question they had not asked, and the question they had
    // asked ("what is this, and is it for me") was not answered at all.
    //
    // So the machine contract moved to `after_long_help`, where a script
    // author finds it under `--help` and a newcomer is not made to read it
    // first. Nothing was deleted: both halves are still one command away.
    about = "Measure how good a steganography detector is, using images whose answers are already known",
    long_about = "Stegobench measures DETECTORS. Give it a folder of images that \
                  are already labelled (this one is clean, this one hides a \
                  payload), and it reports how often a detector was right, in a \
                  document naming the exact bytes the number came from.\n\n\
                  IT DOES NOT EXAMINE YOUR OWN IMAGES. That is the opposite \
                  direction: `stegobench help scope` says where to go \
                  instead.\n\n\
                  START HERE\n  \
                  stegobench list detectors   what this installation can run\n  \
                  stegobench doctor           what is installed, and what it \
                  needs\n  \
                  stegobench help             the reasoning, one topic at a time",
    after_help = "Start with `stegobench list detectors`, then `stegobench doctor`.\n\
                  This measures detectors; it does not examine your own images \
                  (`stegobench help scope`).\n\
                  Exit codes and the stdout/stderr contract are under `--help`.",
    after_long_help = "OUTPUT STREAMS\n  \
                  --json writes machine-readable output to stdout. Progress and \
                  diagnostics go to stderr.\n\n\
                  EXIT CODES\n  \
                  0    success\n  \
                  1    generic failure\n  \
                  2    usage error\n  \
                  3    pre-flight refusal\n  \
                  4    plugin failure\n  \
                  5    verification mismatch\n  \
                  6    schema invalid\n  \
                  7    licence refusal\n  \
                  8    environment unfit\n  \
                  130  interrupted\n  \
                  Codes 3 and 7 are refusals, not errors: do not retry them.\n\n\
                  WHERE THE REGISTRY COMES FROM\n  \
                  In order: --registry or STEGOBENCH_REGISTRY, ./plugins/registry, \
                  beside this executable, your user data directory, the system \
                  data directory, then the copy compiled in. A path you name is \
                  used as given. `stegobench doctor` prints which one answered."
)]
pub struct Cli {
    /// Machine-readable output on stdout. Accepted by every subcommand.
    #[arg(long, global = true)]
    pub json: bool,

    /// Where the tool registry lives.
    ///
    /// Left out, stegobench searches: see WHERE THE REGISTRY COMES FROM under
    /// `--help`. Named here, the path is used as given, and a path that is not
    /// there is an error rather than a fall back.
    ///
    /// Setting STEGOBENCH_REGISTRY does the same thing for every command. An
    /// empty value counts as not set.
    // Deliberately not clap's `env`: clap reads the variable before any of our
    // code does, and treats one that is set but empty as a flag supplied
    // without its value, so `STEGOBENCH_REGISTRY=` in a shell profile refused
    // every command including `--help`. The resolver reads it instead.
    #[arg(long, global = true, value_name = "DIR")]
    pub registry: Option<std::path::PathBuf>,

    /// Optional so a bare `stegobench` can print a short orientation rather
    /// than the whole help. Somebody who types the bare name is asking what
    /// this is, and the answer to that is three lines, not three screens.
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Print a published schema, generated from the types the tool writes
    ///
    /// Example:
    ///   stegobench schema result-v1 > result-v1.schema.json
    Schema {
        /// One of result-v1, run-v1, manifest-v1, or `all` to emit every
        /// schema this version knows, keyed by name.
        #[arg(value_name = "NAME", default_value = "result-v1")]
        name: String,
    },

    /// Check a document against its schema and the rules the schema cannot hold
    ///
    /// Exits 6 when the document is invalid, naming every problem rather than
    /// only the first.
    ///
    /// Example:
    ///   stegobench validate results/rich-model-suniward-0400.json
    Validate {
        /// Path to a JSON document.
        #[arg(value_name = "FILE")]
        file: std::path::PathBuf,
    },

    /// List what this installation can do
    ///
    /// Generated from the registry, so it is what the tool will actually run.
    ///
    /// Example:
    ///   stegobench list detectors --json | jq '.[].name'
    List {
        /// One of: detectors, embedders, corpora, all.
        #[arg(value_name = "KIND", default_value = "all")]
        kind: String,
    },

    /// Show everything registered about one tool or corpus
    ///
    /// Takes a tool name or a corpus id.
    ///
    /// Example:
    ///   stegobench describe steghide
    ///   stegobench describe reveal
    Describe {
        /// A name or id as `list` prints it.
        #[arg(value_name = "NAME")]
        name: String,
    },

    /// Estimate what a run would cost, without running anything
    ///
    /// The command is typed exactly as you would run it, flags and all, so
    /// anything `score` requires is required here too.
    ///
    /// Example:
    ///   stegobench plan score --corpus corpora/starter --detector stegexpose
    Plan {
        /// The command that would be run, as it would be typed.
        #[arg(value_name = "COMMAND", trailing_var_arg = true, num_args = 0..)]
        command: Vec<String>,
    },

    /// Check that this machine can run what it claims to
    ///
    /// Reports what is installed, what is missing, and which tools passed
    /// their self-test. "Present" is never reported as "working".
    ///
    /// Exits 8 when something needed is missing.
    ///
    /// Example:
    ///   stegobench doctor
    Doctor {
        /// Where the self-test fixtures live. Defaults to the checkout, then
        /// beside the executable, then your data directories, then the copy
        /// compiled into this binary.
        ///
        /// Setting STEGOBENCH_FIXTURES does the same thing. An empty value
        /// counts as not set.
        // Not clap's `env`, for the reason given on `--registry`.
        #[arg(long, value_name = "DIR")]
        fixtures: Option<std::path::PathBuf>,
        /// Report what is installed without running the self-tests. Faster,
        /// and cannot tell a working tool from a broken one.
        #[arg(long)]
        no_selftest: bool,
    },

    /// Score a corpus with one detector, several, or every registered one
    ///
    /// Reads every sample under --corpus, asks each detector about each one,
    /// and writes one result-v1 document per detector naming the exact bytes
    /// it measured. `stegobench report` turns those into a table.
    ///
    /// Resumable per detector: running the same command again picks up where
    /// it stopped. A detector that is missing or fails is reported and does
    /// not lose the others' work.
    ///
    /// EXIT CODES here: 0 every detector produced a result; 3 at least one was
    /// skipped as unavailable; 4 at least one failed while running. 3 and 4
    /// are returned even when others succeeded, so 0 never means "some".
    ///
    /// Example:
    ///   stegobench score --corpus ./pentimento-nano --detector zsteg
    ///
    /// Example, every registered detector at once:
    ///   stegobench score --corpus ./pentimento-nano --detector all --out ./results
    Score {
        /// A directory of samples: images with a JSON record beside each.
        #[arg(long, value_name = "DIR")]
        corpus: std::path::PathBuf,
        /// Which registered detector to ask. See `stegobench list detectors`.
        ///
        /// Repeatable, and `all` means every registered detector. With more
        /// than one, --out and --records name DIRECTORIES rather than files
        /// and each detector gets its own file inside them.
        #[arg(long, value_name = "NAME", num_args = 1.., required = true)]
        detector: Vec<String>,
        /// Which registered corpus the directory holds. See
        /// `stegobench list corpora`.
        ///
        /// The run is marked `named` only if that entry declares a records
        /// digest and this directory matches it. Otherwise it is `custom`.
        #[arg(long, value_name = "ID")]
        corpus_id: Option<String>,
        /// The corpus this detector was trained on, if it was trained at all.
        ///
        /// Recorded in the result as a declaration, because a detector scored
        /// on what it trained on is not being measured, and nothing here can
        /// tell from the outside. Naming the corpus being scored is allowed
        /// and says so loudly in the output: it is a real thing to do while
        /// developing and a bad number to quote.
        #[arg(long, value_name = "ID")]
        trained_on: Option<String>,
        /// Where the per-item answers are kept, and where a resumed run reads
        /// what is already done.
        ///
        /// Defaults to <corpus>.<detector>.records.jsonl beside the corpus.
        /// The detector's name is in it deliberately: two detectors sharing
        /// one records file resume from each other's answers.
        ///
        /// With more than one detector this is a DIRECTORY, and each
        /// detector's records go in <dir>/<detector>.records.jsonl.
        #[arg(long, value_name = "FILE")]
        records: Option<std::path::PathBuf>,
        /// Where to write the result-v1 document. Defaults to stdout.
        ///
        /// With more than one detector this is a DIRECTORY, created if it is
        /// not there, and each document is written to <dir>/<detector>.json.
        /// With no --out they go to <corpus>.results/ beside the corpus.
        #[arg(long, value_name = "FILE")]
        out: Option<std::path::PathBuf>,
        /// Seconds any single image is given before the detector is killed and
        /// that item is recorded as an error.
        #[arg(long, value_name = "SECONDS", default_value = "60")]
        timeout: u64,
        /// Score at most this many items, for a smoke test.
        ///
        /// Marks the result `custom`: a prefix of a corpus is not the corpus,
        /// so the figure cannot be quoted as a tier number.
        #[arg(long, value_name = "N")]
        limit: Option<u64>,
    },

    /// Turn scores and labels into detection metrics
    ///
    /// The one implementation of these numbers in this project. `score` uses
    /// it, and so can anything else that can start a process and write JSON,
    /// which is what keeps a second copy of the arithmetic from growing
    /// somewhere else and quietly disagreeing.
    ///
    /// The input is a JSON object of scores and labels, read from a file or
    /// from standard input, so a few hundred thousand answers never have to
    /// fit on a command line:
    ///
    ///   {"scores": [0.91, 0.02], "labels": [true, false]}
    ///
    /// A score higher means more like stego. A label of true means the image
    /// really does hide something. An image the detector could not score is
    /// written null, and is refused by name rather than counted as a zero.
    ///
    /// EXIT CODES here: 2 the input is not scores and labels, or a budget is
    /// not a rate; 3 the numbers are well formed and cannot be ranked, which
    /// is a refusal and will refuse again.
    ///
    /// Example:
    ///   stegobench metrics scores.json --at 0.01 --at 0.10 --json
    Metrics {
        /// A JSON file of scores and labels. Left out, or given as `-`, reads
        /// standard input.
        #[arg(value_name = "FILE")]
        file: Option<std::path::PathBuf>,
        /// A false-alarm budget to report the detection rate at, as a fraction
        /// between 0 and 1. Repeatable; 0.01 is one clean image in a hundred
        /// wrongly flagged.
        // `allow_hyphen_values` so a negative reaches this command's own
        // refusal rather than clap's "unexpected argument". A budget of -0.1
        // is a caller's mistake and deserves the message that says what a
        // budget is, not the message for a misspelled flag.
        #[arg(
            long = "at",
            value_name = "RATE",
            allow_hyphen_values = true,
            default_values = ["0.01", "0.05", "0.10"]
        )]
        at: Vec<String>,
    },

    /// Download one tier of a registered corpus, and check what arrives
    ///
    /// The registry declares the URL, the SHA-256 and the exact size before
    /// anything is downloaded, and the bytes are checked against all three.
    /// Anything else is thrown away rather than kept.
    ///
    /// A corpus whose terms do not permit redistribution is refused before a
    /// connection opens, because fetching somebody else's dataset for you would
    /// make this project the mirror. `stegobench describe <id>` prints how to
    /// obtain those yourself.
    ///
    /// Interrupting it is safe. The partly downloaded bytes are kept and the
    /// next run continues from them; nothing that looks complete is ever left
    /// behind half written.
    ///
    /// It does not unpack. It reports the verified file and what it is.
    ///
    /// EXIT CODES here: 2 the tier is not one this corpus declares, or the
    /// route is larger than --max-bytes allows; 3 there is nothing to fetch;
    /// 5 what arrived is not what the registry declared; 7 the terms say no;
    /// 8 curl is not on PATH.
    ///
    /// Whether an id can be fetched at all is a property of the registry
    /// rather than of this command. `stegobench describe <id>` says how that
    /// corpus is obtained, and this refuses with exit 3 for one that names no
    /// download route. At the time of writing none of the registered corpora
    /// declares one, so expect that refusal and follow what `describe` says.
    ///
    /// Example:
    ///   stegobench fetch <corpus> --tier nano
    Fetch {
        /// A corpus id as `stegobench list corpora` prints it.
        #[arg(value_name = "CORPUS")]
        corpus: String,
        /// Which tier to fetch: the vocabulary the corpus publishes, such as
        /// nano, lite or core. `stegobench describe <id>` lists the ones it
        /// declares a route for.
        #[arg(long, value_name = "TIER")]
        tier: String,
        /// Where verified bytes are kept, laid out by content address.
        ///
        /// Defaults to a directory under your user data directory, so the same
        /// tier fetched from two working directories is downloaded once.
        ///
        /// Setting STEGOBENCH_CORPUS_DIR does the same thing for every fetch.
        /// An empty value counts as not set.
        // Not clap's `env`, for the reason given on `--registry`.
        #[arg(long, value_name = "DIR")]
        dest: Option<std::path::PathBuf>,
        /// Refuse a route that declares more bytes than this.
        ///
        /// Checked against the size the REGISTRY declares, before anything
        /// opens, so a tier larger than you meant to fetch costs nothing.
        #[arg(long, value_name = "BYTES")]
        max_bytes: Option<u64>,
        /// Wall-clock ceiling for the whole download, in minutes.
        ///
        /// The default fits the largest published tier over an ordinary
        /// connection. Fetching a small one, set something small: a budget
        /// sized for the worst case never fires for the ordinary one.
        #[arg(long, value_name = "MINUTES", default_value = "720")]
        budget_minutes: u64,
    },

    /// Re-check a result against the corpus it says it measured
    ///
    /// A result names the bytes it was measured on by digest. This recomputes
    /// that digest from a corpus on disk and says whether the two agree.
    ///
    /// Exits 5 when they disagree: the document and the corpus are not about
    /// each other, whatever either one is called.
    ///
    /// Example:
    ///   stegobench verify result.json --corpus ./pentimento-nano
    Verify {
        /// A result-v1 document.
        #[arg(value_name = "FILE")]
        file: std::path::PathBuf,
        /// The corpus to check it against.
        #[arg(long, value_name = "DIR")]
        corpus: std::path::PathBuf,
    },

    /// Turn result documents into a table a person can put in a report
    ///
    /// Takes result files, directories of them, or both. Every row carries the
    /// conditions the number was measured under, so a figure cannot be lifted
    /// out without them.
    ///
    /// Results over different corpora never share a table, nor do `custom` and
    /// `named` runs. Rows are ordered by arm then detector, never by score.
    ///
    /// An invalid document is named with its reason at the top and the command
    /// exits non-zero, so a short table cannot pass as a whole one.
    ///
    /// Example:
    ///   stegobench report results/v1 --format markdown --out results.md
    Report {
        /// Result documents, or directories holding them.
        #[arg(value_name = "PATH", num_args = 1..)]
        paths: Vec<std::path::PathBuf>,
        /// text, markdown or csv.
        ///
        /// The default does not change when stdout is redirected: two runs on
        /// the same input produce the same bytes.
        #[arg(long, value_name = "FORMAT", default_value = "text")]
        format: ReportFormat,
        /// Where to write it. Defaults to stdout, and a file is written by
        /// rename-on-close so a reader never opens half a table.
        #[arg(long, value_name = "FILE")]
        out: Option<std::path::PathBuf>,
    },

    /// Emit a shell completion script
    ///
    /// Example:
    ///   stegobench completions bash > /etc/bash_completion.d/stegobench
    Completions {
        /// bash, zsh, fish, or powershell.
        #[arg(value_name = "SHELL")]
        shell: clap_complete::Shell,
    },

    /// Conceptual documentation that does not belong on a flag
    ///
    /// Example:
    ///   stegobench help pairing
    Help {
        /// One of: scope, pairing, splits, licences, plugins, results,
        /// reports. Omit to list.
        #[arg(value_name = "TOPIC")]
        topic: Option<String>,
    },

    /// Not a command. `check` is one of the two words somebody looking for an
    /// image examiner guesses, and it is routed here so the refusal can
    /// explain the difference rather than leave clap to suggest `schema`.
    ///
    /// Hidden because it is a signpost rather than a feature; listing it would
    /// imply the tool does the thing the signpost exists to say it does not.
    #[command(hide = true)]
    Check {
        #[arg(value_name = "ARGS", trailing_var_arg = true, num_args = 0..)]
        args: Vec<String>,
    },

    /// Not a command. The other guess. See `check`.
    #[command(hide = true)]
    Scan {
        #[arg(value_name = "ARGS", trailing_var_arg = true, num_args = 0..)]
        args: Vec<String>,
    },
}
