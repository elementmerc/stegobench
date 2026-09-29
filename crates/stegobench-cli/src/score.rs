// Author:  Daniel Iwugo
// Comment: Christ is King
// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Daniel Iwugo

//! `stegobench score`: the headline verb, joined up.
//!
//! Three pieces that already existed do the work. The corpus reader streams
//! samples off disk, the runner asks the detector about each one and records
//! the answer as it goes, and the metrics crate turns scores and labels into
//! numbers. This module is the seam between them, and the place the result
//! document is assembled.
//!
//! WHERE THE MEMORY GOES, STATED RATHER THAN IMPLIED
//!
//! The scoring pass holds one item at a time, so a corpus of any size costs
//! the same. The metrics pass does not and cannot: a ROC AUC is defined over
//! the whole set and needs every score before it can rank them. That is one
//! f64 and one bool per item, so a Core tier is a few megabytes, and it is
//! bounded by the corpus rather than by anything this code chooses.
//!
//! It is worth naming because it is the one place a bigger corpus eventually
//! bites, and a reader of this file should meet that fact here rather than
//! discover it at ten times the scale.
//!
//! WHY THE WORK IS IN TWO HALVES
//!
//! [`prepare`] establishes what the corpus IS: its digest, whether it is the
//! corpus a registry entry named, whether every image is the file its record
//! describes, whether a cover and its stego twin land on the same side of the
//! split, and what each item's label is. None of those answers depends on
//! which detector is asked, and on a Core tier they cost tens of minutes and a
//! full read of every byte.
//!
//! [`score_one`] then asks one detector, and can be called again for the next
//! one against the same [`Prepared`]. Scoring seven detectors therefore pays
//! for the corpus once rather than seven times, which is the difference
//! between an afternoon and a day and a half, and the reason a baseline over
//! the whole registry is a command somebody will actually run.

use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use stegobench_core::corpus::CorpusEntry;
use stegobench_core::header::{self, Shape};
use stegobench_core::registry::{Entry, Kind};
use stegobench_core::result::{
    Arm, Configuration, CorpusRef, CorpusSource, Declarations, Determinism, Domain, Host,
    Isolation, Metrics, Pairing, PinnedBy, PluginRef, Provenance, Rate, RateUnit, Result1,
    SplitDiscipline, Subject, SubjectKind, RESULT_SCHEMA_ID,
};
use stegobench_core::samples::{Role, Sample, Samples};
use stegobench_plugin::runner::{self, JsonLines, Tally};
use stegobench_plugin::{Record, WorkItem};

/// What the caller asked for, gathered so the signature stays readable.
pub struct Request<'a> {
    pub corpus: &'a Path,
    pub records: PathBuf,
    pub timeout: Duration,
    pub limit: Option<u64>,
    /// The registry's entry for this corpus, where the caller named one.
    ///
    /// Carries the name, the tier and, if anybody has computed it, the digest
    /// this run is checked against. See [`Configuration`] for why the check
    /// rather than the name is what earns `named`.
    pub registered: Option<&'a CorpusEntry>,
}

/// Why a run could not produce a result.
#[derive(Debug, thiserror::Error)]
pub enum ScoreError {
    #[error("{0}")]
    Corpus(#[from] stegobench_core::samples::SampleError),
    #[error("{0}")]
    Run(#[from] runner::RunError),
    #[error("could not open the records file at {path}: {source}")]
    Records {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "{name} is registered as an embedder, and an embedder cannot be asked \
         to tell two images apart. Pick a detector: `stegobench list detectors`"
    )]
    NotADetector { name: String },
    #[error(
        "the corpus at {path} puts {count} stego image(s) on a different side \
         of the train and test split from the cover they were made from, for \
         example: {examples}. A cover and its stego twin on opposite sides \
         inflates every number computed from this corpus, and the inflation is \
         invisible in the output, so this refuses rather than reporting a \
         confident wrong answer"
    )]
    SplitLeaks {
        path: String,
        count: usize,
        examples: String,
    },
    #[error(
        "the directory at {path} is not {id}. That corpus is registered with \
         the digest {want}, and these records come to {got}. Whatever the \
         directory is called, it is not the corpus you named, and a number \
         measured here would be filed under a name it does not belong to"
    )]
    NotThatCorpus {
        path: String,
        id: String,
        want: String,
        got: String,
    },
    #[error(
        "the corpus at {path} holds {clean} clean and {stego} stego image(s), \
         and a measurement needs both. A detector scored on one side of the \
         question has not been measured, it has been asked a leading one"
    )]
    OneSided {
        path: String,
        clean: u64,
        stego: u64,
    },
    #[error(
        "{name} answered about {answered} image(s), and no ROC AUC can be \
         computed from those answers, so this run has no headline number. \
         {why}\n\nA result-v1 document has to carry an AUC, and the value \
         that would have been written is 0.5, which is exactly the figure \
         that means a detector carries no information at all. Publishing it \
         would be indistinguishable from publishing a measurement that came \
         out at chance, so the run is refused instead and nothing is written."
    )]
    NoAuc {
        name: String,
        answered: usize,
        why: String,
    },
    #[error(
        "{id} is not the image its own record describes. The record states \
         {want} and the file on disk is {got}. A run cannot be named over a \
         corpus whose images and records disagree: the digest that names the \
         corpus is taken over what the records SAY, so swapping the images \
         underneath them leaves it unchanged"
    )]
    ImageChanged {
        id: String,
        want: String,
        got: String,
    },
}

impl ScoreError {
    /// Which documented exit code this is.
    ///
    /// Every one of these used to leave the process with 4, plugin failure,
    /// including four cases where no plugin was involved and one where nothing
    /// had run yet. The codes are a contract a script and an agent act on, and
    /// the distinction that matters to both is whether retrying unchanged
    /// could work: a refusal will refuse again, a breakage might not.
    pub fn exit_code(&self) -> i32 {
        use stegobench_core::exit;
        match self {
            // The caller asked for something incoherent. Fix the command.
            ScoreError::NotADetector { .. } => exit::USAGE,
            // A claim about the bytes was checked and did not hold.
            ScoreError::NotThatCorpus { .. } | ScoreError::ImageChanged { .. } => {
                exit::VERIFY_MISMATCH
            }
            // The corpus is unfit for a measurement, and will be as unfit the
            // second time. Refusals, not breakages.
            ScoreError::SplitLeaks { .. } | ScoreError::OneSided { .. } => exit::PREFLIGHT_REFUSED,
            // Something broke: a tool, a disk, a file that is not what it
            // claimed to be. A run whose answers cannot be turned into a
            // metric belongs here rather than with the refusals: the corpus
            // was fit to measure and what came back was not fit to rank, so
            // retrying against a fixed tool genuinely could work.
            ScoreError::Run(_) | ScoreError::NoAuc { .. } => exit::PLUGIN_FAILED,
            ScoreError::Corpus(_) | ScoreError::Records { .. } => exit::FAILURE,
        }
    }
}

/// Everything about the corpus that does not depend on which detector is asked.
///
/// Built once by [`prepare`] and handed to [`score_one`] for each detector in
/// turn. A field here is a property of the bytes on disk; anything that varies
/// with the subject stays out.
pub struct Prepared {
    corpus: PathBuf,
    limit: Option<u64>,
    /// The registry entry's own name and tier for the corpus, copied out so a
    /// `Prepared` does not borrow the registry for its whole life.
    registered: Option<(String, Option<String>)>,
    checks: Checks,
    /// Whether the directory was proved to be the corpus a registry entry
    /// named, images included.
    claim_holds: bool,
    /// One label per scorable item, in corpus order, truncated by `limit`.
    ///
    /// Held once for the whole command rather than re-walked per detector. It
    /// is one bool per item, so a Core tier is 344 KB, which is an order of
    /// magnitude below the score vector the metrics pass already needs.
    labels: Vec<bool>,
    /// When the shared preparation began, and how long it took.
    ///
    /// Every result written from this `Prepared` reports `started_utc` as the
    /// moment preparation began and folds `preflight` into its own elapsed
    /// time. Both are true of each result: establishing what the corpus was is
    /// work that result rests on. What no result claims is the time spent on
    /// the OTHER detectors, because waiting for a different measurement is not
    /// part of making this one.
    started_utc: String,
    preflight: Duration,
}

impl Prepared {
    /// How many items each detector will be asked about.
    pub fn items(&self) -> u64 {
        self.labels.len() as u64
    }

    /// Seconds spent establishing what the corpus is, before any detector ran.
    pub fn preflight_seconds(&self) -> f64 {
        self.preflight.as_secs_f64()
    }
}

/// Run the detector over the corpus and build the result document.
///
/// The single-detector path, kept as one call. It is [`prepare`] followed by
/// [`score_one`], and a caller scoring several detectors uses those two
/// directly so the corpus is established once.
pub fn score<P>(
    entry: &Entry,
    request: &Request,
    mut progress: P,
) -> Result<(Result1, Tally), ScoreError>
where
    P: FnMut(&str),
{
    // Asked before the corpus is touched. An embedder cannot be scored however
    // good the corpus is, and finding that out after a Core-tier walk would be
    // finding it out far too late.
    refuse_embedder(entry)?;
    let prepared = prepare(
        request.corpus,
        request.registered,
        request.limit,
        &mut progress,
    )?;
    score_one(
        entry,
        &prepared,
        &request.records,
        request.timeout,
        &mut progress,
    )
}

fn refuse_embedder(entry: &Entry) -> Result<(), ScoreError> {
    if entry.kind == Kind::Embedder {
        return Err(ScoreError::NotADetector {
            name: entry.name.clone(),
        });
    }
    Ok(())
}

/// Establish what the corpus is, once, before any detector is asked anything.
///
/// Every refusal this can raise is a fact about the corpus rather than about a
/// tool, so raising them here means a seven-detector command refuses before it
/// spends an hour on the first one rather than after.
pub fn prepare<P>(
    corpus: &Path,
    registered: Option<&CorpusEntry>,
    limit: Option<u64>,
    mut progress: P,
) -> Result<Prepared, ScoreError>
where
    P: FnMut(&str),
{
    // WHY NAMING A CORPUS IS NOT ENOUGH TO EARN `named`
    //
    // Started HERE, before the corpus is identified rather than after. The
    // pre-flight passes below read every record and, for a named run, hash
    // every image, which on a Core tier is tens of minutes. A reader takes
    // elapsed_seconds as how long the run took, and time spent establishing
    // what was being measured is part of how long it took.
    let started = Instant::now();
    let started_utc = now_utc();

    // `Configuration` is documented as set by the harness from what it
    // actually ran, never by the person running it, and that is the whole
    // value of the field: one somebody can set in their own favour is not
    // worth having. A `--corpus-id` flag that took the user's word would hand
    // them exactly that.
    //
    // So the flag states a claim and this checks it, against a digest an
    // INDEPENDENT registry entry declared in advance. Checked HERE, before a
    // single image is scored, because the answer does not depend on the run
    // and a Core tier is hours: discovering at the end that the directory was
    // never the corpus named is discovering it far too late.
    let claim_holds = match registered {
        None => false,
        Some(entry) => match entry
            .integrity
            .as_ref()
            .and_then(|i| i.records_sha256.as_deref())
        {
            None => {
                progress(&format!(
                    "{} is registered but its entry declares no records \
                     digest, so there is nothing to check this directory \
                     against and the run is marked custom",
                    entry.id
                ));
                false
            }
            Some(want) => {
                let got = corpus_digest(corpus)?;
                if got.as_deref() == Some(want) {
                    // The manifest is the one the registry named. Now check
                    // that the images are the ones the manifest describes,
                    // because the digest above would not notice if they were
                    // not. See `verify_bytes`.
                    progress(
                        "the corpus matches the digest its registry entry \
                         declares; checking the images against their own \
                         records before naming the run",
                    );
                    let checked = verify_bytes(corpus, &mut progress)?;
                    progress(&format!(
                        "{checked} image(s) are the files their records \
                         describe"
                    ));
                    true
                } else {
                    // Refused rather than downgraded. The user asserted
                    // something about these bytes that is not true of them,
                    // and filing the run quietly as custom would answer a
                    // different question from the one they asked.
                    return Err(ScoreError::NotThatCorpus {
                        path: corpus.display().to_string(),
                        id: entry.id.clone(),
                        want: want.to_string(),
                        got: got.unwrap_or_else(|| {
                            "nothing, because at least one record states no \
                             digest for its own image"
                                .into()
                        }),
                    });
                }
            }
        },
    };

    // Checked before a single image is scored, because a corpus that leaks a
    // cover across the boundary produces a confident wrong answer, and
    // producing it first and mentioning the problem afterwards is how a bad
    // number gets quoted. It used to run after the scoring pass, which was
    // defensible for one detector and indefensible for seven: a corpus this
    // refuses would have cost every one of them a full run first.
    let checks = check(corpus)?;
    if !checks.split_leaks.is_empty() {
        return Err(ScoreError::SplitLeaks {
            path: corpus.display().to_string(),
            count: checks.split_leaks.count,
            examples: checks.split_leaks.examples(),
        });
    }

    // A confounded corpus is reported rather than refused, which is the
    // opposite of what a split leak gets, and the difference is deliberate.
    // A leak makes a number wrong while looking right. A second variable
    // between the clean and stego halves is a real property of some arms, kept
    // on purpose to demonstrate what it does, so the run happens and the
    // document says what it measured.
    match checks.pairing {
        Pairing::Confounded => progress(&format!(
            "WARNING: {} stego image(s) differ from their cover in more than \
             the payload, for example: {}. This run measures that difference \
             as well as the payload, and the result says so",
            checks.pairing_breaks.count,
            checks.pairing_breaks.examples()
        )),
        Pairing::Unverified => progress(
            "the pairing rule could not be checked: no stego image here names \
             a cover this could read alongside it. The result says unverified \
             rather than claiming the rule held",
        ),
        Pairing::SingleVariable => {
            if checks.unreadable > 0 {
                progress(&format!(
                    "{} of {} stego image(s) could not be compared with their \
                     cover, so the pairing check covered the rest",
                    checks.unreadable,
                    checks.unreadable + checks.compared
                ));
            }
        }
    }

    // A prefix of a tier is not the tier, whatever the whole of it hashes to,
    // and the digest above is taken over the whole corpus rather than over
    // what was scored.
    if claim_holds && limit.is_some() {
        progress(
            "this run scored part of the corpus, so it is marked custom \
             rather than named: a prefix of a tier is not the tier",
        );
    }

    // Read once here rather than once per detector. Walking the corpus for
    // labels is cheap next to hashing it, but it is still a walk of every
    // record on disk, and seven of them over a Core tier is seven times
    // 344,357 file reads for an answer that cannot have changed.
    let mut labels = Vec::new();
    for sample in Samples::open(corpus)? {
        labels.push(sample?.role == Role::Stego);
        if limit.is_some_and(|n| labels.len() as u64 >= n) {
            break;
        }
    }

    // The corpus itself is one-sided, which no detector can fix. Refused here
    // so a seven-detector command says so once, before anything runs, rather
    // than seven times after seven full runs. `score_one` asks the same
    // question again of what the detector actually ANSWERED, which is a
    // different fact and can only be known afterwards.
    let stego = labels.iter().filter(|l| **l).count() as u64;
    let clean = labels.len() as u64 - stego;
    if clean == 0 || stego == 0 {
        return Err(ScoreError::OneSided {
            path: corpus.display().to_string(),
            clean,
            stego,
        });
    }

    Ok(Prepared {
        corpus: corpus.to_path_buf(),
        limit,
        registered: registered.map(|e| (e.name.clone(), e.tier.clone())),
        checks,
        claim_holds,
        labels,
        started_utc,
        preflight: started.elapsed(),
    })
}

