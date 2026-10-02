// Author:  Daniel Iwugo
// Comment: Christ is King
// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Daniel Iwugo
//! `result-v1`: one measurement, in the form other people are meant to adopt.
//!
//! WHY THE TYPES ARE THE SOURCE OF TRUTH
//! -------------------------------------
//! The published JSON Schema is generated from these structs rather than
//! maintained beside them, because a hand-kept schema drifts from the writer
//! and then lies about a format other people have committed to.
//!
//! WHAT THIS FORMAT IS FOR
//! -----------------------
//! A benchmark's durability is its format, not its code. If stegobench
//! disappeared and people still exchanged `result-v1` documents, the point
//! would have been served. So the shape is conservative, every field earns its
//! place, and the compatibility rules are written down before anybody depends
//! on them (see `docs/schemas.md`).

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The value of the `schema` field. Readers match on this exactly.
pub const RESULT_SCHEMA_ID: &str = "stegobench/result-v1";

/// One detector measured on one arm of one corpus.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Result1 {
    /// Always [`RESULT_SCHEMA_ID`]. Present so a reader can dispatch without
    /// guessing from the shape.
    pub schema: String,
    pub subject: Subject,
    pub corpus: CorpusRef,
    pub arm: Arm,
    pub metrics: Metrics,
    pub provenance: Provenance,
    pub declarations: Declarations,
    /// A digest over everything in this document except when it was run.
    ///
    /// Reproduction is the claim this whole format exists to support, and
    /// checking it used to mean diffing two documents and knowing which two
    /// fields to forgive. Two runs of the same detector over the same corpus
    /// differ in `provenance.started_utc` and `provenance.elapsed_seconds`
    /// and in nothing else, so a digest that leaves those out turns the
    /// check into a string comparison a script can do in one line.
    ///
    /// Optional because a hand-written or submitted document need not carry
    /// one, and absent is honest: it means nobody offered a digest, not that
    /// the document failed to match one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_digest: Option<String>,
}

impl Result1 {
    /// The bytes [`Result1::content_digest`] covers.
    ///
    /// Excludes the two timing fields and the digest itself, since a digest
    /// cannot cover its own value. `serde_json` orders object keys, so two
    /// runs serialise identically without a canonicalisation pass of our own.
    ///
    fn content_bytes(&self) -> Vec<u8> {
        let mut v = serde_json::to_value(self).expect("a result document serialises");
        if let Some(o) = v.as_object_mut() {
            o.remove("content_digest");
            if let Some(p) = o.get_mut("provenance").and_then(|p| p.as_object_mut()) {
                p.remove("started_utc");
                p.remove("elapsed_seconds");
            }
        }
        serde_json::to_vec(&v).expect("a json value serialises")
    }

    /// What [`Result1::content_digest`] should hold for this document.
    ///
    /// IT IS A DIGEST OF THE PARSED DOCUMENT, NOT OF THE FILE
    ///
    /// The document is deserialised, two timing fields and the digest itself
    /// are dropped, and what remains is re-serialised and hashed. So the
    /// digest is defined as "what this implementation recomputes" rather than
    /// "a hash of these bytes", and the difference is not academic: a writer
    /// whose float printer differs from `serde_json`'s can emit
    /// `0.9074239216648787` where this emits `...788`. The two parse to
    /// identical bits and hash differently, so a document sealed by that
    /// writer could never match here.
    ///
    /// That is the right trade, because the alternative makes reformatting
    /// a file break its seal, and it is why nothing outside this crate should
    /// compute one. If a generator ever needs to seal a document it must call
    /// this rather than hash its own output.
    pub fn compute_content_digest(&self) -> String {
        digest_of(&self.content_bytes())
    }

    /// Fill in [`Result1::content_digest`] from the document's own contents.
    pub fn seal(&mut self) {
        self.content_digest = Some(self.compute_content_digest());
    }
}

fn digest_of(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::new().chain_update(bytes).finalize())
}

/// What was measured.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Subject {
    pub name: String,
    /// A version or an image digest. A mutable tag is not acceptable here and
    /// the registry refuses one, because a result naming `:latest` cannot be
    /// reproduced by anybody including us.
    pub version: String,
    pub kind: SubjectKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SubjectKind {
    Detector,
    Embedder,
}

/// Where the bytes came from.
///
/// Two of the four registered corpora may not be redistributed, so a corpus
/// this harness fetched and one the user already had are different provenance
/// claims about the same name. A reader deciding whether to trust a number is
/// entitled to know which they have, and a submission path deciding which
/// division an entry belongs in needs it: a result the harness can fetch and
/// re-score is a different kind of evidence from one it cannot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum CorpusSource {
    /// The harness downloaded it from the route the registry declares, and
    /// checked it against the digest recorded there.
    Fetched,
    /// The user pointed at a copy they already had. Correct and ordinary for
    /// a corpus nobody may redistribute, and the reason this field exists
    /// rather than being assumed.
    Supplied,
}

/// The exact bytes the measurement was taken on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CorpusRef {
    pub name: String,
    /// The registry id, where the run resolved one.
    ///
    /// Recorded beside the display name because the two differ and a reader
    /// checking a contamination claim needs the id: `--trained-on` is written
    /// with the id a person read out of `list corpora`, and comparing it with
    /// the display name clears exactly the person who used the right one.
    /// Absent for a directory that resolved to no registry entry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tier: Option<String>,
    /// Fetched by us, or supplied by the user. See [`CorpusSource`].
    pub source: CorpusSource,
    /// Digest over the corpus manifest. A result that cannot name the bytes it
    /// was measured on is an anecdote, and `stegobench verify` recomputes this
    /// from a corpus on disk rather than trusting it.
    ///
    /// Empty where the corpus cannot honestly be named, which is any corpus
    /// whose records do not all state a digest for their own image.
    pub digest: String,
    /// How many matched pairs the measurement covered.
    ///
    /// A pair is one cover and the stego image made from it, so this is the
    /// stego side of the set and NOT the number of images: a run over 6 covers
    /// and 12 stego images reports 12, not 18.
    ///
    /// Written down because it was not. The Rust and the Python writers
    /// disagreed about this field for as long as both existed, one recording
    /// items and the other pairs, and an undocumented field is what let them:
    /// there was nothing either could be wrong against. A reader comparing
    /// `pairs` with `n_stego` across two documents would have seen them agree
    /// in one and differ by the clean count in the other, with no way to tell
    /// which convention they were holding.
    pub pairs: u64,
    /// How many items the corpus holds in total, where the run covered only
    /// some of them.
    ///
    /// Absent means the run covered the whole corpus, which is the ordinary
    /// case and should not have to be stated.
    ///
    /// THE DIGEST DOES NOT SAY THIS AND CANNOT
    ///
    /// `digest` is computed over the corpus manifest, so it names the
    /// directory rather than the subset that was scored. A run over eight
    /// images of eighteen therefore produced a byte-identical digest to the
    /// full run, and `verify` recomputed it, found it matching, and said the
    /// document "was measured on the corpus ... all 18 image(s) hash to what
    /// their records state". That sentence was false and exit 0 asserted it.
    ///
    /// `--limit` is otherwise a free dial for anybody who wants a flattering
    /// number: sweep it, keep the value that suits, and the tool a reviewer
    /// runs blesses the result. So a truncated run says so here, and `verify`
    /// reads it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub of_items: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub split: Option<String>,
}

/// What a plugin's two-sided self-test said.
///
/// `Answered` is not a pass and not a fault. A SUBJECT is a tool being
/// measured, so "did not detect the fixture" is the finding rather than a
/// broken installation, and collapsing it into `Failed` would mean the
/// benchmark permanently describing what it measures as faulty.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SelfTest {
    /// Correct on both fixtures.
    Passed,
    /// Wrong on at least one.
    Failed,
    /// Could not be asked, which is not the same as being wrong.
    Skipped,
    /// It answered, and settled neither fixture.
    Answered,
}

/// Which hiding method, at what strength.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Arm {
    pub embedder: String,
    /// How much was hidden, or None where strength is not the variable, as in
    /// the appended-data control.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate: Option<Rate>,
    pub domain: Domain,
    pub format: String,
}

