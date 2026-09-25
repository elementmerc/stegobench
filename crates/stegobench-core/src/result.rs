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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tier: Option<String>,
    /// Fetched by us, or supplied by the user. See [`CorpusSource`].
    pub source: CorpusSource,
    /// Digest over the corpus manifest. A result that cannot name the bytes it
    /// was measured on is an anecdote, and `stegobench verify` re-checks this
    /// rather than trusting it.
    pub digest: String,
    pub pairs: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub split: Option<String>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verdict_rate: Option<f64>,
    pub n_clean: u64,
    pub n_stego: u64,
    /// Images the detector could not score. **Required**, so that zero is an
    /// assertion rather than an absence: a metric computed over the survivors
    /// of a partly failed run is how an evaluation misleads without anyone
    /// intending it.
    pub n_error: u64,
}

/// Everything needed to run it again.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Provenance {
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
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PluginRef {
    pub name: String,
    /// Image digest, never a tag.
    pub image: String,
    pub determinism: Determinism,
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
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Host {
    pub cores: u32,
    pub memory_gb: u32,
    pub os: String,
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
    /// Set by the submission path, never by the submitter.
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
    SingleVariable,
    /// Clean and stego differ in something else as well, and the arm is kept
    /// deliberately as a demonstration of what that does.
    Confounded,
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
        for p in &self.provenance.plugins {
            if !p.image.contains('@') {
                bad.push(format!(
                    "plugin {:?} names image {:?}, which is not pinned by digest",
                    p.name, p.image
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
                tier: Some("core".into()),
                source: CorpusSource::Fetched,
                digest: "sha256:b633b019".into(),
                pairs: 1000,
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
                verdict_rate: None,
                n_clean: 300,
                n_stego: 300,
                n_error: 0,
            },
            provenance: Provenance {
                seed: Some(20260917),
                plugins: vec![PluginRef {
                    name: "aletheia-rich".into(),
                    image: "ghcr.io/x/y@sha256:abc".into(),
                    determinism: Determinism::Exact,
                }],
                harness_version: "0.1.0".into(),
                started_utc: "2026-09-17T09:02:11Z".into(),
                elapsed_seconds: 10754.8,
                network_reachable: false,
                host: None,
            },
            declarations: Declarations {
                split_discipline: SplitDiscipline::ByCover,
                pairing: Pairing::SingleVariable,
                configuration: Configuration::Named,
                trained_on: None,
                self_reported: false,
            },
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
}