/// Ask one detector about a corpus [`prepare`] has already established.
///
/// Callable repeatedly against the same [`Prepared`]. Nothing it does touches
/// the corpus beyond reading the images the detector is handed, so the second
/// detector costs a scoring pass and nothing else.
pub fn score_one<P>(
    entry: &Entry,
    prepared: &Prepared,
    records: &Path,
    timeout: Duration,
    mut progress: P,
) -> Result<(Result1, Tally), ScoreError>
where
    P: FnMut(&str),
{
    refuse_embedder(entry)?;

    // The scoring pass. Streams, and every answer is on disk before the next
    // item begins, so an interrupted run resumes rather than restarts.
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(records)
        .map_err(|e| ScoreError::Records {
            path: records.display().to_string(),
            source: e,
        })?;
    let mut sink = JsonLines::new(file);

    // Streamed, not collected. Building a Vec of 344,357 items here would put
    // the corpus back in memory one layer above the runner that was written
    // specifically not to hold it.
    let mut feed = Feed::open(&prepared.corpus, prepared.limit)?;
    let own_started = Instant::now();
    let tally = runner::score(entry, &mut feed, records, &mut sink, timeout, |t| {
        progress(&format!("{} scored, {} errored", t.scored, t.errored))
    })?;
    // A corpus defect part way through is a failure, not a short run. Checked
    // after the loop because the iterator cannot return one.
    if let Some(e) = feed.fault {
        return Err(ScoreError::Corpus(e));
    }

    // The metrics pass. Labels came from the corpus in `prepare` and scores
    // come from the records, joined by position, because both are produced in
    // the same deterministic order and the runner refuses to continue when
    // they disagree.
    let (scores, labels, errored) = join(records, &prepared.labels)?;
    let n_stego = labels.iter().filter(|l| **l).count() as u64;
    let n_clean = labels.len() as u64 - n_stego;
    if n_clean == 0 || n_stego == 0 {
        return Err(ScoreError::OneSided {
            path: prepared.corpus.display().to_string(),
            clean: n_clean,
            stego: n_stego,
        });
    }

    let checks = &prepared.checks;

    // A confounded corpus is reported rather than refused, which is the
    // opposite of what a split leak gets, and the difference is deliberate.
    // A leak makes a number wrong while looking right. A second variable
    // between the clean and stego halves is a real property of some arms, kept
    // on purpose to demonstrate what it does, so the run happens and the
    // document says what it measured.
    match checks.pairing {
        Pairing::Confounded => progress(&format!(
            "WARNING: {} stego image(s) differ from their cover in more than \
             the payload, for example: {}. This run measures that difference \
             as well as the payload, and the result says so",
            checks.pairing_breaks.count,
            checks.pairing_breaks.examples()
        )),
        Pairing::Unverified => progress(
            "the pairing rule could not be checked: no stego image here names \
             a cover this could read alongside it. The result says unverified \
             rather than claiming the rule held",
        ),
        Pairing::SingleVariable => {
            if checks.unreadable > 0 {
                progress(&format!(
                    "{} of {} stego image(s) could not be compared with their \
                     cover, so the pairing check covered the rest",
                    checks.unreadable,
                    checks.unreadable + checks.compared
                ));
            }
        }
    }

    let named = if prepared.claim_holds && prepared.limit.is_none() {
        Configuration::Named
    } else {
        Configuration::Custom
    };

    // NOTHING HERE INVENTS A NUMBER WHEN THE METRIC CANNOT BE COMPUTED.
    //
    // This line used to read `.unwrap_or(0.5)`, and 0.5 is the one value that
    // must never be a fallback: it is exactly what an AUC says when a detector
    // carries no information. A run that could not be measured was therefore
    // published as a run that came out at chance, and no field in the document
    // told the two apart. That is the class of fault this project exists to
    // correct, so it is refused rather than defaulted.
    //
    // `result-v1` requires the field, so the choice is refuse or lie, and it
    // is not this code's to make the field optional.
    let auc = match stegobench_metrics::roc_auc(&scores, &labels) {
        Some(auc) => auc,
        None => {
            return Err(ScoreError::NoAuc {
                name: entry.name.clone(),
                answered: scores.len(),
                why: why_no_auc(&scores, &labels, ""),
            })
        }
    };

    // A detector that gave ONE answer to everything scores exactly 0.5, and so
    // does a coin flip. The number is correctly computed in both cases and it
    // means completely different things: one detector could not tell these
    // images apart, the other ranked them no better than chance.
    //
    // Measured 2026-09-29 on the first end to end run this project ever did:
    // zsteg answered `false` for all 18 images of the starter corpus, 12 of
    // them stego, and the run reported `AUC 0.5000 ... 0 unanswered` with
    // nothing saying the answers were identical. `n_error` was correctly zero,
    // because the tool did answer; it just said the same thing every time.
    //
    // Said out loud here rather than recorded in the document, because
    // `result-v1` has no field for it and inventing one quietly is how a
    // schema stops meaning what it says. See DEFERRED.md.
    if scores.len() > 1 && scores.windows(2).all(|w| w[0] == w[1]) {
        progress(&format!(
            "every one of the {} answers from {} was identical, so this AUC is 0.5 \
             by construction and not by measurement: it did not separate these images",
            scores.len(),
            entry.name
        ));
    }
    let mut tpr_at_fpr = BTreeMap::new();
    for fpr in [0.01, 0.05, 0.10] {
        match stegobench_metrics::tpr_at_fpr(&scores, &labels, fpr) {
            Some(tpr) => {
                tpr_at_fpr.insert(format!("{fpr:.2}"), tpr);
            }
            // Unreachable given the AUC above succeeded: these three budgets
            // are constants inside [0, 1], and the only other way this answers
            // None is a curve that could not be drawn, which is the same
            // condition the AUC just cleared. It is checked rather than
            // assumed, because a silently missing key in the published map
            // would read as "this detector was measured and reached nothing at
            // that budget", which is a different and much worse claim.
            None => {
                return Err(ScoreError::NoAuc {
                    name: entry.name.clone(),
                    answered: scores.len(),
                    why: why_no_auc(
                        &scores,
                        &labels,
                        &format!(" at a false-alarm budget of {fpr:.2}"),
                    ),
                })
            }
        }
    }

    let (version, pinned_by) = pinning(entry);

    let result = Result1 {
        schema: RESULT_SCHEMA_ID.to_string(),
        subject: Subject {
            name: entry.name.clone(),
            version: version.clone(),
            kind: SubjectKind::Detector,
        },
        corpus: CorpusRef {
            name: match &prepared.registered {
                Some((name, _)) => name.clone(),
                None => corpus_name(&prepared.corpus),
            },
            tier: prepared
                .registered
                .as_ref()
                .and_then(|(_, tier)| tier.clone()),
            // The user pointed at a directory. Nothing here downloaded it, and
            // saying otherwise would be the harness vouching for bytes it
            // never saw arrive. Naming the corpus does not change that: a
            // registry entry describes a dataset, it does not deliver one.
            source: CorpusSource::Supplied,
            digest: checks.digest.clone().unwrap_or_default(),
            // PAIRS, not items. This wrote `labels.len()`, which is clean plus
            // stego, while `generators/emit_results.py` wrote the stego rows,
            // which is pairs. One published field meant two things depending on
            // which half of this project produced the document, and the field
            // carried no doc comment to arbitrate: a shipped result says
            // `pairs: 200` beside `n_stego: 200`, and a run of the starter
            // corpus said `pairs: 18` for 6 covers and 12 stego images.
            //
            // A pair is a cover and its stego twin, so the count is the stego
            // side. See the field's own documentation in `result.rs`.
            pairs: labels.iter().filter(|&&stego| stego).count() as u64,
            split: None,
        },
        arm: checks.arm.clone(),
        metrics: Metrics {
            auc,
            auc_ci95: None,
            tpr_at_fpr,
            verdict_rate: None,
            n_clean,
            n_stego,
            n_error: errored,
        },
        provenance: Provenance {
            seed: None,
            plugins: vec![PluginRef {
                name: entry.name.clone(),
                image: version,
                determinism: Determinism::Nondeterministic,
                // Two fields because an entry can declare an image AND
                // `invoke.host`: the image digest names the subject while what
                // executes is an adapter here that posts to a running
                // instance. The pin and the sandbox disagree for that shape,
                // and a single value could only ever be right about one of
                // them.
                pinned_by,
                isolation: isolation(entry),
            }],
            harness_version: env!("CARGO_PKG_VERSION").to_string(),
            started_utc: prepared.started_utc.clone(),
            // The shared pre-flight plus this detector's own pass. Both are
            // work this number rests on. What it does NOT include is time
            // spent on the other detectors of the same command: waiting for a
            // different measurement is not part of making this one.
            elapsed_seconds: prepared.preflight.as_secs_f64() + own_started.elapsed().as_secs_f64(),
            // Three routes, three answers, and the one-line version used to
            // get two of them wrong.
            //
            // A container is launched with `--network=none` (see
            // `stegobench_plugin::selftest`), so nothing inside it can reach
            // anything. A locally installed binary runs here with this
            // machine's network and nothing constrains it. A host adapter runs
            // here too, and for a service entry the network is not merely
            // reachable, it is the only way the tool is asked anything at all,
            // so reporting it as unreachable was the most misleading of the
            // three.
            network_reachable: !runs_in_container(entry),
            host: Some(host()),
        },
        declarations: Declarations {
            split_discipline: checks.split,
            pairing: checks.pairing,
            configuration: named,
            trained_on: None,
            self_reported: false,
        },
    };
    Ok((result, tally))
}

/// Which of the conditions stopped a metric being computed, in words.
///
/// The sentences are [`crate::metrics::MetricsError`]'s and are not repeated
/// here: this path and `stegobench metrics` refuse for the same reasons, and
/// two sets of words for one condition is the copy that drifts. What this adds
/// is the part that is only true here, where the answers came from a detector
/// this harness ran rather than from a file somebody wrote, so the next step
/// differs even though the condition does not.
///
/// `at` names the operating point, and is empty for the AUC itself.
fn why_no_auc(scores: &[f64], labels: &[bool], at: &str) -> String {
    let refusal = crate::metrics::why_unrankable(scores, labels, at);
    let here = match refusal.reason() {
        // Nothing the user did: `score` builds both lists from the same
        // records in the same order, so they cannot legitimately disagree.
        "length-mismatch" => {
            " Both are built from the same records in the same order here, so \
             this is a bug in stegobench rather than anything you did. Please \
             report it with this message."
        }
        // Every image errored. The sentence underneath is `metrics`' one, and
        // there a user really did hand over an empty file; here nobody gave
        // anything, the detector produced nothing, and a reader who is told
        // "no scores were given" has no reason to look at the tool.
        "empty" => {
            " Every image was put to this tool and none of them came back with \
             a number, which is the tool or its parser rather than the corpus: \
             `stegobench doctor` runs its self-test and says which."
        }
        // The numbers came from the detector, so the next step is the detector.
        "not-a-number" => {
            " This is the detector's output, so the fix is with the tool or \
             its parser: `stegobench doctor` runs its self-test."
        }
        // The corpus may well hold both sides. What is one-sided is the set of
        // answers this tool managed to produce, and saying so points at the
        // tool rather than sending somebody to re-examine their corpus.
        "one-sided" => {
            " Whatever the corpus holds, this tool produced a usable answer \
             for only one of them."
        }
        _ => "",
    };
    format!("{refusal}{here}")
}

