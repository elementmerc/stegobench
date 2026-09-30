// Author:  Daniel Iwugo
// Comment: Christ is King
// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Daniel Iwugo
//! `stegobench report`: the last mile, where a number meets a page.
//!
//! WHAT THIS EXISTS TO STOP
//!
//! Without it, somebody building an evaluation document opens a `result-v1`
//! file, copies the AUC into a table, and leaves behind the four fields that
//! decide what the AUC is worth: which corpus, which configuration, whether
//! the pairing held, and which side of the train and test split the pairs
//! landed on. The number survives the journey to the page and the conditions
//! do not. That is the exact failure this repository was built to correct,
//! happening one step after the part that was built to correct it.
//!
//! So every row this module emits carries its conditions in the row. Not in a
//! legend, not in a heading, not in a footnote: in the cells, so a reader who
//! lifts one line out of the table takes the caveats with it whether they
//! meant to or not.
//!
//! WHAT IT REFUSES
//!
//! Three refusals, each of them a rule from `docs/leaderboard.md` rather than
//! a preference:
//!
//! 1. Results over different corpora are never put in one table. An AUC on
//!    one corpus and an AUC on another are measurements of two populations,
//!    not two scores on one scale. Two documents naming the same corpus with
//!    different digests are two corpora here, because the digest is what
//!    names the bytes and the name is only a label somebody chose.
//! 2. `custom` runs sit in their own sections, never interleaved with `named`
//!    ones, each section carrying a plain sentence saying what it is
//!    comparable with.
//! 3. Nothing is ordered by score. Rows are ordered by arm and then detector,
//!    so the table cannot be read as a ranking it was never entitled to be.
//!
//! WHERE THE MEMORY GOES
//!
//! Rows are gathered into one bounded `Vec` and sorted, rather than streamed
//! in two passes. Grouping and ordering both need every row before either can
//! be decided, and a two-pass version would re-read and re-parse every file
//! to save memory it never needed to spend: a row is a few hundred bytes and
//! the cap is [`MAX_RESULTS`], so the worst case is a few megabytes. The cap
//! is a hard refusal rather than a truncation, because a table that quietly
//! stopped at ten thousand rows is precisely the "looks complete and is not"
//! failure the rest of this module is about.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};

use stegobench_core::exit;
use stegobench_core::result::{
    ArmMetrics, Configuration, CorpusSource, Determinism, Domain, Isolation, Pairing, PinnedBy,
    PluginRef, RateUnit, Result1, SplitDiscipline,
};

use crate::cli::ReportFormat;

/// The largest a single `result-v1` document may be before this refuses to
/// read it.
///
/// The documents this repository publishes are under two kilobytes each. The
/// only field that can grow without bound is `metrics.tpr_at_fpr`, and a map
/// with a thousand false-alarm rates in it would still be a few tens of
/// kilobytes. One mebibyte is therefore roughly a five-hundredfold headroom
/// over anything a real run writes, and it stops a report over a directory
/// from loading whatever else somebody happened to leave in there.
pub const MAX_RESULT_BYTES: u64 = 1024 * 1024;

/// The largest number of result documents one report may cover.
///
/// See the module docstring for why this is a refusal rather than a
/// truncation. Ten thousand rows is already far more than anybody reads; a
/// report that needs more is a query, and the answer is to narrow the paths
/// or work from `--format csv`.
pub const MAX_RESULTS: usize = 10_000;

/// How deep a directory walk descends before refusing.
///
/// Results live one or two directories under whatever a user points at.
/// Eight levels is generous, bounded, and stops a cycle made of directory
/// links from turning a report into an infinite walk. Symbolic links to
/// directories are not followed at all, so the depth cap is a second line
/// rather than the only one.
pub const MAX_DEPTH: usize = 8;

/// How many per-arm lines one table prints before it names the rest instead.
///
/// A Core tier run breaks down into 39 arms and a report may hold thousands of
/// documents, so an uncapped breakdown is a wall of text nobody reads with the
/// main table buried somewhere above it. Two hundred lines is five such runs,
/// which is more than anybody compares by eye and far short of the wall.
///
/// This one truncates where [`MAX_RESULTS`] refuses, and the difference is
/// that nothing is lost here: the documents are all still in the table above,
/// the count of what is not shown is printed, and `--format csv` and `--json`
/// carry every arm of every document with no cap at all.
pub const MAX_ARM_LINES_SHOWN: usize = 200;

/// Why a whole report could not be produced.
///
/// A single unreadable file is NOT one of these: that is a recorded skip
/// which appears in the output (see [`Skipped`]), because dropping one
/// document silently is how a table comes to look complete while it is not.
/// These are the cases where there is no honest table to print at all.
#[derive(Debug, thiserror::Error)]
pub enum ReportError {
    #[error("cannot read {path}: {}", crate::plain_io(.source))]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    /// A path named on the command line that is not there, or cannot be
    /// opened at all.
    ///
    /// Kept apart from [`ReportError::Read`] because it is the same fault
    /// `score --corpus /nope` answers, and the two disagreed: `score` crafted
    /// an explanation and returned a refusal, while `report` handed back
    /// `No such file or directory (os error 2)` under exit 1. One kind of
    /// mistake, two exit codes and two registers.
    #[error(
        "{path} cannot be read: {why}.\n\
         `report` takes result documents, or the directories `score --out` \
         wrote them to. Check the path, and `stegobench score --help` says \
         where the documents land"
    )]
    PathUnusable { path: String, why: String },
    #[error(
        "nothing under {paths} is a result document, so there is no table to \
         print. An empty table under an exit code of zero reads as \"checked, \
         nothing to worry about\", which is a different claim from \"nothing \
         was found\". Point this at result-v1 files, or at a directory holding \
         them"
    )]
    NothingFound { paths: String },
    #[error(
        "{found} result documents were found under {paths} and this refuses \
         past {cap}. A table nobody reads is not the problem; a table that \
         quietly stopped short would be. Narrow the paths, or ask for \
         --format csv over smaller batches"
    )]
    TooMany {
        found: usize,
        cap: usize,
        paths: String,
    },
    #[error(
        "the directory tree under {path} goes deeper than {cap} levels. \
         Refusing rather than walking further: results sit one or two levels \
         down, and anything deeper is more likely a link cycle than a corpus \
         of measurements"
    )]
    TooDeep { path: String, cap: usize },
    #[error("could not write the report to {path}: {source}")]
    Write {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

impl ReportError {
    /// Which documented exit code this is.
    ///
    /// The split follows the same rule the rest of this binary uses: a
    /// refusal is something that will refuse again if retried unchanged, and
    /// a failure is something that might not.
    pub fn exit_code(&self) -> i32 {
        match self {
            // The command points somewhere with nothing in it. The fix is in
            // the command, so this is a usage error rather than a breakage.
            ReportError::NothingFound { .. } => exit::USAGE,
            // Capable of it, declining. Retrying unchanged refuses again.
            // The same code `score` returns for a `--corpus` that is not
            // there. Retrying unchanged refuses again.
            ReportError::TooMany { .. }
            | ReportError::TooDeep { .. }
            | ReportError::PathUnusable { .. } => exit::PREFLIGHT_REFUSED,
            ReportError::Read { .. } | ReportError::Write { .. } => exit::FAILURE,
        }
    }
}

/// A file that was found, could not be turned into a row, and is named in the
/// output for it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Skipped {
    pub path: String,
    pub reason: String,
    /// True where the file was found but is not a valid `result-v1`, as
    /// opposed to not being readable at all. The two exit differently.
    pub invalid_document: bool,
}

/// One measurement, flattened into the cells a table needs.
#[derive(Debug, Clone)]
pub struct Row {
    pub source: String,
    pub detector: String,
    pub subject_version: String,
    /// What names the bytes that ran: `image-digest`, `executable-hash`,
    /// `unpinned`, `mixed` where the plugins of one run disagree, or
    /// `unrecorded` where the document named none.
    pub pinned_by: &'static str,
    /// What those bytes could reach, in the same five shapes.
    pub isolation: &'static str,
    pub corpus_name: String,
    pub corpus_digest: String,
    pub corpus_source: CorpusSource,
    pub corpus_tier: Option<String>,
    pub corpus_pairs: u64,
    pub configuration: Configuration,
    pub embedder: String,
    pub rate: Option<(f64, RateUnit)>,
    pub domain: Domain,
    pub format: String,
    pub auc: f64,
    /// The 95 per cent interval, in the same cell as the figure it qualifies.
    ///
    /// In the cell rather than in a column of its own, for the reason every
    /// other condition is in a cell: a row somebody copies out of the middle
    /// of a table takes its caveats with it whether they meant to or not,
    /// and an AUC of 0.60 whose interval runs from 0.29 to 0.91 is a
    /// different claim from one that runs from 0.59 to 0.61.
    pub auc_ci95: Option<[f64; 2]>,
    pub tpr_at_fpr: BTreeMap<String, f64>,
    /// The false-alarm rate each of those figures actually came from, keyed
    /// identically. Empty for a document written before the field existed.
    pub tpr_at_fpr_achieved: BTreeMap<String, f64>,
    /// The same measurement taken again within each arm, ordered by arm name.
    ///
    /// Empty for a document whose corpus holds fewer than two named arms, and
    /// a row whose breakdown is empty renders exactly as it did before there
    /// was one: an empty column of "not applicable" on the common case is
    /// noise, and noise is what a reader learns to skip past.
    pub per_arm: Vec<ArmMetrics>,
    pub pairing: Pairing,
    pub split: SplitDiscipline,
    pub n_clean: u64,
    pub n_stego: u64,
    pub n_error: u64,
    pub trained_on: Option<String>,
    pub self_reported: bool,
    pub network_reachable: bool,
    pub harness_version: String,
    pub started_utc: String,
    /// The conditions that a reader must not be able to miss, spelled out as
    /// words rather than symbols, because a legend is something a lifted row
    /// leaves behind.
    pub flags: Vec<String>,
}

impl Row {
    /// The arm, as a person describes it out loud.
    pub fn arm(&self) -> String {
        match self.rate {
            None => self.embedder.clone(),
            Some((v, RateUnit::Bpp)) => format!("{} at {v} bpp", self.embedder),
            // Rendered as a percentage because that is how the JPEG tools are
            // driven and how every paper states it. The unit travels with the
            // number: 5% of capacity and 0.05 bits per pixel are different
            // quantities and a bare 0.05 in a table invites the comparison.
            Some((v, RateUnit::CapacityFraction)) => {
                format!("{} at {:.3}% of capacity", self.embedder, v * 100.0)
            }
        }
    }

    /// The sort key. Deliberately not the score: see the module docstring.
    fn order(&self) -> (String, String, String, String, String) {
        let rate = match self.rate {
            // Zero-padded so it sorts numerically as text, and the unit leads
            // so two units never interleave into a false ordering.
            Some((v, unit)) => format!("{}-{:016.6}", unit_str(unit), v),
            None => "0-none".to_string(),
        };
        (
            self.embedder.clone(),
            rate,
            self.detector.clone(),
            self.subject_version.clone(),
            self.source.clone(),
        )
    }
}

/// Rows that may honestly sit in one table: one corpus, by digest, and one
/// configuration.
#[derive(Debug, Clone)]
pub struct Group {
    pub corpus_name: String,
    pub corpus_digest: String,
    pub configuration: Configuration,
    pub rows: Vec<Row>,
}

impl Group {
    /// The sentence that goes under the heading, saying what these numbers
    /// may be put beside.
    pub fn comparability(&self) -> String {
        match self.configuration {
            Configuration::Named => format!(
                "NAMED: quotable beside another `named` run over this same \
                 digest of {}, and beside nothing else.",
                self.corpus_name
            ),
            Configuration::Custom => format!(
                "CUSTOM: {} declared no digest in advance. Quote these \
                 figures beside nothing else.",
                self.corpus_name
            ),
        }
    }
}

/// Everything a report renders from, already grouped, sorted and bounded.
#[derive(Debug, Clone)]
pub struct Report {
    pub groups: Vec<Group>,
    pub skipped: Vec<Skipped>,
    /// How many result documents made it into a group.
    pub read: usize,
    /// The paths the user asked for, as typed, for the preamble.
    pub asked_for: Vec<String>,
}

impl Report {
    /// Whether any figure in this report may be put beside any other.
    ///
    /// False whenever there is more than one group, which is the common case
    /// and the one a reader is most likely to get wrong.
    pub fn any_cross_group_comparison_is_valid(&self) -> bool {
        self.groups.len() <= 1
    }

    /// The exit code the command should leave with.
    ///
    /// A report that dropped a document is not a success. It still prints,
    /// because a named skip in the output is far more use than a bare error,
    /// but the code says the table is incomplete so a script cannot treat it
    /// as whole. An invalid document is a schema failure; an unreadable file
    /// is an ordinary one.
    pub fn exit_code(&self) -> i32 {
        if self.skipped.iter().any(|s| s.invalid_document) {
            exit::SCHEMA_INVALID
        } else if !self.skipped.is_empty() {
            exit::FAILURE
        } else {
            exit::OK
        }
    }
}

// ── gathering ───────────────────────────────────────────────────────────────

/// Collects every `*.json` under `paths`, in a deterministic order.
///
/// A path named on the command line is taken as given, whatever its
/// extension, because the user asked for that file by name. A directory is
/// walked for `*.json` only, so a README beside the results is not reported
/// as a broken document.
fn discover(paths: &[PathBuf]) -> Result<(Vec<PathBuf>, Vec<Skipped>), ReportError> {
    let mut found = Vec::new();
    let mut skipped = Vec::new();
    for path in paths {
        let meta = std::fs::metadata(path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied => {
                ReportError::PathUnusable {
                    path: path.display().to_string(),
                    why: crate::plain_io(&e),
                }
            }
            _ => ReportError::Read {
                path: path.display().to_string(),
                source: e,
            },
        })?;
        if meta.is_dir() {
            walk(path, 0, &mut found, &mut skipped)?;
        } else {
            found.push(path.clone());
        }
        if found.len() > MAX_RESULTS {
            return Err(ReportError::TooMany {
                found: found.len(),
                cap: MAX_RESULTS,
                paths: display_paths(paths),
            });
        }
    }
    // Sorted here, at the boundary between the filesystem and everything
    // downstream, because `read_dir` order is whatever the filesystem feels
    // like and two runs over the same directory must produce identical bytes.
    found.sort();
    found.dedup();
    Ok((found, skipped))
}