/// A payload size, carrying its unit.
///
/// The unit is not decoration and this type replaced a bare `rate_bpp` field
/// within hours of that field existing. The adaptive schemes are driven in
/// bits per pixel, so `suniward/0400` is 0.4 bpp. The JPEG tools are driven as
/// a fraction of whatever capacity the tool reports for that cover, so
/// `outguess/0050` is 5% of capacity and is not 0.05 bpp or any other fixed
/// number of bits. Recording both under one name would publish a false unit
/// for half the arms, and a reader comparing 0.4 against 0.05 would be
/// comparing nothing.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Rate {
    pub value: f64,
    pub unit: RateUnit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RateUnit {
    /// Bits per pixel. The usual axis for spatial adaptive schemes.
    Bpp,
    /// A fraction of the capacity the embedding tool reports for that cover.
    CapacityFraction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Domain {
    Spatial,
    Jpeg,
    /// Hidden in the container rather than the image, so no pixel changes.
    Structural,
    /// The run covered arms in more than one domain, so no single one
    /// describes it. A Core tier holds spatial, JPEG and container arms
    /// together, and an aggregate over all of them is a real measurement that
    /// would be mislabelled by any one of the three.
    Mixed,
    /// The corpus did not say and nothing could work it out. Better here than
    /// in a reader's head: `spatial` written by default was wrong for every
    /// JPEG arm and looked exactly like a fact.
    Unstated,
}

/// The numbers.
///
/// There is deliberately no `accuracy` field. On a balanced set it flatters a
/// weak detector, and on an unbalanced one answering "clean" every time scores
/// well. The curve and two points on it are what a practitioner can act on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Metrics {
    /// Tie-aware ROC AUC. Ties matter here: several reference detectors return
    /// identical scores for many images, and the naive rank sum over-counts
    /// them into a number that looks like signal.
    pub auc: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auc_ci95: Option<[f64; 2]>,
    /// Detection rate keyed by false-alarm rate, the key written as the decimal
    /// fraction ("0.01" for one per cent) so the map sorts in a sane order.
    pub tpr_at_fpr: BTreeMap<String, f64>,
    /// The false-alarm rate each figure in `tpr_at_fpr` actually came from.
    ///
    /// Keyed identically to `tpr_at_fpr`. A value below its key means the
    /// corpus could not express the budget that was asked for: with six clean
    /// images the only rates that exist are multiples of 1/6, so a request for
    /// 0.01 is answered at 0.0 and the heading "TPR@1%FA" is a claim the
    /// measurement cannot support. The figure is real and conservative; the
    /// label is what it cannot carry, and a reader acting on the column is
    /// entitled to know which of the two they have.
    ///
    /// Empty where nobody recorded it, which is every document written before
    /// this field existed. Empty is not a claim that the budgets were met.
    ///
    /// The alias reads documents written under the first name this carried,
    /// `tpr_at_fpr_achieved`, which said "true-positive rate" over a false-alarm
    /// rate and had an examiner disclosing raw JSON read 0.0 as a detection rate
    /// of zero. Without the alias those documents would still parse, because
    /// this field is optional, and would come back empty: the qualifier that
    /// keeps `0.0833` from being read as detection at one per cent would go
    /// missing silently, which is the overclaim the field exists to stop.
    #[serde(
        default,
        alias = "tpr_at_fpr_achieved",
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub fpr_achieved: BTreeMap<String, f64>,
    /// The detector score to compare against to land on each budget.
    ///
    /// Keyed identically to `tpr_at_fpr`. Flag an image when its score is
    /// greater than or equal to this value.
    ///
    /// WHY A MEASUREMENT DOCUMENT CARRIES AN OPERATING INSTRUCTION
    ///
    /// Every other figure here says how good a detector is. This one says what
    /// to put in the `if`, and it is the only field a developer wiring detection
    /// into a product needs. It was computed internally to produce `tpr_at_fpr`
    /// and then discarded, so a developer whose entire task was choosing a
    /// cutoff searched a result document for it, found nothing, and wrote his
    /// own ROC sweep against the records file. His numbers matched ours, which
    /// is the point: the tool made somebody build a second copy of a metric it
    /// had already computed.
    ///
    /// **It is a raw score in the detector's own units and does not travel.**
    /// Another tool's scores are on another scale, and so are the same tool's
    /// after a version change, so this is a threshold for this subject at this
    /// version over this corpus and nothing wider.
    ///
    /// A key is absent where the chosen point is the origin, at which nothing is
    /// flagged and there is no cutoff. Absent rather than `0.0`, because 0.0 is
    /// a number somebody would type.
    ///
    /// Empty where nobody recorded it, which is every document written before
    /// this field existed, and empty is not a claim that no threshold exists.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub threshold_at_fpr: BTreeMap<String, f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verdict_rate: Option<f64>,
    pub n_clean: u64,
    pub n_stego: u64,
    /// Images the detector could not score. **Required**, so that zero is an
    /// assertion rather than an absence: a metric computed over the survivors
    /// of a partly failed run is how an evaluation misleads without anyone
    /// intending it.
    pub n_error: u64,
    /// The same measurement taken again within each arm.
    ///
    /// Detection at 0.1 bits per pixel and detection at 0.4 are different
    /// questions, and a corpus holding both answers neither when the two are
    /// pooled: the headline AUC lands somewhere between them and describes no
    /// arm that exists. A reader calibrating a threshold needs the arm, not
    /// the average over a mixture whose proportions came from how the corpus
    /// happened to be built.
    ///
    /// Empty when the corpus holds one arm, or none that its records name,
    /// because a breakdown with one row is the headline figure written twice.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub per_arm: Vec<ArmMetrics>,
}

/// One arm's share of a measurement.
///
/// The clean images are shared. Every arm is scored against the whole clean
/// set rather than against a slice of it, because the clean images are not
/// part of any arm: an arm is a way of hiding something, and an image with
/// nothing hidden in it belongs to all of them equally. Splitting the covers
/// between arms would shrink each comparison for no reason and make the arms
/// disagree about what a false alarm is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ArmMetrics {
    /// The arm as its records name it, such as `wow-0200`.
    pub arm: String,
    pub auc: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auc_ci95: Option<[f64; 2]>,
    /// Shared with every other arm in this document, and repeated on each row
    /// so a row can be read on its own.
    pub n_clean: u64,
    pub n_stego: u64,
}

/// Everything needed to run it again.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Provenance {
    /// The seed the detector's plugin was run with, as declared by whoever
    /// ran it.
    ///
    /// The harness has no seed of its own: nothing in scoring is random, so
    /// there is no `--seed` flag and there is not going to be one. A seed
    /// this tool generated and nothing consumed would make a document look
    /// more controlled than the run was.
    ///
    /// Absent means the plugin is deterministic, which is an answer rather
    /// than an omission. Nothing checks the value; what catches a wrong seed
    /// is somebody re-running the measurement and getting a different number.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    pub plugins: Vec<PluginRef>,
    pub harness_version: String,
    pub started_utc: String,
    pub elapsed_seconds: f64,
    /// Whether the plugins could reach the network. A number produced by
    /// something that could phone home is a different kind of number, and the
    /// reader is entitled to know which they have.
    pub network_reachable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<Host>,
    /// The per-item answers this document's metrics were computed from.
    ///
    /// Every number above is a summary of one score per image, and those live
    /// in a separate file that nothing in the document referred to. So a
    /// result and a records file could be handed over together with no way to
    /// tell whether they were about each other, which is the first thing
    /// anybody re-deriving a figure needs to know.
    ///
    /// READ WHAT THIS IS AND IS NOT. It ties a records file to this document.
    /// It does not say the scores are the detector's: whoever ran this wrote
    /// that file and could have written anything into it, and a digest taken
    /// afterwards would agree with whatever they wrote.
    /// `declarations.self_reported` is the field that speaks to that, and the
    /// submission path is what clears it.
    ///
    /// Absent where the run wrote no records file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub records: Option<RecordsRef>,
}

/// The digest and record count of a per-item scores file.
///
/// ONE IMPLEMENTATION ON PURPOSE. `score` writes this field and `verify`
/// recomputes it to compare, and those were briefly two copies of the same
/// loop in two crates. Two implementations of a digest that must agree is a
/// silent mismatch waiting to happen: they drift by one blank line and
/// `verify` starts refusing honest pairs, which is worse than not checking at
/// all because it teaches a reader to ignore the refusal.
///
/// Streams rather than reading the file in. A Core tier run writes 344,357
/// records and a digest is not a reason to hold them all in memory.
pub fn records_digest(path: &std::path::Path) -> std::io::Result<RecordsRef> {
    use sha2::Digest;
    use std::io::BufRead;
    let mut reader = std::io::BufReader::new(std::fs::File::open(path)?);
    let mut hasher = sha2::Sha256::new();
    let mut count = 0u64;
    let mut line = Vec::new();
    loop {
        line.clear();
        if reader.read_until(b'\n', &mut line)? == 0 {
            break;
        }
        hasher.update(&line);
        // A trailing newline is not a record. Counting it would put every
        // properly terminated file one over, every time.
        if !line.iter().all(|b| b.is_ascii_whitespace()) {
            count += 1;
        }
    }
    Ok(RecordsRef {
        digest: format!("sha256:{:x}", hasher.finalize()),
        count,
    })
}

