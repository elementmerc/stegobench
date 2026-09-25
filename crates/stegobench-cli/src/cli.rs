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

use clap::{Parser, Subcommand};

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
    about = "A reproducible benchmark for image steganalysis",
    long_about = "Build a labelled corpus, run detectors over identical bytes, \
                  and report numbers somebody else can check.\n\n\
                  Machine-readable output goes to stdout with --json; progress \
                  and diagnostics go to stderr, so the two can be separated.\n\n\
                  EXIT CODES: 0 success, 1 generic failure, 2 usage error, \
                  3 pre-flight refusal, 4 plugin failure, 5 verification \
                  mismatch, 6 schema invalid, 7 licence refusal, \
                  8 environment unfit, 130 interrupted. Codes 3 and 7 are the \
                  tool refusing something it is capable of, distinct from an \
                  ordinary error, so a script or an agent can tell not to retry."
)]
pub struct Cli {
    /// Machine-readable output on stdout. Accepted by every subcommand.
    #[arg(long, global = true)]
    pub json: bool,

    /// Where the tool registry lives.
    #[arg(
        long,
        global = true,
        value_name = "DIR",
        env = "STEGOBENCH_REGISTRY",
        default_value = "plugins/registry"
    )]
    pub registry: std::path::PathBuf,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Print a published schema, generated from the types the tool writes
    ///
    /// The schema is not maintained by hand beside the code; it is derived
    /// from it, so a document that validates is one this version can read.
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
    /// only the first, because fixing them one round trip at a time is how a
    /// format gets a reputation for being fussy.
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
    /// Generated from the registry, so what it prints is what the tool will
    /// actually run. A README goes stale; this cannot.
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
    /// Takes a tool name or a corpus id: one vocabulary, whichever kind of
    /// thing it names.
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
    /// Example:
    ///   stegobench plan score --corpus pentimento-core
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
    ///
    /// Example:
    ///   stegobench doctor
    Doctor {
        /// Where the self-test fixtures live.
        #[arg(long, value_name = "DIR", default_value = "fixtures")]
        fixtures: std::path::PathBuf,
        /// Skip the self-tests and only report what is installed. Faster, and
        /// honest about being weaker: it cannot tell a working tool from a
        /// broken one.
        #[arg(long)]
        no_selftest: bool,
    },

    /// Score a corpus with a detector
    ///
    /// Reads every sample under --corpus, asks the detector about each one,
    /// and writes a result-v1 document naming the exact bytes it measured.
    ///
    /// The run is resumable. Every answer is written to the records file as it
    /// is produced, and running the same command again picks up where it
    /// stopped rather than starting over.
    ///
    /// Example:
    ///   stegobench score --corpus ./pentimento-nano --detector zsteg
    Score {
        /// A directory of samples: images with a JSON record beside each.
        #[arg(long, value_name = "DIR")]
        corpus: std::path::PathBuf,
        /// Which registered detector to ask. See `stegobench list detectors`.
        #[arg(long, value_name = "NAME")]
        detector: String,
        /// Where the per-item answers are kept, and where a resumed run reads
        /// what is already done. Defaults to <corpus>.records.jsonl beside the
        /// corpus.
        #[arg(long, value_name = "FILE")]
        records: Option<std::path::PathBuf>,
        /// Where to write the result-v1 document. Defaults to stdout.
        #[arg(long, value_name = "FILE")]
        out: Option<std::path::PathBuf>,
        /// Seconds any single image is given before the detector is killed and
        /// that item is recorded as an error.
        #[arg(long, value_name = "SECONDS", default_value = "60")]
        timeout: u64,
        /// Score at most this many items, for a smoke test.
        ///
        /// A run that uses this is marked `custom` in the result and cannot be
        /// quoted as a tier number, because a prefix of a corpus is not the
        /// corpus.
        #[arg(long, value_name = "N")]
        limit: Option<u64>,
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
        /// One of: pairing, splits, licences, plugins. Omit to list topics.
        #[arg(value_name = "TOPIC")]
        topic: Option<String>,
    },
}