fn walk(
    dir: &Path,
    depth: usize,
    found: &mut Vec<PathBuf>,
    skipped: &mut Vec<Skipped>,
) -> Result<(), ReportError> {
    if depth >= MAX_DEPTH {
        return Err(ReportError::TooDeep {
            path: dir.display().to_string(),
            cap: MAX_DEPTH,
        });
    }
    let entries = std::fs::read_dir(dir).map_err(|e| ReportError::Read {
        path: dir.display().to_string(),
        source: e,
    })?;
    for entry in entries {
        let entry = entry.map_err(|e| ReportError::Read {
            path: dir.display().to_string(),
            source: e,
        })?;
        let path = entry.path();
        // `file_type` does not follow the link, which is the point: a link
        // pointing back up the tree would otherwise be a cycle.
        let kind = entry.file_type().map_err(|e| ReportError::Read {
            path: path.display().to_string(),
            source: e,
        })?;
        if kind.is_symlink() {
            // Recorded rather than passed over. A walk that quietly dropped a
            // linked result would produce a table short by one row with
            // nothing in the output saying so, which is the failure this
            // whole module is built around. Only links that look like they
            // were meant to be results are worth naming; a link to a README
            // is not a gap in anything.
            if path.extension().is_some_and(|e| e == "json") {
                skipped.push(Skipped {
                    path: path.display().to_string(),
                    reason: "is a symbolic link, and a directory walk does \
                             not follow links. Name it on the command line to \
                             include it"
                        .to_string(),
                    invalid_document: false,
                });
            }
            continue;
        }
        if kind.is_dir() {
            walk(&path, depth + 1, found, skipped)?;
        } else if path.extension().is_some_and(|e| e == "json") {
            found.push(path);
        }
        if found.len() > MAX_RESULTS {
            return Err(ReportError::TooMany {
                found: found.len(),
                cap: MAX_RESULTS,
                paths: dir.display().to_string(),
            });
        }
    }
    Ok(())
}

/// Reads one file into a `Result1`, or into the reason it is not one.
fn load(path: &Path) -> Result<Result1, Skipped> {
    let shown = path.display().to_string();
    let unreadable = |e: std::io::Error| Skipped {
        path: shown.clone(),
        reason: format!("cannot be read: {e}"),
        invalid_document: false,
    };
    // Bounded at the read itself rather than by asking for the size first and
    // then reading without a limit. The two-step version has a window where
    // the file grows between the question and the answer, and the whole point
    // of the cap is that no answer this gives depends on a file being the
    // size it was a moment ago.
    let file = std::fs::File::open(path).map_err(unreadable)?;
    let mut text = String::new();
    std::io::Read::read_to_string(&mut file.take(MAX_RESULT_BYTES + 1), &mut text)
        .map_err(unreadable)?;
    if text.len() as u64 > MAX_RESULT_BYTES {
        return Err(Skipped {
            path: shown,
            reason: format!(
                "is larger than the {MAX_RESULT_BYTES} byte cap this reads a \
                 result document up to. A real result-v1 document is a couple \
                 of kilobytes"
            ),
            invalid_document: false,
        });
    }
    let parsed: Result1 = serde_json::from_str(&text).map_err(|e| Skipped {
        path: shown.clone(),
        reason: format!("is not a result-v1 document: {e}"),
        invalid_document: true,
    })?;
    parsed.validate().map_err(|problems| Skipped {
        path: shown,
        reason: format!("does not validate: {}", problems.join("; ")),
        invalid_document: true,
    })?;
    Ok(parsed)
}

/// One word for a whole run, where a run can name several plugins.
///
/// Neither "mixed" nor "unrecorded" is a value the schema carries: they are
/// what a row says when the plugins disagree, or when the document named no
/// plugin at all. Both are said rather than collapsed into the commonest
/// value, because every answer here is a claim about what produced the number.
fn agreed(plugins: &[PluginRef], of: impl Fn(&PluginRef) -> &'static str) -> &'static str {
    let mut seen = plugins.iter().map(of);
    match seen.next() {
        None => "unrecorded",
        Some(first) if seen.all(|other| other == first) => first,
        Some(_) => "mixed",
    }
}

/// Turns a document into a row, working out the conditions that go beside the
/// number.
/// The plugins in one result whose determinism is the given value.
///
/// Names them rather than counting, because the reader's next question after
/// "this run is not reproducible" is always "which part of it".
fn named(r: &Result1, want: Determinism) -> String {
    let mut names: Vec<&str> = r
        .provenance
        .plugins
        .iter()
        .filter(|p| p.determinism == want)
        .map(|p| p.name.as_str())
        .collect();
    names.sort_unstable();
    names.join(", ")
}

fn to_row(source: &Path, r: Result1) -> Row {
    let pinned_by = agreed(&r.provenance.plugins, |p| match p.pinned_by {
        PinnedBy::ImageDigest => "image-digest",
        PinnedBy::ExecutableHash => "executable-hash",
        PinnedBy::Unpinned => "unpinned",
    });
    let isolation = agreed(&r.provenance.plugins, |p| match p.isolation {
        Isolation::SandboxNoNetwork => "sandbox-no-network",
        Isolation::Host => "host",
        Isolation::RemoteService => "remote-service",
        Isolation::Unstated => "unstated",
    });

    let mut flags = Vec::new();
    match r.declarations.pairing {
        Pairing::SingleVariable => {}
        // These repeat on every flagged row, so each is a label and a clause,
        // not a sentence. `stegobench help pairing` and `help splits` carry
        // the reasoning.
        Pairing::Confounded => {
            flags.push("CONFOUNDED: the pair differs in more than the payload".to_string())
        }
        Pairing::Unverified => {
            flags.push("PAIRING UNVERIFIED: nothing could be compared".to_string())
        }
    }
    if r.declarations.split_discipline == SplitDiscipline::ByFile {
        flags.push("SPLIT BY FILE: inflates every number here".to_string());
    }
    if r.metrics.n_error > 0 {
        flags.push(format!(
            "{} image(s) unscored and not counted",
            r.metrics.n_error
        ));
    }
    // Against the id as well as the display name, and case folded. The two
    // differ, and comparing only the name cleared the one person who had
    // declared the contamination precisely: `--trained-on stegobench-starter`
    // is the id `list corpora` prints, while the corpus renders as
    // `Stegobench starter corpus`, so the careful declaration read as a
    // different corpus and the flag stayed off.
    if r.declarations.trained_on.as_deref().is_some_and(|t| {
        let t = t.trim();
        t.eq_ignore_ascii_case(r.corpus.name.trim())
            || r.corpus
                .id
                .as_deref()
                .is_some_and(|id| t.eq_ignore_ascii_case(id.trim()))
    }) {
        flags.push("TRAINED ON THIS CORPUS: not being measured".to_string());
    }
    if r.declarations.self_reported {
        flags.push("self-reported, not re-run by anybody else".to_string());
    }
    if r.corpus.digest.is_empty() {
        flags.push("NO CORPUS DIGEST: nobody can check which bytes this measured".to_string());
    }
    // Named rather than blanket, and the two cases are kept apart. This
    // warning used to fire on every row in every report, because the value
    // was hardcoded rather than read from anywhere, and a warning that is
    // always on is one a reader learns to skip past. "Somebody measured this
    // tool and it varies" and "nobody has checked" are different facts and
    // only the first is a reason to distrust the number.
    let varies = named(&r, Determinism::Nondeterministic);
    if !varies.is_empty() {
        flags.push(format!(
            "NONDETERMINISTIC: two runs need not agree ({varies})"
        ));
    }
    let unchecked = named(&r, Determinism::Unstated);
    if !unchecked.is_empty() {
        flags.push(format!(
            "determinism unstated, so reproducibility is unverified ({unchecked})"
        ));
    }
    if r.provenance.network_reachable {
        flags.push("the plugins could reach the network during this run".to_string());
    }

    // By arm name and deliberately not by score, for the reason the rows
    // themselves are not ordered by score: an ordering is read as a ranking
    // whatever the prose beside it says.
    let mut per_arm = r.metrics.per_arm;
    per_arm.sort_by(|a, b| a.arm.cmp(&b.arm));

    Row {
        source: source.display().to_string(),
        detector: r.subject.name,
        subject_version: r.subject.version,
        pinned_by,
        isolation,
        corpus_name: r.corpus.name,
        corpus_digest: r.corpus.digest,
        corpus_source: r.corpus.source,
        corpus_tier: r.corpus.tier,
        corpus_pairs: r.corpus.pairs,
        configuration: r.declarations.configuration,
        embedder: r.arm.embedder,
        rate: r.arm.rate.map(|x| (x.value, x.unit)),
        domain: r.arm.domain,
        format: r.arm.format,
        auc: r.metrics.auc,
        auc_ci95: r.metrics.auc_ci95,
        tpr_at_fpr: r.metrics.tpr_at_fpr,
        tpr_at_fpr_achieved: r.metrics.tpr_at_fpr_achieved,
        per_arm,
        pairing: r.declarations.pairing,
        split: r.declarations.split_discipline,
        n_clean: r.metrics.n_clean,
        n_stego: r.metrics.n_stego,
        n_error: r.metrics.n_error,
        trained_on: r.declarations.trained_on,
        self_reported: r.declarations.self_reported,
        network_reachable: r.provenance.network_reachable,
        harness_version: r.provenance.harness_version,
        started_utc: r.provenance.started_utc,
        flags,
    }
}

/// Reads every result under `paths` and groups what it finds.
pub fn build(paths: &[PathBuf]) -> Result<Report, ReportError> {
    let (files, mut skipped) = discover(paths)?;
    let mut rows: Vec<Row> = Vec::new();

    for file in &files {
        match load(file) {
            Ok(doc) => rows.push(to_row(file, doc)),
            Err(s) => skipped.push(s),
        }
    }

    if rows.is_empty() && skipped.is_empty() {
        return Err(ReportError::NothingFound {
            paths: display_paths(paths),
        });
    }

    // Two documents naming the same corpus with two different digests are two
    // corpora here. The digest names the bytes; the name is a label somebody
    // chose, and putting the two in one table on the strength of the label is
    // exactly the comparison this refuses to make.
    let mut buckets: BTreeMap<(u8, String, String), Vec<Row>> = BTreeMap::new();
    for row in rows {
        // `named` sections lead, because they are the ones somebody else's
        // number can join. The rank is part of the key so the order is a
        // property of the sort rather than of a later shuffle.
        let rank = match row.configuration {
            Configuration::Named => 0,
            Configuration::Custom => 1,
        };
        buckets
            .entry((rank, row.corpus_name.clone(), row.corpus_digest.clone()))
            .or_default()
            .push(row);
    }

    let mut read = 0;
    let mut groups = Vec::new();
    for ((rank, corpus_name, corpus_digest), mut rows) in buckets {
        rows.sort_by_key(|r| r.order());
        mark_repeats(&mut rows);
        read += rows.len();
        groups.push(Group {
            corpus_name,
            corpus_digest,
            configuration: if rank == 0 {
                Configuration::Named
            } else {
                Configuration::Custom
            },
            rows,
        });
    }

    skipped.sort();
    Ok(Report {
        groups,
        skipped,
        read,
        asked_for: paths.iter().map(|p| p.display().to_string()).collect(),
    })
}

/// Marks the case where one group holds more than one measurement of the same
/// detector on the same arm.
///
/// Two rows that look like one thing measured twice are a reader's invitation
/// to pick the flattering one, or to assume one of them is a typo. Saying so
/// in the row costs nothing and removes the guess.
fn mark_repeats(rows: &mut [Row]) {
    let mut counts: HashMap<(String, String, String), usize> = HashMap::new();
    for row in rows.iter() {
        *counts
            .entry((row.detector.clone(), row.embedder.clone(), row.arm()))
            .or_default() += 1;
    }
    for row in rows.iter_mut() {
        let key = (row.detector.clone(), row.embedder.clone(), row.arm());
        if let Some(&n) = counts.get(&key) {
            if n > 1 {
                row.flags.push(format!(
                    "one of {n} runs of this detector on this arm in this \
                     report; they are separate measurements, not a best-of"
                ));
            }
        }
    }
}

fn display_paths(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

// ── rendering ───────────────────────────────────────────────────────────────

/// Eight hex characters of a digest, with the full value left to the heading
/// and to `--format csv`.
///
/// Abbreviated rather than dropped: a reader comparing two rows needs to see
/// at a glance that two corpora with one name are not one corpus, and eight
/// characters does that without a sixty-four character column nobody reads.
pub fn abbreviate(digest: &str) -> String {
    if digest.is_empty() {
        return "none".to_string();
    }
    // Sliced by characters rather than by bytes. A digest is hex in every
    // document this repository has ever written, and this string arrived from
    // a JSON file somebody else may have produced, so a byte slice through
    // the middle of a multi-byte character would panic on their input rather
    // than ours.
    let first_eight = |s: &str| s.chars().take(8).collect::<String>();
    match digest.split_once(':') {
        Some((algo, hex)) => {
            if hex.chars().count() > 8 {
                format!("{algo}:{}", first_eight(hex))
            } else {
                digest.to_string()
            }
        }
        None => {
            if digest.chars().count() > 8 {
                first_eight(digest)
            } else {
                digest.to_string()
            }
        }
    }
}

fn unit_str(unit: RateUnit) -> &'static str {
    match unit {
        RateUnit::Bpp => "bpp",
        RateUnit::CapacityFraction => "capacity_fraction",
    }
}

fn domain_str(d: Domain) -> &'static str {
    match d {
        Domain::Spatial => "spatial",
        Domain::Jpeg => "jpeg",
        Domain::Structural => "structural",
        Domain::Mixed => "mixed",
        Domain::Unstated => "unstated",
    }
}

fn pairing_str(p: Pairing) -> &'static str {
    match p {
        Pairing::SingleVariable => "single-variable",
        Pairing::Confounded => "confounded",
        Pairing::Unverified => "unverified",
    }
}

fn split_str(s: SplitDiscipline) -> &'static str {
    match s {
        SplitDiscipline::ByCover => "by-cover",
        SplitDiscipline::ByFile => "by-file",
        SplitDiscipline::NotApplicable => "not-applicable",
    }
}

fn configuration_str(c: Configuration) -> &'static str {
    match c {
        Configuration::Named => "named",
        Configuration::Custom => "custom",
    }
}

fn source_str(s: CorpusSource) -> &'static str {
    match s {
        CorpusSource::Fetched => "fetched",
        CorpusSource::Supplied => "supplied",
    }
}

/// The detection rate at a given false-alarm rate, matched numerically.
///
/// The keys are decimal fractions written as text, so "0.1" and "0.10" are
/// the same rate written two ways and a string lookup would find one of them.
fn tpr_at(map: &BTreeMap<String, f64>, target: f64) -> Option<f64> {
    map.iter()
        .find(|(k, _)| k.parse::<f64>().is_ok_and(|v| (v - target).abs() < 1e-9))
        .map(|(_, v)| *v)
}

/// Whether a figure filed under `target` was measured at a lower false-alarm
/// rate than `target`, because the corpus is too coarse to express it.
///
/// With six clean images the only rates that exist are multiples of one sixth,
/// so a one per cent budget buys exactly what a zero per cent budget buys. The
/// arithmetic is right and conservative; the heading is what it cannot
/// support, and this is the column somebody acts on.
fn budget_unexpressible(row: &Row, target: f64) -> Option<f64> {
    tpr_at(&row.tpr_at_fpr_achieved, target).filter(|a| *a < target - 1e-9)
}

