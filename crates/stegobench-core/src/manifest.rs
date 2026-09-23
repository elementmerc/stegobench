// Author:  Daniel Iwugo
// Comment: Christ is King
// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Daniel Iwugo
//! `manifest-v1`: one row per corpus file.
//!
//! Formalises what `manifest.jsonl` already carries, with the fields added
//! during the 2026-09-18 repair now mandatory rather than bolted on. Every row
//! is a self-contained document, because the manifest is a JSONL stream and a
//! reader that only ever sees one line at a time still has to know what
//! licence governs the file it names.
//!
//! `pristine` is mandatory and honest. Every Pentimento row says
//! `original_mime: image/jpeg, pristine: false`, and has since the first
//! fetch: it was there and nobody read it, which is how a document came to
//! claim BOSSbase comparability. A validator that requires the field does not
//! fix inattention on its own; `stegobench describe corpus` printing it at the
//! top is the other half.
//!
//! `licence` and `licence_url` are per row, never per collection, because
//! 5,429 of Pentimento's covers (54.3%) are CC BY and carry attribution
//! obligations that a collection-level licence would erase.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The value of the `schema` field. Readers match on this exactly.
pub const MANIFEST_SCHEMA_ID: &str = "stegobench/manifest-v1";

/// One row: one file, its identity, its provenance and its licence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ManifestV1 {
    /// Always [`MANIFEST_SCHEMA_ID`].
    pub schema: String,
    pub file: String,
    /// Lower-case hex, 64 characters. Checked in [`ManifestV1::validate`]
    /// because a manifest whose digest is not actually a SHA-256 defeats the
    /// entire point of `stegobench verify`.
    pub sha256: String,
    pub width: u32,
    pub height: u32,
    pub original_mime: String,
    /// False whenever the file passed through any lossy re-encode before it
    /// reached us. Mandatory, not defaulted, so a reader cannot mistake an
    /// omission for a true claim of pristine capture.
    pub pristine: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crop_box: Option<[u32; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attribution: Option<String>,
    pub licence: String,
    pub licence_url: String,
    pub capture_class: String,
    pub capture_class_basis: String,
    pub split: String,
    /// The only ordering. Both builders originally used `rng.sample()`, which
    /// does not nest, so Nano was not a prefix of Lite; tiers now nest by
    /// construction and it is proven by checksum.
    pub tier_order: u64,
    pub split_salt: String,
}

impl ManifestV1 {
    /// Checks the things the type system cannot.
    pub fn validate(&self) -> std::result::Result<(), Vec<String>> {
        let mut bad = Vec::new();
        if self.schema != MANIFEST_SCHEMA_ID {
            bad.push(format!(
                "schema is {:?}, expected {MANIFEST_SCHEMA_ID:?}",
                self.schema
            ));
        }
        if self.sha256.len() != 64 || !self.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
            bad.push(format!("sha256 {:?} is not 64 hex characters", self.sha256));
        } else if self.sha256.bytes().any(|b| b.is_ascii_uppercase()) {
            bad.push(format!(
                "sha256 {:?} has upper case hex; digests are compared as text \
                 elsewhere in the harness and must be canonically lower case",
                self.sha256
            ));
        }
        if self.licence.trim().is_empty() {
            bad.push(format!(
                "{} declares no licence, and a collection level licence is not \
                 acceptable here: every cover needs its own",
                self.file
            ));
        }
        if self.licence_url.trim().is_empty() {
            bad.push(format!(
                "{} declares a licence but no licence_url; add a link to the \
                 licence text itself, not just its name, so a downstream user \
                 does not have to guess which version of {:?} applies",
                self.file, self.licence
            ));
        }
        if self.width == 0 || self.height == 0 {
            bad.push(format!(
                "{} has a zero dimension ({}x{})",
                self.file, self.width, self.height
            ));
        }
        if let Some(cb) = self.crop_box {
            let [x0, y0, x1, y1] = cb;
            if x1 <= x0 || y1 <= y0 {
                bad.push(format!(
                    "{} has a crop_box {cb:?} that is empty or inverted",
                    self.file
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

    fn sample() -> ManifestV1 {
        ManifestV1 {
            schema: MANIFEST_SCHEMA_ID.into(),
            file: "core/000123.jpg".into(),
            sha256: "b".repeat(64),
            width: 1024,
            height: 768,
            original_mime: "image/jpeg".into(),
            pristine: false,
            crop_box: None,
            source_url: Some("https://example.org/photo/123".into()),
            attribution: Some("Jane Doe".into()),
            licence: "CC BY 4.0".into(),
            licence_url: "https://creativecommons.org/licenses/by/4.0/".into(),
            capture_class: "camera".into(),
            capture_class_basis: "exif".into(),
            split: "test".into(),
            tier_order: 42,
            split_salt: "pentimento-core-2026".into(),
        }
    }

    #[test]
    fn a_well_formed_row_validates() {
        assert_eq!(sample().validate(), Ok(()));
    }

    #[test]
    fn round_trips_through_json_unchanged() {
        let a = sample();
        let text = serde_json::to_string(&a).unwrap();
        let b: ManifestV1 = serde_json::from_str(&text).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn a_short_digest_is_refused() {
        let mut r = sample();
        r.sha256 = "abc123".into();
        assert!(r.validate().unwrap_err()[0].contains("64 hex"));
    }

    #[test]
    fn an_upper_case_digest_is_refused() {
        let mut r = sample();
        r.sha256 = "B".repeat(64);
        assert!(r.validate().unwrap_err()[0].contains("upper case"));
    }

    #[test]
    fn a_missing_licence_is_refused() {
        let mut r = sample();
        r.licence = String::new();
        assert!(r.validate().unwrap_err()[0].contains("no licence"));
    }

    #[test]
    fn a_missing_licence_url_is_refused() {
        let mut r = sample();
        r.licence_url = String::new();
        assert!(r.validate().unwrap_err()[0].contains("licence_url"));
    }

    #[test]
    fn pristine_false_is_a_real_value_not_an_absence() {
        // Serialised, the field must still be present: a default-on-missing
        // parser would let a stripped record silently read as pristine.
        let v = serde_json::to_value(sample()).unwrap();
        assert_eq!(v["pristine"], serde_json::json!(false));
    }

    #[test]
    fn pristine_is_required_on_parse() {
        let mut v = serde_json::to_value(sample()).unwrap();
        v.as_object_mut().unwrap().remove("pristine");
        assert!(serde_json::from_value::<ManifestV1>(v).is_err());
    }

    #[test]
    fn a_zero_dimension_is_refused() {
        let mut r = sample();
        r.width = 0;
        assert!(r.validate().unwrap_err()[0].contains("zero dimension"));
    }

    #[test]
    fn an_inverted_crop_box_is_refused() {
        let mut r = sample();
        r.crop_box = Some([100, 100, 50, 50]);
        assert!(r.validate().unwrap_err()[0].contains("inverted"));
    }
}
