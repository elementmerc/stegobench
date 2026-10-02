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

/// A per-image deadline that can actually bound something.
///
/// Zero parsed and meant "kill it before it can answer": every item recorded
/// a timeout and the run produced a document full of errors under exit zero.
/// It is refused rather than redefined as "no timeout", because the runner
/// below has no way to express an unbounded wait and inventing one here would
/// be a flag that lies about what happens.
pub fn positive_seconds(text: &str) -> Result<u64, String> {
    match text.parse::<u64>() {
        Ok(0) => Err("0 would kill the detector before it could answer. \
                      Give the number of seconds one image is worth; there is \
                      no value meaning no timeout."
            .to_string()),
        Ok(n) => Ok(n),
        Err(_) => Err(format!("{text:?} is not a whole number of seconds")),
    }
}

/// Workers must be at least one, and a cap keeps a typo from felling the box.
///
/// The ceiling is deliberately generous rather than tied to the core count:
/// a container-backed detector spends most of its wall clock waiting rather
/// than computing, so more workers than cores is a reasonable thing to ask
/// for. What it stops is the slipped digit, where 8 becomes 800 and the
/// machine spends its afternoon out-of-memory killing its own workers.
pub fn positive_jobs(text: &str) -> Result<usize, String> {
    const CEILING: usize = 256;
    match text.parse::<usize>() {
        Ok(0) => Err("0 workers would score nothing. Give 1 for one image at \
                      a time, or more to run several at once."
            .to_string()),
        Ok(n) if n > CEILING => Err(format!(
            "{n} workers is beyond the {CEILING} this accepts. Each one starts \
             its own container or process, so a number this size is usually a \
             slipped digit."
        )),
        Ok(n) => Ok(n),
        Err(_) => Err(format!("{text:?} is not a whole number of workers")),
    }
}

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
    // The URL rather than the bare number, because `--version` is where
    // somebody who installed this from crates.io goes looking for where it
    // came from, and there was nowhere in the binary that said. Taken from the
    // manifest so it cannot drift from what crates.io shows.
    version = concat!(env!("CARGO_PKG_VERSION"), "\n", env!("CARGO_PKG_REPOSITORY")),
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
                  IT WILL ALSO RUN THOSE DETECTORS OVER IMAGES OF YOUR OWN, \
                  with `examine`. That answers the opposite question and it is \
                  not a measurement: your images carry no labels, so there is \
                  nothing a detector can be right or wrong about and nothing \
                  in the output is quotable. `stegobench help scope` has the \
                  difference in full.\n\n\
                  START HERE\n  \
                  stegobench list detectors   what this installation can run\n  \
                  stegobench examine <image>  what the detectors say about a \
                  file\n  \
                  stegobench doctor           what is installed, and what it \
                  needs\n  \
                  stegobench help             the reasoning, one topic at a time",
    after_help = concat!(
        "Start with `stegobench list detectors`, then `stegobench doctor`.\n\
         `score` measures a detector against labelled images; `examine` says \
         what the detectors make of your own, which is not a measurement \
         (`stegobench help scope`).\n\
         Exit codes and the stdout/stderr contract are under `--help`.\n\
         Source and issues: ",
        env!("CARGO_PKG_REPOSITORY")
    ),
    after_long_help = concat!(
        "OUTPUT STREAMS\n  \
                  Content goes to stdout: a listing, a table, a schema, a \
                  completion script, a help topic, `doctor`'s report, and \
                  everything `--json` writes. Progress, warnings and refusals \
                  go to stderr, so a pipe carries the answer and nothing \
                  else.\n\n\
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
         used as given. `stegobench doctor` prints which one answered.\n\n\
         SOURCE AND ISSUES\n  ",
        env!("CARGO_PKG_REPOSITORY")
    )
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
    /// Takes a tool name or a corpus id, never a path. A directory of samples
    /// on this machine is not a registered thing and has nothing to describe;
    /// `stegobench score --corpus <dir>` is what points at one of those.
    ///
    /// Prints a summary written for a reader. `--toml` prints the registered
    /// entry verbatim instead, and nothing else, so it can be piped.
    ///
    /// Example:
    ///   stegobench describe steghide
    ///   stegobench describe reveal --toml
    Describe {
        /// A tool name or a corpus id as `list` prints it. Not a path.
        #[arg(value_name = "NAME")]
        name: String,
        /// Print the registered entry as TOML, and nothing else.
        ///
        /// The whole entry, byte for byte as the registry holds it, with no
        /// summary above it and no advice below it, so a pipe receives one
        /// document rather than a document with prose stapled to it.
        #[arg(long)]
        toml: bool,
    },

    /// Hide a payload in one image, with a registered embedder
    ///
    /// The other half of the registry. `list embedders` has always shown six
    /// tools and, until this existed, offered no way to run one: they could
    /// be read about and self-tested and not used.
    ///
    /// This is for making one stego image: a demonstration, a test case, a
    /// fixture to poke a detector with. It is NOT how a corpus is built. A
    /// corpus needs a cover and its stego twin written from the same source
    /// through the same code path, and that is what `generators/` does; doing
    /// it a file at a time is how the pairing rule gets broken.
    ///
    /// The tool runs the way every other plugin here runs: a container with
    /// no network and no capabilities, or a local program you installed
    /// yourself, whichever the registry declares.
    ///
    /// By default the payload is extracted again and compared byte for byte,
    /// because an embedder that exits cleanly having written an image the
    /// payload is not in is the failure that produces covers labelled stego.
    ///
    /// EXIT CODES here: 2 the name is not a registered embedder, or it is not
    /// an embedder at all; 3 the cover or the payload is unreadable, or the
    /// entry declares no way to embed; 4 the tool failed or wrote nothing;
    /// 5 the payload did not survive the round trip.
    ///
    /// Example:
    ///   stegobench embed --embedder steghide --cover in.jpg --payload secret.txt --out hidden.jpg
    Embed {
        /// Which registered embedder to use. See `stegobench list embedders`.
        #[arg(long, value_name = "NAME")]
        embedder: String,
        /// The image to hide the payload in. Left untouched.
        #[arg(long, value_name = "FILE")]
        cover: std::path::PathBuf,
        /// The file to hide.
        #[arg(long, value_name = "FILE")]
        payload: std::path::PathBuf,
        /// Where to write the stego image.
        #[arg(long, value_name = "FILE")]
        out: std::path::PathBuf,
        /// The passphrase, for a tool that wants one.
        ///
        /// Defaults to the one the registry entry declares, which is a fixed
        /// public string rather than a secret: it exists so the self-test is
        /// reproducible. Set this when the stego image is for anything more
        /// than a demonstration.
        #[arg(long, value_name = "TEXT")]
        passphrase: Option<String>,
        /// Skip extracting the payload again to check it survived.
        ///
        /// Faster, and it gives up the only evidence that the embedder did
        /// what it said. Worth it for a tool you have already checked and a
        /// batch you are timing; not worth it once.
        #[arg(long)]
        no_verify: bool,
    },

    /// Estimate what a run would cost, without running anything
    ///
    /// The command is typed exactly as you would run it, flags and all, so
    /// anything `score` requires is required here too.
    ///
    /// WHERE THE RATE COMES FROM, AND WHY THE TOTAL IS A FLOOR
    ///
    /// Every per-image rate is a number whoever registered the tool wrote
    /// down. Nothing here has ever timed a detector, so the totals are
    /// arithmetic over a declaration. The rate also covers the work on an
    /// image and not the cost of starting the tool once per image, and for a
    /// container that start-up dominates: measured on 2026-09-30, an estimate
    /// of about 3 minutes against roughly 11 minutes on the same machine.
    ///
    /// So read the total as a floor and an order of magnitude. "Worst case"
    /// is the other end, and it is a ceiling rather than a forecast: every
    /// item hitting the timeout deadline, which a real run will not do.
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
    /// Exits 8 when nothing here is usable, or an installed tool fails its
    /// own self-test. A tool you have not installed is reported and is not a
    /// fault. `--strict` fails on that too.
    ///
    /// THE WORDS IN THE SELF-TEST SUMMARY
    ///   passed     flagged the planted signal and cleared the clean image
    ///   responded  ran and answered, without settling either fixture, so
    ///              nothing was proved about its accuracy
    ///   failed     got one of the two wrong
    ///   not run    was not asked, and its line says why
    ///
    /// Example:
    ///   stegobench doctor
    Doctor {
        /// Where the self-test fixtures live: the two known images every
        /// self-test is run against, one with a planted signal and one clean.
        ///
        /// Defaults to the checkout, then beside the executable, then your
        /// data directories, then the copy compiled into this binary. The
        /// first line of `doctor` names the directory that answered.
        ///
        /// Setting STEGOBENCH_FIXTURES does the same thing. An empty value
        /// counts as not set.
        // Not clap's `env`, for the reason given on `--registry`.
        #[arg(long, value_name = "DIR")]
        fixtures: Option<std::path::PathBuf>,
        /// Ask each registry whether the pinned images are actually there.
        ///
        /// A digest names bytes exactly and says nothing about where they
        /// are. An image built on this machine and pushed nowhere pins the
        /// run perfectly and reproduces for nobody, and a result document
        /// looks identical either way. This is the check that tells the two
        /// apart, and it is the one to run before quoting a number to
        /// somebody who will want to repeat it.
        ///
        /// Needs a network, so it is off by default: scoring runs sandboxed
        /// with none. A tool it cannot ask is reported as not asked rather
        /// than as absent.
        #[arg(long)]
        registry_reach: bool,
        /// Report what is installed without running the self-tests. Faster,
        /// and cannot tell a working tool from a broken one.
        ///
        /// The verdict is then about what is INSTALLED, because nothing was
        /// proved: a machine with a tool on it is fit, and exit 8 is reserved
        /// for one with none.
        #[arg(long)]
        no_selftest: bool,
        /// Fail unless every registered tool is installed and working.
        ///
        /// Without this, a tool you have not installed is reported and is
        /// not a failure, because you can measure with the ones you have.
        /// Exit 8 is then reserved for a machine that can measure nothing,
        /// and for a tool that is installed and fails its own self-test.
        ///
        /// With it, anything missing or undetermined fails too, which is
        /// what a release gate wants and what a person at a terminal does
        /// not.
        #[arg(long)]
        strict: bool,
    },

    /// Ask one detector or several about images of your own
    ///
    /// Runs each registered detector over each image you name and prints one
    /// row per image and one column per detector, so several tools can be
    /// compared on the same files without installing or invoking any of them
    /// yourself. A container detector runs in the same sandbox `score` uses:
    /// no network, no capabilities, the image mounted read only.
    ///
    /// THIS IS NOT A MEASUREMENT, AND THE DIFFERENCE MATTERS
    ///
    /// Nothing you name here is labelled, so there is no accuracy to report
    /// and no result document is written. A cell says what one tool said
    /// about one file. It is not an accuracy, it is not comparable between
    /// columns, and it is not a figure to quote. `stegobench help scope` has
    /// the argument in full, including why a detector that is right nine
    /// times in ten still raises a hundred false alarms over a thousand
    /// holiday photos. `stegobench score` is what produces a defensible
    /// number.
    ///
    /// EXIT CODES here: 0 every detector answered about every image; 2 a name
    /// is not a registered detector, or is registered and is not one; 3 an
    /// image is unreadable, or some but not all of the detectors are
    /// installed here; 4 a detector ran and answered nothing; 8 none of the
    /// detectors asked for is installed, so the table is empty. 3 and 8 are
    /// told apart because a partial answer and no answer are different things
    /// to act on, and `stegobench doctor` says what is missing.
    ///
    /// Example:
    ///   stegobench examine photo.png --detector zsteg
    ///
    /// Example, three tools over a folder of photographs:
    ///   stegobench examine ./holiday --detector zsteg --detector stegexpose --detector aletheia-spa
    ///
    /// `check`, `scan`, `inspect`, `detect` and `analyse` all run this. They
    /// are the words people guess, and before this command existed they were
    /// signposts that explained the tool could not do it.
    #[command(
        alias = "check",
        alias = "scan",
        alias = "inspect",
        alias = "detect",
        alias = "analyse",
        alias = "analyze"
    )]
    Examine {
        /// The images to ask about. Left untouched.
        ///
        /// A directory stands for the image files directly inside it, in
        /// sorted order, and nothing from any subdirectory. Naming a file and
        /// the directory holding it asks about it once.
        #[arg(value_name = "IMAGE", num_args = 1.., required = true)]
        images: Vec<std::path::PathBuf>,
        /// Which registered detector to ask. See `stegobench list detectors`.
        ///
        /// Repeatable. Every detector sees every image, which is what makes
        /// the columns comparable as answers even though their scales are not
        /// comparable as numbers.
        ///
        // Not marked required, although one is: clap's own message for a
        // missing required flag names the flag and not one value that would
        // satisfy it, and a reader met that message with no idea which of
        // seven detectors to name. The refusal is ours so that it can answer
        // that.
        //
        // A `//` COMMENT RATHER THAN A `///` ONE, DELIBERATELY. Clap renders
        // doc comments into `--help`, so this paragraph was being printed to
        // users, internal process vocabulary and all, and a forensic analyst
        // reading the help reported it as this project's private language
        // showing through. Rationale for a decision belongs where maintainers
        // read it; `--help` gets what the reader needs to type.
        #[arg(short = 'd', long = "detector", value_name = "NAME", num_args = 1..)]
        detectors: Vec<String>,
        /// Seconds one image gets before the detector is killed.
        #[arg(long, value_name = "SECONDS", default_value = "120")]
        timeout: u64,
        /// How many images to ask about at once.
        #[arg(short = 'j', long, value_name = "N", default_value = "1")]
        jobs: usize,
        /// Keep what the tools printed, which is where a failure explains
        /// itself and where a verdict's own evidence lives.
        #[arg(long, value_name = "FILE")]
        raw: Option<std::path::PathBuf>,
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
        ///
        /// A PATH, never a registered id. An id names a dataset somebody
        /// publishes; this names the bytes on this machine. Get those with
        /// `stegobench fetch <id> --tier <tier>`, which writes the starter
        /// corpus out as a directory and leaves a downloaded archive for you
        /// to unpack, then name the directory here and the id under
        /// --corpus-id.
        #[arg(short = 'c', long, value_name = "DIR")]
        corpus: std::path::PathBuf,
        /// Which registered detector to ask. See `stegobench list detectors`.
        ///
        /// Repeatable, and `all` means every registered detector. With more
        /// than one, --out and --records name DIRECTORIES rather than files
        /// and each detector gets its own file inside them.
        #[arg(short = 'd', long, value_name = "NAME", num_args = 1.., required = true)]
        detector: Vec<String>,
        /// Score only one half of the train and test split.
        ///
        /// `test` is the one a trained detector's number has to come from.
        /// Without this every run covers train and test together, which is
        /// harmless for a detector that learned nothing and makes the figure
        /// unquotable for one that did, while `split_discipline: by-cover`
        /// in the document still reads as though a held-out set was used.
        ///
        /// A stego image takes its cover's side, because the split is a
        /// property of the cover. Refused on a corpus whose records carry no
        /// split, and on one where the half you asked for has only clean or
        /// only stego images in it.
        #[arg(long, value_name = "SIDE", value_parser = ["train", "test"])]
        split: Option<String>,
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
        ///
        /// Refused when it lands inside the corpus: a file written there
        /// joins the corpus and the next run measures a different set.
        #[arg(long, value_name = "FILE")]
        records: Option<std::path::PathBuf>,
        /// Where to write the result-v1 document.
        ///
        /// With no --out it goes to <corpus>.results/<detector>.json beside
        /// the corpus, whether you named one detector or several. The run
        /// says where it put it.
        ///
        /// A path that is already a directory is treated as one, and each
        /// document is written to <dir>/<detector>.json. Naming several
        /// detectors and a single file is refused before anything runs.
        ///
        /// Refused when it lands inside the corpus: a file written there
        /// joins the corpus and the next run measures a different set.
        #[arg(short = 'o', long, value_name = "FILE")]
        out: Option<std::path::PathBuf>,
        /// Seconds any single image is given before the detector is killed and
        /// that item is recorded as an error.
        ///
        /// At least 1. There is no value meaning "no timeout": a detector
        /// that never answers would hang the run for ever, and nothing under
        /// this flag can tell that apart from one that is merely slow.
        #[arg(
            long,
            value_name = "SECONDS",
            default_value = "60",
            value_parser = positive_seconds
        )]
        timeout: u64,
        /// How many images to score at once. Defaults to 1, one at a time.
        ///
        /// A run is serial unless you ask for otherwise, because the cost of
        /// getting this wrong is paid by the detector rather than by us: each
        /// worker starts its own container or process, and several of them
        /// competing for one machine's memory can make a tool fail in ways
        /// that look like a detection result rather than a resource problem.
        ///
        /// Records are written in corpus order whatever this is set to, so a
        /// parallel run produces the same file a serial one would and can be
        /// interrupted and resumed the same way.
        ///
        /// Start low and watch the machine. A number above the core count
        /// usually buys nothing: on 2026-09-17, 128 workers on 16 cores
        /// produced 298 out-of-memory kills for a 1.2x speedup.
        #[arg(
            long,
            value_name = "N",
            default_value = "1",
            value_parser = positive_jobs
        )]
        jobs: usize,
        /// The seed your detector was run with, recorded as your
        /// declaration.
        ///
        /// Nothing in this harness is random, so this is not a seed it uses:
        /// it is the one YOU set inside your plugin, written into the
        /// document because a learned detector with a sampling step is not
        /// reproducible without it. Leave it out if your plugin is
        /// deterministic; absent is an answer rather than an omission.
        ///
        /// Nothing checks the value. What catches a wrong one is somebody
        /// re-running the measurement and getting a different number.
        #[arg(long, value_name = "N")]
        seed: Option<u64>,
        /// Keep what the detector printed, for every image rather than only
        /// the ones it could not be read on.
        ///
        /// The output of an image the harness could not read an answer from
        /// is kept either way, because a detector that found nothing and an
        /// adapter misreading its output produce the same record and only the
        /// tool's own words tell them apart. This flag widens that to
        /// everything, which is what you want while writing an adapter.
        ///
        /// It goes beside the records, in <records>.raw.jsonl, one line per
        /// image. Expect it to be several times the size of the records file.
        #[arg(long)]
        keep_raw: bool,
        /// Score the first this many items, for a smoke test.
        ///
        /// A PREFIX in corpus order, not a sample spread across the corpus.
        /// A records file resumes by position, so the item at position 4 has
        /// to be the same image whatever limit was given, and a sample that
        /// moved with N would make every earlier records file unresumable.
        ///
        /// Corpora list their covers before their stego arms, so a small
        /// limit reaches clean images only and the run is refused: a
        /// measurement needs both sides. Raise it past the covers, or use
        /// `--split test` to score a smaller whole.
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
    /// A downloaded archive is not unpacked: the verified file is reported
    /// and left as it arrived. The starter corpus is the exception, because
    /// it is carried inside this binary rather than downloaded, and it is
    /// written out as a directory ready to score.
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
        ///
        /// An ID, never a path. This command exists to GET bytes you do not
        /// have; a directory you already hold is scored directly with
        /// `stegobench score --corpus <dir>`.
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
        //
        // `--out`, BECAUSE THREE OTHER COMMANDS ALREADY CALL IT THAT. `embed`,
        // `score` and `report` all write their output to `--out`, and this one
        // alone said `--dest`. The README and the corpus guide both wrote
        // `--out` here, which was not a typo: it was the name the rest of the
        // tree teaches, and a journey copied the README's own first command
        // and got `unexpected argument '--out' found` with exit 2 before it
        // had done anything else. `--dest` keeps working, unadvertised, so
        // nothing written against the old name breaks.
        #[arg(long = "out", alias = "dest", value_name = "DIR")]
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
        ///
        /// A PATH to an unpacked corpus, never a registered id: this reads
        /// the bytes, so it needs the bytes. `stegobench fetch <id>` is how
        /// an id becomes a directory.
        #[arg(short = 'c', long, value_name = "DIR")]
        corpus: std::path::PathBuf,
        /// Also check the per-item scores against the digest the document
        /// records for them.
        ///
        /// Every metric in a result is a summary of one score per image, and
        /// those live in a separate file. This says whether the file you were
        /// given is the one this document was computed from, which is what
        /// anybody re-deriving a figure has to establish first.
        ///
        /// It does NOT say the scores are the detector's. Whoever ran the
        /// measurement wrote that file and could have written anything into
        /// it, and a digest taken afterwards agrees with whatever they wrote.
        /// `declarations.self_reported` is the field that speaks to that.
        #[arg(long, value_name = "FILE")]
        records: Option<std::path::PathBuf>,
        /// Compare the records only, without re-reading the images.
        ///
        /// The default re-reads every image and checks it against the digest
        /// its own record states, because the corpus digest is computed from
        /// what the records SAY: a stego image can be swapped for an easier
        /// one, its record left untouched, and every digest still agree. Only
        /// re-reading the bytes catches that, and it is the difference
        /// between "these bytes" and "this list of names".
        ///
        /// Use this for a corpus too large to re-read, and say which check
        /// you ran when you quote the result.
        #[arg(long)]
        shallow: bool,
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
        #[arg(short = 'o', long, value_name = "FILE")]
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
}