fn tpr_cell(row: &Row, target: f64) -> String {
    match tpr_at(&row.tpr_at_fpr, target) {
        // The achieved rate goes in the cell rather than in a footnote,
        // because the cell is what gets copied out on its own.
        Some(v) => match budget_unexpressible(row, target) {
            Some(achieved) => format!("{v:.4} at {} FA", percent(achieved)),
            None => format!("{v:.4}"),
        },
        // Not a dash and not a zero. Both read as an answer, and the truth is
        // that this run did not report that point on the curve.
        None => "not reported".to_string(),
    }
}

/// A false-alarm rate as the headings write one, so the cell and the column it
/// sits under are in the same units.
fn percent(rate: f64) -> String {
    let shown = format!("{:.2}", rate * 100.0);
    format!("{}%", shown.trim_end_matches('0').trim_end_matches('.'))
}

/// The sentence under a table holding at least one such cell.
const FA_NOTE: &str = "Where a cell names a false-alarm rate after the figure, \
                       the run was measured at that rate rather than at the one \
                       its column asks for: this corpus has too few clean \
                       images to express that budget, so the figure is real and \
                       the heading is not.";

fn any_budget_unexpressible(group: &Group) -> bool {
    group.rows.iter().any(|r| {
        FIXED_FPR_COLUMNS
            .iter()
            .any(|t| budget_unexpressible(r, *t).is_some())
    })
}

/// The two points on the curve every table carries a column for.
const FIXED_FPR_COLUMNS: [f64; 2] = [0.01, 0.10];

/// The flags, joined for a single cell.
fn flags_cell(flags: &[String]) -> String {
    if flags.is_empty() {
        // Deliberately not "ok" or "clean". Neither is a claim this can make:
        // nothing was flagged, which is a weaker statement and the true one.
        return "nothing flagged".to_string();
    }
    flags.join(" · ")
}

/// A figure with its interval where there is one, and bare where there is not.
///
/// Never an invented interval and never a placeholder in place of one: an AUC
/// of 0.60 whose interval runs from 0.29 to 0.91 is a different claim from one
/// that runs from 0.59 to 0.61, and a run that reported no interval has made
/// neither claim.
fn auc_cell(auc: f64, ci95: Option<[f64; 2]>) -> String {
    match ci95 {
        Some([lo, hi]) => format!("{auc:.4} [{lo:.3}, {hi:.3}]"),
        None => format!("{auc:.4}"),
    }
}

/// The columns every format agrees on, in order.
const HEADINGS: [&str; 12] = [
    "detector",
    "isolation",
    "corpus",
    "config",
    "arm",
    "domain",
    "AUC",
    "TPR@1%FA",
    "TPR@10%FA",
    "pairing",
    "split",
    "clean/stego/unscored",
];

fn cells(row: &Row) -> [String; 12] {
    [
        row.detector.clone(),
        row.isolation.to_string(),
        format!("{} @ {}", row.corpus_name, abbreviate(&row.corpus_digest)),
        configuration_str(row.configuration).to_string(),
        row.arm(),
        domain_str(row.domain).to_string(),
        auc_cell(row.auc, row.auc_ci95),
        tpr_cell(row, FIXED_FPR_COLUMNS[0]),
        tpr_cell(row, FIXED_FPR_COLUMNS[1]),
        pairing_str(row.pairing).to_string(),
        split_str(row.split).to_string(),
        format!("{}/{}/{}", row.n_clean, row.n_stego, row.n_error),
    ]
}

/// The columns of the per-arm breakdown.
///
/// Each line names its detector and its corpus rather than inheriting them
/// from a heading, for the reason every other row in this module does: a line
/// lifted out of the middle takes its conditions with it.
const ARM_HEADINGS: [&str; 5] = ["detector", "corpus", "arm", "AUC", "clean/stego"];

fn arm_cells(row: &Row, arm: &ArmMetrics) -> [String; 5] {
    [
        row.detector.clone(),
        format!("{} @ {}", row.corpus_name, abbreviate(&row.corpus_digest)),
        arm.arm.clone(),
        auc_cell(arm.auc, arm.auc_ci95),
        format!("{}/{}", arm.n_clean, arm.n_stego),
    ]
}

/// The sentence above a per-arm breakdown.
///
/// The reasoning is one line here and at length in `stegobench help reports`,
/// because it is the same reasoning every time and a paragraph repeated under
/// every table is a paragraph nobody reads twice.
const ARM_INTRO: &str = "The AUC above pools the arms this run covered, and a \
                         pooled figure describes none of them: chance on one \
                         arm beside detection on another averages to \
                         something in between that nothing measured.";

/// The per-arm lines for one table, and how many documents' breakdowns the cap
/// left out.
///
/// None where no document in the table carries a breakdown, which is the
/// common case and the one that must render as it did before.
///
/// Whole documents are kept or left out together, and once one is left out
/// every later one is too, so the cap cannot produce a breakdown that looks
/// complete for a document it cut in half or an order that depends on which
/// documents happened to be small.
fn arm_table(group: &Group) -> Option<(Vec<[String; 5]>, usize)> {
    let mut lines: Vec<[String; 5]> = Vec::new();
    let mut withheld = 0usize;
    let mut full = false;
    for row in group.rows.iter().filter(|r| !r.per_arm.is_empty()) {
        // The first document is printed whatever its size, so a corpus with
        // more arms than the cap shows its breakdown rather than nothing.
        let fits = lines.is_empty() || lines.len() + row.per_arm.len() <= MAX_ARM_LINES_SHOWN;
        if full || !fits {
            full = true;
            withheld += 1;
            continue;
        }
        for arm in &row.per_arm {
            lines.push(arm_cells(row, arm));
        }
    }
    if lines.is_empty() {
        return None;
    }
    Some((lines, withheld))
}

fn arm_withheld_note(withheld: usize) -> String {
    format!(
        "{withheld} further document(s) in this table carry a per-arm \
         breakdown that is not printed here, because this stops at \
         {MAX_ARM_LINES_SHOWN} lines. --format csv and --json carry every arm \
         of every document."
    )
}

/// The block naming what did not make it into the table.
///
/// Written at the TOP of the report rather than at the bottom. A reader who
/// stops after the first table would otherwise never learn that the table is
/// short, and the whole point of naming a skipped file is that it reaches the
/// person reading the number.
fn incompleteness(report: &Report) -> Option<String> {
    if report.skipped.is_empty() {
        return None;
    }
    let mut s = format!(
        "THIS REPORT IS INCOMPLETE. {} file(s) could not be turned into a \
         row:\n",
        report.skipped.len()
    );
    for skip in &report.skipped {
        // A two-space bullet, not a deeper indent: four spaces inside a
        // Markdown blockquote turn the line into a code block, which would
        // hide the reason behind a horizontal scrollbar in exactly the
        // artefact this block exists to be read in.
        let _ = writeln!(s, "  - {}: {}", skip.path, skip.reason);
    }
    Some(s)
}

fn preamble(report: &Report) -> String {
    let mut s = format!(
        "{} result document(s) from {}, in {} table(s).\n",
        report.read,
        display_list(&report.asked_for),
        report.groups.len()
    );
    // Both lines are load-bearing and both are one line. Why a corpus
    // boundary is not comparable, and why an ordering is not a ranking, live
    // in `stegobench help reports`.
    if !report.any_cross_group_comparison_is_valid() {
        s.push_str("Nothing may be read across a table boundary.\n");
    }
    s.push_str(
        "Ordered by arm then detector, never by score. Not a ranking. \
         (`stegobench help reports`)\n",
    );
    s
}

fn render_text(report: &Report) -> String {
    let mut out = String::new();
    if let Some(block) = incompleteness(report) {
        // Wrapped line by line rather than as one blob, so the indented list
        // of skipped paths keeps its shape.
        for line in block.lines() {
            let _ = writeln!(out, "{}", wrap(line, 78));
        }
        out.push('\n');
    }
    for line in preamble(report).lines() {
        let _ = writeln!(out, "{}", wrap(line, 78));
    }

    for group in &report.groups {
        let _ = write!(
            out,
            "\n{} ({})\ncorpus digest: {}\n\n{}\n\n",
            group.corpus_name,
            configuration_str(group.configuration),
            if group.corpus_digest.is_empty() {
                "none, so nobody can check which bytes these numbers came from"
            } else {
                &group.corpus_digest
            },
            wrap(&group.comparability(), 78)
        );

        let mut table: Vec<Vec<String>> = vec![HEADINGS.iter().map(|h| h.to_string()).collect()];
        for row in &group.rows {
            table.push(cells(row).to_vec());
        }
        // Two spaces rather than one, because twelve columns of mostly numbers
        // run together at a single space and the eye loses which figure sits
        // under which heading.
        for line in stegobench_core::table::align_with(&table, "  ") {
            let _ = writeln!(out, "{line}");
        }
        if any_budget_unexpressible(group) {
            let _ = writeln!(out, "\n{}", wrap(FA_NOTE, 78));
        }

        if let Some((lines, withheld)) = arm_table(group) {
            let _ = write!(out, "\n{}\n\n", wrap(&format!("Per arm. {ARM_INTRO}"), 78));
            let mut arms: Vec<Vec<String>> =
                vec![ARM_HEADINGS.iter().map(|h| h.to_string()).collect()];
            arms.extend(lines.into_iter().map(|l| l.to_vec()));
            for line in stegobench_core::table::align_with(&arms, "  ") {
                let _ = writeln!(out, "{line}");
            }
            if withheld > 0 {
                let _ = writeln!(out, "{}", wrap(&arm_withheld_note(withheld), 78));
            }
        }

        // The flags get their own lines under the table rather than a
        // thirteenth column, because they are sentences and a column would
        // force them into abbreviations a reader has to decode. Each line
        // names its own row, so it travels with it.
        // The conditions, spelled out under the table rather than squeezed
        // into a thirteenth column. Each one is a sentence, and a column
        // would force it into an abbreviation the reader has to decode from
        // a legend, which is the thing this whole command exists to avoid.
        //
        // Only the rows that have something to say appear here: a page of
        // "nothing flagged" would bury the handful of lines that matter, and
        // the count below says how many rows had nothing rather than leaving
        // the reader to subtract.
        let flagged: Vec<&Row> = group.rows.iter().filter(|r| !r.flags.is_empty()).collect();
        out.push('\n');
        if flagged.is_empty() {
            let _ = writeln!(
                out,
                "{}",
                wrap(
                    &format!(
                        "Nothing was flagged on any of the {} row(s).",
                        group.rows.len()
                    ),
                    78
                )
            );
        } else {
            let _ = writeln!(out, "Conditions, read before quoting a figure:");
            for row in &flagged {
                let _ = writeln!(
                    out,
                    "{}",
                    wrap(
                        &format!(
                            "  {} / {}: {}",
                            row.detector,
                            row.arm(),
                            flags_cell(&row.flags)
                        ),
                        78
                    )
                );
            }
            let quiet = group.rows.len() - flagged.len();
            if quiet > 0 {
                let _ = writeln!(
                    out,
                    "{}",
                    wrap(
                        &format!(
                            "  the other {quiet} row(s) in this table had \
                             nothing flagged"
                        ),
                        78
                    )
                );
            }
        }
        let _ = writeln!(
            out,
            "{}",
            wrap(
                "  --format csv carries the full digests and each row's \
                 source file.",
                78
            )
        );
    }
    out
}

fn render_markdown(report: &Report) -> String {
    let mut out = String::from("# Steganalysis results\n\n");
    if let Some(block) = incompleteness(report) {
        // A blockquote rather than a paragraph, so it survives being pasted
        // into a document with its own styling and still reads as a warning.
        for line in block.lines() {
            let _ = writeln!(out, "> {line}");
        }
        out.push('\n');
    }
    out.push_str(&preamble(report));
    out.push('\n');

    for group in &report.groups {
        let _ = write!(
            out,
            "## {} ({})\n\nCorpus digest: `{}`\n\n{}\n\n",
            group.corpus_name,
            configuration_str(group.configuration),
            if group.corpus_digest.is_empty() {
                "none"
            } else {
                &group.corpus_digest
            },
            group.comparability()
        );
        if group.corpus_digest.is_empty() {
            out.push_str(
                "This corpus carries no digest, so nobody can check which \
                 bytes these numbers came from.\n\n",
            );
        }
        let mut headings: Vec<String> = HEADINGS.iter().map(|h| h.to_string()).collect();
        headings.push("conditions".to_string());
        let _ = writeln!(out, "| {} |", headings.join(" | "));
        let _ = writeln!(
            out,
            "|{}|",
            headings.iter().map(|_| "---").collect::<Vec<_>>().join("|")
        );
        for row in &group.rows {
            let mut line: Vec<String> = cells(row).iter().map(|c| escape_md(c)).collect();
            line.push(escape_md(&flags_cell(&row.flags)));
            let _ = writeln!(out, "| {} |", line.join(" | "));
        }
        out.push('\n');
        if any_budget_unexpressible(group) {
            let _ = writeln!(out, "{FA_NOTE}\n");
        }

        if let Some((lines, withheld)) = arm_table(group) {
            let _ = write!(out, "### Per arm\n\n{ARM_INTRO}\n\n");
            let _ = writeln!(out, "| {} |", ARM_HEADINGS.join(" | "));
            let _ = writeln!(
                out,
                "|{}|",
                ARM_HEADINGS
                    .iter()
                    .map(|_| "---")
                    .collect::<Vec<_>>()
                    .join("|")
            );
            for line in lines {
                let cells: Vec<String> = line.iter().map(|c| escape_md(c)).collect();
                let _ = writeln!(out, "| {} |", cells.join(" | "));
            }
            let _ = writeln!(out);
            if withheld > 0 {
                let _ = writeln!(out, "{}\n", arm_withheld_note(withheld));
            }
        }
    }
    out
}

/// A pipe in a cell would end the cell early and silently shift every figure
/// on that row one column left.
fn escape_md(cell: &str) -> String {
    cell.replace('|', "\\|").replace('\n', " ")
}