/// The per-item answers a document's metrics were computed from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RecordsRef {
    /// SHA-256 of the records file as this run left it.
    pub digest: String,
    /// How many records it held. Cheap to check by eye, and it catches a
    /// truncated file before anybody computes a digest and wonders why.
    pub count: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PluginRef {
    pub name: String,
    /// Image digest, never a tag. For a locally installed program, the SHA-256
    /// of the executable that actually ran.
    pub image: String,
    pub determinism: Determinism,
    /// What `image` above is, and therefore what it is worth to somebody who
    /// wants to run this again. It says nothing about where the tool ran or
    /// what it could reach; `isolation` says that, and the two disagree often
    /// enough that one field answering both was misleading by construction.
    pub pinned_by: PinnedBy,
    /// What the thing that produced these numbers could reach while it ran.
    /// It says nothing about how the tool is pinned; `pinned_by` says that.
    pub isolation: Isolation,
    /// What the plugin's own two-sided self-test said, at the time of this
    /// run.
    ///
    /// The self-test is mandatory to register a detector and used to stop
    /// there, so a reviewer holding a result had no way to tell a plugin that
    /// passed from one that has never settled either fixture. The check is
    /// two images against a corpus of thousands, so running it beside the
    /// measurement costs nothing worth saving.
    ///
    /// Absent in a document written before this was recorded, or where no
    /// fixtures could be found to ask with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selftest: Option<SelfTest>,
    /// The script that turned the tool's output into the numbers above.
    ///
    /// A detector prints text. Something has to read that text and decide
    /// which part of it is the score, and that something is a file on the
    /// machine that ran it, resolved from `invoke.adapter`. Change one line of
    /// it and every number in this document changes, while `image` and
    /// `pinned_by` stay exactly as they are.
    ///
    /// It was recorded nowhere, so a reader holding a pinned container image
    /// still could not reproduce the measurement: the sandbox is handed that
    /// file from the host at run time, which also makes it the one part of a
    /// hardened run that is neither pinned nor examined.
    ///
    /// Absent where the tool needs no adapter, or where the file could not be
    /// read to hash it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter: Option<AdapterRef>,
    /// How the tool was asked, as the registry entry declares it.
    ///
    /// One image can serve several detectors. `aletheia-rs` and
    /// `aletheia-spa` run the same container from the same digest through the
    /// same adapter file, and differ only in a trailing word, so two results
    /// measuring two different things carried identical `image`, identical
    /// `adapter` and identical `subject.version`. Nothing in either document
    /// said which of the two had been measured.
    ///
    /// The DECLARED form, with its `{file}` and `{adapter}` placeholders
    /// still in it, rather than the command as it ran. The expanded form
    /// carries absolute paths from the machine that ran it, and this is a
    /// document written to be handed to somebody else.
    ///
    /// Absent where the entry declares no argv.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub argv: Option<Vec<String>>,
}

/// The script that read the tool's output, and what it was.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AdapterRef {
    /// As the registry entry declares it, which is how somebody else would
    /// find the same one. Not the absolute path it resolved to here: that
    /// names a directory on one machine and tells a reader nothing they can
    /// act on.
    pub declared: String,
    /// SHA-256 of the file that actually ran. The point of the hash is that
    /// two people who think they have the same adapter can find out.
    pub sha256: String,
}

/// How the bytes that ran are named, and how somebody else would get them.
// History, kept out of the doc comment because these become the published
// schema's descriptions and no reader ever saw the field they explain: this
// and `Isolation` replaced a single `route`, which tried to carry pinning and
// isolation at once. They agree for a container and for a locally installed
// program, and they disagree for an entry that names an image and sets
// `invoke.host`, where the image names the subject and an adapter on the
// operator's machine is what executes. One field could only be right about one
// of the two.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum PinnedBy {
    /// `image` is a container image digest, and the runtime started the
    /// container by it, so the bytes that ran here are named exactly.
    ///
    /// WHAT THIS DOES NOT SAY IS THAT YOU CAN OBTAIN THEM. It said so for
    /// months: "anybody can pull those exact bytes, so two people's runs are
    /// comparable by construction". Every image this project pins was built
    /// locally and pushed nowhere, so a reader following that sentence got
    /// `denied: requested access to the resource is denied` and a measurement
    /// they could not reproduce or disclose. Nothing here checks a registry,
    /// and a run with no network could not. `stegobench doctor --registry-reach`
    /// asks, and records the answer where it can be read.
    ImageDigest,
    /// `image` is the SHA-256 of an executable on the machine that ran it. Two
    /// people who both built the tool from source get different hashes for the
    /// same version and neither is wrong, so the pin is real and does not
    /// travel.
    ExecutableHash,
    /// Nothing in this document ties these numbers to particular bytes. Two
    /// runs arrive here: a program whose file could not be hashed, and a tool
    /// reached as a service, where `image` names what the operator was told to
    /// start and nothing checked that it is what answered. Either way a rerun
    /// is a rerun of whatever responds to that name at the time. It is written
    /// out rather than left to an absent field, because an absent answer and
    /// "nothing pinned this" look identical to a reader and only one of them is
    /// a statement.
    Unpinned,
}

/// What the tool could reach while it produced these numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Isolation {
    /// A container started with `--network=none`. It saw the images it was
    /// handed and nothing else.
    SandboxNoNetwork,
    /// A process on the machine that ran the benchmark, holding that machine's
    /// network and that user's privileges. Nothing constrained it.
    Host,
    /// The measurement went over the network. Something on this machine sent
    /// each image to an instance the operator started and read the answers
    /// back. Nothing here constrained the tool, and this document cannot say
    /// what the instance could reach or vouch that it was the version named.
    RemoteService,
    /// The document does not say. Hand-written and submitted documents can land
    /// here, and it is an explicit "nobody recorded this" rather than a default
    /// quietly standing in for `sandbox-no-network`.
    Unstated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Determinism {
    /// Two runs agree bit for bit.
    Exact,
    /// Two runs agree given the same seed.
    Seeded,
    /// They do not, and a result from this plugin says so rather than being
    /// presented as reproducible.
    Nondeterministic,
    /// Nobody has recorded whether two runs agree.
    ///
    /// Here for the same reason `Isolation::Unstated` is: the alternative is
    /// a default standing in silently for a measurement. `Nondeterministic`
    /// was that default for every entry in the registry, so every report
    /// carried the warning and none of them meant it, which is the same as
    /// carrying no warning at all.
    Unstated,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Host {
    /// Optional because no portable way of asking exists for all of these, and
    /// a zero would read as an answer. Absent means nobody measured it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cores: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_gb: Option<u32>,
    /// `linux`, `macos`, `windows`: the same vocabulary the registry's
    /// `platforms` field uses, because the two get compared.
    pub os: String,
    /// `x86_64`, `aarch64`. A detector's timing and sometimes its numbers
    /// depend on it, and a result that does not say cannot be reproduced on
    /// purpose.
    pub arch: String,
}

/// Claims the harness cannot check, stated so a reader knows what they are
/// trusting rather than having to assume it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Declarations {
    pub split_discipline: SplitDiscipline,
    pub pairing: Pairing,
    /// Whether this run measured something anybody else can name and repeat.
    /// See [`Configuration`].
    pub configuration: Configuration,
    /// The corpus a trained detector saw, or None. A detector scored on what it
    /// trained on is not being measured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trained_on: Option<String>,
    /// Whether this run scored a detector against the corpus it declared it
    /// was trained on.
    ///
    /// **Set by the harness from what it actually ran, never by the person
    /// running it**, the same rule `configuration` follows and for the same
    /// reason: the one person who should not be deciding whether a number is
    /// contaminated is the one who wants it to be clean.
    ///
    /// The leaderboard promised to reject a contaminated submission "without
    /// an explicit contamination flag" while no such flag existed, so the
    /// rule could not be applied to anything. The run has always warned about
    /// this in the terminal; the warning is what a reviewer never sees.
    ///
    /// Defaults to false so a document written before this existed still
    /// parses, which is safe because every such document predates any means
    /// of setting it.
    #[serde(default)]
    pub contaminated: bool,
    /// Whether this number came from whoever owns the detector, rather than
    /// from an independent run.
    ///
    /// `score` writes `true`, because it is being run by somebody and the
    /// harness has no way to know who. The submission path clears it when a
    /// run is reproduced independently.
    ///
    /// It used to be written `false` unconditionally, which meant a vendor
    /// scoring their own product got a document asserting the opposite, and
    /// nothing stopped that document being published as though a third party
    /// had produced it. A field that defaults to the flattering answer is
    /// worse than no field.
    pub self_reported: bool,
}