/// The items to score, one at a time, stopping at `limit` where one is set.
///
/// An iterator cannot return an error, and a corpus defect must not read as
/// the end of the corpus: that is the difference between "scored everything"
/// and "scored as far as the broken file", and they have the same shape from
/// the outside. So a fault stops the feed and is picked up by the caller.
struct Feed {
    samples: Samples,
    limit: Option<u64>,
    taken: u64,
    fault: Option<stegobench_core::samples::SampleError>,
}

impl Feed {
    fn open(corpus: &Path, limit: Option<u64>) -> Result<Self, ScoreError> {
        Ok(Self {
            samples: Samples::open(corpus)?,
            limit,
            taken: 0,
            fault: None,
        })
    }
}

/// Whether a run of this entry happens inside a container, which is the only
/// route that is sandboxed away from the network.
///
/// An entry naming an image is not enough. `invoke.host` says the thing that
/// runs is a program on this machine, and the image reference is then the
/// subject's identity rather than a description of what executes.
fn runs_in_container(entry: &Entry) -> bool {
    entry.image.is_some() && !entry.invoke.as_ref().is_some_and(|i| i.host)
}

impl Iterator for Feed {
    type Item = WorkItem;

    fn next(&mut self) -> Option<WorkItem> {
        if self.limit.is_some_and(|n| self.taken >= n) {
            return None;
        }
        match self.samples.next()? {
            Ok(sample) => {
                self.taken += 1;
                Some(WorkItem {
                    id: sample.id,
                    path: sample.image,
                })
            }
            Err(e) => {
                self.fault = Some(e);
                None
            }
        }
    }
}

/// How many example violations are kept to show the user.
///
/// The count is exact and the examples are capped, which is the only shape
/// that works: a corpus built by a broken script fails on every one of its
/// 344,357 rows, and a list of them would put the whole corpus in memory to
/// print three lines of it.
const MAX_EXAMPLES: usize = 3;

/// A tally of one kind of corpus defect, with a few examples kept to show.
#[derive(Default)]
struct Violations {
    count: usize,
    examples: Vec<String>,
}

impl Violations {
    fn note(&mut self, what: String) {
        self.count += 1;
        if self.examples.len() < MAX_EXAMPLES {
            self.examples.push(what);
        }
    }

    fn is_empty(&self) -> bool {
        self.count == 0
    }

    fn examples(&self) -> String {
        self.examples.join("; ")
    }
}

/// What the corpus turned out to be, as opposed to what it claims.
struct Checks {
    split: SplitDiscipline,
    split_leaks: Violations,
    pairing: Pairing,
    pairing_breaks: Violations,
    /// Stego images whose cover could be found and compared.
    compared: u64,
    /// Stego images naming a cover that could not be compared, because one of
    /// the two images could not be read or is in a format this does not
    /// measure.
    unreadable: u64,
    /// A name for this corpus's content, or `None` where it cannot have one.
    ///
    /// Computed over every record's own stated digest, in corpus order, so two
    /// people holding the same corpus compute the same value and a result can
    /// say which bytes it was measured on. It identifies the MANIFEST rather
    /// than the bytes: nothing here reopens an image to check the claim, which
    /// is why `corpus.source` sits beside it and says whether the harness
    /// fetched the corpus or was handed it.
    digest: Option<String>,
    /// What was embedded, as the corpus itself states it.
    arm: Arm,
}

/// Names a corpus by what its records declare about their own images.
///
/// Fed in corpus order, which is sorted by id and therefore identical on every
/// machine, so two people holding the same corpus compute the same value and a
/// result can say which bytes it was measured on. Constant memory: it is a
/// hash state and two counters.
///
/// It identifies the MANIFEST rather than the bytes. Nothing here reopens an
/// image to check the claim a record makes about it, which is a real limit and
/// is why the result document carries `corpus.source` beside the digest to say
/// whether the harness fetched the corpus or was handed it.
#[derive(Default)]
pub struct CorpusDigest {
    hash: Sha256,
    seen: u64,
    every_record_declares_one: bool,
}

impl CorpusDigest {
    fn new() -> Self {
        Self {
            hash: Sha256::new(),
            seen: 0,
            every_record_declares_one: true,
        }
    }

    fn note(&mut self, sample: &Sample) {
        self.seen += 1;
        self.hash.update(sample.id.as_bytes());
        self.hash.update(b"\0");
        match &sample.digest {
            Some(d) => self.hash.update(d.as_bytes()),
            None => self.every_record_declares_one = false,
        }
        self.hash.update(b"\n");
    }

    /// The digest, or `None` where the corpus cannot honestly have one.
    ///
    /// A digest over a corpus where some records state none of their own would
    /// name the file list, and the field it goes in is read as naming the
    /// content. Carrying nothing is the smaller claim, which is to say it is
    /// not a false one.
    fn finish(self) -> Option<String> {
        (self.every_record_declares_one && self.seen > 0)
            .then(|| format!("sha256:{:x}", self.hash.finalize()))
    }
}

/// Checks that every image is the file its own record says it is.
///
/// THE HOLE THIS CLOSES
///
/// The corpus digest is taken over what the records STATE about their images,
/// not over the images. That is what lets it survive a corpus being extracted
/// from a shard and moved, and it is the right property for identifying a
/// corpus. It is the wrong property to rest a `named` result on by itself:
/// somebody could take the real manifest records, pair them with easier
/// images, and match the registry's digest exactly while measuring something
/// else entirely.
///
/// So a run that claims a registered corpus pays for one pass over the bytes.
/// It costs a read of the corpus, which the detector was going to do anyway,
/// and it turns `named` from "the manifest matches" into "the images are the
/// ones the manifest describes", which is what a reader assumes the word means.
///
/// Not done for a `custom` run. There is no external claim to check one
/// against, the records are the only description of the corpus that exists,
/// and comparing them with themselves would be theatre.
fn verify_bytes<P>(root: &Path, progress: &mut P) -> Result<u64, ScoreError>
where
    P: FnMut(&str),
{
    let mut checked = 0u64;
    let mut skipped = 0u64;
    let mut last_beat = Instant::now();
    for sample in Samples::open(root)? {
        let sample = sample?;
        let Some(want) = sample.digest.as_deref() else {
            // A corpus reaching here has a digest, which means every record
            // stated one, so this cannot fire. Counted rather than asserted.
            skipped += 1;
            continue;
        };
        let got = hash_file(&sample.image).map_err(|e| ScoreError::Records {
            path: sample.image.display().to_string(),
            source: e,
        })?;
        if got != want {
            return Err(ScoreError::ImageChanged {
                id: sample.id,
                want: want.to_string(),
                got,
            });
        }
        checked += 1;
        // Baseline Section 2.1: a long loop emits a heartbeat. Hashing a Core
        // tier is tens of gigabytes and a silent minute reads as a hang.
        if last_beat.elapsed() >= HEARTBEAT {
            progress(&format!("{checked} image(s) checked against their records"));
            last_beat = Instant::now();
        }
    }
    debug_assert_eq!(skipped, 0, "a digested corpus had a record with no digest");
    Ok(checked)
}

/// How often a long pass says it is still alive.
const HEARTBEAT: Duration = Duration::from_secs(30);

/// The sha256 of a file as lower case hex, streamed rather than read whole.
///
/// A corpus image is a few hundred kilobytes and reading one whole would be
/// fine; reading one that turns out to be a gigabyte would not, and this is
/// pointed at files somebody else produced.
fn hash_file(path: &Path) -> std::io::Result<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// The digest of a corpus on disk, for a caller that wants only that.
///
/// `score` folds this into a walk it was doing anyway. `verify` has no such
/// walk, so it pays for its own.
pub fn corpus_digest(root: &Path) -> Result<Option<String>, ScoreError> {
    let mut digest = CorpusDigest::new();
    for sample in Samples::open(root)? {
        digest.note(&sample?);
    }
    Ok(digest.finish())
}

/// One value seen across a corpus, or the fact that there was not one.
///
/// Three answers, because three things are true of a real corpus: it states one
/// arm, or it states several, or it states none. Collapsing the last two into
/// "unknown" is how `spatial` came to be written on JPEG runs.
///
/// Constant memory whatever the corpus holds: once a second distinct value
/// arrives, nothing more needs keeping.
#[derive(Debug, Clone, PartialEq, Default)]
enum Uniform<T> {
    #[default]
    Nothing,
    One(T),
    Many,
}

impl<T: PartialEq> Uniform<T> {
    fn note(&mut self, value: Option<T>) {
        let Some(value) = value else { return };
        match self {
            Uniform::Nothing => *self = Uniform::One(value),
            Uniform::One(seen) if *seen == value => {}
            Uniform::One(_) => *self = Uniform::Many,
            Uniform::Many => {}
        }
    }

    fn one(&self) -> Option<&T> {
        match self {
            Uniform::One(v) => Some(v),
            _ => None,
        }
    }
}

impl Uniform<String> {
    /// The single value, or a word saying which of the other two cases it is.
    /// Both go in a string field a person reads, so both have to be words.
    fn or_say_why(&self) -> String {
        match self {
            Uniform::One(v) => v.clone(),
            Uniform::Many => "mixed".into(),
            Uniform::Nothing => "unstated".into(),
        }
    }
}

/// What the corpus says about the arm it holds, gathered while walking it.
#[derive(Default)]
struct ArmFacts {
    tool: Uniform<String>,
    rate: Uniform<(String, f64)>,
    domain: Uniform<String>,
    format: Uniform<String>,
}

/// What a cover contributes to both checks, gathered in one pass.
#[derive(Clone)]
struct Cover {
    split: Option<String>,
    shape: Option<Shape>,
}

/// Checks the two reliability claims the result document carries.
///
/// SPLIT DISCIPLINE, WHICH FILES ON DISK CAN PROVE
///
/// A split label travels with a cover, and a stego image names the cover it was
/// made from, so a stego image carrying a DIFFERENT split from its own cover is
/// a corpus that will leak a photograph across the train and test boundary.
/// Violating it inflates every number computed from the corpus and is invisible
/// in the output, which is exactly why the harness must not take it on trust.
///
/// PAIRING, WHERE ONLY THE NEGATIVE IS PROVABLE
///
/// The rule is that a clean image and its stego twin differ in nothing but the
/// payload. Nothing short of decoding both images could prove that, and even
/// that would miss a cover re-encoded before the payload went in. The reachable
/// half is the refutation: if the two differ in format, width, height, bit depth
/// or channel count, then something other than the payload changed, and the
/// number about to be computed is measuring that too.
///
/// So this returns `SingleVariable` where every comparable pair matched,
/// `Confounded` where any did not, and `Unverified` where nothing could be
/// compared. The third answer is the point. A benchmark that says "single
/// variable" having looked at nothing is making the unchecked claim this
/// project exists to stop repeating.
///
/// WHAT THIS HOLDS IN MEMORY, AND WHY THAT IS BOUNDED
///
/// One entry per COVER, not one per sample, and the stego rows are streamed
/// against it rather than gathered. A Core tier is 10,000 covers against
/// 344,357 samples, so this is an order of magnitude below the corpus and
/// bounded by a number the corpus states rather than by anything this code
/// chooses. It costs a second walk of the directory tree, because a cover can
/// sort after the arm that used it and the map has to be complete before any
/// row is judged against it.
fn check(corpus: &Path) -> Result<Checks, ScoreError> {
    let mut covers: HashMap<String, Cover> = HashMap::new();
    let mut any_split = false;
    let mut digest = CorpusDigest::new();
    let mut facts = ArmFacts::default();

    for sample in Samples::open(corpus)? {
        let sample = sample?;

        // Folded into a walk that was happening anyway.
        digest.note(&sample);

        if sample.role == Role::Stego {
            // Gathered from the stego side only. The arm is what was embedded,
            // and a clean row states `clean` as its tool, which would turn
            // every corpus into a mixed one.
            let arm = &sample.arm;
            facts
                .tool
                .note(arm.tool.clone().or_else(|| arm.name.clone()));
            facts.domain.note(arm.domain.clone());
            facts.format.note(image_format(&sample.image));
            facts
                .rate
                .note(arm.rate.zip(arm.rate_unit.clone()).map(|(r, u)| (u, r)));
        }

        if sample.role != Role::Clean {
            continue;
        }
        any_split |= sample.split.is_some();

        // Filed under BOTH names a stego row might use to reach it, and the
        // second one is the one that matters. The packer renames every member
        // to its position in the tier, so a cover whose record says
        // `file = "09710.png"` is extracted as `000123.png`, while every stego
        // row made from it still says `09710.png`. Joining on the name on disk
        // alone matches nothing on a packed release, and a check that cannot
        // look reports clean.
        let on_disk = sample
            .image
            .file_name()
            .and_then(|n| n.to_str())
            .map(str::to_string);
        let declared = sample.declared_name.clone();
        // Read once, whichever names it goes in under. An unreadable cover is
        // not a corpus defect this should refuse over: the sample reader has
        // already accepted the file, the detector will be asked about it
        // regardless, and the honest consequence is that this pair cannot be
        // compared rather than that the run cannot happen.
        let shape = header::read(&sample.image).ok();
        let cover = Cover {
            split: sample.split.clone(),
            shape,
        };

        // The record's own name wins, and the name on disk only fills a gap it
        // left. The two are not equally trustworthy: a stego row joins on
        // `source_png`, which names a cover the way its RECORD does, and on a
        // real release the names on disk collide. Every arm restarts its
        // numbering at 000000, so four clean arms extracted beside a cover
        // tier all offer `000000.png`, and letting those overwrite an
        // authoritative entry would silently attach a stego row to the wrong
        // photograph.
        if let Some(name) = declared {
            covers.insert(name, cover.clone());
        }
        if let Some(name) = on_disk {
            covers.entry(name).or_insert(cover);
        }
    }

    let mut checks = Checks {
        split: SplitDiscipline::NotApplicable,
        split_leaks: Violations::default(),
        pairing: Pairing::Unverified,
        pairing_breaks: Violations::default(),
        compared: 0,
        unreadable: 0,
        digest: digest.finish(),
        arm: arm_of(&facts),
    };

    for sample in Samples::open(corpus)? {
        let sample = sample?;
        if sample.role != Role::Stego {
            continue;
        }
        let Some(cover_name) = sample.cover else {
            continue;
        };
        let Some(cover) = covers.get(&cover_name) else {
            continue;
        };

        // A stego row with no split of its own inherits its cover's, which is
        // the corpus doing the right thing by construction and cannot be a
        // violation. Only a row that states one can contradict.
        if let (Some(split), Some(cover_split)) = (&sample.split, &cover.split) {
            any_split = true;
            if split != cover_split {
                checks.split_leaks.note(format!(
                    "{} is in the {split} split while its cover {cover_name} is \
                     in the {cover_split} split",
                    sample.id
                ));
            }
        }

        match (cover.shape, header::read(&sample.image).ok()) {
            (Some(cover_shape), Some(stego_shape)) => match difference(cover_shape, stego_shape) {
                None => checks.compared += 1,
                Some(what) => {
                    checks.compared += 1;
                    checks
                        .pairing_breaks
                        .note(format!("{} and its cover {cover_name} {what}", sample.id));
                }
            },
            _ => checks.unreadable += 1,
        }
    }

    if any_split {
        checks.split = SplitDiscipline::ByCover;
    }
    checks.pairing = if !checks.pairing_breaks.is_empty() {
        Pairing::Confounded
    } else if checks.compared > 0 {
        Pairing::SingleVariable
    } else {
        Pairing::Unverified
    };
    Ok(checks)
}