fn render_csv(report: &Report) -> String {
    // A CSV cannot carry a paragraph, so the incompleteness travels as rows
    // of their own with a `record_type` that a consumer has to look at. A
    // comment line would be dropped by a strict parser, and the one thing
    // that must not be droppable is the fact that the table is short.
    let rates = fpr_columns(report);
    let mut columns: Vec<String> = [
        "record_type",
        "corpus_name",
        "corpus_digest",
        "corpus_source",
        "corpus_tier",
        "corpus_pairs",
        "configuration",
        "detector",
        "subject_version",
        "pinned_by",
        "isolation",
        "embedder",
        "arm_name",
        "rate_value",
        "rate_unit",
        "domain",
        "image_format",
        "auc",
        "auc_ci95_low",
        "auc_ci95_high",
    ]
    .iter()
    .map(|c| (*c).to_string())
    .collect();
    for (name, _) in &rates {
        columns.push(name.clone());
        columns.push(format!("{name}_achieved_fpr"));
    }
    columns.extend(
        [
            "pairing",
            "split_discipline",
            "n_clean",
            "n_stego",
            "n_error",
            "trained_on",
            "self_reported",
            "network_reachable",
            "harness_version",
            "started_utc",
            "conditions",
            "source_path",
        ]
        .iter()
        .map(|c| (*c).to_string()),
    );
    let mut out = String::new();
    let _ = writeln!(out, "{}", columns.join(","));

    for skip in &report.skipped {
        let mut line = vec![String::new(); columns.len()];
        line[0] = "skipped_file".into();
        line[columns.len() - 2] = skip.reason.clone();
        line[columns.len() - 1] = skip.path.clone();
        let _ = writeln!(
            out,
            "{}",
            line.iter()
                .map(|c| csv_field(c))
                .collect::<Vec<_>>()
                .join(",")
        );
    }

    for group in &report.groups {
        for row in &group.rows {
            let mut emit = |line: Vec<String>| {
                debug_assert_eq!(line.len(), columns.len());
                let _ = writeln!(
                    out,
                    "{}",
                    line.iter()
                        .map(|c| csv_field(c))
                        .collect::<Vec<_>>()
                        .join(",")
                );
            };
            emit(csv_line(row, None, &rates));
            // Straight after the document they break down, and uncapped:
            // this is the format a script reads, and a script that asked for
            // every field is not helped by a figure being left out of it.
            for arm in &row.per_arm {
                emit(csv_line(row, Some(arm), &rates));
            }
        }
    }
    out
}

/// One CSV line: the document as a whole, or one arm within it.
///
/// An arm line repeats every condition of the document it came from rather
/// than pointing back at a line above it. A spreadsheet gets sorted, and a row
/// whose conditions live in a neighbouring row loses them the first time
/// somebody clicks a column heading.
fn csv_line(row: &Row, arm: Option<&ArmMetrics>, rates: &[(String, f64)]) -> Vec<String> {
    let (rate_value, rate_unit) = match row.rate {
        Some((v, u)) => (v.to_string(), unit_str(u).to_string()),
        None => (String::new(), String::new()),
    };
    let (auc, ci95) = match arm {
        Some(a) => (a.auc, a.auc_ci95),
        None => (row.auc, row.auc_ci95),
    };
    let (ci_low, ci_high) = match ci95 {
        Some([lo, hi]) => (format!("{lo:.4}"), format!("{hi:.4}")),
        // Empty rather than a placeholder: this run reported no interval, and
        // any number here would be one nobody measured.
        None => (String::new(), String::new()),
    };
    // The curve was reported for the run rather than per arm, so an arm line
    // leaves these empty instead of repeating a figure that is about a
    // different population. An empty cell is also what a run that never
    // reported a given rate gets: the same "nobody measured this" the interval
    // columns already use, and a rate a document does not carry must not read
    // as a zero.
    let mut curve: Vec<String> = Vec::with_capacity(rates.len() * 2);
    for (_, rate) in rates {
        let (tpr, achieved) = match arm {
            Some(_) => (None, None),
            None => (
                tpr_at(&row.tpr_at_fpr, *rate),
                tpr_at(&row.tpr_at_fpr_achieved, *rate),
            ),
        };
        curve.push(tpr.map(|v| format!("{v:.4}")).unwrap_or_default());
        curve.push(achieved.map(|v| format!("{v:.4}")).unwrap_or_default());
    }

    let mut line = vec![
        match arm {
            Some(_) => "arm".to_string(),
            None => "result".to_string(),
        },
        row.corpus_name.clone(),
        row.corpus_digest.clone(),
        source_str(row.corpus_source).to_string(),
        row.corpus_tier.clone().unwrap_or_default(),
        row.corpus_pairs.to_string(),
        configuration_str(row.configuration).to_string(),
        row.detector.clone(),
        row.subject_version.clone(),
        row.pinned_by.to_string(),
        row.isolation.to_string(),
        row.embedder.clone(),
        arm.map(|a| a.arm.clone()).unwrap_or_default(),
        rate_value,
        rate_unit,
        domain_str(row.domain).to_string(),
        row.format.clone(),
        format!("{auc:.4}"),
        ci_low,
        ci_high,
    ];
    line.extend(curve);
    line.extend([
        pairing_str(row.pairing).to_string(),
        split_str(row.split).to_string(),
        match arm {
            Some(a) => a.n_clean.to_string(),
            None => row.n_clean.to_string(),
        },
        match arm {
            Some(a) => a.n_stego.to_string(),
            None => row.n_stego.to_string(),
        },
        // Unscored images are counted for the run and not attributed to an
        // arm, so an arm line leaves the cell empty rather than claiming zero.
        match arm {
            Some(_) => String::new(),
            None => row.n_error.to_string(),
        },
        row.trained_on.clone().unwrap_or_default(),
        row.self_reported.to_string(),
        row.network_reachable.to_string(),
        row.harness_version.clone(),
        row.started_utc.clone(),
        flags_cell(&row.flags),
        row.source.clone(),
    ]);
    line
}

/// One column per false-alarm rate any document in this report was scored at,
/// in numerical order.
///
/// The union across every row rather than the set the first row happened to
/// carry. Two documents scored at different budgets are ordinary, and a column
/// set taken from one of them would drop the other's figures with nothing in
/// the file saying so, which is the failure the whole module is built around.
///
/// The rate is rendered from the parsed number rather than copied from the
/// key, so "0.1" and "0.10" are one column rather than two names for one rate.
fn fpr_columns(report: &Report) -> Vec<(String, f64)> {
    let mut rates: Vec<f64> = Vec::new();
    for group in &report.groups {
        for row in &group.rows {
            // A key that is not a decimal fraction is refused by
            // `Result1::validate`, so nothing reaching here loses a column.
            for rate in row.tpr_at_fpr.keys().filter_map(|k| k.parse::<f64>().ok()) {
                if !rates.iter().any(|r| (r - rate).abs() < 1e-9) {
                    rates.push(rate);
                }
            }
        }
    }
    rates.sort_by(|a, b| a.total_cmp(b));
    rates
        .into_iter()
        .map(|rate| (format!("tpr_at_fpr_{rate}"), rate))
        .collect()
}

