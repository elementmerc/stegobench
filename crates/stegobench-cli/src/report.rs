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
    Configuration, CorpusSource, Determinism, Domain, Isolation, Pairing, PinnedBy, PluginRef,
    RateUnit, Result1, SplitDiscipline,
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

/// Why a whole report could not be produced.
///
/// A single unreadable file is NOT one of these: that is a recorded skip
/// which appears in the output (see [`Skipped`]), because dropping one
/// document silently is how a table comes to look complete while it is not.
/// These are the cases where there is no honest table to print at all.
#[derive(Debug, thiserror::Error)]
pub enum ReportError {
    #[error("cannot read {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
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
            ReportError::TooMany { .. } | ReportError::TooDeep { .. } => exit::PREFLIGHT_REFUSED,
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
    pub tpr_at_fpr: BTreeMap<String, f64>,
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
        let meta = std::fs::metadata(path).map_err(|e| ReportError::Read {
            path: path.display().to_string(),
            source: e,
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
    if r.provenance
        .plugins
        .iter()
        .any(|p| p.determinism == Determinism::Nondeterministic)
    {
        flags.push("NONDETERMINISTIC: two runs need not agree".to_string());
    }
    if r.provenance.network_reachable {
        flags.push("the plugins could reach the network during this run".to_string());
    }

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
        tpr_at_fpr: r.metrics.tpr_at_fpr,
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

fn tpr_cell(map: &BTreeMap<String, f64>, target: f64) -> String {
    match tpr_at(map, target) {
        Some(v) => format!("{v:.4}"),
        // Not a dash and not a zero. Both read as an answer, and the truth is
        // that this run did not report that point on the curve.
        None => "not reported".to_string(),
    }
}

/// The whole `tpr_at_fpr` map, for the format that carries everything.
fn tpr_all(map: &BTreeMap<String, f64>) -> String {
    map.iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join(";")
}

/// The flags, joined for a single cell.
fn flags_cell(flags: &[String]) -> String {
    if flags.is_empty() {
        // Deliberately not "ok" or "clean". Neither is a claim this can make:
        // nothing was flagged, which is a weaker statement and the true one.
        return "nothing flagged".to_string();
    }
    flags.join(" · ")
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
        format!("{:.4}", row.auc),
        tpr_cell(&row.tpr_at_fpr, 0.01),
        tpr_cell(&row.tpr_at_fpr, 0.10),
        pairing_str(row.pairing).to_string(),
        split_str(row.split).to_string(),
        format!("{}/{}/{}", row.n_clean, row.n_stego, row.n_error),
    ]
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
    let columns = [
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
        "rate_value",
        "rate_unit",
        "domain",
        "image_format",
        "auc",
        "tpr_at_fpr",
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
    ];
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
            let (rate_value, rate_unit) = match row.rate {
                Some((v, u)) => (v.to_string(), unit_str(u).to_string()),
                None => (String::new(), String::new()),
            };
            let line = vec![
                "result".to_string(),
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
                rate_value,
                rate_unit,
                domain_str(row.domain).to_string(),
                row.format.clone(),
                format!("{:.4}", row.auc),
                tpr_all(&row.tpr_at_fpr),
                pairing_str(row.pairing).to_string(),
                split_str(row.split).to_string(),
                row.n_clean.to_string(),
                row.n_stego.to_string(),
                row.n_error.to_string(),
                row.trained_on.clone().unwrap_or_default(),
                row.self_reported.to_string(),
                row.network_reachable.to_string(),
                row.harness_version.clone(),
                row.started_utc.clone(),
                flags_cell(&row.flags),
                row.source.clone(),
            ];
            debug_assert_eq!(line.len(), columns.len());
            let _ = writeln!(
                out,
                "{}",
                line.iter()
                    .map(|c| csv_field(c))
                    .collect::<Vec<_>>()
                    .join(",")
            );
        }
    }
    out
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
                "tpr_at_fpr": r.tpr_at_fpr,
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

    #[test]
    fn a_missing_path_fails_with_the_path_named() {
        let err = build(&[PathBuf::from("/definitely/not/here")]).unwrap_err();
        assert!(matches!(err, ReportError::Read { .. }));
        assert_eq!(err.exit_code(), exit::FAILURE);
        assert!(err.to_string().contains("/definitely/not/here"));
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
        assert_eq!(tpr_all(&map), "0.010=0.4;0.10=0.9");
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