/// The arm block, built from what the corpus states rather than from a guess.
///
/// Every field here used to be a constant, and two of the three constants were
/// wrong for most of the corpus: `spatial` on a JPEG arm and `png` on a corpus
/// of JPEGs are not placeholders a reader can see through, they are facts the
/// document appears to assert.
fn arm_of(facts: &ArmFacts) -> Arm {
    let rate = facts.rate.one().and_then(|(unit, value)| {
        let unit = match unit.as_str() {
            "bits per pixel" => RateUnit::Bpp,
            // Both JPEG tools are driven as a share of whatever capacity they
            // report for that cover, which is the distinction the unit exists
            // to carry.
            "bits per non-zero AC coefficient" => return None,
            u if u.contains("capacity") => RateUnit::CapacityFraction,
            _ => return None,
        };
        Some(Rate {
            value: *value,
            unit,
        })
    });

    let domain = match &facts.domain {
        Uniform::One(d) => match d.as_str() {
            "spatial" => Domain::Spatial,
            "jpeg-dct" | "jpeg" => Domain::Jpeg,
            "container" | "structural" => Domain::Structural,
            _ => Domain::Unstated,
        },
        Uniform::Many => Domain::Mixed,
        // The corpus said nothing, so the format is the only evidence left.
        // A corpus of JPEGs is not proof that the payload went into the
        // coefficients, which is why this is the fallback and not the rule.
        Uniform::Nothing => match facts.format.one().map(String::as_str) {
            Some("jpeg") => Domain::Jpeg,
            Some(_) => Domain::Spatial,
            None => Domain::Unstated,
        },
    };

    Arm {
        embedder: facts.tool.or_say_why(),
        rate,
        domain,
        format: facts.format.or_say_why(),
    }
}

/// What a file calls itself, normalised, for the arm's `format` field.
///
/// The extension rather than the header, deliberately. This field records what
/// the corpus claims to be, and the pairing check reads the headers separately,
/// so a corpus whose names and bytes disagree shows up as a disagreement rather
/// than being quietly resolved in one direction.
fn image_format(path: &Path) -> Option<String> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "jpg" | "jpeg" => "jpeg".into(),
        "tif" | "tiff" => "tiff".into(),
        _ => ext,
    })
}

/// The first way two images differ other than in their payload, in words.
///
/// Ordered so the reader is told the most fundamental difference rather than
/// the first one an arbitrary field order happens to reach: a PNG against a
/// JPEG is a different fact from a PNG one pixel wider than another.
fn difference(cover: Shape, stego: Shape) -> Option<String> {
    if cover.format != stego.format {
        return Some(format!(
            "are different formats: {} against {}",
            cover.format.name(),
            stego.format.name()
        ));
    }
    let (Some(c), Some(s)) = (cover.geometry, stego.geometry) else {
        return None;
    };
    if (c.width, c.height) != (s.width, s.height) {
        return Some(format!(
            "are different sizes: {} by {} against {} by {}",
            c.width, c.height, s.width, s.height
        ));
    }
    if c.depth != s.depth {
        return Some(format!(
            "have different bit depths: {} against {}",
            c.depth, s.depth
        ));
    }
    if c.channels != s.channels {
        return Some(format!(
            "have different channel counts: {} against {}",
            c.channels, s.channels
        ));
    }
    None
}

/// Scores and labels, joined by position.
///
/// The labels are handed in rather than re-read, because they are a property
/// of the corpus and every detector of one command shares them.
fn join(records: &Path, labels: &[bool]) -> Result<(Vec<f64>, Vec<bool>, u64), ScoreError> {
    let file = std::fs::File::open(records).map_err(|e| ScoreError::Records {
        path: records.display().to_string(),
        source: e,
    })?;
    let mut scores = Vec::new();
    let mut errored = 0;
    let mut kept = Vec::new();
    for (line, label) in BufReader::new(file).lines().zip(labels.iter()) {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(record) = serde_json::from_str::<Record>(&line) else {
            break;
        };
        match record
            .score
            .or(record.verdict.map(|v| if v { 1.0 } else { 0.0 }))
        {
            Some(s) => {
                scores.push(s);
                kept.push(*label);
            }
            // An item the tool could not answer about is counted, never
            // guessed at. A metric computed over the survivors of a partly
            // failed run is how an evaluation misleads without anyone
            // intending it, which is why n_error is required.
            None => errored += 1,
        }
    }
    Ok((scores, kept, errored))
}

/// What machine this ran on, as far as it can be established portably.
///
/// The operating system and the architecture always, because both are compile
/// time constants and both change what a number means: a detector's timing
/// certainly, and occasionally its answers, where a library dispatches on the
/// instruction set. The core count where the standard library will say. Memory
/// has no portable answer at all, and a zero there would read as a measurement
/// rather than as a gap, so it is left out.
fn host() -> Host {
    Host {
        cores: std::thread::available_parallelism()
            .ok()
            .map(|n| n.get() as u32),
        memory_gb: None,
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
    }
}

/// How the subject identifies itself: an image digest, or the binary's hash.
fn pinning(entry: &Entry) -> (String, PinnedBy) {
    if let Some(image) = &entry.image {
        // AN IMAGE IS ONLY A PIN WHERE THIS HARNESS ACTUALLY RAN IT. A
        // container is started by digest, so the runtime enforces it. An entry
        // that names an image and sets `invoke.host` ran an adapter here
        // instead, against an instance somebody else started, and nothing in
        // this run checked that the instance was built from those bytes.
        // Calling that `image-digest` would tell a reader they are holding a
        // comparable-by-construction number when they are holding a claim.
        let pinned = if runs_in_container(entry) {
            PinnedBy::ImageDigest
        } else {
            PinnedBy::Unpinned
        };
        return (image.reference.clone(), pinned);
    }
    match entry
        .binary
        .as_ref()
        .and_then(|b| b.command.first())
        .and_then(|program| stegobench_plugin::which(program))
        .and_then(|path| stegobench_plugin::hash_file(&path).ok())
    {
        Some(hash) => (hash, PinnedBy::ExecutableHash),
        // The program ran and its bytes could not be hashed afterwards. Said
        // as "unpinned" rather than written as a hash-shaped "unknown" that a
        // reader could mistake for one.
        None => ("unknown".into(), PinnedBy::Unpinned),
    }
}

/// What the tool could reach, which is a different question from what pins it.
///
/// A container is started with `--network=none` and sees nothing. A service
/// entry is reached over HTTP, so the network is not merely available, it is
/// the only channel the tool is asked anything through. Anything else is a
/// program on this machine holding this machine's network.
fn isolation(entry: &Entry) -> Isolation {
    if runs_in_container(entry) {
        return Isolation::SandboxNoNetwork;
    }
    if entry
        .invoke
        .as_ref()
        .is_some_and(|i| i.host && i.endpoint_env.is_some())
    {
        return Isolation::RemoteService;
    }
    Isolation::Host
}

/// The corpus directory's own name, which is the only name it has.
fn corpus_name(path: &Path) -> String {
    path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("corpus")
        .to_string()
}

/// A UTC timestamp in ISO 8601, captured once per run and reused.
///
/// Written out by hand rather than by pulling in a date library. A result
/// document is read by people and by other tools, so a bare count of seconds
/// would be a number nobody can check against a log, and a dependency whose
/// only job is printing one string is a dependency to audit for ever.
///
/// The civil-date arithmetic is Howard Hinnant's days-from-civil, inverted.
/// It is exact for every date this will ever see and does not need a table.
fn now_utc() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    iso8601(secs)
}