/// RFC 4180 quoting, written here rather than pulled in as a dependency: a
/// dozen lines of our own beats a crate in the graph for one call site.
fn csv_field(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

/// Wraps a sentence to a column, for the plain-text format.
///
/// Leading whitespace on the input is kept and repeated on every wrapped
/// line, so an indented list stays an indented list rather than collapsing
/// into the margin on its second line.
fn wrap(text: &str, width: usize) -> String {
    let indent: String = text.chars().take_while(|c| *c == ' ').collect();
    // A word longer than the column is never cut: breaking a digest or a path
    // in half would produce something that looks like a shorter digest.
    let room = width.saturating_sub(indent.chars().count()).max(1);
    let mut out = String::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > room {
            out.push_str(&indent);
            out.push_str(&line);
            out.push('\n');
            line.clear();
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        out.push_str(&indent);
        out.push_str(&line);
    }
    out
}

fn display_list(items: &[String]) -> String {
    if items.is_empty() {
        return "nowhere".to_string();
    }
    items.join(", ")
}

/// Renders the report in the requested format.
pub fn render(report: &Report, format: ReportFormat) -> String {
    match format {
        ReportFormat::Text => render_text(report),
        ReportFormat::Markdown => render_markdown(report),
        ReportFormat::Csv => render_csv(report),
    }
}

/// The report as a machine reads it.
pub fn to_json(report: &Report) -> serde_json::Value {
    serde_json::json!({
        "ok": report.skipped.is_empty(),
        "results_read": report.read,
        "files_skipped": report.skipped.len(),
        "complete": report.skipped.is_empty(),
        "cross_table_comparison_valid": report.any_cross_group_comparison_is_valid(),
        "skipped": report.skipped.iter().map(|s| serde_json::json!({
            "path": s.path,
            "reason": s.reason,
            "invalid_document": s.invalid_document,
        })).collect::<Vec<_>>(),
        "groups": report.groups.iter().map(|g| serde_json::json!({
            "corpus": { "name": g.corpus_name, "digest": g.corpus_digest },
            "configuration": configuration_str(g.configuration),
            "comparability": g.comparability(),
            "rows": g.rows.iter().map(|r| serde_json::json!({
                "source": r.source,
                "detector": r.detector,
                "subject_version": r.subject_version,
                "pinned_by": r.pinned_by,
                "isolation": r.isolation,
                "corpus": {
                    "name": r.corpus_name,
                    "digest": r.corpus_digest,
                    "digest_short": abbreviate(&r.corpus_digest),
                    "source": source_str(r.corpus_source),
                    "tier": r.corpus_tier,
                    "pairs": r.corpus_pairs,
                },
                "configuration": configuration_str(r.configuration),
                "arm": r.arm(),
                "embedder": r.embedder,
                "domain": domain_str(r.domain),
                "format": r.format,
                "auc": r.auc,
                "auc_ci95": r.auc_ci95,
                "tpr_at_fpr": r.tpr_at_fpr,
                // Always present, empty object and all, for the reason
                // `per_arm` is: a key that appears for some documents and not
                // others makes every consumer handle two shapes of one answer.
                "tpr_at_fpr_achieved": r.tpr_at_fpr_achieved,
                // Always present, empty array and all, because the key being
                // absent for most documents would make every consumer handle
                // two shapes of the same answer. The values are the schema's
                // own, so an arm that reported no interval carries no
                // `auc_ci95` key rather than a null somebody reads as zero.
                "per_arm": r.per_arm,
                "pairing": pairing_str(r.pairing),
                "split_discipline": split_str(r.split),
                "n_clean": r.n_clean,
                "n_stego": r.n_stego,
                "n_error": r.n_error,
                "trained_on": r.trained_on,
                "self_reported": r.self_reported,
                "network_reachable": r.network_reachable,
                "harness_version": r.harness_version,
                "started_utc": r.started_utc,
                "conditions": r.flags,
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    })
}

/// Writes the rendered report, complete or not at all.
///
/// Rename-on-close, per the robustness mandate: a reader who opens the path
/// sees either the previous file or the whole new one, never half a table. A
/// half-written table is the worst artefact this command could produce,
/// because it looks exactly like a complete one.
pub fn write_atomically(path: &Path, body: &str) -> Result<(), ReportError> {
    let temp = temp_path_for(path);

    let write = |temp: &Path| -> std::io::Result<()> {
        let mut file = std::fs::File::create(temp)?;
        file.write_all(body.as_bytes())?;
        // Flushed to the disk before the rename, so a crash between the two
        // cannot leave a correctly named file with nothing in it.
        file.sync_all()?;
        Ok(())
    };

    if let Err(source) = write(&temp) {
        // Never leave a .part behind, per the robustness mandate.
        let _ = std::fs::remove_file(&temp);
        return Err(ReportError::Write {
            path: path.display().to_string(),
            source,
        });
    }
    if let Err(source) = std::fs::rename(&temp, path) {
        let _ = std::fs::remove_file(&temp);
        return Err(ReportError::Write {
            path: path.display().to_string(),
            source,
        });
    }
    Ok(())
}

/// Where the half-written file lives until the rename makes it real.
///
/// Beside the destination rather than in a system temporary directory,
/// because a rename across filesystems is a copy and a copy is not atomic.
/// The process id keeps two concurrent runs writing to one destination off
/// each other's temporary file, and it never reaches the finished artefact,
/// so it costs nothing in reproducibility.
fn temp_path_for(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "report".to_string());
    let temp_name = format!(".{name}.part-{}", std::process::id());
    match path.parent().filter(|p| !p.as_os_str().is_empty()) {
        Some(dir) => dir.join(temp_name),
        // A bare file name: `Path::parent` is an empty path rather than None,
        // and joining onto it would produce a path starting with a separator.
        None => PathBuf::from(temp_name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The plain-text format wraps its prose to a column, so an assertion on
    /// a sentence has to be made against the words rather than against the
    /// line breaks a particular column happened to produce.
    fn flat(text: &str) -> String {
        text.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    fn group_named<'a>(report: &'a Report, corpus: &str) -> &'a Group {
        report
            .groups
            .iter()
            .find(|g| g.corpus_name == corpus)
            .unwrap_or_else(|| panic!("no group for {corpus}"))
    }

    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    fn shipped_results() -> PathBuf {
        repo_root().join("results/v1")
    }

    /// A result-v1 document, as JSON, with the knobs a test needs.
    struct Doc {
        detector: String,
        corpus: String,
        digest: String,
        configuration: String,
        pairing: String,
        split: String,
        embedder: String,
        auc: f64,
        n_error: u64,
        trained_on: Option<String>,
    }

    impl Default for Doc {
        fn default() -> Self {
            Doc {
                detector: "detector-a".into(),
                corpus: "corpus-a".into(),
                digest: format!("sha256:{}", "a".repeat(64)),
                configuration: "custom".into(),
                pairing: "single-variable".into(),
                split: "not-applicable".into(),
                embedder: "suniward".into(),
                auc: 0.9,
                n_error: 0,
                trained_on: None,
            }
        }
    }

    impl Doc {
        fn value(&self) -> serde_json::Value {
            let mut declarations = serde_json::json!({
                "split_discipline": self.split,
                "pairing": self.pairing,
                "configuration": self.configuration,
                "self_reported": false,
            });
            if let Some(t) = &self.trained_on {
                declarations["trained_on"] = serde_json::json!(t);
            }
            serde_json::json!({
                "schema": stegobench_core::result::RESULT_SCHEMA_ID,
                "subject": {
                    "name": self.detector,
                    "version": format!("sha256:{}", "b".repeat(64)),
                    "kind": "detector",
                },
                "corpus": {
                    "name": self.corpus,
                    "source": "supplied",
                    "digest": self.digest,
                    "pairs": 10,
                },
                "arm": {
                    "embedder": self.embedder,
                    "rate": {"value": 0.4, "unit": "bpp"},
                    "domain": "spatial",
                    "format": "png",
                },
                "metrics": {
                    "auc": self.auc,
                    "tpr_at_fpr": {"0.01": 0.11, "0.1": 0.55},
                    "n_clean": 5,
                    "n_stego": 5,
                    "n_error": self.n_error,
                },
                "provenance": {
                    "plugins": [{
                        "name": self.detector,
                        "image": format!("sha256:{}", "c".repeat(64)),
                        "determinism": "exact",
                        "pinned_by": "executable-hash",
                        "isolation": "host",
                    }],
                    "harness_version": "0.1.0",
                    "started_utc": "2026-09-27T00:00:00Z",
                    "elapsed_seconds": 1.0,
                    "network_reachable": false,
                },
                "declarations": declarations,
            })
        }

        fn write(&self, dir: &Path, name: &str) -> PathBuf {
            let path = dir.join(name);
            std::fs::write(&path, serde_json::to_string_pretty(&self.value()).unwrap()).unwrap();
            path
        }
    }

    /// A default document with one edit applied before it is written.
    fn write_with(dir: &Path, name: &str, edit: impl FnOnce(&mut serde_json::Value)) -> PathBuf {
        let mut v = Doc::default().value();
        edit(&mut v);
        let path = dir.join(name);
        std::fs::write(&path, serde_json::to_string_pretty(&v).unwrap()).unwrap();
        path
    }

    /// The same, sealed the way `score` seals what it writes.
    fn write_sealed(dir: &Path, name: &str, edit: impl FnOnce(&mut serde_json::Value)) -> PathBuf {
        let mut v = Doc::default().value();
        edit(&mut v);
        let mut doc: Result1 = serde_json::from_value(v).expect("the fixture is a result-v1");
        doc.seal();
        let path = dir.join(name);
        std::fs::write(&path, serde_json::to_string_pretty(&doc).unwrap()).unwrap();
        path
    }

    /// How many documents `results/v1` actually holds.
    ///
    /// Counted rather than written down. This used to be the literal 24, and
    /// when three results were withdrawn the test failed with an arithmetic
    /// complaint rather than saying anything about rendering, which is what it
    /// is for. The shipped set is a fixture that changes as measurements are
    /// added and retired, and a test over it should only assert the things
    /// that are true whatever it holds.
    fn shipped_count() -> usize {
        std::fs::read_dir(shipped_results())
            .expect("results/v1 is in the repository")
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .count()
    }

    #[test]
    fn the_shipped_results_produce_a_report_in_every_format() {
        let paths = vec![shipped_results()];
        let report = build(&paths).expect("the shipped results build a report");
        assert_eq!(report.read, shipped_count());
        assert!(report.read > 0, "results/v1 is empty");
        assert!(report.skipped.is_empty(), "{:?}", report.skipped);
        assert_eq!(report.exit_code(), exit::OK);
        // Every shipped document names the same corpus today, so this says
        // nothing about how several tables render. That property is covered
        // against built fixtures instead, where the shape can be chosen:
        // see the tests around one name with two digests below.
        for group in &report.groups {
            assert!(!group.rows.is_empty());
        }
        for format in [
            ReportFormat::Text,
            ReportFormat::Markdown,
            ReportFormat::Csv,
        ] {
            let text = render(&report, format);
            assert!(text.contains("round3-q95"), "{format:?}");
            assert!(!text.is_empty());
        }
    }

    /// The whole point, asserted on the real documents: a row lifted out of
    /// the table still says which corpus, which configuration, which pairing
    /// and which split.
    #[test]
    fn every_markdown_row_carries_its_conditions() {
        let report = build(&[shipped_results()]).unwrap();
        let md = render(&report, ReportFormat::Markdown);
        let rows: Vec<&str> = md
            .lines()
            .filter(|l| l.starts_with("| ") && !l.contains("detector") && !l.starts_with("|---"))
            .collect();
        assert_eq!(rows.len(), report.read, "expected one row per result");
        for row in rows {
            assert!(row.contains(" @ sha256:"), "no corpus digest in: {row}");
            assert!(row.contains("custom"), "no configuration in: {row}");
            assert!(
                row.contains("single-variable") || row.contains("confounded"),
                "no pairing in: {row}"
            );
            assert!(row.contains("not-applicable"), "no split in: {row}");
        }
    }

    /// A confounded run has to be readable as confounded from the row itself,
    /// not from a legend the row leaves behind when it is copied.
    #[test]
    fn a_confounded_run_is_marked_in_its_own_row() {
        let report = build(&[shipped_results()]).unwrap();
        let md = render(&report, ReportFormat::Markdown);
        let confounded: Vec<&str> = md
            .lines()
            .filter(|l| l.starts_with("| ") && l.contains("outguess"))
            .collect();
        assert!(!confounded.is_empty(), "no outguess rows found");
        for row in confounded {
            assert!(
                row.contains("CONFOUNDED"),
                "an outguess row does not say so: {row}"
            );
        }
    }

    #[test]
    fn an_unverified_pairing_is_marked_in_the_row() {
        let dir = tempfile::tempdir().unwrap();
        Doc {
            pairing: "unverified".into(),
            ..Doc::default()
        }
        .write(dir.path(), "a.json");
        let report = build(&[dir.path().to_path_buf()]).unwrap();
        for format in [
            ReportFormat::Text,
            ReportFormat::Markdown,
            ReportFormat::Csv,
        ] {
            assert!(
                flat(&render(&report, format)).contains("PAIRING UNVERIFIED"),
                "{format:?} hid an unverified pairing"
            );
        }
    }

    #[test]
    fn a_by_file_split_is_marked_because_it_inflates_the_number() {
        let dir = tempfile::tempdir().unwrap();
        Doc {
            split: "by-file".into(),
            ..Doc::default()
        }
        .write(dir.path(), "a.json");
        let report = build(&[dir.path().to_path_buf()]).unwrap();
        assert!(flat(&render(&report, ReportFormat::Text)).contains("SPLIT BY FILE"));
    }

    #[test]
    fn unscored_images_and_contamination_reach_the_row() {
        let dir = tempfile::tempdir().unwrap();
        Doc {
            n_error: 7,
            trained_on: Some("corpus-a".into()),
            ..Doc::default()
        }
        .write(dir.path(), "a.json");
        let report = build(&[dir.path().to_path_buf()]).unwrap();
        let text = flat(&render(&report, ReportFormat::Text));
        assert!(text.contains("7 image(s) unscored"), "{text}");
        assert!(text.contains("TRAINED ON THIS CORPUS"), "{text}");
    }

    #[test]
    fn a_detector_trained_on_a_different_corpus_is_not_flagged_as_contaminated() {
        let dir = tempfile::tempdir().unwrap();
        Doc {
            trained_on: Some("somewhere-else".into()),
            ..Doc::default()
        }
        .write(dir.path(), "a.json");
        let report = build(&[dir.path().to_path_buf()]).unwrap();
        assert!(!flat(&render(&report, ReportFormat::Text)).contains("TRAINED ON THIS CORPUS"));
    }

    #[test]
    fn two_corpora_never_share_a_table() {
        let dir = tempfile::tempdir().unwrap();
        Doc {
            corpus: "one".into(),
            ..Doc::default()
        }
        .write(dir.path(), "a.json");
        Doc {
            corpus: "two".into(),
            digest: format!("sha256:{}", "d".repeat(64)),
            ..Doc::default()
        }
        .write(dir.path(), "b.json");
        let report = build(&[dir.path().to_path_buf()]).unwrap();
        assert_eq!(report.groups.len(), 2);
        assert!(!report.any_cross_group_comparison_is_valid());
        assert!(flat(&render(&report, ReportFormat::Text))
            .contains("Nothing may be read across a table boundary"));
    }

    /// The label is not the bytes. Two documents naming one corpus with two
    /// digests describe two sets of images, and putting them in one table on
    /// the strength of the name is the comparison this refuses to make.
    #[test]
    fn one_corpus_name_with_two_digests_is_two_tables() {
        let dir = tempfile::tempdir().unwrap();
        Doc::default().write(dir.path(), "a.json");
        Doc {
            digest: format!("sha256:{}", "e".repeat(64)),
            ..Doc::default()
        }
        .write(dir.path(), "b.json");
        let report = build(&[dir.path().to_path_buf()]).unwrap();
        assert_eq!(report.groups.len(), 2, "one name, two digests, one table");
    }

    #[test]
    fn named_and_custom_never_interleave_and_named_leads() {
        let dir = tempfile::tempdir().unwrap();
        Doc {
            configuration: "named".into(),
            ..Doc::default()
        }
        .write(dir.path(), "a.json");
        Doc {
            configuration: "custom".into(),
            detector: "detector-b".into(),
            ..Doc::default()
        }
        .write(dir.path(), "b.json");
        let report = build(&[dir.path().to_path_buf()]).unwrap();
        assert_eq!(report.groups.len(), 2);
        assert_eq!(report.groups[0].configuration, Configuration::Named);
        assert_eq!(report.groups[1].configuration, Configuration::Custom);
        for group in &report.groups {
            assert_eq!(group.rows.len(), 1, "a group mixed the two");
        }
        let text = flat(&render(&report, ReportFormat::Text));
        assert!(text.contains("Quote these figures beside nothing else"));
    }

    #[test]
    fn a_malformed_json_file_is_named_rather_than_vanishing() {
        let dir = tempfile::tempdir().unwrap();
        Doc::default().write(dir.path(), "good.json");
        std::fs::write(dir.path().join("bad.json"), "{not json").unwrap();
        let report = build(&[dir.path().to_path_buf()]).unwrap();
        assert_eq!(report.read, 1);
        assert_eq!(report.skipped.len(), 1);
        assert!(report.skipped[0].invalid_document);
        assert_eq!(report.exit_code(), exit::SCHEMA_INVALID);
        for format in [
            ReportFormat::Text,
            ReportFormat::Markdown,
            ReportFormat::Csv,
        ] {
            let text = render(&report, format);
            assert!(text.contains("bad.json"), "{format:?} lost the bad file");
        }
        assert!(flat(&render(&report, ReportFormat::Text)).contains("THIS REPORT IS INCOMPLETE"));
        assert!(render(&report, ReportFormat::Csv).contains("skipped_file"));
    }

    #[test]
    fn valid_json_that_is_not_a_result_is_named_with_its_reason() {
        let dir = tempfile::tempdir().unwrap();
        Doc::default().write(dir.path(), "good.json");
        std::fs::write(dir.path().join("other.json"), r#"{"hello":"world"}"#).unwrap();
        let report = build(&[dir.path().to_path_buf()]).unwrap();
        assert_eq!(report.skipped.len(), 1);
        assert!(report.skipped[0]
            .reason
            .contains("not a result-v1 document"));
        assert_eq!(report.exit_code(), exit::SCHEMA_INVALID);
    }

    /// Parses as a `result-v1` and fails the rules the type system cannot
    /// hold. It must not reach a table: an AUC of 1.4 in a row a reader
    /// quotes is worse than a named skip.
    #[test]
    fn a_result_that_parses_but_does_not_validate_is_refused_from_the_table() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = Doc::default().value();
        v["metrics"]["auc"] = serde_json::json!(1.4);
        std::fs::write(
            dir.path().join("impossible.json"),
            serde_json::to_string(&v).unwrap(),
        )
        .unwrap();
        Doc::default().write(dir.path(), "good.json");
        let report = build(&[dir.path().to_path_buf()]).unwrap();
        assert_eq!(report.read, 1);
        assert_eq!(report.skipped.len(), 1);
        assert!(report.skipped[0].reason.contains("does not validate"));
        assert!(!render(&report, ReportFormat::Markdown).contains("1.4000"));
    }

    #[test]
    fn a_file_over_the_size_cap_is_skipped_with_the_cap_named() {
        let dir = tempfile::tempdir().unwrap();
        let big = "x".repeat((MAX_RESULT_BYTES + 1) as usize);
        std::fs::write(dir.path().join("huge.json"), big).unwrap();
        let report = build(&[dir.path().to_path_buf()]).unwrap();
        assert!(report.groups.is_empty());
        assert_eq!(report.skipped.len(), 1);
        assert!(report.skipped[0].reason.contains("byte cap"));
        // Not a schema failure: nothing was parsed, so nothing was judged.
        assert_eq!(report.exit_code(), exit::FAILURE);
    }

    #[test]
    fn an_empty_directory_refuses_rather_than_printing_an_empty_table() {
        let dir = tempfile::tempdir().unwrap();
        let err = build(&[dir.path().to_path_buf()]).unwrap_err();
        assert!(matches!(err, ReportError::NothingFound { .. }));
        assert_eq!(err.exit_code(), exit::USAGE);
        assert!(err.to_string().contains("no table to print"));
    }

    #[test]
    fn an_empty_input_set_refuses() {
        let err = build(&[]).unwrap_err();
        assert!(matches!(err, ReportError::NothingFound { .. }));
        assert!(err.to_string().contains("nowhere") || err.to_string().contains("nothing under"));
    }

    #[test]
    fn a_directory_of_non_json_files_refuses() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("README.md"), "not a result").unwrap();
        let err = build(&[dir.path().to_path_buf()]).unwrap_err();
        assert!(matches!(err, ReportError::NothingFound { .. }));
    }

    /// `score --corpus /nope` refuses with exit 3 and an explanation, and
    /// this answered the identical mistake with the operating system's own
    /// error number under exit 1. Same fault, same code, and no `os error 2`
    /// in front of a reader.
    #[test]
    fn a_missing_path_refuses_the_way_score_refuses_one() {
        let err = build(&[PathBuf::from("/definitely/not/here")]).unwrap_err();
        assert!(matches!(err, ReportError::PathUnusable { .. }));
        assert_eq!(err.exit_code(), exit::PREFLIGHT_REFUSED);
        let said = err.to_string();
        assert!(said.contains("/definitely/not/here"), "{said}");
        assert!(said.contains("nothing is there"), "{said}");
        assert!(!said.contains("os error"), "{said}");
    }

    /// A file named on the command line is read whatever it is called. Only a
    /// directory walk filters on the extension.
    #[test]
    fn a_named_file_without_a_json_extension_is_still_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("result.txt");
        std::fs::write(
            &path,
            serde_json::to_string(&Doc::default().value()).unwrap(),
        )
        .unwrap();
        let report = build(&[path]).unwrap();
        assert_eq!(report.read, 1);
    }

    #[test]
    fn a_tree_deeper_than_the_cap_refuses() {
        let dir = tempfile::tempdir().unwrap();
        let mut deep = dir.path().to_path_buf();
        for i in 0..(MAX_DEPTH + 1) {
            deep = deep.join(format!("d{i}"));
        }
        std::fs::create_dir_all(&deep).unwrap();
        let err = build(&[dir.path().to_path_buf()]).unwrap_err();
        assert!(matches!(err, ReportError::TooDeep { .. }));
        assert_eq!(err.exit_code(), exit::PREFLIGHT_REFUSED);
    }

    #[test]
    fn a_nested_directory_within_the_cap_is_walked() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a").join("b");
        std::fs::create_dir_all(&nested).unwrap();
        Doc::default().write(&nested, "r.json");
        let report = build(&[dir.path().to_path_buf()]).unwrap();
        assert_eq!(report.read, 1);
    }

    /// Two runs over the same input produce identical bytes, whatever order
    /// the filesystem hands the directory back in.
    #[test]
    fn two_runs_over_the_same_input_are_byte_identical() {
        let dir = tempfile::tempdir().unwrap();
        for (i, name) in ["z", "m", "a", "q"].iter().enumerate() {
            Doc {
                detector: format!("detector-{name}"),
                auc: 0.5 + i as f64 / 100.0,
                ..Doc::default()
            }
            .write(dir.path(), &format!("{name}.json"));
        }
        for format in [
            ReportFormat::Text,
            ReportFormat::Markdown,
            ReportFormat::Csv,
        ] {
            let first = render(&build(&[dir.path().to_path_buf()]).unwrap(), format);
            let second = render(&build(&[dir.path().to_path_buf()]).unwrap(), format);
            assert_eq!(first, second, "{format:?} is not reproducible");
        }
        // And the shipped corpus of results, which is the realistic case.
        let a = render(&build(&[shipped_results()]).unwrap(), ReportFormat::Csv);
        let b = render(&build(&[shipped_results()]).unwrap(), ReportFormat::Csv);
        assert_eq!(a, b);
    }

    /// The same directory given twice must not double every row.
    #[test]
    fn a_path_named_twice_is_read_once() {
        let dir = tempfile::tempdir().unwrap();
        Doc::default().write(dir.path(), "a.json");
        let report = build(&[dir.path().to_path_buf(), dir.path().to_path_buf()]).unwrap();
        assert_eq!(report.read, 1);
    }

    #[test]
    fn rows_are_ordered_by_arm_and_detector_rather_than_by_score() {
        let dir = tempfile::tempdir().unwrap();
        Doc {
            detector: "zeta".into(),
            auc: 0.99,
            ..Doc::default()
        }
        .write(dir.path(), "a.json");
        Doc {
            detector: "alpha".into(),
            auc: 0.51,
            ..Doc::default()
        }
        .write(dir.path(), "b.json");
        let report = build(&[dir.path().to_path_buf()]).unwrap();
        let names: Vec<&str> = report.groups[0]
            .rows
            .iter()
            .map(|r| r.detector.as_str())
            .collect();
        assert_eq!(names, ["alpha", "zeta"], "the table was ranked by score");
        assert!(flat(&render(&report, ReportFormat::Text)).contains("never by score"));
    }

    #[test]
    fn two_runs_of_one_detector_on_one_arm_are_marked_as_separate_measurements() {
        let dir = tempfile::tempdir().unwrap();
        Doc::default().write(dir.path(), "a.json");
        Doc {
            auc: 0.7,
            ..Doc::default()
        }
        .write(dir.path(), "b.json");
        let report = build(&[dir.path().to_path_buf()]).unwrap();
        assert_eq!(report.groups[0].rows.len(), 2);
        let text = flat(&render(&report, ReportFormat::Text));
        assert!(text.contains("one of 2 runs"), "{text}");
    }

    #[test]
    fn a_corpus_with_no_digest_says_so_in_the_row_and_in_the_heading() {
        let dir = tempfile::tempdir().unwrap();
        Doc {
            digest: String::new(),
            ..Doc::default()
        }
        .write(dir.path(), "a.json");
        let report = build(&[dir.path().to_path_buf()]).unwrap();
        let text = flat(&render(&report, ReportFormat::Text));
        assert!(text.contains("NO CORPUS DIGEST"), "{text}");
        assert!(text.contains("nobody can check which bytes"), "{text}");
        assert!(render(&report, ReportFormat::Markdown).contains("carries no digest"));
    }

    #[test]
    fn a_missing_point_on_the_curve_is_not_reported_rather_than_zero() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = Doc::default().value();
        v["metrics"]["tpr_at_fpr"] = serde_json::json!({"0.05": 0.3});
        std::fs::write(
            dir.path().join("a.json"),
            serde_json::to_string(&v).unwrap(),
        )
        .unwrap();
        let report = build(&[dir.path().to_path_buf()]).unwrap();
        let text = flat(&render(&report, ReportFormat::Text));
        assert!(text.contains("not reported"), "{text}");
        assert!(
            !text.contains("0.0000"),
            "a missing point read as zero: {text}"
        );
    }

    #[test]
    fn a_rate_carries_its_unit_into_the_arm_cell() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = Doc::default().value();
        v["arm"]["rate"] = serde_json::json!({"value": 0.05, "unit": "capacity_fraction"});
        std::fs::write(
            dir.path().join("a.json"),
            serde_json::to_string(&v).unwrap(),
        )
        .unwrap();
        let mut w = Doc::default().value();
        w["arm"].as_object_mut().unwrap().remove("rate");
        w["arm"]["embedder"] = serde_json::json!("structural");
        std::fs::write(
            dir.path().join("b.json"),
            serde_json::to_string(&w).unwrap(),
        )
        .unwrap();

        let report = build(&[dir.path().to_path_buf()]).unwrap();
        let text = flat(&render(&report, ReportFormat::Text));
        assert!(text.contains("5.000% of capacity"), "{text}");
        assert!(text.contains("structural"), "{text}");
        assert!(
            !text.contains("structural at"),
            "a rateless arm invented one"
        );
    }

    #[test]
    fn the_nondeterminism_warning_names_the_plugin_it_is_about() {
        // It used to fire on every row of every report, because the value was
        // hardcoded rather than read from anywhere. A warning that is always
        // on is one a reader stops seeing, and this one was also false: it
        // said "two runs need not agree" about tools nobody had measured.
        let dir = tempfile::tempdir().unwrap();
        let mut v = Doc::default().value();
        v["provenance"]["plugins"] = serde_json::json!([
            {"name": "steady", "image": format!("sha256:{}", "c".repeat(64)), "determinism": "exact", "pinned_by": "executable-hash", "isolation": "host"},
            {"name": "wobbly", "image": format!("sha256:{}", "d".repeat(64)), "determinism": "nondeterministic", "pinned_by": "executable-hash", "isolation": "host"},
        ]);
        std::fs::write(
            dir.path().join("a.json"),
            serde_json::to_string(&v).unwrap(),
        )
        .unwrap();
        let text = flat(&render(
            &build(&[dir.path().to_path_buf()]).unwrap(),
            ReportFormat::Text,
        ));
        assert!(text.contains("NONDETERMINISTIC"), "{text}");
        assert!(
            text.contains("wobbly"),
            "the warning should name it: {text}"
        );
        assert!(
            !text.contains("NONDETERMINISTIC: two runs need not agree (steady"),
            "the deterministic plugin was accused: {text}"
        );
    }

    #[test]
    fn a_report_of_deterministic_plugins_carries_no_determinism_warning() {
        // The case that proves the flag can now clear. Before, it could not.
        let dir = tempfile::tempdir().unwrap();
        let mut v = Doc::default().value();
        v["provenance"]["plugins"] = serde_json::json!([
            {"name": "steady", "image": format!("sha256:{}", "c".repeat(64)), "determinism": "exact", "pinned_by": "executable-hash", "isolation": "host"},
        ]);
        std::fs::write(
            dir.path().join("a.json"),
            serde_json::to_string(&v).unwrap(),
        )
        .unwrap();
        let text = flat(&render(
            &build(&[dir.path().to_path_buf()]).unwrap(),
            ReportFormat::Text,
        ));
        assert!(!text.contains("NONDETERMINISTIC"), "{text}");
        assert!(!text.contains("determinism unstated"), "{text}");
    }

    #[test]
    fn nobody_having_checked_is_said_differently_from_having_checked_and_found_drift() {
        // Two different facts. Only one of them is a reason to distrust the
        // number, and collapsing them is how the original warning came to
        // mean nothing.
        let dir = tempfile::tempdir().unwrap();
        let mut v = Doc::default().value();
        v["provenance"]["plugins"] = serde_json::json!([
            {"name": "unchecked", "image": format!("sha256:{}", "c".repeat(64)), "determinism": "unstated", "pinned_by": "executable-hash", "isolation": "host"},
        ]);
        std::fs::write(
            dir.path().join("a.json"),
            serde_json::to_string(&v).unwrap(),
        )
        .unwrap();
        let text = flat(&render(
            &build(&[dir.path().to_path_buf()]).unwrap(),
            ReportFormat::Text,
        ));
        assert!(text.contains("determinism unstated"), "{text}");
        assert!(text.contains("unchecked"), "{text}");
        assert!(
            !text.contains("NONDETERMINISTIC"),
            "an unmeasured tool was called nondeterministic: {text}"
        );
    }

    #[test]
    fn plugins_that_disagree_are_said_to_be_mixed_rather_than_picked_between() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = Doc::default().value();
        v["provenance"]["plugins"] = serde_json::json!([
            {"name": "a", "image": format!("sha256:{}", "c".repeat(64)), "determinism": "exact", "pinned_by": "executable-hash", "isolation": "host"},
            {"name": "b", "image": format!("ghcr.io/x/y@sha256:{}", "c".repeat(64)), "determinism": "nondeterministic", "pinned_by": "image-digest", "isolation": "sandbox-no-network"},
        ]);
        v["provenance"]["network_reachable"] = serde_json::json!(true);
        std::fs::write(
            dir.path().join("a.json"),
            serde_json::to_string(&v).unwrap(),
        )
        .unwrap();
        let report = build(&[dir.path().to_path_buf()]).unwrap();
        assert_eq!(report.groups[0].rows[0].pinned_by, "mixed");
        assert_eq!(report.groups[0].rows[0].isolation, "mixed");
        assert!(flat(&render(&report, ReportFormat::Text)).contains("NONDETERMINISTIC"));
    }

    #[test]
    fn a_network_reachable_run_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = Doc::default().value();
        v["provenance"]["network_reachable"] = serde_json::json!(true);
        std::fs::write(
            dir.path().join("a.json"),
            serde_json::to_string(&v).unwrap(),
        )
        .unwrap();
        let report = build(&[dir.path().to_path_buf()]).unwrap();
        assert!(flat(&render(&report, ReportFormat::Text)).contains("reach the network"));
    }

    #[test]
    fn a_self_reported_run_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = Doc::default().value();
        v["declarations"]["self_reported"] = serde_json::json!(true);
        std::fs::write(
            dir.path().join("a.json"),
            serde_json::to_string(&v).unwrap(),
        )
        .unwrap();
        let report = build(&[dir.path().to_path_buf()]).unwrap();
        assert!(render(&report, ReportFormat::Csv).contains("self-reported"));
    }

    #[test]
    fn a_row_with_nothing_flagged_does_not_claim_to_be_clean() {
        let dir = tempfile::tempdir().unwrap();
        Doc::default().write(dir.path(), "a.json");
        let report = build(&[dir.path().to_path_buf()]).unwrap();
        // Never "ok" and never "clean": nothing was flagged, which is the
        // weaker statement and the only true one.
        let text = flat(&render(&report, ReportFormat::Text));
        assert!(text.contains("Nothing was flagged"), "{text}");
        assert!(
            !text.contains(" ok ") && !text.contains("clean run"),
            "{text}"
        );
        assert!(render(&report, ReportFormat::Markdown).contains("| nothing flagged |"));
    }

    /// A page of "nothing flagged" would bury the handful of lines that
    /// matter, so the quiet rows are counted rather than listed. The count has
    /// to be there: a reader must not have to subtract to learn that every
    /// other row was looked at.
    #[test]
    fn the_text_conditions_list_only_the_rows_with_something_to_say() {
        let report = build(&[shipped_results()]).unwrap();
        let text = flat(&render(&report, ReportFormat::Text));
        assert!(text.contains("Conditions, read before quoting"), "{text}");
        // Derived from the report rather than typed, so adding a result to
        // the repository does not make this test wrong about arithmetic it
        // is not testing.
        let jpeg = group_named(&report, "round3-q95");
        let quiet = jpeg.rows.iter().filter(|r| r.flags.is_empty()).count();
        assert!(quiet > 0 && quiet < jpeg.rows.len(), "fixture has no mix");
        assert!(
            text.contains(&format!("the other {quiet} row(s) in this table")),
            "{text}"
        );
    }

    /// The other half of the same rule: a table where nothing was flagged says
    /// so, rather than printing an empty conditions block a reader has to
    /// interpret.
    ///
    /// Built here rather than taken from `results/v1`, which used to hold a
    /// second corpus with nothing flagged on any row. That corpus was
    /// withdrawn, and a test that depends on the shipped set happening to
    /// contain a clean table is a test that breaks for a reason unrelated to
    /// what it checks.
    #[test]
    fn a_table_with_nothing_flagged_says_so_rather_than_printing_nothing() {
        let dir = tempfile::tempdir().unwrap();
        for (i, detector) in ["detector-a", "detector-b"].iter().enumerate() {
            Doc {
                detector: (*detector).into(),
                ..Doc::default()
            }
            .write(dir.path(), &format!("{i}.json"));
        }
        let report = build(&[dir.path().to_path_buf()]).unwrap();
        let group = group_named(&report, "corpus-a");
        assert!(group.rows.iter().all(|r| r.flags.is_empty()));
        let text = flat(&render(&report, ReportFormat::Text));
        assert!(
            text.contains(&format!(
                "Nothing was flagged on any of the {} row(s)",
                group.rows.len()
            )),
            "{text}"
        );
    }

    #[test]
    fn the_csv_quotes_a_field_that_would_otherwise_break_the_row() {
        assert_eq!(csv_field("plain"), "plain");
        assert_eq!(csv_field("a,b"), "\"a,b\"");
        assert_eq!(csv_field("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(csv_field("two\nlines"), "\"two\nlines\"");
    }

    #[test]
    fn the_csv_has_one_field_per_column_on_every_row() {
        let report = build(&[shipped_results()]).unwrap();
        let csv = render(&report, ReportFormat::Csv);
        let mut lines = csv.lines();
        let columns = lines.next().unwrap().split(',').count();
        for line in lines {
            // No shipped field contains a comma or a quote, so a naive split
            // is exact here and a change that introduces one fails loudly.
            assert_eq!(
                line.split(',').count(),
                columns,
                "row has the wrong number of fields: {line}"
            );
        }
    }

    #[test]
    fn a_forged_document_is_named_as_skipped_rather_than_tabled() {
        // The finding: one field edited after the run, and the document came
        // out as an ordinary row under an exit code of zero.
        let dir = tempfile::tempdir().unwrap();
        let path = write_sealed(dir.path(), "a.json", |_| {});
        let mut v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        v["metrics"]["auc"] = serde_json::json!(0.99);
        std::fs::write(&path, serde_json::to_string_pretty(&v).unwrap()).unwrap();

        let report = build(&[dir.path().to_path_buf()]).unwrap();
        assert!(report.groups.is_empty(), "the forgery made a table");
        assert_eq!(report.skipped.len(), 1, "{:?}", report.skipped);
        assert!(report.skipped[0].invalid_document);
        assert!(
            report.skipped[0]
                .reason
                .contains("does not match its own content digest"),
            "{:?}",
            report.skipped[0]
        );
        assert_eq!(report.exit_code(), exit::SCHEMA_INVALID);
        for format in [
            ReportFormat::Text,
            ReportFormat::Markdown,
            ReportFormat::Csv,
        ] {
            let out = render(&report, format);
            assert!(!out.contains("0.9900"), "{format:?} tabled the forgery");
            assert!(
                flat(&out).contains("content digest"),
                "{format:?} did not name it"
            );
        }
    }

    #[test]
    fn a_sealed_document_nobody_touched_makes_an_ordinary_row() {
        let dir = tempfile::tempdir().unwrap();
        write_sealed(dir.path(), "a.json", |_| {});
        let report = build(&[dir.path().to_path_buf()]).unwrap();
        assert!(report.skipped.is_empty(), "{:?}", report.skipped);
        assert_eq!(report.exit_code(), exit::OK);
    }

    #[test]
    fn an_auc_outside_its_own_interval_never_reaches_a_table() {
        // Visible with no digest at all, and it used to render as a row.
        let dir = tempfile::tempdir().unwrap();
        write_with(dir.path(), "a.json", |v| {
            v["metrics"]["auc"] = serde_json::json!(0.99);
            v["metrics"]["auc_ci95"] = serde_json::json!([0.5, 0.5]);
        });
        let report = build(&[dir.path().to_path_buf()]).unwrap();
        assert!(report.groups.is_empty());
        assert_eq!(report.exit_code(), exit::SCHEMA_INVALID);
        assert!(
            report.skipped[0]
                .reason
                .contains("outside its own interval"),
            "{:?}",
            report.skipped[0]
        );
    }

    #[test]
    fn a_budget_the_corpus_could_not_express_is_said_in_the_cell() {
        // "TPR@1%FA" over six clean images is TPR@0%FA, and this is the
        // column an engineer acts on.
        let dir = tempfile::tempdir().unwrap();
        write_with(dir.path(), "a.json", |v| {
            v["metrics"]["tpr_at_fpr_achieved"] = serde_json::json!({"0.01": 0.0, "0.1": 0.1});
        });
        let report = build(&[dir.path().to_path_buf()]).unwrap();
        for format in [ReportFormat::Text, ReportFormat::Markdown] {
            let out = flat(&render(&report, format));
            assert!(out.contains("0.1100 at 0% FA"), "{format:?}: {out}");
            // The rate that was met is left alone, or the mark means nothing.
            assert!(out.contains("0.5500 "), "{format:?}: {out}");
            assert!(!out.contains("0.5500 at"), "{format:?}: {out}");
            assert!(out.contains("too few clean images"), "{format:?}: {out}");
        }
    }

    #[test]
    fn a_document_that_recorded_no_achieved_rates_renders_as_it_always_did() {
        let dir = tempfile::tempdir().unwrap();
        Doc::default().write(dir.path(), "a.json");
        let report = build(&[dir.path().to_path_buf()]).unwrap();
        for format in [ReportFormat::Text, ReportFormat::Markdown] {
            let out = flat(&render(&report, format));
            assert!(!out.contains(" FA"), "{format:?} invented a mark: {out}");
            assert!(!out.contains("too few clean images"), "{format:?}: {out}");
        }
    }

    #[test]
    fn the_csv_gives_every_false_alarm_rate_its_own_column() {
        let dir = tempfile::tempdir().unwrap();
        write_with(dir.path(), "a.json", |v| {
            v["metrics"]["tpr_at_fpr"] = serde_json::json!({"0.01": 0.11, "0.1": 0.55});
            v["metrics"]["tpr_at_fpr_achieved"] = serde_json::json!({"0.01": 0.0, "0.1": 0.1});
        });
        let report = build(&[dir.path().to_path_buf()]).unwrap();
        let csv = render(&report, ReportFormat::Csv);
        let header: Vec<&str> = csv.lines().next().unwrap().split(',').collect();
        for column in [
            "tpr_at_fpr_0.01",
            "tpr_at_fpr_0.01_achieved_fpr",
            "tpr_at_fpr_0.1",
            "tpr_at_fpr_0.1_achieved_fpr",
        ] {
            assert!(header.contains(&column), "no {column} in {header:?}");
        }
        assert!(!header.contains(&"tpr_at_fpr"), "the packed cell survived");
        let row: Vec<&str> = csv.lines().nth(1).unwrap().split(',').collect();
        let at = |name: &str| row[header.iter().position(|h| *h == name).unwrap()];
        assert_eq!(at("tpr_at_fpr_0.01"), "0.1100");
        assert_eq!(at("tpr_at_fpr_0.01_achieved_fpr"), "0.0000");
        assert_eq!(at("tpr_at_fpr_0.1"), "0.5500");
        assert_eq!(at("tpr_at_fpr_0.1_achieved_fpr"), "0.1000");
    }

    #[test]
    fn the_csv_columns_cover_the_rates_of_every_document_not_just_the_first() {
        // A column set taken from the first row would drop the second
        // document's figures with nothing in the file saying so.
        let dir = tempfile::tempdir().unwrap();
        write_with(dir.path(), "a.json", |v| {
            v["metrics"]["tpr_at_fpr"] = serde_json::json!({"0.01": 0.11});
        });
        write_with(dir.path(), "b.json", |v| {
            v["subject"]["name"] = serde_json::json!("detector-b");
            v["metrics"]["tpr_at_fpr"] = serde_json::json!({"0.05": 0.33});
        });
        let report = build(&[dir.path().to_path_buf()]).unwrap();
        let csv = render(&report, ReportFormat::Csv);
        let header: Vec<&str> = csv.lines().next().unwrap().split(',').collect();
        assert!(header.contains(&"tpr_at_fpr_0.01"), "{header:?}");
        assert!(header.contains(&"tpr_at_fpr_0.05"), "{header:?}");
        assert!(
            csv.contains("0.3300"),
            "the second document's figure: {csv}"
        );
        // A rate a document never reported is empty rather than zero.
        let a_line = csv
            .lines()
            .find(|l| l.contains("detector-a"))
            .expect("a row for detector-a");
        let cells: Vec<&str> = a_line.split(',').collect();
        let at = |name: &str| cells[header.iter().position(|h| *h == name).unwrap()];
        assert_eq!(at("tpr_at_fpr_0.05"), "");
    }

    #[test]
    fn a_csv_arm_line_leaves_the_curve_columns_empty() {
        // The curve was measured for the run, not for the arm, and an arm
        // line repeating it would describe a different population.
        let dir = tempfile::tempdir().unwrap();
        with_arms(dir.path(), two_arms());
        let report = build(&[dir.path().to_path_buf()]).unwrap();
        let csv = render(&report, ReportFormat::Csv);
        let header: Vec<&str> = csv.lines().next().unwrap().split(',').collect();
        let index = header
            .iter()
            .position(|h| *h == "tpr_at_fpr_0.01")
            .expect("a column for the rate the fixture reports");
        for line in csv.lines().filter(|l| l.starts_with("arm,")) {
            assert_eq!(line.split(',').nth(index).unwrap(), "", "{line}");
        }
    }

    #[test]
    fn a_pipe_in_a_cell_cannot_shift_a_markdown_row() {
        assert_eq!(escape_md("a|b"), "a\\|b");
        assert_eq!(escape_md("a\nb"), "a b");
    }

    #[test]
    fn abbreviating_a_digest_keeps_the_algorithm_and_refuses_to_invent_one() {
        assert_eq!(abbreviate(""), "none");
        assert_eq!(abbreviate("sha256:0123456789abcdef"), "sha256:01234567");
        assert_eq!(abbreviate("short"), "short");
        assert_eq!(abbreviate("0123456789"), "01234567");
        assert_eq!(abbreviate("sha256:abc"), "sha256:abc");
    }

    #[test]
    fn the_atomic_write_leaves_the_whole_file_and_no_part_file() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("report.md");
        write_atomically(&out, "first").unwrap();
        assert_eq!(std::fs::read_to_string(&out).unwrap(), "first");
        write_atomically(&out, "second").unwrap();
        assert_eq!(std::fs::read_to_string(&out).unwrap(), "second");
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains(".part-"))
            .collect();
        assert!(leftovers.is_empty(), "left behind: {leftovers:?}");
    }

    #[test]
    fn an_unwritable_destination_fails_with_the_path_and_leaves_nothing() {
        let dir = tempfile::tempdir().unwrap();
        // A directory that does not exist: the create fails, so the error
        // path that removes the temporary file is the one that runs.
        let out = dir.path().join("no-such-dir").join("report.md");
        let err = write_atomically(&out, "body").unwrap_err();
        assert!(matches!(err, ReportError::Write { .. }));
        assert_eq!(err.exit_code(), exit::FAILURE);
        assert!(err.to_string().contains("report.md"));
        assert!(!dir.path().join("no-such-dir").exists());
    }

    /// A bare file name has an empty parent rather than no parent, and
    /// joining onto it would produce a path starting with a separator. Tested
    /// on the path calculation rather than by changing the process's working
    /// directory, which would race every other test in this binary.
    #[test]
    fn a_destination_with_no_parent_gets_a_temporary_file_beside_it() {
        let bare = temp_path_for(Path::new("bare.md"));
        assert_eq!(bare.parent(), Some(Path::new("")));
        assert!(bare
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with(".bare.md.part-")));

        let nested = temp_path_for(Path::new("/tmp/some/report.md"));
        assert_eq!(nested.parent(), Some(Path::new("/tmp/some")));
        assert!(nested
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with(".report.md.part-")));
    }

    #[test]
    fn the_json_output_says_whether_the_table_is_complete() {
        let dir = tempfile::tempdir().unwrap();
        Doc::default().write(dir.path(), "a.json");
        std::fs::write(dir.path().join("bad.json"), "{").unwrap();
        let report = build(&[dir.path().to_path_buf()]).unwrap();
        let json = to_json(&report);
        assert_eq!(json["complete"], serde_json::json!(false));
        assert_eq!(json["files_skipped"], serde_json::json!(1));
        assert_eq!(json["results_read"], serde_json::json!(1));
        assert_eq!(
            json["groups"][0]["rows"][0]["conditions"]
                .as_array()
                .map(|a| a.len()),
            Some(0)
        );
        assert!(json["groups"][0]["comparability"]
            .as_str()
            .is_some_and(|s| s.starts_with("NAMED:") || s.starts_with("CUSTOM:")));
        assert!(json["groups"][0]["rows"][0]["corpus"]["digest"]
            .as_str()
            .is_some_and(|d| d.len() > 8));
    }

    #[test]
    fn the_json_marks_a_single_table_as_internally_comparable() {
        let dir = tempfile::tempdir().unwrap();
        Doc::default().write(dir.path(), "a.json");
        let report = build(&[dir.path().to_path_buf()]).unwrap();
        assert_eq!(
            to_json(&report)["cross_table_comparison_valid"],
            serde_json::json!(true)
        );
    }

    #[test]
    fn wrapping_keeps_every_word_and_respects_the_column() {
        let text = "one two three four five six seven eight nine ten";
        let wrapped = wrap(text, 12);
        for line in wrapped.lines() {
            assert!(line.chars().count() <= 12, "too long: {line:?}");
        }
        assert_eq!(
            wrapped.split_whitespace().collect::<Vec<_>>(),
            text.split_whitespace().collect::<Vec<_>>()
        );
        assert_eq!(wrap("", 10), "");
        // A word longer than the column is kept whole rather than cut.
        assert_eq!(
            wrap("antidisestablishmentarianism", 5),
            "antidisestablishmentarianism"
        );
    }

    #[test]
    fn display_helpers_say_nowhere_rather_than_nothing() {
        assert_eq!(display_list(&[]), "nowhere");
        assert_eq!(display_paths(&[]), "");
        assert_eq!(display_list(&["a".to_string(), "b".to_string()]), "a, b");
    }

    #[test]
    fn tpr_lookup_matches_a_rate_however_it_was_written() {
        let map = BTreeMap::from([("0.10".to_string(), 0.9), ("0.010".to_string(), 0.4)]);
        assert_eq!(tpr_at(&map, 0.10), Some(0.9));
        assert_eq!(tpr_at(&map, 0.01), Some(0.4));
        assert_eq!(tpr_at(&map, 0.05), None);
    }

    #[test]
    fn a_rate_written_two_ways_is_one_csv_column() {
        // "0.1" and "0.10" are one rate, and two columns for it would put
        // half the figures under each.
        let dir = tempfile::tempdir().unwrap();
        write_with(dir.path(), "a.json", |v| {
            v["metrics"]["tpr_at_fpr"] = serde_json::json!({"0.1": 0.5});
        });
        write_with(dir.path(), "b.json", |v| {
            v["metrics"]["tpr_at_fpr"] = serde_json::json!({"0.10": 0.6});
        });
        let report = build(&[dir.path().to_path_buf()]).unwrap();
        let header = render(&report, ReportFormat::Csv)
            .lines()
            .next()
            .unwrap()
            .to_string();
        assert_eq!(header.matches("tpr_at_fpr_0.1,").count(), 1, "{header}");
    }

    #[test]
    fn every_enum_value_renders_to_a_word_rather_than_a_debug_string() {
        for d in [
            Domain::Spatial,
            Domain::Jpeg,
            Domain::Structural,
            Domain::Mixed,
            Domain::Unstated,
        ] {
            assert!(!domain_str(d).is_empty());
        }
        for p in [
            Pairing::SingleVariable,
            Pairing::Confounded,
            Pairing::Unverified,
        ] {
            assert!(!pairing_str(p).is_empty());
        }
        for s in [
            SplitDiscipline::ByCover,
            SplitDiscipline::ByFile,
            SplitDiscipline::NotApplicable,
        ] {
            assert!(!split_str(s).is_empty());
        }
        assert_eq!(source_str(CorpusSource::Fetched), "fetched");
        assert_eq!(source_str(CorpusSource::Supplied), "supplied");
        assert_eq!(unit_str(RateUnit::Bpp), "bpp");
        assert_eq!(configuration_str(Configuration::Named), "named");
    }

    /// A run with no plugins recorded must not be filed under any of the
    /// values the schema carries, on either field.
    #[test]
    fn an_unrecorded_plugin_list_is_named_as_unrecorded() {
        let mut v = Doc::default().value();
        v["provenance"]["plugins"] = serde_json::json!([]);
        let parsed: Result1 = serde_json::from_value(v).unwrap();
        let row = to_row(Path::new("x.json"), parsed);
        assert_eq!(row.pinned_by, "unrecorded");
        assert_eq!(row.isolation, "unrecorded");
    }

    /// A linked result is a row the walk cannot deliver. Dropping it quietly
    /// would produce a table short by one with nothing saying so, which is
    /// the exact failure this module is built around.
    #[test]
    #[cfg(unix)]
    fn a_symlinked_result_file_is_named_rather_than_skipped_silently() {
        let dir = tempfile::tempdir().unwrap();
        let elsewhere = dir.path().join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        let real = Doc::default().write(&elsewhere, "real.json");
        let here = dir.path().join("here");
        std::fs::create_dir_all(&here).unwrap();
        std::os::unix::fs::symlink(&real, here.join("linked.json")).unwrap();
        Doc::default().write(&here, "ordinary.json");

        let report = build(std::slice::from_ref(&here)).unwrap();
        assert_eq!(report.read, 1);
        assert_eq!(report.skipped.len(), 1);
        assert!(report.skipped[0].reason.contains("symbolic link"));
        assert_eq!(report.exit_code(), exit::FAILURE);
        assert!(flat(&render(&report, ReportFormat::Text)).contains("linked.json"));

        // And naming it on the command line does include it, as the message
        // says it will.
        let named = build(&[here.join("linked.json")]).unwrap();
        assert_eq!(named.read, 1);
        assert!(named.skipped.is_empty());
    }

    /// A link that is not trying to be a result is not a gap in anything.
    #[test]
    #[cfg(unix)]
    fn a_symlink_that_is_not_a_json_file_is_not_reported_as_a_gap() {
        let dir = tempfile::tempdir().unwrap();
        Doc::default().write(dir.path(), "a.json");
        std::fs::write(dir.path().join("notes.md"), "notes").unwrap();
        std::os::unix::fs::symlink(dir.path().join("notes.md"), dir.path().join("link.md"))
            .unwrap();
        let report = build(&[dir.path().to_path_buf()]).unwrap();
        assert!(report.skipped.is_empty(), "{:?}", report.skipped);
    }

    /// The cap is applied to what is read, not to what `metadata` said a
    /// moment earlier, so a file that grows between the two cannot slip past.
    #[test]
    fn the_size_cap_is_applied_to_the_bytes_actually_read() {
        let dir = tempfile::tempdir().unwrap();
        // One byte over, which is the boundary the cap has to get right.
        std::fs::write(
            dir.path().join("edge.json"),
            "x".repeat(MAX_RESULT_BYTES as usize + 1),
        )
        .unwrap();
        let report = build(&[dir.path().to_path_buf()]).unwrap();
        assert!(report.skipped[0].reason.contains("byte cap"));

        // And exactly at the cap it is read, and refused for being the wrong
        // shape rather than for being too big.
        let at_cap = dir.path().join("at-cap.json");
        std::fs::write(&at_cap, "x".repeat(MAX_RESULT_BYTES as usize)).unwrap();
        let report = build(&[at_cap]).unwrap();
        assert!(
            report.skipped[0].reason.contains("not a result-v1"),
            "{:?}",
            report.skipped
        );
    }

    /// A document whose corpus held more than one named arm, written to
    /// `dir/a.json`.
    fn with_arms(dir: &Path, arms: serde_json::Value) -> PathBuf {
        let mut v = Doc::default().value();
        v["metrics"]["per_arm"] = arms;
        let path = dir.join("a.json");
        std::fs::write(&path, serde_json::to_string(&v).unwrap()).unwrap();
        path
    }

    /// Two arms, of which one is chance and the other is not. It is the case
    /// the breakdown exists for: the pooled figure describes neither.
    fn two_arms() -> serde_json::Value {
        serde_json::json!([
            {"arm": "lsb-0400", "auc": 0.6944, "n_clean": 5, "n_stego": 5},
            {"arm": "lsb-0100", "auc": 0.5, "auc_ci95": [0.4, 0.6], "n_clean": 5, "n_stego": 5},
        ])
    }

    #[test]
    fn a_two_arm_document_breaks_down_into_both_arms_in_every_format() {
        let dir = tempfile::tempdir().unwrap();
        with_arms(dir.path(), two_arms());
        let report = build(&[dir.path().to_path_buf()]).unwrap();
        for format in [
            ReportFormat::Text,
            ReportFormat::Markdown,
            ReportFormat::Csv,
        ] {
            let text = flat(&render(&report, format));
            assert!(text.contains("lsb-0100"), "{format:?} lost an arm: {text}");
            assert!(text.contains("lsb-0400"), "{format:?} lost an arm: {text}");
            assert!(text.contains("0.6944"), "{format:?} lost a figure: {text}");
            assert!(text.contains("0.5000"), "{format:?} lost a figure: {text}");
        }
        // And the headline figure is still there beside them, because the
        // breakdown is additional rather than a replacement.
        assert!(flat(&render(&report, ReportFormat::Text)).contains("0.9000"));
    }

    /// The common case. A document with no breakdown must render exactly as it
    /// did before there was such a thing: no empty column, no "not
    /// applicable" on every row.
    #[test]
    fn a_document_with_no_arms_gains_nothing_in_any_format() {
        let dir = tempfile::tempdir().unwrap();
        Doc::default().write(dir.path(), "a.json");
        let report = build(&[dir.path().to_path_buf()]).unwrap();
        assert!(arm_table(&report.groups[0]).is_none());
        for format in [ReportFormat::Text, ReportFormat::Markdown] {
            let text = flat(&render(&report, format));
            assert!(!text.contains("Per arm"), "{format:?}: {text}");
            assert!(!text.contains("clean/stego "), "{format:?}: {text}");
        }
        let csv = render(&report, ReportFormat::Csv);
        assert!(
            !csv.lines().any(|l| l.starts_with("arm,")),
            "an arm line with no arms: {csv}"
        );
    }

    #[test]
    fn an_arm_interval_is_shown_where_there_is_one_and_not_invented_where_there_is_not() {
        let dir = tempfile::tempdir().unwrap();
        with_arms(dir.path(), two_arms());
        let report = build(&[dir.path().to_path_buf()]).unwrap();
        let text = flat(&render(&report, ReportFormat::Text));
        assert!(text.contains("0.5000 [0.400, 0.600]"), "{text}");
        // The arm that reported none is bare, not bracketed with anything.
        assert!(
            !text.contains("0.6944 ["),
            "an interval was invented: {text}"
        );

        let csv = render(&report, ReportFormat::Csv);
        let arm_lines: Vec<&str> = csv.lines().filter(|l| l.starts_with("arm,")).collect();
        assert_eq!(arm_lines.len(), 2);
        assert!(
            arm_lines.iter().any(|l| l.contains("0.4000,0.6000")),
            "{csv}"
        );
        assert!(
            arm_lines.iter().any(|l| l.contains("0.6944,,")),
            "the missing interval was filled in: {csv}"
        );

        let arms = &to_json(&report)["groups"][0]["rows"][0]["per_arm"];
        assert!(arms[0]["auc_ci95"].is_array());
        assert!(
            arms[1].get("auc_ci95").is_none(),
            "an absent interval became a value: {arms}"
        );
    }

    /// By name, and deliberately not by score. An ordering is read as a
    /// ranking whatever the prose beside it says, which is why the rows above
    /// are not ordered by score either.
    #[test]
    fn the_breakdown_is_ordered_by_arm_name_and_is_reproducible() {
        let dir = tempfile::tempdir().unwrap();
        with_arms(
            dir.path(),
            serde_json::json!([
                {"arm": "wow-0200", "auc": 0.99, "n_clean": 5, "n_stego": 5},
                {"arm": "lsb-0400", "auc": 0.6, "n_clean": 5, "n_stego": 5},
                {"arm": "hugo-0100", "auc": 0.7, "n_clean": 5, "n_stego": 5},
            ]),
        );
        let report = build(&[dir.path().to_path_buf()]).unwrap();
        let names: Vec<&str> = report.groups[0].rows[0]
            .per_arm
            .iter()
            .map(|a| a.arm.as_str())
            .collect();
        assert_eq!(names, ["hugo-0100", "lsb-0400", "wow-0200"]);
        for format in [
            ReportFormat::Text,
            ReportFormat::Markdown,
            ReportFormat::Csv,
        ] {
            let first = render(&build(&[dir.path().to_path_buf()]).unwrap(), format);
            let second = render(&build(&[dir.path().to_path_buf()]).unwrap(), format);
            assert_eq!(first, second, "{format:?} is not reproducible");
        }
    }

    #[test]
    fn the_json_carries_the_arm_figures_as_numbers_rather_than_as_a_rendered_string() {
        let dir = tempfile::tempdir().unwrap();
        with_arms(dir.path(), two_arms());
        let report = build(&[dir.path().to_path_buf()]).unwrap();
        let row = &to_json(&report)["groups"][0]["rows"][0];
        assert_eq!(row["per_arm"][0]["arm"], serde_json::json!("lsb-0100"));
        assert_eq!(row["per_arm"][0]["auc"], serde_json::json!(0.5));
        assert_eq!(row["per_arm"][0]["n_clean"], serde_json::json!(5));
        assert_eq!(row["per_arm"][1]["auc"], serde_json::json!(0.6944));
        assert_eq!(row["per_arm"].as_array().map(|a| a.len()), Some(2));
    }

    /// The key is there on every row, empty array and all. An absent key would
    /// make a consumer handle two shapes of one answer.
    #[test]
    fn the_json_carries_an_empty_breakdown_as_an_empty_list() {
        let dir = tempfile::tempdir().unwrap();
        Doc::default().write(dir.path(), "a.json");
        let report = build(&[dir.path().to_path_buf()]).unwrap();
        assert_eq!(
            to_json(&report)["groups"][0]["rows"][0]["per_arm"],
            serde_json::json!([])
        );
    }

    /// The breakdown of one arm is the headline figure written twice, and the
    /// core type says so by leaving it empty. Nothing here second-guesses it.
    #[test]
    fn a_single_arm_breakdown_is_rendered_if_a_document_carries_one() {
        let dir = tempfile::tempdir().unwrap();
        with_arms(
            dir.path(),
            serde_json::json!([{"arm": "only", "auc": 0.9, "n_clean": 5, "n_stego": 5}]),
        );
        let report = build(&[dir.path().to_path_buf()]).unwrap();
        assert!(flat(&render(&report, ReportFormat::Text)).contains("only"));
    }

    fn many_arms(count: usize, prefix: &str) -> serde_json::Value {
        serde_json::Value::Array(
            (0..count)
                .map(|i| {
                    serde_json::json!({
                        "arm": format!("{prefix}-{i:04}"),
                        "auc": 0.6,
                        "n_clean": 5,
                        "n_stego": 5,
                    })
                })
                .collect(),
        )
    }

    /// A Core tier run breaks down into 39 arms and a report may hold
    /// thousands of documents. The cap stops the wall of text, and it says how
    /// many documents it left out rather than trailing off.
    #[test]
    fn the_breakdown_stops_at_the_cap_and_names_what_it_left_out() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..4 {
            let mut v = Doc::default().value();
            v["subject"]["name"] = serde_json::json!(format!("detector-{i}"));
            v["provenance"]["plugins"][0]["name"] = serde_json::json!(format!("detector-{i}"));
            v["metrics"]["per_arm"] = many_arms(80, "lsb");
            std::fs::write(
                dir.path().join(format!("{i}.json")),
                serde_json::to_string(&v).unwrap(),
            )
            .unwrap();
        }
        let report = build(&[dir.path().to_path_buf()]).unwrap();
        let (lines, withheld) = arm_table(&report.groups[0]).unwrap();
        assert_eq!(lines.len(), 160, "the cap cut a document in half");
        assert_eq!(withheld, 2);
        for format in [ReportFormat::Text, ReportFormat::Markdown] {
            let text = flat(&render(&report, format));
            assert!(
                text.contains("2 further document(s) in this table"),
                "{format:?}: {text}"
            );
        }
        // The machine formats are uncapped: four documents of eighty arms is
        // 320 arm lines and every one of them is there.
        let csv = render(&report, ReportFormat::Csv);
        assert_eq!(csv.lines().filter(|l| l.starts_with("arm,")).count(), 320);
        let json = to_json(&report);
        for row in json["groups"][0]["rows"].as_array().unwrap() {
            assert_eq!(row["per_arm"].as_array().map(|a| a.len()), Some(80));
        }
    }

    /// A corpus with more arms than the cap shows its breakdown rather than
    /// nothing at all.
    #[test]
    fn one_document_larger_than_the_cap_is_still_printed_whole() {
        let dir = tempfile::tempdir().unwrap();
        with_arms(dir.path(), many_arms(MAX_ARM_LINES_SHOWN + 5, "lsb"));
        let report = build(&[dir.path().to_path_buf()]).unwrap();
        let (lines, withheld) = arm_table(&report.groups[0]).unwrap();
        assert_eq!(lines.len(), MAX_ARM_LINES_SHOWN + 5);
        assert_eq!(withheld, 0);
    }

    /// An arm line is a line somebody sorts a spreadsheet by, so it repeats
    /// every condition rather than pointing back at the line above it.
    #[test]
    fn a_csv_arm_line_carries_the_conditions_of_the_document_it_came_from() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = Doc::default().value();
        v["declarations"]["pairing"] = serde_json::json!("confounded");
        v["metrics"]["per_arm"] = two_arms();
        std::fs::write(
            dir.path().join("a.json"),
            serde_json::to_string(&v).unwrap(),
        )
        .unwrap();
        let report = build(&[dir.path().to_path_buf()]).unwrap();
        let csv = render(&report, ReportFormat::Csv);
        let columns = csv.lines().next().unwrap().split(',').count();
        for line in csv.lines().filter(|l| l.starts_with("arm,")) {
            assert_eq!(line.split(',').count(), columns, "{line}");
            assert!(line.contains("confounded"), "{line}");
            assert!(line.contains("corpus-a"), "{line}");
            assert!(line.contains("detector-a"), "{line}");
            assert!(line.contains("CONFOUNDED"), "{line}");
        }
    }

    /// Two corpora never share a table, and a per-arm figure is no more
    /// comparable across that boundary than a pooled one. Each breakdown sits
    /// under its own corpus, and every line names it.
    #[test]
    fn a_breakdown_never_crosses_a_corpus_boundary() {
        let dir = tempfile::tempdir().unwrap();
        for (name, digest, arm) in [("one", "d", "lsb-0100"), ("two", "e", "wow-0200")] {
            let mut v = Doc::default().value();
            v["corpus"]["name"] = serde_json::json!(name);
            v["corpus"]["digest"] = serde_json::json!(format!("sha256:{}", digest.repeat(64)));
            v["metrics"]["per_arm"] = serde_json::json!([
                {"arm": arm, "auc": 0.6, "n_clean": 5, "n_stego": 5},
                {"arm": "shared", "auc": 0.7, "n_clean": 5, "n_stego": 5},
            ]);
            std::fs::write(
                dir.path().join(format!("{name}.json")),
                serde_json::to_string(&v).unwrap(),
            )
            .unwrap();
        }
        let report = build(&[dir.path().to_path_buf()]).unwrap();
        assert_eq!(report.groups.len(), 2);
        for group in &report.groups {
            let (lines, _) = arm_table(group).unwrap();
            assert_eq!(lines.len(), 2, "a breakdown gathered two corpora");
            for line in &lines {
                assert!(
                    line[1].starts_with(&group.corpus_name),
                    "an arm line does not name its corpus: {line:?}"
                );
            }
        }
    }

    #[test]
    fn an_auc_cell_brackets_an_interval_only_where_there_is_one() {
        assert_eq!(auc_cell(0.5, None), "0.5000");
        assert_eq!(auc_cell(0.5, Some([0.4, 0.6])), "0.5000 [0.400, 0.600]");
    }

    #[test]
    #[cfg(unix)]
    fn a_symlinked_directory_is_not_followed() {
        {
            let dir = tempfile::tempdir().unwrap();
            let real = dir.path().join("real");
            std::fs::create_dir_all(&real).unwrap();
            Doc::default().write(&real, "a.json");
            // A link pointing at its own ancestor is the cycle the walk must
            // not enter.
            std::os::unix::fs::symlink(dir.path(), real.join("loop")).unwrap();
            let report = build(&[dir.path().to_path_buf()]).unwrap();
            assert_eq!(report.read, 1);
        }
    }
}