/// Whether this measurement is comparable to anybody else's.
///
/// The tier structure exists because two independently sampled subsets produce
/// numbers that look comparable and are not: somebody trains on one and
/// evaluates on another, and every figure they publish is inflated by an
/// amount nobody can recover afterwards. Nesting the tiers closes that at the
/// distribution layer.
///
/// This field closes it at the layer where a user can reopen it. A run over a
/// hand-picked subset is a perfectly good thing to do and a perfectly bad
/// thing to quote as a tier number, so it is marked rather than forbidden.
///
/// **Set by the harness from what it actually ran, never by the person
/// running it**, which is the same rule `self_reported` follows and for the
/// same reason: a field the subject can set in their own favour is not worth
/// having.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Configuration {
    /// A registered corpus tier, scored whole, with the arm and rate named in
    /// the registry. Comparable with any other result that says the same.
    Named,
    /// Anything else. A hand-picked subset, a run stopped early, a corpus
    /// edited locally. Comparable with itself and nothing else.
    Custom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum SplitDiscipline {
    /// A cover and its stego twin land on the same side of the split.
    /// Violating this inflates every number and is invisible in the output.
    ByCover,
    ByFile,
    /// No train/test split applies, as for an untrained detector.
    NotApplicable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Pairing {
    /// Clean and stego differ only in the payload. This is the whole
    /// reliability of the method: two measurement rounds have been voided here
    /// by a second variable, when outguess re-encoded at quality 75 against a
    /// clean half written at 95.
    ///
    /// **Read this as "no second variable was found", not as proof there is
    /// none.** Two files on disk cannot prove the positive claim; proving it
    /// would mean decoding both images and diffing the pixel arrays, and even
    /// that would miss a cover re-encoded before the payload went in. What the
    /// harness does is the reachable half: it compares the format, size, bit
    /// depth and channel count of every stego image against the cover it names,
    /// and any disagreement is a second variable it can point at.
    SingleVariable,
    /// Clean and stego differ in something else as well, and the arm is kept
    /// deliberately as a demonstration of what that does.
    Confounded,
    /// Nothing could be compared, so neither of the above is claimed.
    ///
    /// A corpus whose stego rows never name their covers, or whose images this
    /// cannot read, leaves the question open. Recording that plainly is the
    /// point: a benchmark that answers "single variable" when it looked at
    /// nothing is making exactly the unchecked claim this project exists to
    /// stop repeating.
    Unverified,
}

impl Result1 {
    /// Checks the things the type system cannot.
    ///
    /// Deliberately not a constructor guard: a document arriving from outside
    /// is parsed first and judged second, so that an invalid one produces a
    /// reason rather than a parse error.
    pub fn validate(&self) -> std::result::Result<(), Vec<String>> {
        let mut bad = Vec::new();
        if self.schema != RESULT_SCHEMA_ID {
            bad.push(format!(
                "schema is {:?}, expected {RESULT_SCHEMA_ID:?}",
                self.schema
            ));
        }
        if !(0.0..=1.0).contains(&self.metrics.auc) {
            bad.push(format!(
                "auc {} is outside 0 to 1, which is not a valid ROC AUC; check \
                 the scorer that computed it rather than the document",
                self.metrics.auc
            ));
        }
        if let Some([lo, hi]) = self.metrics.auc_ci95 {
            if !(0.0..=1.0).contains(&lo) || !(0.0..=1.0).contains(&hi) || lo > hi {
                bad.push(format!(
                    "auc_ci95 [{lo}, {hi}] is not an interval inside 0 to 1"
                ));
            } else if self.metrics.auc < lo || self.metrics.auc > hi {
                // Arithmetically impossible, so the document is reporting two
                // numbers that cannot both have come from one run. The per-arm
                // rows were checked for this and the headline was not, which
                // let `0.9900 [0.500, 0.500]` through validation and into a
                // rendered table.
                bad.push(format!(
                    "auc {} is outside its own interval [{lo}, {hi}]",
                    self.metrics.auc
                ));
            }
        }
        // The same checks the headline gets. An arm figure reaches a reader
        // through `report` looking exactly as measured as the pooled one, so
        // a per-arm AUC of 1.4 slipping through validation would be the very
        // failure the headline check exists to stop, one level down.
        let mut seen_arms = std::collections::BTreeSet::new();
        for a in &self.metrics.per_arm {
            if !(0.0..=1.0).contains(&a.auc) {
                bad.push(format!(
                    "per_arm {:?} has auc {}, which is outside 0 to 1 and is                      not a valid ROC AUC",
                    a.arm, a.auc
                ));
            }
            if let Some([lo, hi]) = a.auc_ci95 {
                if !(0.0..=1.0).contains(&lo) || !(0.0..=1.0).contains(&hi) || lo > hi {
                    bad.push(format!(
                        "per_arm {:?} has the interval [{lo}, {hi}], which is                          not an interval inside 0 to 1",
                        a.arm
                    ));
                } else if a.auc < lo || a.auc > hi {
                    // An interval that does not contain its own estimate is
                    // arithmetically impossible, so the document is reporting
                    // two numbers that cannot both have come from one run.
                    bad.push(format!(
                        "per_arm {:?} has auc {} outside its own interval                          [{lo}, {hi}]",
                        a.arm, a.auc
                    ));
                }
            }
            if a.arm.trim().is_empty() {
                bad.push("a per_arm entry names no arm".to_string());
            } else if !seen_arms.insert(a.arm.as_str()) {
                // Two rows for one arm means a reader picks whichever they
                // see first, and the two need not agree.
                bad.push(format!("per_arm names {:?} more than once", a.arm));
            }
        }
        for (fpr, tpr) in &self.metrics.tpr_at_fpr {
            match fpr.parse::<f64>() {
                Ok(f) if (0.0..=1.0).contains(&f) => {}
                _ => bad.push(format!(
                    "tpr_at_fpr key {fpr:?} is not a rate between 0 and 1; keys \
                     are false-positive rates as decimal fractions, for example \
                     \"0.01\" for one per cent"
                )),
            }
            if !(0.0..=1.0).contains(tpr) {
                bad.push(format!(
                    "tpr_at_fpr[{fpr}] = {tpr} is outside 0 to 1, which is not a \
                     valid detection rate; check the scorer that computed it"
                ));
            }
        }
        for (fpr, achieved) in &self.metrics.fpr_achieved {
            if !self.metrics.tpr_at_fpr.contains_key(fpr) {
                bad.push(format!(
                    "fpr_achieved names {fpr:?} and tpr_at_fpr carries no \
                     figure for it, so this document describes a measurement it \
                     does not report"
                ));
                continue;
            }
            match fpr.parse::<f64>() {
                Ok(requested) if (0.0..=1.0).contains(achieved) => {
                    // A budget is a ceiling, so the rate the run landed on is
                    // at or below it. Above it is not a coarse corpus, it is a
                    // figure measured outside the budget it is filed under.
                    if *achieved > requested + 1e-9 {
                        bad.push(format!(
                            "fpr_achieved[{fpr}] = {achieved} is above the \
                             rate it is filed under, and a false-alarm budget is \
                             a ceiling rather than a target"
                        ));
                    }
                }
                Ok(_) => bad.push(format!(
                    "fpr_achieved[{fpr}] = {achieved} is outside 0 to 1, \
                     which is not a false-alarm rate"
                )),
                // The key itself is already reported against `tpr_at_fpr`
                // above, so nothing is added here.
                Err(_) => {}
            }
        }
        for (fpr, threshold) in &self.metrics.threshold_at_fpr {
            if !self.metrics.tpr_at_fpr.contains_key(fpr) {
                bad.push(format!(
                    "threshold_at_fpr names {fpr:?} and tpr_at_fpr carries no \
                     figure for it, so this document hands out a cutoff for a \
                     budget it does not report"
                ));
            }
            // Deliberately NOT range checked. A threshold is a raw detector
            // score in that detector's own units, and those are not rates: one
            // tool answers a probability, another a chi-squared statistic,
            // another a negative estimate of an embedding rate. Any bound here
            // would be this project's opinion about somebody else's scale, and
            // the first detector it was wrong about would have a correct cutoff
            // refused. Only a value that cannot be compared at all is refused.
            if !threshold.is_finite() {
                bad.push(format!(
                    "threshold_at_fpr[{fpr}] = {threshold} is not a finite \
                     number, so nothing could be compared against it"
                ));
            }
        }
        if self.metrics.n_clean == 0 || self.metrics.n_stego == 0 {
            bad.push(
                "a measurement needs both clean and stego images; set n_clean \
                 and n_stego to the number actually scored on each side, not \
                 zero on either"
                    .into(),
            );
        }
        if let Some(rate) = self.arm.rate {
            let sane = match rate.unit {
                // A fraction of capacity above 1 is more than the cover holds.
                RateUnit::CapacityFraction => rate.value > 0.0 && rate.value <= 1.0,
                // 8 bits per pixel is every bit of an 8-bit channel.
                RateUnit::Bpp => rate.value > 0.0 && rate.value <= 8.0,
            };
            if !sane {
                bad.push(format!(
                    "rate {} is not a sensible {:?}",
                    rate.value, rate.unit
                ));
            }
        }
        // A named configuration is a claim that somebody else can reproduce
        // this, and reproducing it means fetching the same corpus. A result
        // that says "named" over a corpus only its author has is a claim
        // nobody can act on, and it is the shape a flattering number would
        // take if one were ever submitted.
        if self.declarations.configuration == Configuration::Named
            && self.corpus.source == CorpusSource::Supplied
            && self.corpus.digest.is_empty()
        {
            bad.push(
                "this result calls itself a named configuration over a corpus \
                 the harness did not fetch and cannot identify: with no digest \
                 there is nothing for anybody to check it against. Record the \
                 corpus digest, or mark the run custom"
                    .into(),
            );
        }
        // A tag where a digest belongs is the single most common way a result
        // becomes unreproducible, so it is named rather than left to the reader.
        //
        // TWO SHAPES ARE PINNED, NOT ONE. A container carries its digest after
        // an `@`. A binary plugin carries the SHA-256 of the bytes that were
        // actually executed, which has no `@` and is the stronger of the two:
        // a registry can move a tag under you, and it cannot change a file you
        // have already hashed. Insisting on the `@` refused every result this
        // harness produces for its own binary entries, which was found by the
        // harness refusing its own first real run.
        for p in &self.provenance.plugins {
            let pinned = p.image.contains('@') || p.image.starts_with("sha256:");
            if !pinned {
                bad.push(format!(
                    "plugin {:?} names image {:?}, which is not pinned by digest",
                    p.name, p.image
                ));
            }
        }
        // `isolation` and `network_reachable` are two statements about the same
        // run, and two of their combinations cannot both be true. Caught here
        // rather than left for a reader to notice, because the whole reason
        // `route` was split into two fields was that a document was able to
        // say a contradictory thing without anything objecting.
        if !self.provenance.plugins.is_empty()
            && self.provenance.network_reachable
            && self
                .provenance
                .plugins
                .iter()
                .all(|p| p.isolation == Isolation::SandboxNoNetwork)
        {
            bad.push(
                "every plugin here says it ran in a sandbox with no network, \
                 and network_reachable says the network was reachable. One of \
                 the two is wrong"
                    .into(),
            );
        }
        if !self.provenance.network_reachable
            && self
                .provenance
                .plugins
                .iter()
                .any(|p| p.isolation == Isolation::RemoteService)
        {
            bad.push(
                "a plugin here was reached over the network as a service, and \
                 network_reachable says there was no network. One of the two is \
                 wrong"
                    .into(),
            );
        }
        // A digest is the document's own statement about its own bytes, so a
        // claimed one that does not match is not a document to be judged on
        // its other merits: something changed after the run wrote it, and
        // every other field in it is a number nobody can stand behind. Absent
        // is not a fault, because absent means nobody offered one.
        //
        // THE RECOMPUTED DIGEST IS NOT PRINTED, AND THAT IS THE WHOLE POINT.
        //
        // It used to be, on the reasonable-sounding grounds that a reader
        // debugging a mismatch wants both numbers. An adversarial researcher
        // used it as an oracle: edit the AUC to 0.99, read the digest of the
        // edited document out of the refusal, paste it into the document's own
        // `content_digest`, and `validate` returns 0 and `verify` returns 0. The
        // seal was never broken; the refusal handed over the one thing he did
        // not have, which was the canonicalisation.
        //
        // So the refusal says the document does not match, and what it declares,
        // and nothing about what it would have to declare to pass. A reader with
        // a genuine mismatch has lost nothing: their next step is re-running the
        // measurement, not reconciling two hex strings by hand.
        if let Some(claimed) = &self.content_digest {
            if *claimed != self.compute_content_digest() {
                bad.push(format!(
                    "this document does not match its own content digest, which \
                     it declares as {claimed}. Every field except the two \
                     recording when the run happened is covered, so something in \
                     it changed after it was written. Re-run the measurement \
                     rather than trusting the number in it"
                ));
            }
        }
        if bad.is_empty() {
            Ok(())
        } else {
            Err(bad)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arm(name: &str, auc: f64, ci: Option<[f64; 2]>) -> ArmMetrics {
        ArmMetrics {
            arm: name.into(),
            auc,
            auc_ci95: ci,
            n_clean: 6,
            n_stego: 6,
        }
    }

    #[test]
    fn an_arm_auc_outside_the_unit_interval_is_refused_like_the_headline_one() {
        // It reaches a reader through `report` looking exactly as measured
        // as the pooled figure, so it has to clear the same bar.
        let mut d = sample();
        d.metrics.per_arm = vec![arm("wow-0200", 1.4, None)];
        let bad = d.validate().expect_err("refused");
        assert!(bad.iter().any(|m| m.contains("wow-0200")), "{bad:?}");
    }

    #[test]
    fn an_arm_interval_that_does_not_contain_its_own_estimate_is_refused() {
        // Arithmetically impossible, so the document is reporting two
        // numbers that cannot both have come from one run.
        let mut d = sample();
        d.metrics.per_arm = vec![arm("wow-0200", 0.9, Some([0.1, 0.5]))];
        let bad = d.validate().expect_err("refused");
        assert!(
            bad.iter().any(|m| m.contains("outside its own interval")),
            "{bad:?}"
        );
    }

    #[test]
    fn one_arm_named_twice_is_refused_because_a_reader_would_pick_one() {
        let mut d = sample();
        d.metrics.per_arm = vec![arm("wow-0200", 0.6, None), arm("wow-0200", 0.8, None)];
        let bad = d.validate().expect_err("refused");
        assert!(bad.iter().any(|m| m.contains("more than once")), "{bad:?}");
    }

    #[test]
    fn an_arm_with_no_name_is_refused() {
        let mut d = sample();
        d.metrics.per_arm = vec![arm("   ", 0.6, None)];
        let bad = d.validate().expect_err("refused");
        assert!(bad.iter().any(|m| m.contains("names no arm")), "{bad:?}");
    }

    #[test]
    fn a_well_formed_breakdown_validates() {
        let mut d = sample();
        d.metrics.per_arm = vec![
            arm("lsb-0100", 0.5, Some([0.1135, 0.8865])),
            arm("lsb-0400", 0.6944, Some([0.3588, 1.0])),
        ];
        assert_eq!(d.validate(), Ok(()));
    }

    #[test]
    fn two_runs_that_differ_only_in_when_they_ran_seal_to_the_same_digest() {
        // The whole point. Reproduction used to mean diffing two documents
        // and knowing which two fields to forgive; now it is a string
        // comparison, which a script can do and a reader can eyeball.
        let mut a = sample();
        let mut b = sample();
        b.provenance.started_utc = "2030-01-01T00:00:00Z".into();
        b.provenance.elapsed_seconds = a.provenance.elapsed_seconds + 41.5;
        a.seal();
        b.seal();
        assert_eq!(a.content_digest, b.content_digest);
        assert!(a.content_digest.is_some());
    }

    #[test]
    fn a_different_number_seals_to_a_different_digest() {
        let mut a = sample();
        let mut b = sample();
        b.metrics.auc = a.metrics.auc / 2.0;
        assert_ne!(
            a.metrics.auc, b.metrics.auc,
            "the fixture must carry an auc"
        );
        a.seal();
        b.seal();
        assert_ne!(a.content_digest, b.content_digest);
    }

    #[test]
    fn a_different_corpus_seals_to_a_different_digest() {
        // Two identical numbers measured on different bytes are different
        // results, and this is the field that says so.
        let mut a = sample();
        let mut b = sample();
        b.corpus.digest = "0".repeat(64);
        a.seal();
        b.seal();
        assert_ne!(a.content_digest, b.content_digest);
    }

    #[test]
    fn sealing_twice_does_not_change_the_answer() {
        // A digest cannot cover its own value, so the second seal has to see
        // the same bytes as the first. Getting this wrong would make the
        // field depend on how many times it was written.
        let mut a = sample();
        a.seal();
        let once = a.content_digest.clone();
        a.seal();
        assert_eq!(once, a.content_digest);
    }

    #[test]
    fn a_document_carrying_no_digest_round_trips_without_growing_one() {
        // Absent means nobody offered one. A submitted document is allowed
        // to say nothing here, and reading it must not invent an answer.
        let a = sample();
        assert_eq!(a.content_digest, None);
        let text = serde_json::to_string(&a).unwrap();
        assert!(!text.contains("content_digest"), "{text}");
        let back: Result1 = serde_json::from_str(&text).unwrap();
        assert_eq!(back.content_digest, None);
    }

    #[test]
    fn a_headline_interval_that_does_not_contain_its_own_auc_is_refused() {
        // `0.9900 [0.500, 0.500]` reached a rendered table because only the
        // per-arm rows were checked for this.
        let mut d = sample();
        d.metrics.auc = 0.99;
        d.metrics.auc_ci95 = Some([0.5, 0.5]);
        let bad = d.validate().expect_err("refused");
        assert!(
            bad.iter().any(|m| m.contains("outside its own interval")),
            "{bad:?}"
        );
    }

    #[test]
    fn a_headline_interval_outside_the_unit_interval_is_refused() {
        for ci in [[-0.1, 0.9], [0.5, 1.4], [0.9, 0.2]] {
            let mut d = sample();
            d.metrics.auc_ci95 = Some(ci);
            let bad = d.validate().expect_err("refused");
            assert!(
                bad.iter().any(|m| m.contains("not an interval inside")),
                "{ci:?} gave {bad:?}"
            );
        }
    }

    #[test]
    fn a_headline_interval_that_contains_its_own_auc_validates() {
        let mut d = sample();
        d.metrics.auc_ci95 = Some([0.9312, 0.9821]);
        assert_eq!(d.validate(), Ok(()));
    }

    #[test]
    fn an_achieved_rate_above_the_budget_it_is_filed_under_is_refused() {
        // A false-alarm budget is a ceiling. Landing above it is a figure
        // measured outside the column it is being reported in.
        let mut d = sample();
        d.metrics.fpr_achieved = BTreeMap::from([("0.01".into(), 0.1667)]);
        let bad = d.validate().expect_err("refused");
        assert!(bad.iter().any(|m| m.contains("is a ceiling")), "{bad:?}");
    }

    #[test]
    fn an_achieved_rate_for_a_figure_the_document_does_not_carry_is_refused() {
        let mut d = sample();
        d.metrics.fpr_achieved = BTreeMap::from([("0.05".into(), 0.0)]);
        let bad = d.validate().expect_err("refused");
        assert!(bad.iter().any(|m| m.contains("does not report")), "{bad:?}");
    }

    #[test]
    fn an_achieved_rate_outside_zero_to_one_is_refused() {
        let mut d = sample();
        d.metrics.fpr_achieved = BTreeMap::from([("0.01".into(), -0.5)]);
        let bad = d.validate().expect_err("refused");
        assert!(
            bad.iter().any(|m| m.contains("not a false-alarm rate")),
            "{bad:?}"
        );
    }

    #[test]
    fn an_achieved_rate_below_its_budget_is_recorded_rather_than_refused() {
        // The whole point of the field: a coarse corpus answers a 1 per cent
        // budget at zero, and that is an honest measurement to be labelled
        // rather than a fault to be rejected.
        let mut d = sample();
        d.metrics.fpr_achieved = BTreeMap::from([("0.01".into(), 0.0), ("0.10".into(), 0.0833)]);
        assert_eq!(d.validate(), Ok(()));
    }

    #[test]
    fn a_document_written_before_the_achieved_rates_existed_still_parses_and_validates() {
        let mut v: serde_json::Value = serde_json::to_value(sample()).unwrap();
        v["metrics"].as_object_mut().unwrap().remove("fpr_achieved");
        let parsed: Result1 = serde_json::from_value(v).expect("an older document still reads");
        assert!(parsed.metrics.fpr_achieved.is_empty());
        assert_eq!(parsed.validate(), Ok(()));
    }

    #[test]
    fn the_achieved_rates_are_not_written_under_a_name_that_says_detection_rate() {
        // The finding: this field went out as `tpr_at_fpr_achieved`, which
        // names a true-positive rate and holds a false-alarm rate. An examiner
        // disclosing the raw JSON read the 0.0 beside a 0.0833 detection rate
        // as a detection rate of zero. The rendered table was never wrong; the
        // bytes that get disclosed were.
        let mut d = sample();
        d.metrics.fpr_achieved = BTreeMap::from([("0.01".into(), 0.0)]);
        let text = serde_json::to_string(&d).unwrap();
        assert!(text.contains(r#""fpr_achieved":{"0.01":0.0}"#), "{text}");
        assert!(!text.contains("tpr_at_fpr_achieved"), "{text}");
    }

    #[test]
    fn a_document_written_under_the_old_achieved_rate_name_keeps_its_figures() {
        // The field is optional, so a rename without an alias would let an
        // already-written document parse with the rates silently gone, and a
        // reader would get the bare detection rate back with nothing saying
        // which false-alarm rate it came from.
        let mut v: serde_json::Value = serde_json::to_value(sample()).unwrap();
        let metrics = v["metrics"].as_object_mut().unwrap();
        metrics.remove("fpr_achieved");
        metrics.insert(
            "tpr_at_fpr_achieved".into(),
            serde_json::json!({"0.01": 0.0}),
        );
        let parsed: Result1 = serde_json::from_value(v).expect("an older document still reads");
        assert_eq!(
            parsed.metrics.fpr_achieved,
            BTreeMap::from([("0.01".to_string(), 0.0)])
        );
        assert_eq!(parsed.validate(), Ok(()));
    }

    #[test]
    fn empty_achieved_rates_are_left_out_of_the_serialised_document() {
        let text = serde_json::to_string(&sample()).unwrap();
        assert!(!text.contains("fpr_achieved"), "{text}");
    }

    #[test]
    fn a_tampered_document_no_longer_matches_its_own_seal() {
        // The finding: one edited field, and `validate` said the document was
        // valid while only `verify` noticed.
        let mut d = sample();
        d.seal();
        d.metrics.auc = 0.99;
        let bad = d.validate().expect_err("refused");
        assert!(
            bad.iter()
                .any(|m| m.contains("does not match its own content digest")),
            "{bad:?}"
        );
    }

    /// Rung 5 of the journey round: the refusal used to print the digest it had
    /// computed over the edited document, which is exactly the value a forger
    /// needs and the only thing standing between editing the JSON and resealing
    /// it. The researcher pasted it in and the document validated clean.
    #[test]
    fn the_refusal_does_not_hand_over_the_digest_that_would_make_it_pass() {
        let mut d = sample();
        d.seal();
        let honest = d.content_digest.clone().expect("sealed");
        d.metrics.auc = 0.99;
        let forged = d.compute_content_digest();
        assert_ne!(honest, forged, "the edit did not change the digest");

        let bad = d.validate().expect_err("refused");
        let said = bad.join(" ");
        assert!(
            said.contains(&honest),
            "the refusal no longer says what the document declares: {said}"
        );
        assert!(
            !said.contains(&forged),
            "the refusal still hands over the forging digest: {said}"
        );
    }

    #[test]
    fn a_sealed_document_nobody_touched_validates() {
        let mut d = sample();
        d.seal();
        assert_eq!(d.validate(), Ok(()));
    }

    /// The seal has to survive the journey it is actually made for, which is
    /// disk, not memory.
    ///
    /// Every other seal test sealed and validated the same in-memory value,
    /// so all of them passed while every written document with an awkward
    /// number in it failed the moment a reader opened it. The digest is
    /// computed over the PARSED document, so the round trip through text is
    /// the thing under test and leaving it out tested nothing.
    ///
    /// `0.9065609581885931` is not decoration. serde_json's default parser
    /// is not bit-exact, and this is one of the values it returns a
    /// neighbouring float for, so without `float_roundtrip` this document
    /// seals correctly, writes correctly and then fails its own seal.
    #[test]
    fn a_sealed_document_survives_being_written_and_read_back() {
        let mut d = sample();
        d.metrics.auc = 0.9722222222222222;
        d.metrics.auc_ci95 = Some([0.9065609581885931, 1.0]);
        d.provenance.elapsed_seconds = 18.179600147000002;
        d.seal();

        let text = serde_json::to_string_pretty(&d).expect("serialises");
        let read: Result1 = serde_json::from_str(&text).expect("parses");

        assert_eq!(
            read.content_digest, d.content_digest,
            "the seal did not survive the write"
        );
        assert_eq!(
            read.compute_content_digest(),
            d.compute_content_digest(),
            "the document re-read hashes differently from the one written"
        );
        assert_eq!(read.validate(), Ok(()));
    }

    /// Every float the document can carry has to mean the same number after
    /// a round trip, because the seal is a digest over all of them.
    #[test]
    fn a_float_the_parser_rounds_would_break_every_seal_that_carries_it() {
        for x in [
            0.9065609581885931f64,
            18.179600147000002,
            0.8156077410606579,
            0.9722222222222222,
        ] {
            let back: f64 = serde_json::from_str(&serde_json::to_string(&x).unwrap()).unwrap();
            assert_eq!(
                back.to_bits(),
                x.to_bits(),
                "{x} did not survive a JSON round trip, so any document \
                 carrying it cannot be sealed"
            );
        }
    }

    #[test]
    fn the_two_timing_fields_may_change_after_a_seal_without_breaking_it() {
        // They are the two fields the digest deliberately leaves out, so a
        // document re-timed by a rerun is still the same measurement.
        let mut d = sample();
        d.seal();
        d.provenance.started_utc = "2030-01-01T00:00:00Z".into();
        d.provenance.elapsed_seconds += 41.5;
        assert_eq!(d.validate(), Ok(()));
    }

    #[test]
    fn a_document_carrying_no_seal_is_not_accused_of_breaking_one() {
        // Absent means nobody offered a digest, which is legitimate for a
        // hand-written or submitted document.
        let d = sample();
        assert_eq!(d.content_digest, None);
        assert_eq!(d.validate(), Ok(()));
    }

    fn sample() -> Result1 {
        Result1 {
            schema: RESULT_SCHEMA_ID.into(),
            subject: Subject {
                name: "stegashield".into(),
                version: "sha256:64a6a378".into(),
                kind: SubjectKind::Detector,
            },
            corpus: CorpusRef {
                name: "pentimento-core".into(),
                id: Some("pentimento-core".into()),
                tier: Some("core".into()),
                source: CorpusSource::Fetched,
                digest: "sha256:b633b019".into(),
                pairs: 1000,
                of_items: None,
                split: Some("test".into()),
            },
            arm: Arm {
                embedder: "suniward".into(),
                rate: Some(Rate {
                    value: 0.4,
                    unit: RateUnit::Bpp,
                }),
                domain: Domain::Spatial,
                format: "png".into(),
            },
            metrics: Metrics {
                auc: 0.9634,
                auc_ci95: None,
                tpr_at_fpr: BTreeMap::from([("0.01".into(), 0.4133), ("0.10".into(), 0.9033)]),
                fpr_achieved: BTreeMap::new(),
                threshold_at_fpr: BTreeMap::new(),
                verdict_rate: None,
                n_clean: 300,
                n_stego: 300,
                per_arm: Vec::new(),
                n_error: 0,
            },
            provenance: Provenance {
                seed: Some(20260917),
                plugins: vec![PluginRef {
                    name: "aletheia-rich".into(),
                    image: "ghcr.io/x/y@sha256:abc".into(),
                    determinism: Determinism::Exact,
                    selftest: None,
                    pinned_by: PinnedBy::ImageDigest,
                    adapter: None,
                    argv: None,
                    isolation: Isolation::SandboxNoNetwork,
                }],
                harness_version: "0.1.0".into(),
                started_utc: "2026-09-17T09:02:11Z".into(),
                elapsed_seconds: 10754.8,
                network_reachable: false,
                host: None,
                records: None,
            },
            declarations: Declarations {
                split_discipline: SplitDiscipline::ByCover,
                pairing: Pairing::SingleVariable,
                configuration: Configuration::Named,
                trained_on: None,
                contaminated: false,
                self_reported: false,
            },
            content_digest: None,
        }
    }

    #[test]
    fn a_well_formed_result_validates() {
        assert_eq!(sample().validate(), Ok(()));
    }

    #[test]
    fn round_trips_through_json_unchanged() {
        let a = sample();
        let text = serde_json::to_string(&a).unwrap();
        let b: Result1 = serde_json::from_str(&text).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn unknown_fields_are_ignored_so_a_v1_reader_survives_a_later_writer() {
        // The stated compatibility rule: adding an optional field is not a
        // version bump, so a strict reader would break on our own next release.
        let mut v: serde_json::Value = serde_json::to_value(sample()).unwrap();
        v["metrics"]["some_future_metric"] = serde_json::json!(1.0);
        let parsed: Result1 = serde_json::from_value(v).unwrap();
        assert_eq!(parsed.metrics.auc, 0.9634);
    }

    #[test]
    fn an_auc_outside_the_unit_interval_is_refused() {
        let mut r = sample();
        r.metrics.auc = 1.4;
        assert!(r.validate().unwrap_err()[0].contains("outside 0 to 1"));
    }

    #[test]
    fn a_mutable_image_tag_is_refused_because_it_cannot_be_reproduced() {
        let mut r = sample();
        r.provenance.plugins[0].image = "ghcr.io/x/y:latest".into();
        assert!(r.validate().unwrap_err()[0].contains("not pinned by digest"));
    }

    #[test]
    fn a_binary_plugin_is_pinned_by_the_hash_of_what_ran() {
        // A local program has no registry and no tag. The SHA-256 of the file
        // that was executed pins it harder than a digest does, and refusing it
        // rejected every result this harness produces for its own binary
        // entries, which is how this was found.
        let mut r = sample();
        r.provenance.plugins[0].image =
            "sha256:e5cb26609a59ac554cb4bca9763fa206977f3e5e584eb8bf62afa260c1cb36f0".into();
        assert!(r.validate().is_ok(), "{:?}", r.validate());
    }

    #[test]
    fn two_detectors_sharing_one_image_are_told_apart_by_what_was_asked() {
        // ONE IMAGE CAN SERVE SEVERAL DETECTORS. `aletheia-rs` and
        // `aletheia-spa` run the same container from the same digest through
        // the same adapter file and differ only in a trailing word, so two
        // documents measuring two different things carried identical `image`,
        // identical `adapter` and identical `subject.version`. Nothing in
        // either said which had been measured, and a reader comparing them
        // had no way to find out.
        let mut rs = sample();
        rs.provenance.plugins[0].argv =
            Some(vec!["{adapter}".into(), "{file}".into(), "rs".into()]);
        let mut spa = sample();
        spa.provenance.plugins[0].argv =
            Some(vec!["{adapter}".into(), "{file}".into(), "spa".into()]);
        assert_eq!(
            rs.provenance.plugins[0].image,
            spa.provenance.plugins[0].image
        );
        assert_ne!(
            rs.provenance.plugins[0].argv,
            spa.provenance.plugins[0].argv
        );
        assert!(rs.validate().is_ok(), "{:?}", rs.validate());
    }

    #[test]
    fn the_recorded_argv_is_the_declared_one_and_carries_no_host_path() {
        // DECLARED, with its placeholders still in it. The expanded command
        // bind mounts the adapter from an absolute path on the machine that
        // ran it, and this document exists to be handed to somebody else: a
        // disclosure package is the last place to put somebody's home
        // directory.
        let mut r = sample();
        r.provenance.plugins[0].argv = Some(vec!["{adapter}".into(), "{file}".into(), "rs".into()]);
        let written = serde_json::to_string(&r).unwrap();
        assert!(written.contains("{adapter}"), "{written}");
        assert!(!written.contains("/home/"), "{written}");
    }

    #[test]
    fn a_bare_version_string_is_still_refused() {
        // The rule has to keep catching the thing it was written for: a
        // version that names no particular bytes.
        let mut r = sample();
        r.provenance.plugins[0].image = "1.4.2".into();
        assert!(r.validate().unwrap_err()[0].contains("not pinned by digest"));
    }

    #[test]
    fn a_named_configuration_over_an_unidentifiable_corpus_is_refused() {
        // The shape a flattering submission would take: claim the tier
        // everybody compares against, over a copy only the author has, with
        // nothing anybody can check it against.
        let mut r = sample();
        r.declarations.configuration = Configuration::Named;
        r.corpus.source = CorpusSource::Supplied;
        r.corpus.digest = String::new();
        let problems = r.validate().unwrap_err();
        assert!(
            problems.iter().any(|p| p.contains("mark the run custom")),
            "got {problems:?}"
        );
    }

    #[test]
    fn a_supplied_corpus_is_fine_when_it_can_be_identified() {
        // Two of the four registered corpora may not be redistributed, so
        // pointing at a copy you already have is the ordinary case and must
        // not be treated as suspect on its own.
        let mut r = sample();
        r.corpus.source = CorpusSource::Supplied;
        assert!(r.validate().is_ok(), "{:?}", r.validate());
    }

    #[test]
    fn a_custom_run_needs_no_digest_to_be_valid() {
        // A run over a subset somebody assembled is a reasonable thing to do.
        // It is marked rather than forbidden, so it must still validate.
        let mut r = sample();
        r.declarations.configuration = Configuration::Custom;
        r.corpus.source = CorpusSource::Supplied;
        r.corpus.digest = String::new();
        assert!(r.validate().is_ok(), "{:?}", r.validate());
    }

    #[test]
    fn the_configuration_and_the_corpus_source_survive_a_round_trip() {
        // Both are read by the submission path to decide a division, so a
        // field that silently defaulted would put an entry in the wrong one.
        let mut r = sample();
        r.declarations.configuration = Configuration::Custom;
        r.corpus.source = CorpusSource::Supplied;
        let text = serde_json::to_string(&r).unwrap();
        assert!(text.contains("\"configuration\":\"custom\""), "{text}");
        assert!(text.contains("\"source\":\"supplied\""), "{text}");
        let back: Result1 = serde_json::from_str(&text).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn neither_new_field_may_be_omitted() {
        // Both are claims, and an omitted claim that defaults to the
        // flattering value is worse than no field at all.
        for (object, field) in [("corpus", "source"), ("declarations", "configuration")] {
            let mut v: serde_json::Value = serde_json::to_value(sample()).unwrap();
            v[object].as_object_mut().unwrap().remove(field);
            assert!(
                serde_json::from_value::<Result1>(v).is_err(),
                "{object}.{field} was allowed to be missing"
            );
        }
    }

    #[test]
    fn n_error_is_required_rather_than_optional() {
        // Serialised without n_error, parsing must fail rather than defaulting
        // to zero: a silent zero is exactly the claim we refuse to invent.
        let mut v: serde_json::Value = serde_json::to_value(sample()).unwrap();
        v["metrics"].as_object_mut().unwrap().remove("n_error");
        assert!(serde_json::from_value::<Result1>(v).is_err());
    }

    #[test]
    fn a_capacity_fraction_above_one_is_refused() {
        // 5% of capacity and 0.4 bits per pixel are different quantities, and
        // the unit is what stops a reader comparing them.
        let mut r = sample();
        r.arm.rate = Some(Rate {
            value: 1.6,
            unit: RateUnit::CapacityFraction,
        });
        assert!(r.validate().unwrap_err()[0].contains("not a sensible"));
    }

    #[test]
    fn the_same_number_can_be_valid_in_one_unit_and_not_the_other() {
        let mut r = sample();
        r.arm.rate = Some(Rate {
            value: 4.0,
            unit: RateUnit::Bpp,
        });
        assert_eq!(r.validate(), Ok(()));
        r.arm.rate = Some(Rate {
            value: 4.0,
            unit: RateUnit::CapacityFraction,
        });
        assert!(r.validate().is_err());
    }

    #[test]
    fn an_empty_side_is_not_a_measurement() {
        let mut r = sample();
        r.metrics.n_stego = 0;
        assert!(r.validate().is_err());
    }

    #[test]
    fn neither_half_of_the_old_route_field_may_be_omitted() {
        // The whole reason there are two fields is that a reader must be able
        // to ask each question separately. A missing one defaulting to
        // anything would put the reader back where `route` left them.
        for field in ["pinned_by", "isolation"] {
            let mut v: serde_json::Value = serde_json::to_value(sample()).unwrap();
            v["provenance"]["plugins"][0]
                .as_object_mut()
                .unwrap()
                .remove(field);
            assert!(
                serde_json::from_value::<Result1>(v).is_err(),
                "plugins[].{field} was allowed to be missing"
            );
        }
    }

    #[test]
    fn a_service_measured_with_no_network_is_refused() {
        let mut r = sample();
        r.provenance.plugins[0].isolation = Isolation::RemoteService;
        let problems = r.validate().expect_err("contradictory");
        assert!(
            problems.iter().any(|p| p.contains("as a service")),
            "{problems:?}"
        );
        r.provenance.network_reachable = true;
        assert_eq!(r.validate(), Ok(()));
    }

    #[test]
    fn a_sandbox_beside_a_reachable_network_is_refused() {
        let mut r = sample();
        r.provenance.network_reachable = true;
        let problems = r.validate().expect_err("contradictory");
        assert!(
            problems.iter().any(|p| p.contains("sandbox")),
            "{problems:?}"
        );
    }

    #[test]
    fn a_host_run_may_reach_the_network_without_objection() {
        let mut r = sample();
        r.provenance.plugins[0].isolation = Isolation::Host;
        r.provenance.plugins[0].pinned_by = PinnedBy::ExecutableHash;
        r.provenance.plugins[0].image = "sha256:abc".into();
        r.provenance.network_reachable = true;
        assert_eq!(r.validate(), Ok(()));
    }

    #[test]
    fn a_document_with_no_plugins_is_not_accused_of_contradicting_itself() {
        // Nothing recorded is not a claim about a sandbox, and treating it as
        // one would refuse documents whose only fault is being thin.
        let mut r = sample();
        r.provenance.plugins.clear();
        r.provenance.network_reachable = true;
        assert_eq!(r.validate(), Ok(()));
    }

    #[test]
    fn an_unpinned_plugin_still_has_to_name_a_digest_or_be_refused() {
        // `pinned_by: unpinned` is an admission, not a licence to skip the
        // digest check: a document saying both is saying nothing at all.
        let mut r = sample();
        r.provenance.plugins[0].pinned_by = PinnedBy::Unpinned;
        r.provenance.plugins[0].image = "unknown".into();
        assert!(r.validate().is_err());
    }

    #[test]
    fn every_isolation_value_survives_a_round_trip_under_the_name_it_publishes() {
        for (value, written) in [
            (Isolation::SandboxNoNetwork, "sandbox-no-network"),
            (Isolation::Host, "host"),
            (Isolation::RemoteService, "remote-service"),
            (Isolation::Unstated, "unstated"),
        ] {
            assert_eq!(serde_json::to_value(value).unwrap(), written);
            assert_eq!(
                serde_json::from_value::<Isolation>(serde_json::json!(written)).unwrap(),
                value
            );
        }
        for (value, written) in [
            (Determinism::Exact, "exact"),
            (Determinism::Seeded, "seeded"),
            (Determinism::Nondeterministic, "nondeterministic"),
            (Determinism::Unstated, "unstated"),
        ] {
            assert_eq!(serde_json::to_value(value).unwrap(), written);
            assert_eq!(
                serde_json::from_value::<Determinism>(serde_json::json!(written)).unwrap(),
                value
            );
        }
        for (value, written) in [
            (PinnedBy::ImageDigest, "image-digest"),
            (PinnedBy::ExecutableHash, "executable-hash"),
            (PinnedBy::Unpinned, "unpinned"),
        ] {
            assert_eq!(serde_json::to_value(value).unwrap(), written);
            assert_eq!(
                serde_json::from_value::<PinnedBy>(serde_json::json!(written)).unwrap(),
                value
            );
        }
    }

    #[test]
    fn the_old_route_value_is_not_quietly_accepted_as_either_new_field() {
        // Unknown fields are ignored by design, so a document left on the old
        // shape must fail on the two it is missing rather than half-parse.
        let mut v: serde_json::Value = serde_json::to_value(sample()).unwrap();
        let plugin = v["provenance"]["plugins"][0].as_object_mut().unwrap();
        plugin.remove("pinned_by");
        plugin.remove("isolation");
        plugin.insert("route".into(), serde_json::json!("container"));
        assert!(serde_json::from_value::<Result1>(v).is_err());
    }
}