fn iso8601(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);

    // Shift the epoch to 0000-03-01 so a leap day lands at the end of the
    // cycle and the month arithmetic has no special cases.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { y + 1 } else { y };
    format!("{year:04}-{month:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_timestamp_is_iso_8601_and_correct() {
        // A result document is read by people and by tools. This is written
        // by hand rather than by a dependency, so it is checked against dates
        // somebody can verify independently.
        assert_eq!(iso8601(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso8601(1_000_000_000), "2001-09-09T01:46:40Z");
        // A leap day, which is where hand-rolled date arithmetic goes wrong.
        assert_eq!(iso8601(1_709_164_800), "2024-02-29T00:00:00Z");
        // The turn of a century that is not a leap year.
        assert_eq!(iso8601(4_102_444_800), "2100-01-01T00:00:00Z");
    }

    /// A PNG that is a real header and nothing after it.
    ///
    /// The pairing check reads headers, so a fixture of eight magic bytes would
    /// exercise only the branch where nothing can be compared. `padding` puts
    /// bytes after the header without changing what the header says, which is
    /// what a payload does to a file's size.
    fn png(width: u32, height: u32, depth: u8, colour: u8, padding: usize) -> Vec<u8> {
        let mut v = b"\x89PNG\r\n\x1a\n".to_vec();
        v.extend_from_slice(&13u32.to_be_bytes());
        v.extend_from_slice(b"IHDR");
        v.extend_from_slice(&width.to_be_bytes());
        v.extend_from_slice(&height.to_be_bytes());
        v.extend_from_slice(&[depth, colour, 0, 0, 0]);
        v.extend_from_slice(&[0, 0, 0, 0]);
        v.extend_from_slice(&vec![0u8; padding]);
        v
    }

    fn corpus(root: &Path, clean: usize, stego: usize) {
        std::fs::create_dir_all(root).expect("corpus");
        for i in 0..clean {
            std::fs::write(root.join(format!("c{i:03}.png")), png(32, 32, 8, 2, 0)).unwrap();
            std::fs::write(
                root.join(format!("c{i:03}.json")),
                r#"{"role":"clean","sha256":"0"}"#,
            )
            .unwrap();
        }
        for i in 0..stego {
            // Same header, more bytes. A payload changes the file, not its
            // shape, which is the whole premise of the pairing check.
            std::fs::write(root.join(format!("s{i:03}.png")), png(32, 32, 8, 2, 64)).unwrap();
            std::fs::write(
                root.join(format!("s{i:03}.json")),
                r#"{"role":"stego","source_png":"c000.png","sha256":"0"}"#,
            )
            .unwrap();
        }
    }

    fn request(dir: &Path, limit: Option<u64>) -> Request<'_> {
        Request {
            corpus: dir,
            registered: None,
            records: dir.with_extension("records.jsonl"),
            timeout: Duration::from_secs(5),
            limit,
        }
    }

    #[test]
    fn the_feed_hands_out_one_item_at_a_time_in_corpus_order() {
        let tmp = tempfile::tempdir().expect("tmp");
        let root = tmp.path().join("corpus");
        corpus(&root, 2, 2);
        let feed = Feed::open(&root, None).expect("opens");
        let ids: Vec<String> = feed.map(|w| w.id).collect();
        assert_eq!(ids, ["c000", "c001", "s000", "s001"]);
    }

    #[test]
    fn a_limit_stops_the_feed_early() {
        // The smoke-test path. It is also why such a run is marked custom: a
        // prefix of a corpus is not the corpus.
        let tmp = tempfile::tempdir().expect("tmp");
        let root = tmp.path().join("corpus");
        corpus(&root, 4, 4);
        let feed = Feed::open(&root, Some(3)).expect("opens");
        assert_eq!(feed.count(), 3);
    }

    #[test]
    fn a_corpus_defect_stops_the_feed_and_is_kept_rather_than_read_as_the_end() {
        // The difference between "scored everything" and "scored as far as
        // the broken file" is invisible from outside an iterator, and they
        // are very different claims about a number.
        let tmp = tempfile::tempdir().expect("tmp");
        let root = tmp.path().join("corpus");
        corpus(&root, 2, 2);
        // An image with no record beside it.
        std::fs::write(root.join("orphan.png"), b"\x89PNG\r\n\x1a\n").unwrap();
        let mut feed = Feed::open(&root, None).expect("opens");
        let seen = feed.by_ref().count();
        assert!(
            feed.fault.is_some(),
            "a defect was read as the end, after {seen}"
        );
    }

    #[cfg(unix)]
    /// A detector that scores by file size, so a test can measure without
    /// installing anything.
    fn sizing_detector(dir: &Path) -> Entry {
        use std::os::unix::fs::PermissionsExt;
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
        toml::from_str(&format!(
            "name = \"sizer\"\nkind = \"detector\"\nlicence = \"MIT\"\n\
             [binary]\ncommand = [{:?}]\nversion_args = [\"--version\"]\n\
             [emits]\noutput = \"score\"\nhigher_means_stego = true\n\
             [invoke]\nargv = [\"{{file}}\"]\nparser = \"number\"\n\
             [selftest]\nmust_detect = \"fixtures/a.png\"\nmust_clear = \"fixtures/b.png\"\n",
            script.display().to_string()
        ))
        .expect("parses")
    }

    #[cfg(unix)]
    #[test]
    fn two_runs_over_the_same_corpus_agree_on_every_number() {
        // Baseline Section 2.1 asks for byte-identical output from two runs,
        // and a result document cannot give that: it records when the run
        // started and how long it took, and those genuinely differ. So what
        // is asserted is the part a reader acts on. If the metrics moved
        // between two runs over unchanged bytes, the measurement would not be
        // a measurement.
        let tmp = tempfile::tempdir().expect("tmp");
        let root = tmp.path().join("corpus");
        corpus(&root, 3, 3);
        let entry = sizing_detector(tmp.path());

        let first = score(&entry, &request(&root, None), |_| {})
            .expect("first run")
            .0;
        // A second records file, so the second run genuinely re-scores rather
        // than resuming and agreeing with itself by construction.
        let mut second_request = request(&root, None);
        second_request.records = root.with_extension("second.jsonl");
        let second = score(&entry, &second_request, |_| {})
            .expect("second run")
            .0;

        assert_eq!(first.metrics, second.metrics);
        assert_eq!(first.corpus, second.corpus);
        assert_eq!(first.declarations, second.declarations);
        assert_eq!(first.subject, second.subject);
    }

    /// A detector that gives the same answer to every image.
    fn constant_detector(dir: &Path) -> Entry {
        use std::os::unix::fs::PermissionsExt;
        let script = dir.join("constant.sh");
        std::fs::write(
            &script,
            "#!/bin/sh\n[ \"$1\" = \"--version\" ] && { echo v1; exit 0; }\necho 0.5\n",
        )
        .unwrap();
        let mut perms = std::fs::metadata(&script).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script, perms).unwrap();
        toml::from_str(&format!(
            "name = \"constant\"\nkind = \"detector\"\nlicence = \"MIT\"\n\
             [binary]\ncommand = [{:?}]\nversion_args = [\"--version\"]\n\
             [emits]\noutput = \"score\"\nhigher_means_stego = true\n\
             [invoke]\nargv = [\"{{file}}\"]\nparser = \"number\"\n\
             [selftest]\nmust_detect = \"fixtures/a.png\"\nmust_clear = \"fixtures/b.png\"\n",
            script.display().to_string()
        ))
        .expect("parses")
    }

    #[cfg(unix)]
    #[test]
    fn one_answer_to_everything_is_named_rather_than_published_as_chance() {
        // The case this exists for, measured on the first end to end run this
        // project did: zsteg answered `false` for all 18 images of the starter
        // corpus, 12 of them stego, and the run said `AUC 0.5000 ... 0
        // unanswered`. Every number was right. Nothing said the detector had
        // not separated anything, and AUC 0.5 from one repeated answer reads
        // identically to AUC 0.5 from a detector ranking at chance.
        let tmp = tempfile::tempdir().expect("tmp");
        let root = tmp.path().join("corpus");
        corpus(&root, 3, 3);
        let entry = constant_detector(tmp.path());

        let mut said = Vec::new();
        let result = score(&entry, &request(&root, None), |m: &str| {
            said.push(m.to_string())
        })
        .expect("a constant detector still produces a result")
        .0;

        // The figure itself is honest and stays: 0.5 is what a set of ties is.
        assert!((result.metrics.auc - 0.5).abs() < 1e-12, "{:?}", result.metrics);
        assert_eq!(result.metrics.n_error, 0, "it answered every image");

        let warned = said.iter().any(|m| m.contains("identical"));
        assert!(warned, "nothing said the answers were all the same: {said:?}");
        assert!(
            said.iter().any(|m| m.contains("by construction")),
            "the warning does not say why the 0.5 is not a measurement: {said:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_detector_that_separates_the_images_is_not_warned_about() {
        // The other half, so the warning cannot be made to pass by always
        // firing. The sizing detector gives stego and clean different scores.
        let tmp = tempfile::tempdir().expect("tmp");
        let root = tmp.path().join("corpus");
        corpus(&root, 3, 3);
        let entry = sizing_detector(tmp.path());

        let mut said = Vec::new();
        score(&entry, &request(&root, None), |m: &str| {
            said.push(m.to_string())
        })
        .expect("run");
        assert!(
            !said.iter().any(|m| m.contains("identical")),
            "warned about a detector that did separate the images: {said:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_resumed_run_reports_the_same_numbers_as_an_uninterrupted_one() {
        // The failure this prevents is the worst kind: a resumed run that
        // quietly measures less than it claims, because the resumed items
        // were counted as done but never joined to their labels.
        let tmp = tempfile::tempdir().expect("tmp");
        let root = tmp.path().join("corpus");
        corpus(&root, 3, 3);
        let entry = sizing_detector(tmp.path());

        let whole = score(&entry, &request(&root, None), |_| {})
            .expect("whole")
            .0;

        // Score part, then finish, against one records file. The limit has to
        // reach past the clean images: enumeration is sorted, so a limit of
        // three here would score `c000` to `c002` and nothing else, and a
        // one-sided run is correctly refused before it can be resumed.
        let mut partial = request(&root, Some(4));
        partial.records = root.with_extension("resumed.jsonl");
        score(&entry, &partial, |_| {}).expect("partial");
        let mut rest = request(&root, None);
        rest.records = partial.records.clone();
        let (resumed, tally) = score(&entry, &rest, |_| {}).expect("resumed");

        assert_eq!(tally.resumed, 4, "the scored part should have been reused");
        assert_eq!(whole.metrics, resumed.metrics);
    }

    /// A corpus carrying split labels, with one knob: whether the stego rows
    /// agree with their covers.
    fn split_corpus(root: &Path, leak: bool) {
        std::fs::create_dir_all(root).expect("corpus");
        for i in 0..3 {
            let split = if i == 0 { "test" } else { "train" };
            std::fs::write(root.join(format!("c{i}.png")), png(32, 32, 8, 2, 0)).unwrap();
            std::fs::write(
                root.join(format!("c{i}.json")),
                format!(r#"{{"role":"clean","split":"{split}","sha256":"0"}}"#),
            )
            .unwrap();

            // The leak puts the stego twin of cover 0 on the other side.
            let stego_split = if leak && i == 0 { "train" } else { split };
            std::fs::write(root.join(format!("s{i}.png")), png(32, 32, 8, 2, 64)).unwrap();
            std::fs::write(
                root.join(format!("s{i}.json")),
                format!(
                    r#"{{"role":"stego","source_png":"c{i}.png","split":"{stego_split}","sha256":"0"}}"#
                ),
            )
            .unwrap();
        }
    }

    #[test]
    fn a_corpus_that_keeps_twins_together_is_reported_as_by_cover() {
        let tmp = tempfile::tempdir().expect("tmp");
        let root = tmp.path().join("corpus");
        split_corpus(&root, false);
        let checks = check(&root).expect("checked");
        assert!(
            checks.split_leaks.is_empty(),
            "{}",
            checks.split_leaks.examples()
        );
        assert_eq!(checks.split, SplitDiscipline::ByCover);
    }

    #[cfg(unix)]
    #[test]
    fn a_cover_split_from_its_twin_stops_the_run() {
        // The whole reason the check exists. A photograph on both sides of the
        // boundary inflates every number computed from the corpus, and the
        // inflation is invisible in the output, so a confident wrong answer is
        // the alternative to refusing.
        let tmp = tempfile::tempdir().expect("tmp");
        let root = tmp.path().join("corpus");
        split_corpus(&root, true);

        let checks = check(&root).expect("checked");
        assert_eq!(checks.split_leaks.count, 1);
        assert!(
            checks.split_leaks.examples().contains("c0.png"),
            "{}",
            checks.split_leaks.examples()
        );

        let entry = sizing_detector(tmp.path());
        let err = score(&entry, &request(&root, None), |_| {}).expect_err("refused");
        assert!(err.to_string().contains("different side"), "{err}");
    }

    #[test]
    fn a_corpus_with_no_split_labels_says_not_applicable_rather_than_by_cover() {
        // "No split applies" and "the split was checked and held" are
        // different facts, and a directory of loose samples is an ordinary
        // thing to score.
        let tmp = tempfile::tempdir().expect("tmp");
        let root = tmp.path().join("corpus");
        corpus(&root, 2, 2);
        let checks = check(&root).expect("checked");
        assert!(checks.split_leaks.is_empty());
        assert_eq!(checks.split, SplitDiscipline::NotApplicable);
    }

    #[test]
    fn a_stego_row_inheriting_its_cover_split_is_not_a_violation() {
        // Arm rows in the real corpus carry no split of their own, precisely
        // because the split is a property of the cover. Reading that absence
        // as a disagreement would refuse every correctly built corpus.
        let tmp = tempfile::tempdir().expect("tmp");
        let root = tmp.path().join("corpus");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("c0.png"), png(32, 32, 8, 2, 0)).unwrap();
        std::fs::write(
            root.join("c0.json"),
            r#"{"role":"clean","split":"test","sha256":"0"}"#,
        )
        .unwrap();
        std::fs::write(root.join("s0.png"), png(32, 32, 8, 2, 64)).unwrap();
        std::fs::write(
            root.join("s0.json"),
            r#"{"role":"stego","source_png":"c0.png","sha256":"0"}"#,
        )
        .unwrap();
        let checks = check(&root).expect("checked");
        assert!(
            checks.split_leaks.is_empty(),
            "{}",
            checks.split_leaks.examples()
        );
        assert_eq!(checks.split, SplitDiscipline::ByCover);
    }

    /// A corpus of one pair, where the test chooses what the stego image is.
    fn pair(root: &Path, stego_image: &[u8]) {
        std::fs::create_dir_all(root).expect("corpus");
        std::fs::write(root.join("c0.png"), png(64, 64, 8, 2, 0)).unwrap();
        std::fs::write(root.join("c0.json"), r#"{"role":"clean","sha256":"0"}"#).unwrap();
        std::fs::write(root.join("s0.png"), stego_image).unwrap();
        std::fs::write(
            root.join("s0.json"),
            r#"{"role":"stego","source_png":"c0.png","sha256":"0"}"#,
        )
        .unwrap();
    }

    #[test]
    fn a_stego_image_shaped_like_its_cover_is_reported_as_single_variable() {
        let tmp = tempfile::tempdir().expect("tmp");
        let root = tmp.path().join("corpus");
        pair(&root, &png(64, 64, 8, 2, 128));
        let checks = check(&root).expect("checked");
        assert!(checks.pairing_breaks.is_empty());
        assert_eq!(checks.compared, 1);
        assert_eq!(checks.pairing, Pairing::SingleVariable);
    }

    #[test]
    fn a_stego_image_a_different_size_from_its_cover_is_confounded() {
        // The failure the check exists for. A resized stego half means the
        // detector is being asked to tell two SIZES apart, and it will do it
        // well, and the number will look like detection.
        let tmp = tempfile::tempdir().expect("tmp");
        let root = tmp.path().join("corpus");
        pair(&root, &png(63, 64, 8, 2, 0));
        let checks = check(&root).expect("checked");
        assert_eq!(checks.pairing, Pairing::Confounded);
        assert_eq!(checks.pairing_breaks.count, 1);
        assert!(
            checks.pairing_breaks.examples().contains("different sizes"),
            "{}",
            checks.pairing_breaks.examples()
        );
    }

    #[test]
    fn a_stego_image_in_another_format_is_confounded_and_says_which_two() {
        // The real case that voided a measurement round: one half written as
        // JPEG against a clean half that was not.
        let tmp = tempfile::tempdir().expect("tmp");
        let root = tmp.path().join("corpus");
        let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x11, 0x08];
        jpeg.extend_from_slice(&64u16.to_be_bytes());
        jpeg.extend_from_slice(&64u16.to_be_bytes());
        jpeg.push(3);
        jpeg.extend_from_slice(&[1, 0x11, 0, 2, 0x11, 0, 3, 0x11, 0]);
        pair(&root, &jpeg);
        let checks = check(&root).expect("checked");
        assert_eq!(checks.pairing, Pairing::Confounded);
        let seen = checks.pairing_breaks.examples();
        assert!(seen.contains("PNG") && seen.contains("JPEG"), "{seen}");
    }

    #[test]
    fn a_stego_image_of_another_bit_depth_or_channel_count_is_confounded() {
        for stego in [png(64, 64, 16, 2, 0), png(64, 64, 8, 6, 0)] {
            let tmp = tempfile::tempdir().expect("tmp");
            let root = tmp.path().join("corpus");
            pair(&root, &stego);
            let checks = check(&root).expect("checked");
            assert_eq!(checks.pairing, Pairing::Confounded);
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_confounded_corpus_is_scored_and_the_document_says_so() {
        // Deliberately the opposite of what a split leak gets. A second
        // variable is a real property of some arms, kept on purpose to show
        // what it does, so the run happens and the result carries the fact.
        let tmp = tempfile::tempdir().expect("tmp");
        let root = tmp.path().join("corpus");
        std::fs::create_dir_all(&root).unwrap();
        for i in 0..2 {
            std::fs::write(root.join(format!("c{i}.png")), png(64, 64, 8, 2, i)).unwrap();
            std::fs::write(
                root.join(format!("c{i}.json")),
                r#"{"role":"clean","sha256":"0"}"#,
            )
            .unwrap();
            std::fs::write(root.join(format!("s{i}.png")), png(48, 48, 8, 2, 64 + i)).unwrap();
            std::fs::write(
                root.join(format!("s{i}.json")),
                format!(r#"{{"role":"stego","source_png":"c{i}.png","sha256":"0"}}"#),
            )
            .unwrap();
        }

        let entry = sizing_detector(tmp.path());
        let mut warnings = Vec::new();
        let (result, _) = score(&entry, &request(&root, None), |line| {
            warnings.push(line.to_string())
        })
        .expect("scored");
        assert_eq!(result.declarations.pairing, Pairing::Confounded);
        assert!(
            warnings.iter().any(|w| w.starts_with("WARNING")),
            "the run said nothing about it: {warnings:?}"
        );
    }

    #[test]
    fn a_corpus_nothing_can_be_compared_in_says_unverified() {
        // Three ways to end up here, and none of them may read as the rule
        // holding: unreadable images, stego rows that name no cover, and a
        // cover this cannot measure.
        let tmp = tempfile::tempdir().expect("tmp");

        let unreadable = tmp.path().join("unreadable");
        std::fs::create_dir_all(&unreadable).unwrap();
        std::fs::write(unreadable.join("c0.png"), b"\x89PNG\r\n\x1a\n").unwrap();
        std::fs::write(
            unreadable.join("c0.json"),
            r#"{"role":"clean","sha256":"0"}"#,
        )
        .unwrap();
        std::fs::write(unreadable.join("s0.png"), b"\x89PNG\r\n\x1a\n").unwrap();
        std::fs::write(
            unreadable.join("s0.json"),
            r#"{"role":"stego","source_png":"c0.png","sha256":"0"}"#,
        )
        .unwrap();
        let checks = check(&unreadable).expect("checked");
        assert_eq!(checks.pairing, Pairing::Unverified);
        assert_eq!(checks.unreadable, 1);
        assert_eq!(checks.compared, 0);

        let unlinked = tmp.path().join("unlinked");
        std::fs::create_dir_all(&unlinked).unwrap();
        std::fs::write(unlinked.join("c0.png"), png(64, 64, 8, 2, 0)).unwrap();
        std::fs::write(unlinked.join("c0.json"), r#"{"role":"clean","sha256":"0"}"#).unwrap();
        std::fs::write(unlinked.join("s0.png"), png(64, 64, 8, 2, 1)).unwrap();
        std::fs::write(unlinked.join("s0.json"), r#"{"role":"stego","sha256":"0"}"#).unwrap();
        let checks = check(&unlinked).expect("checked");
        assert_eq!(checks.pairing, Pairing::Unverified);
        assert_eq!(checks.compared, 0);
    }

    #[test]
    fn a_format_this_cannot_measure_is_compared_as_far_as_its_format_and_no_further() {
        // Two BMPs of unknown size are not evidence that they match, but they
        // are evidence that neither is a JPEG. Claiming more than that would
        // be the unchecked claim in a smaller costume.
        let tmp = tempfile::tempdir().expect("tmp");
        let root = tmp.path().join("corpus");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("c0.bmp"), b"BM\x40\x00\x40\x00").unwrap();
        std::fs::write(root.join("c0.json"), r#"{"role":"clean","sha256":"0"}"#).unwrap();
        std::fs::write(root.join("s0.bmp"), b"BM\x20\x00\x20\x00").unwrap();
        std::fs::write(
            root.join("s0.json"),
            r#"{"role":"stego","source_png":"c0.bmp","sha256":"0"}"#,
        )
        .unwrap();
        let checks = check(&root).expect("checked");
        assert_eq!(checks.compared, 1);
        assert_eq!(checks.pairing, Pairing::SingleVariable);
    }

    #[test]
    fn the_count_of_violations_is_exact_while_the_examples_are_capped() {
        // A corpus built by a broken script fails on every row. The count has
        // to be the real one and the list must not grow with the corpus.
        let tmp = tempfile::tempdir().expect("tmp");
        let root = tmp.path().join("corpus");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("c0.png"), png(64, 64, 8, 2, 0)).unwrap();
        std::fs::write(root.join("c0.json"), r#"{"role":"clean","sha256":"0"}"#).unwrap();
        for i in 0..12 {
            std::fs::write(root.join(format!("s{i:02}.png")), png(48, 48, 8, 2, i)).unwrap();
            std::fs::write(
                root.join(format!("s{i:02}.json")),
                r#"{"role":"stego","source_png":"c0.png","sha256":"0"}"#,
            )
            .unwrap();
        }
        let checks = check(&root).expect("checked");
        assert_eq!(checks.pairing_breaks.count, 12);
        assert_eq!(checks.pairing_breaks.examples.len(), MAX_EXAMPLES);
    }

    /// A corpus whose records state the digests the test chooses.
    /// A corpus whose records state the TRUE digest of each image, except
    /// where the label is empty, which leaves the field off entirely.
    ///
    /// True rather than arbitrary, because a named run checks the images
    /// against their records and a fixture of made up hashes would exercise
    /// only the failure branch. The `label` still varies the content, so two
    /// corpora built from different labels get different digests.
    fn digested(root: &Path, labels: &[&str]) {
        std::fs::create_dir_all(root).expect("corpus");
        for (i, label) in labels.iter().enumerate() {
            let role = if i == 0 { "clean" } else { "stego" };
            let mut bytes = png(64, 64, 8, 2, i);
            bytes.extend_from_slice(label.as_bytes());
            let image = root.join(format!("i{i}.png"));
            std::fs::write(&image, &bytes).unwrap();
            let sha = if label.is_empty() {
                String::new()
            } else {
                format!(r#","sha256":"{}""#, hash_file(&image).expect("hashed"))
            };
            std::fs::write(
                root.join(format!("i{i}.json")),
                format!(r#"{{"role":"{role}"{sha}}}"#),
            )
            .unwrap();
        }
    }

    #[test]
    fn the_same_corpus_in_two_places_gets_the_same_digest() {
        // The point of the field. A corpus extracted from a shard and moved is
        // still the same corpus, and a digest that changed with the path would
        // name the directory rather than the data.
        let tmp = tempfile::tempdir().expect("tmp");
        let here = tmp.path().join("here");
        let there = tmp.path().join("somewhere/else/entirely");
        digested(&here, &["aa", "bb", "cc"]);
        digested(&there, &["aa", "bb", "cc"]);
        let a = check(&here).expect("checked").digest;
        let b = check(&there).expect("checked").digest;
        assert!(a.is_some(), "no digest was computed");
        assert_eq!(a, b);
    }

    #[test]
    fn one_changed_record_changes_the_corpus_digest() {
        let tmp = tempfile::tempdir().expect("tmp");
        let a = tmp.path().join("a");
        let b = tmp.path().join("b");
        digested(&a, &["aa", "bb", "cc"]);
        digested(&b, &["aa", "bb", "cd"]);
        assert_ne!(
            check(&a).expect("checked").digest,
            check(&b).expect("checked").digest
        );
    }

    #[test]
    fn a_corpus_missing_one_digest_carries_none_at_all() {
        // A digest over a partly digested corpus names the file list, and the
        // field is read as naming the content. Carrying nothing is the smaller
        // lie, which is to say it is not one.
        let tmp = tempfile::tempdir().expect("tmp");
        let root = tmp.path().join("corpus");
        digested(&root, &["aa", "", "cc"]);
        assert_eq!(check(&root).expect("checked").digest, None);
    }

    #[cfg(unix)]
    #[test]
    fn the_digest_reaches_the_result_document() {
        let tmp = tempfile::tempdir().expect("tmp");
        let root = tmp.path().join("corpus");
        digested(&root, &["aa", "bb", "cc"]);
        let entry = sizing_detector(tmp.path());
        let (result, _) = score(&entry, &request(&root, None), |_| {}).expect("scored");
        assert!(
            result.corpus.digest.starts_with("sha256:"),
            "{:?}",
            result.corpus.digest
        );
        result.validate().expect("valid");
    }

    /// A corpus of one arm, described the way the shipped packer describes it.
    fn armed(root: &Path, extra: &str, extension: &str) {
        std::fs::create_dir_all(root).expect("corpus");
        std::fs::write(root.join("c0.png"), png(64, 64, 8, 2, 0)).unwrap();
        std::fs::write(
            root.join("c0.json"),
            r#"{"role":"clean","tool":"clean","sha256":"aa"}"#,
        )
        .unwrap();
        std::fs::write(root.join(format!("s0.{extension}")), png(64, 64, 8, 2, 8)).unwrap();
        std::fs::write(
            root.join("s0.json"),
            format!(r#"{{"role":"stego","sha256":"bb"{extra}}}"#),
        )
        .unwrap();
    }

    #[test]
    fn the_arm_is_read_off_the_corpus_rather_than_assumed() {
        // Every field here used to be a constant, and two of the three were
        // wrong for most of the shipped corpus.
        let tmp = tempfile::tempdir().expect("tmp");
        let root = tmp.path().join("corpus");
        armed(
            &root,
            r#","tool":"wow","arm":"wow-0200","rate":0.2,"rate_unit":"bits per pixel","domain":"spatial""#,
            "png",
        );
        let arm = check(&root).expect("checked").arm;
        assert_eq!(arm.embedder, "wow");
        assert_eq!(arm.domain, Domain::Spatial);
        assert_eq!(arm.format, "png");
        let rate = arm.rate.expect("a rate");
        assert_eq!(rate.unit, RateUnit::Bpp);
        assert!((rate.value - 0.2).abs() < f64::EPSILON);
    }

    #[test]
    fn a_clean_row_does_not_make_every_corpus_a_mixed_one() {
        // Clean rows state `clean` as their tool. Counting them would make one
        // arm look like two on every corpus that ships its own clean half.
        let tmp = tempfile::tempdir().expect("tmp");
        let root = tmp.path().join("corpus");
        armed(&root, r#","tool":"hugo","domain":"spatial""#, "png");
        assert_eq!(check(&root).expect("checked").arm.embedder, "hugo");
    }

    #[test]
    fn a_jpeg_arm_is_not_labelled_spatial_and_a_capacity_share_is_not_bpp() {
        // The two wrong constants, in one corpus. A reader comparing 0.05 of a
        // reported capacity against 0.4 bits per pixel is comparing nothing,
        // which is why the unit refuses to be guessed.
        let tmp = tempfile::tempdir().expect("tmp");
        let root = tmp.path().join("corpus");
        armed(
            &root,
            r#","tool":"outguess","rate":0.05,"rate_unit":"fraction of the capacity outguess reports","domain":"jpeg-dct""#,
            "jpg",
        );
        let arm = check(&root).expect("checked").arm;
        assert_eq!(arm.embedder, "outguess");
        assert_eq!(arm.domain, Domain::Jpeg);
        assert_eq!(arm.format, "jpeg", "jpg and jpeg are one format");
        assert_eq!(arm.rate.expect("a rate").unit, RateUnit::CapacityFraction);
    }

    #[test]
    fn a_rate_in_a_unit_this_cannot_carry_is_dropped_rather_than_relabelled() {
        // The JPEG adaptive schemes are driven in bits per non-zero AC
        // coefficient, which `result-v1` has no unit for. Writing the number
        // under one of the two units it does have would publish a false one.
        let tmp = tempfile::tempdir().expect("tmp");
        let root = tmp.path().join("corpus");
        armed(
            &root,
            r#","tool":"juniward","rate":0.4,"rate_unit":"bits per non-zero AC coefficient","domain":"jpeg-dct""#,
            "jpg",
        );
        let arm = check(&root).expect("checked").arm;
        assert_eq!(arm.rate, None);
        assert_eq!(arm.embedder, "juniward");
    }

    #[test]
    fn a_corpus_of_several_arms_says_mixed_rather_than_picking_one() {
        // A whole tier is a legitimate thing to score and its aggregate is a
        // real number. Labelling it with whichever arm sorted first would
        // attribute that number to one scheme out of thirty-nine.
        let tmp = tempfile::tempdir().expect("tmp");
        let root = tmp.path().join("corpus");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("c0.png"), png(64, 64, 8, 2, 0)).unwrap();
        std::fs::write(root.join("c0.json"), r#"{"role":"clean","sha256":"a"}"#).unwrap();
        for (i, (tool, domain)) in [("wow", "spatial"), ("outguess", "jpeg-dct")]
            .into_iter()
            .enumerate()
        {
            std::fs::write(root.join(format!("s{i}.png")), png(64, 64, 8, 2, i + 1)).unwrap();
            std::fs::write(
                root.join(format!("s{i}.json")),
                format!(r#"{{"role":"stego","sha256":"b","tool":"{tool}","domain":"{domain}"}}"#),
            )
            .unwrap();
        }
        let arm = check(&root).expect("checked").arm;
        assert_eq!(arm.embedder, "mixed");
        assert_eq!(arm.domain, Domain::Mixed);
    }

    #[test]
    fn a_corpus_that_says_nothing_about_its_arm_says_unstated() {
        // The honest answer to a directory of loose samples, and a different
        // statement from naming a scheme nobody recorded.
        let tmp = tempfile::tempdir().expect("tmp");
        let root = tmp.path().join("corpus");
        armed(&root, "", "png");
        let arm = check(&root).expect("checked").arm;
        assert_eq!(arm.embedder, "unstated");
        assert_eq!(arm.rate, None);
        // The format is the only evidence left, and a PNG corpus is a spatial
        // one until something says otherwise.
        assert_eq!(arm.domain, Domain::Spatial);
    }

    #[test]
    fn a_cover_renamed_by_the_packer_is_still_found_by_the_name_its_record_gives() {
        // The shape of a real release, and the one that made both checks inert
        // before this: every member is renamed to its position in the tier, so
        // the cover on disk is `000000.png` while every stego row made from it
        // still names `09710.png`. Joining on the name on disk matches nothing,
        // and a check that cannot look reports clean.
        let tmp = tempfile::tempdir().expect("tmp");
        let root = tmp.path().join("corpus");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("000000.png"), png(64, 64, 8, 2, 0)).unwrap();
        std::fs::write(
            root.join("000000.json"),
            r#"{"role":"clean","file":"09710.png","split":"test","sha256":"a"}"#,
        )
        .unwrap();
        // A stego twin that is a different size, and in the other split.
        std::fs::write(root.join("000001.png"), png(48, 48, 8, 2, 0)).unwrap();
        std::fs::write(
            root.join("000001.json"),
            r#"{"role":"stego","file":"000001.png","source_png":"09710.png","split":"train","sha256":"b"}"#,
        )
        .unwrap();

        let checks = check(&root).expect("checked");
        assert_eq!(
            checks.pairing,
            Pairing::Confounded,
            "the cover was never found, so nothing was compared"
        );
        assert_eq!(checks.split_leaks.count, 1, "the split join did not fire");
    }

    #[test]
    fn an_arm_reusing_a_basename_cannot_displace_the_cover_it_collides_with() {
        // The shape of a real multi arm extraction. Every arm restarts its
        // numbering at 000000, so several clean arms offer the same name on
        // disk as a cover tier file, and an overwrite there attaches a stego
        // row to the wrong photograph without saying anything.
        let tmp = tempfile::tempdir().expect("tmp");
        let root = tmp.path().join("corpus");
        std::fs::create_dir_all(root.join("cover")).unwrap();
        std::fs::create_dir_all(root.join("clean-grey")).unwrap();

        // The cover, 64 by 64, extracted as 000000.png but recorded as its
        // own name in the tier.
        std::fs::write(root.join("cover/000000.png"), png(64, 64, 8, 2, 0)).unwrap();
        std::fs::write(
            root.join("cover/000000.json"),
            r#"{"role":"clean","file":"09710.png","sha256":"a"}"#,
        )
        .unwrap();
        // A clean arm image of a DIFFERENT size, whose own name on disk and
        // in its record is also 000000.png.
        std::fs::write(root.join("clean-grey/000000.png"), png(32, 32, 8, 2, 0)).unwrap();
        std::fs::write(
            root.join("clean-grey/000000.json"),
            r#"{"role":"clean","file":"000000.png","sha256":"b"}"#,
        )
        .unwrap();
        // A stego row made from the cover, matching the cover's shape.
        std::fs::write(root.join("s0.png"), png(64, 64, 8, 2, 7)).unwrap();
        std::fs::write(
            root.join("s0.json"),
            r#"{"role":"stego","source_png":"09710.png","sha256":"c"}"#,
        )
        .unwrap();

        let checks = check(&root).expect("checked");
        assert_eq!(checks.compared, 1);
        assert_eq!(
            checks.pairing,
            Pairing::SingleVariable,
            "the stego row was compared against the wrong image: {}",
            checks.pairing_breaks.examples()
        );
    }

    #[test]
    fn a_manifest_carrying_a_path_in_its_file_field_still_joins() {
        // Some manifests write `covers/09710.png` rather than a bare name. The
        // join only ever needs the last component, and a path in that field
        // must not reach out of the corpus either.
        let tmp = tempfile::tempdir().expect("tmp");
        let root = tmp.path().join("corpus");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("000000.png"), png(64, 64, 8, 2, 0)).unwrap();
        std::fs::write(
            root.join("000000.json"),
            r#"{"role":"clean","file":"covers/09710.png","sha256":"a"}"#,
        )
        .unwrap();
        std::fs::write(root.join("000001.png"), png(64, 64, 8, 2, 9)).unwrap();
        std::fs::write(
            root.join("000001.json"),
            r#"{"role":"stego","source_png":"09710.png","sha256":"b"}"#,
        )
        .unwrap();
        let checks = check(&root).expect("checked");
        assert_eq!(checks.compared, 1);
        assert_eq!(checks.pairing, Pairing::SingleVariable);
    }

    /// A registry entry for a corpus, with the digest the test chooses.
    fn registered_corpus(declared: Option<&str>) -> CorpusEntry {
        let integrity = match declared {
            Some(d) => format!("[integrity]\nrecords_sha256 = \"{d}\"\n"),
            None => String::new(),
        };
        toml::from_str(&format!(
            "id = \"example\"\nname = \"Example Tier\"\ntier = \"nano\"\n\
             description = \"For a test.\"\n{integrity}\
             [licence]\nstatus = \"unverified\"\nredistribution = \"unknown\"\n\
             redistribution_reason = \"A fixture.\"\n\
             [obtain]\ninstructions = \"A fixture.\"\n\
             [properties]\nbase_images = 1\nsize_note = \"A fixture.\"\n"
        ))
        .expect("parses")
    }

    #[cfg(unix)]
    #[test]
    fn a_corpus_matching_the_digest_its_registry_entry_declares_is_named() {
        // The only way to `named`. The harness is comparing the bytes in front
        // of it with a claim somebody else wrote down first, which is what
        // makes the field worth having.
        let tmp = tempfile::tempdir().expect("tmp");
        let root = tmp.path().join("corpus");
        digested(&root, &["aa", "bb", "cc"]);
        let digest = corpus_digest(&root).expect("readable").expect("named");
        let entry = registered_corpus(Some(&digest));

        let mut request = request(&root, None);
        request.registered = Some(&entry);
        let (result, _) = score(&sizing_detector(tmp.path()), &request, |_| {}).expect("scored");

        assert_eq!(result.declarations.configuration, Configuration::Named);
        assert_eq!(result.corpus.name, "Example Tier");
        assert_eq!(result.corpus.tier.as_deref(), Some("nano"));
        result.validate().expect("valid");
    }

    /// THE CLAIM THE TWO-HALVES REFACTOR RESTS ON, MEASURED RATHER THAN
    /// DESCRIBED.
    ///
    /// Scoring seven detectors over a Core tier must not hash 344,357 images
    /// seven times. The byte verification is the most expensive thing this
    /// module does and it emits one distinctive progress line, so counting
    /// that line across a two-detector run is a direct measurement of how many
    /// times the corpus was established.
    #[cfg(unix)]
    #[test]
    fn the_expensive_corpus_work_happens_once_however_many_detectors_are_asked() {
        let tmp = tempfile::tempdir().expect("tmp");
        let root = tmp.path().join("corpus");
        digested(&root, &["aa", "bb", "cc"]);
        let digest = corpus_digest(&root).expect("readable").expect("named");
        let entry = registered_corpus(Some(&digest));

        let mut lines: Vec<String> = Vec::new();
        let prepared = prepare(&root, Some(&entry), None, |l: &str| {
            lines.push(l.to_string())
        })
        .expect("prepared");

        let verified = |lines: &[String]| {
            lines
                .iter()
                .filter(|l| l.contains("are the files their records describe"))
                .count()
        };
        assert_eq!(
            verified(&lines),
            1,
            "the byte check should have run exactly once: {lines:?}"
        );

        // Two detectors against the one `Prepared`. Different records files,
        // so neither resumes from the other.
        let detector = sizing_detector(tmp.path());
        let after = lines.len();
        for name in ["first", "second"] {
            let records = tmp.path().join(format!("{name}.jsonl"));
            score_one(
                &detector,
                &prepared,
                &records,
                Duration::from_secs(5),
                |l: &str| lines.push(l.to_string()),
            )
            .unwrap_or_else(|e| panic!("{name} failed: {e}"));
        }
        assert_eq!(
            verified(&lines),
            1,
            "a detector re-ran the byte check: {:?}",
            &lines[after..]
        );

        // And the shared preparation is genuinely attributed to each result
        // rather than dropped: both name the same start.
        let one = score_one(
            &detector,
            &prepared,
            &tmp.path().join("third.jsonl"),
            Duration::from_secs(5),
            |_| {},
        )
        .expect("third")
        .0;
        assert_eq!(one.provenance.started_utc, prepared.started_utc);
        assert!(one.provenance.elapsed_seconds >= prepared.preflight_seconds());
        assert_eq!(one.declarations.configuration, Configuration::Named);
    }

    /// A container is sandboxed with no network; a local program and a host
    /// adapter are not. The one-line version got two of the three wrong, and
    /// the service case got it backwards: the network is not merely reachable
    /// for one of those, it is the only way the tool is asked anything.
    #[test]
    fn what_a_result_says_about_the_network_is_true_for_all_three_routes() {
        let digest = "5".repeat(64);
        let container: Entry = toml::from_str(&format!(
            "name = \"c\"\nkind = \"detector\"\nlicence = \"MIT\"\n\
             [image]\nreference = \"ghcr.io/x/y@sha256:{digest}\"\n\
             [selftest]\nmust_detect = \"a.png\"\nmust_clear = \"b.png\"\n"
        ))
        .expect("parses");
        let local: Entry = toml::from_str(
            "name = \"l\"\nkind = \"detector\"\nlicence = \"MIT\"\n\
             [binary]\ncommand = [\"sh\"]\n\
             [selftest]\nmust_detect = \"a.png\"\nmust_clear = \"b.png\"\n",
        )
        .expect("parses");
        let service: Entry = toml::from_str(&format!(
            "name = \"s\"\nkind = \"detector\"\nlicence = \"MIT\"\n\
             [image]\nreference = \"ghcr.io/x/s@sha256:{digest}\"\n\
             [invoke]\nhost = true\nadapter = \"a.py\"\nentrypoint = \"python3\"\n\
             argv = [\"{{adapter}}\", \"{{file}}\"]\nparser = \"number\"\n\
             [selftest]\nmust_detect = \"a.png\"\nmust_clear = \"b.png\"\n"
        ))
        .expect("parses");

        assert!(runs_in_container(&container));
        assert!(!runs_in_container(&local));
        assert!(
            !runs_in_container(&service),
            "an entry naming an image AND invoke.host does not run in that \
             image, so a result claiming a sandbox and no network is false \
             about both halves"
        );
    }

    /// The two fields that replaced `route`, over the three shapes the
    /// registry actually holds plus the one that used to break the single
    /// field. Each is asserted on its own, because each is read on its own.
    #[test]
    fn the_pin_and_the_sandbox_are_answered_separately_for_every_entry_shape() {
        let digest = "5".repeat(64);
        let image_block = format!("[image]\nreference = \"ghcr.io/x/y@sha256:{digest}\"\n");
        let tail = "[selftest]\nmust_detect = \"a.png\"\nmust_clear = \"b.png\"\n";
        let adapter = "[invoke]\nhost = true\nadapter = \"a.py\"\n\
             entrypoint = \"python3\"\nargv = [\"{adapter}\", \"{file}\"]\n\
             parser = \"number\"\n";

        let parse = |body: String| -> Entry { toml::from_str(&body).expect("parses") };
        let head = |n: &str| format!("name = \"{n}\"\nkind = \"detector\"\nlicence = \"MIT\"\n");

        let container = parse(format!("{}{image_block}{tail}", head("c")));
        assert_eq!(pinning(&container).1, PinnedBy::ImageDigest);
        assert_eq!(isolation(&container), Isolation::SandboxNoNetwork);

        // A program that is not installed cannot be hashed, and the fallback
        // must be the admission rather than a hash-shaped "unknown".
        let local = parse(format!(
            "{}[binary]\ncommand = [\"definitely-not-installed-xyzzy\"]\n{tail}",
            head("l")
        ));
        assert_eq!(pinning(&local).1, PinnedBy::Unpinned);
        assert_eq!(isolation(&local), Isolation::Host);

        // The shape that broke the single field: the image names the subject,
        // an adapter here did the running, and the tool was reached over HTTP.
        let service = parse(format!(
            "{}{image_block}{adapter}endpoint_env = \"X_ENDPOINT\"\n{tail}",
            head("s")
        ));
        assert_eq!(
            pinning(&service).1,
            PinnedBy::Unpinned,
            "nothing in the run checked that the instance came from that image"
        );
        assert_eq!(pinning(&service).0, format!("ghcr.io/x/y@sha256:{digest}"));
        assert_eq!(isolation(&service), Isolation::RemoteService);

        // An adapter with no endpoint reaches no service, so it is an ordinary
        // host run, and the image it names still did not execute.
        let adapter_only = parse(format!("{}{image_block}{adapter}{tail}", head("a")));
        assert_eq!(pinning(&adapter_only).1, PinnedBy::Unpinned);
        assert_eq!(isolation(&adapter_only), Isolation::Host);
    }

    /// A document must never state a sandbox beside a reachable network, which
    /// is the contradiction the old single field forced onto every service
    /// result. Asserted against `validate` so it cannot come back quietly.
    #[test]
    fn the_two_fields_and_the_network_flag_never_contradict_each_other() {
        for entry in [
            "name = \"c\"\nkind = \"detector\"\nlicence = \"MIT\"\n\
             [image]\nreference = \"ghcr.io/x/y@sha256:abc\"\n",
            "name = \"s\"\nkind = \"detector\"\nlicence = \"MIT\"\n\
             [image]\nreference = \"ghcr.io/x/y@sha256:abc\"\n\
             [invoke]\nhost = true\nadapter = \"a.py\"\nentrypoint = \"python3\"\n\
             argv = [\"{adapter}\", \"{file}\"]\nparser = \"number\"\n\
             endpoint_env = \"X_ENDPOINT\"\n",
            "name = \"l\"\nkind = \"detector\"\nlicence = \"MIT\"\n\
             [binary]\ncommand = [\"sh\"]\n",
        ] {
            let e: Entry = toml::from_str(&format!(
                "{entry}[selftest]\nmust_detect = \"a.png\"\nmust_clear = \"b.png\"\n"
            ))
            .expect("parses");
            let sandboxed = isolation(&e) == Isolation::SandboxNoNetwork;
            // `network_reachable` is written as this expression, so the two
            // fields are asserted to be opposites rather than merely to look
            // plausible side by side.
            let network_reachable = !runs_in_container(&e);
            assert_ne!(
                sandboxed, network_reachable,
                "{}: a sandbox with no network beside a reachable network is \
                 the contradiction this change exists to remove",
                e.name
            );
        }
    }

    /// Proved end to end rather than through the helper alone, because the
    /// field a reader acts on is the one in the document.
    #[cfg(unix)]
    #[test]
    fn a_local_run_reports_the_network_as_reachable_in_the_document() {
        let tmp = tempfile::tempdir().expect("tmp");
        let root = tmp.path().join("corpus");
        corpus(&root, 2, 2);
        let (result, _) =
            score(&sizing_detector(tmp.path()), &request(&root, None), |_| {}).expect("scored");
        assert!(
            result.provenance.network_reachable,
            "a program on this machine runs with this machine's network"
        );
    }

    #[test]
    fn every_way_a_run_can_fail_has_its_own_exit_code() {
        // They all used to be 4, plugin failure, including four where no
        // plugin was involved. A script and an agent act on these.
        use stegobench_core::exit;
        let cases = [
            (ScoreError::NotADetector { name: "x".into() }, exit::USAGE),
            (
                ScoreError::NotThatCorpus {
                    path: "p".into(),
                    id: "i".into(),
                    want: "a".into(),
                    got: "b".into(),
                },
                exit::VERIFY_MISMATCH,
            ),
            (
                ScoreError::SplitLeaks {
                    path: "p".into(),
                    count: 1,
                    examples: "e".into(),
                },
                exit::PREFLIGHT_REFUSED,
            ),
            (
                ScoreError::OneSided {
                    path: "p".into(),
                    clean: 1,
                    stego: 0,
                },
                exit::PREFLIGHT_REFUSED,
            ),
            (
                ScoreError::Records {
                    path: "p".into(),
                    source: std::io::Error::other("x"),
                },
                exit::FAILURE,
            ),
            (
                ScoreError::NoAuc {
                    name: "x".into(),
                    answered: 4,
                    why: "w".into(),
                },
                exit::PLUGIN_FAILED,
            ),
        ];
        for (err, want) in cases {
            assert_eq!(err.exit_code(), want, "{err}");
        }
    }

    /// The fallback that used to sit on the AUC, and why it could not stay.
    ///
    /// `.unwrap_or(0.5)` wrote the one value an AUC must never default to: 0.5
    /// is exactly what the metric says when a detector carries no information,
    /// so a run that could not be measured was published as a run that came
    /// out at chance, and no field in the document told the two apart.
    #[test]
    fn a_run_with_no_computable_auc_refuses_rather_than_publishing_a_half() {
        let err = ScoreError::NoAuc {
            name: "zsteg".into(),
            answered: 12,
            why: why_no_auc(&[1.0, f64::NAN], &[false, true], ""),
        };
        let text = err.to_string();
        assert_eq!(err.exit_code(), stegobench_core::exit::PLUGIN_FAILED);
        // The reader is told which condition fired, not merely that one did.
        assert!(text.contains("not numbers"), "{text}");
        // And is told why 0.5 in particular could not be the fallback, since
        // that is the whole argument.
        assert!(text.contains("0.5"), "{text}");
        assert!(text.contains("nothing is written"), "{text}");
    }

    /// Three conditions, three actions, so three messages rather than one.
    #[test]
    fn every_reason_an_auc_cannot_be_computed_is_named_separately() {
        let mismatch = why_no_auc(&[1.0, 2.0], &[true], "");
        assert!(mismatch.contains("2 score(s)") && mismatch.contains("1 label(s)"));
        assert!(
            mismatch.contains("bug in stegobench"),
            "a length mismatch is not the user's doing: {mismatch}"
        );

        let nan = why_no_auc(&[f64::NAN, 1.0], &[true, false], "");
        assert!(
            nan.contains("1 of the 2 answer(s) are not numbers"),
            "{nan}"
        );
        assert!(
            nan.contains("stegobench doctor"),
            "a NaN from a detector needs a next step: {nan}"
        );

        // The case a detector that fails on everything lands in. The shared
        // sentence is `metrics`' one, where a user really did hand over an
        // empty file, so on this path it needs the reason nothing arrived.
        let empty = why_no_auc(&[], &[], "");
        assert!(
            empty.contains("none of them came back with a number"),
            "an empty answer set has to say the tool produced nothing: {empty}"
        );
        assert!(
            empty.contains("stegobench doctor"),
            "and where to look next: {empty}"
        );

        let one_sided = why_no_auc(&[1.0, 2.0], &[true, true], "");
        assert!(one_sided.contains("0 clean and 2 stego"), "{one_sided}");
        assert!(
            one_sided.contains("this tool produced a usable answer"),
            "the tool, not the corpus, is what is one-sided here: {one_sided}"
        );

        // And the residual case is not silently dressed up as one of the
        // three. It cannot be reached from `roc_auc`'s own contract, so it
        // says it is a bug rather than inventing a cause.
        let residual = why_no_auc(&[1.0, 2.0], &[true, false], "");
        assert!(residual.contains("bug in stegobench"), "{residual}");

        // The operating point is carried into the residual message, so a
        // detection rate that could not be formed says which budget it was.
        let at_budget = why_no_auc(&[1.0, 2.0], &[true, false], " at a budget of 0.01");
        assert!(at_budget.contains("at a budget of 0.01"), "{at_budget}");
    }

    /// The sentences come from `MetricsError` and are not a second copy.
    ///
    /// This is the drift guard: the refusal a user reads on the score path
    /// has to be the refusal `stegobench metrics` gives for the same
    /// condition, with only the score path's own next step added.
    #[test]
    fn the_refusals_are_the_metrics_commands_own_words() {
        for (scores, labels) in [
            (vec![1.0, 2.0], vec![true]),
            (vec![f64::NAN, 1.0], vec![true, false]),
            (vec![1.0, 2.0], vec![true, true]),
            (vec![], vec![]),
        ] {
            let shared = crate::metrics::why_unrankable(&scores, &labels, "").to_string();
            let here = why_no_auc(&scores, &labels, "");
            assert!(
                here.starts_with(&shared),
                "{here}\ndoes not open with\n{shared}"
            );
        }
    }

    /// The four conditions above are what `roc_auc` itself answers `None` to,
    /// asserted against the crate rather than against this file's memory of
    /// it. If the metrics crate grows a fifth, this fails and the diagnosis
    /// above gets updated rather than quietly falling through to "a bug".
    #[test]
    fn the_conditions_this_diagnoses_are_the_ones_the_metric_actually_refuses() {
        assert_eq!(stegobench_metrics::roc_auc(&[1.0, 2.0], &[true]), None);
        assert_eq!(
            stegobench_metrics::roc_auc(&[f64::NAN, 1.0], &[true, false]),
            None
        );
        assert_eq!(
            stegobench_metrics::roc_auc(&[1.0, 2.0], &[true, true]),
            None
        );
        // And a well-formed input still produces one, so the test above is
        // not passing because nothing works.
        assert!(stegobench_metrics::roc_auc(&[1.0, 2.0], &[false, true]).is_some());
    }

    #[cfg(unix)]
    #[test]
    fn swapping_the_images_under_a_matching_manifest_is_caught() {
        // The attack the byte check exists for, and the reason a corpus digest
        // alone cannot carry a named result. Take the real records, keep them
        // byte for byte so the corpus digest is unchanged, and put easier
        // images underneath. Everything the registry can compare still agrees.
        let tmp = tempfile::tempdir().expect("tmp");
        let root = tmp.path().join("corpus");
        digested(&root, &["aa", "bb", "cc"]);
        let digest = corpus_digest(&root).expect("readable").expect("named");
        let entry = registered_corpus(Some(&digest));

        // The records are untouched; only the pixels change.
        std::fs::write(root.join("i1.png"), png(64, 64, 8, 2, 99)).unwrap();
        assert_eq!(
            corpus_digest(&root).expect("readable").as_deref(),
            Some(digest.as_str()),
            "the corpus digest noticed, which would make this test prove nothing"
        );

        let mut request = request(&root, None);
        request.registered = Some(&entry);
        let err = score(&sizing_detector(tmp.path()), &request, |_| {}).expect_err("refused");
        assert!(
            err.to_string().contains("is not the image its own record"),
            "{err}"
        );
        assert_eq!(err.exit_code(), stegobench_core::exit::VERIFY_MISMATCH);
    }

    #[cfg(unix)]
    #[test]
    fn a_custom_run_does_not_pay_for_the_byte_check() {
        // There is no external claim to check a custom run against, the
        // records are the only description of that corpus that exists, and
        // comparing them with themselves would be theatre that costs a whole
        // extra read of the corpus.
        let tmp = tempfile::tempdir().expect("tmp");
        let root = tmp.path().join("corpus");
        digested(&root, &["aa", "bb", "cc"]);
        std::fs::write(root.join("i1.png"), png(64, 64, 8, 2, 99)).unwrap();

        let mut said = Vec::new();
        let (result, _) = score(&sizing_detector(tmp.path()), &request(&root, None), |l| {
            said.push(l.to_string())
        })
        .expect("scored");
        assert_eq!(result.declarations.configuration, Configuration::Custom);
        assert!(
            !said.iter().any(|l| l.contains("their records describe")),
            "a custom run paid for the check: {said:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_directory_that_is_not_the_corpus_it_claims_is_refused() {
        // Refused rather than downgraded to custom. The user asserted
        // something about these bytes that is not true of them, and filing the
        // run quietly as custom would answer a different question from the one
        // they asked.
        let tmp = tempfile::tempdir().expect("tmp");
        let root = tmp.path().join("corpus");
        digested(&root, &["aa", "bb", "cc"]);
        let entry = registered_corpus(Some(&format!("sha256:{}", "0".repeat(64))));

        let mut request = request(&root, None);
        request.registered = Some(&entry);
        let err = score(&sizing_detector(tmp.path()), &request, |_| {}).expect_err("refused");
        assert!(err.to_string().contains("is not example"), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn naming_a_corpus_whose_entry_declares_no_digest_stays_custom() {
        // Nobody has computed the digest for most corpora, and that is the
        // ordinary state rather than a fault. What must not happen is the name
        // alone earning `named`, because then the person running the benchmark
        // sets the field.
        let tmp = tempfile::tempdir().expect("tmp");
        let root = tmp.path().join("corpus");
        digested(&root, &["aa", "bb", "cc"]);
        let entry = registered_corpus(None);

        let mut request = request(&root, None);
        request.registered = Some(&entry);
        let mut said = Vec::new();
        let (result, _) = score(&sizing_detector(tmp.path()), &request, |l| {
            said.push(l.to_string())
        })
        .expect("scored");

        assert_eq!(result.declarations.configuration, Configuration::Custom);
        assert!(
            said.iter()
                .any(|l| l.contains("declares no records digest")),
            "it downgraded silently: {said:?}"
        );
        // The name and tier are still carried: they are what the user said,
        // and the digest beside them is what the harness measured.
        assert_eq!(result.corpus.name, "Example Tier");
    }

    #[cfg(unix)]
    #[test]
    fn a_partial_run_over_a_matching_corpus_is_still_custom() {
        // A prefix of a tier is not the tier, whatever the whole of it hashes
        // to, and the digest is taken over the whole corpus rather than over
        // what was scored.
        let tmp = tempfile::tempdir().expect("tmp");
        let root = tmp.path().join("corpus");
        digested(&root, &["aa", "bb", "cc", "dd"]);
        let digest = corpus_digest(&root).expect("readable").expect("named");
        let entry = registered_corpus(Some(&digest));

        let mut request = request(&root, Some(3));
        request.registered = Some(&entry);
        let mut said = Vec::new();
        let (result, _) = score(&sizing_detector(tmp.path()), &request, |l| {
            said.push(l.to_string())
        })
        .expect("scored");

        assert_eq!(result.declarations.configuration, Configuration::Custom);
        assert!(
            said.iter().any(|l| l.contains("prefix of a tier")),
            "{said:?}"
        );
    }
    #[test]
    fn an_embedder_is_refused_rather_than_scored() {
        // Asking an embedder to tell two images apart produces a number that
        // means nothing, and it would look exactly like a weak detector.
        let entry: Entry = toml::from_str(
            "name = \"x\"\nkind = \"embedder\"\nlicence = \"MIT\"\n\
             [image]\nreference = \"a@sha256:b\"\nsize_mb = 1\nbundled = true\n\
             [selftest]\nmust_detect = \"fixtures/a.png\"\nmust_clear = \"fixtures/b.png\"\n",
        )
        .expect("parses");
        let tmp = tempfile::tempdir().expect("tmp");
        let root = tmp.path().join("corpus");
        corpus(&root, 1, 1);
        let err = score(&entry, &request(&root, None), |_| {}).expect_err("refused");
        assert!(err.to_string().contains("cannot be asked"), "{err}");
    }
}
