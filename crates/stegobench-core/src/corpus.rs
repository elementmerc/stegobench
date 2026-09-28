// Author:  Daniel Iwugo
// Comment: Christ is King
// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Daniel Iwugo
//! Corpora as declarative data, beside the tools but not inside them.
//!
//! WHY A SEPARATE TYPE RATHER THAN A THIRD `Kind`
//! ----------------------------------------------
//! A plugin is code you run. A corpus is data you point at. The only field the
//! two genuinely share is a name: a corpus has no image digest, no argv, no
//! self-test and no cost per image, and a plugin has no licence URL, no
//! download route and no cover count. Folding a corpus into [`Entry`] would
//! leave half its fields meaningless and the other half misused, and an image
//! digest on a thing with no image is how a schema rots.
//!
//! They share a vocabulary instead of a struct: `list corpora` and
//! `describe <id>` work exactly as they do for a detector, so a user sees one
//! registry.
//!
//! WHY THE LICENCE BLOCK IS THE BIGGEST PART OF IT
//! -----------------------------------------------
//! `docs/design/cover-source-licensing.md` catalogues mirrors of academic
//! corpora labelled MIT, Apache 2.0 and CC0 where the original granted none of
//! them. Every one of those labels is a plausible guess written into a metadata
//! field by somebody who did not read the terms, and once written it is copied
//! onward by every survey that reads it at face value.
//!
//! So this schema refuses to let a licence be asserted quietly. A licence is
//! `verified` only with an SPDX identifier, a URL, a date and a note saying
//! what was read; otherwise it is `unverified` or `none-granted`, and both of
//! those must say why. **Redistribution is a separate field from the licence**,
//! because the distinction the whole document turns on is that ALASKA2 and
//! BOSSbase may be used and may not be republished, for two different reasons.
//! Scoring is never gated by it; publishing is.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::registry::RegistryError;

/// A sha256 digest written as lowercase hexadecimal: 32 bytes, 64 characters.
const SHA256_HEX_LEN: usize = 64;

/// How far below the registry root the corpus walk will descend.
///
/// The shipped registry is flat: one directory of TOML files. The cap is for
/// the pathological case rather than the real one, and specifically for a
/// directory symbolic link that points back at an ancestor. Following links is
/// what makes that a tree of unbounded depth; without the cap the walk descends
/// it until the operating system's own limit on chained links stops it, which
/// is thousands of directory reads later and reports a path nobody wrote.
const MAX_REGISTRY_DEPTH: usize = 16;

/// A registered dataset. Never executed, only pointed at.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CorpusEntry {
    /// The name this is known by on the command line, and the key `describe`
    /// looks up. Lower case, digits and hyphens, so it survives a shell, a URL
    /// and a file name unchanged.
    pub id: String,
    /// What its authors call it.
    pub name: String,
    /// What it is, in one or two sentences, for somebody who has never heard
    /// of it.
    pub description: String,
    /// How to cite it. Not decoration: several of these carry a citation
    /// obligation as their only condition of use.
    #[serde(default)]
    pub citation: Option<String>,
    /// Which tier of a tiered corpus this entry describes, where it has tiers.
    ///
    /// A result carries it so a reader can tell a run over 200 covers from one
    /// over 10,000 without decoding a name. Declared rather than derived from
    /// the id: `pentimento-core` happens to end in its tier and nothing
    /// guarantees the next corpus will, and a benchmark that guesses this
    /// would mislabel a number rather than decline to label it.
    #[serde(default)]
    pub tier: Option<String>,
    pub licence: Licence,
    pub obtain: Obtain,
    /// A digest or manifest reference, where one exists. Most published corpora
    /// have neither, which is itself worth recording rather than hiding.
    #[serde(default)]
    pub integrity: Option<Integrity>,
    pub properties: Properties,
    #[serde(default)]
    pub notes: Option<String>,
}

/// What is actually known about the terms, as opposed to what is assumed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LicenceStatus {
    /// Somebody read the source's own terms, on a stated date, and wrote down
    /// what they said.
    Verified,
    /// The terms could not be established. This is a legitimate answer and a
    /// far better one than a guess: it is the state every mislabelled mirror
    /// should have been in.
    Unverified,
    /// There is no grant. Not "we could not find one": the rights holder either
    /// never issued terms or issued terms that grant nothing.
    NoneGranted,
}

/// Whether stego images derived from this corpus may be published.
///
/// Separate from the licence because it is a separate question, and it is the
/// question the publish gate asks. A corpus can be perfectly usable and
/// completely unpublishable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Redistribution {
    Permitted,
    Forbidden,
    /// Nobody has established it either way. Treated as forbidden by any gate:
    /// an unknown is not a yes.
    Unknown,
}

impl Redistribution {
    /// What a publish gate acts on. Only an explicit `Permitted` is a yes.
    pub fn allows_publishing(self) -> bool {
        matches!(self, Redistribution::Permitted)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Licence {
    pub status: LicenceStatus,
    /// An SPDX identifier where one applies. Required when `status` is
    /// verified, refused otherwise: a licence name beside "unverified" is the
    /// exact ambiguity this field exists to prevent.
    #[serde(default)]
    pub spdx: Option<String>,
    /// A link to the licence TEXT, not to the dataset's landing page.
    #[serde(default)]
    pub url: Option<String>,
    /// ISO 8601 date the terms were read.
    #[serde(default)]
    pub verified_on: Option<String>,
    /// What was read, precisely enough that somebody else can read the same
    /// thing and disagree.
    #[serde(default)]
    pub source: Option<String>,
    pub redistribution: Redistribution,
    /// Why. Required in every case, including `permitted`, because "yes" with
    /// no reason is indistinguishable from "nobody checked".
    pub redistribution_reason: String,
    #[serde(default)]
    pub attribution_required: bool,
    #[serde(default)]
    pub share_alike: bool,
    /// The SPDX id names a VERSION the source did not state.
    ///
    /// REVEAL is the case this exists for. The DANS deposit's own sentence says
    /// "CC-BY-SA" with no version number, and the `4.0` in its entry comes from
    /// the paper and from this project's licence survey. Every CC BY-SA version
    /// carries the same obligations, so the version decides which licence text
    /// a republication must NAME rather than whether one is allowed, and
    /// downgrading the whole claim to `unverified` over a digit would overstate
    /// the doubt as badly as hiding it would understate it.
    ///
    /// What is not acceptable is a field asserting more than its own `source`
    /// supports with nothing but a paragraph to say so. A prose note is invisible
    /// to anything reading the field, and this corpus argues that licence claims
    /// should be checkable by machine rather than by careful reading.
    #[serde(default)]
    pub spdx_version_inferred: bool,
    /// Anything a reader needs in order to trust or distrust the fields above.
    #[serde(default)]
    pub note: Option<String>,
}

/// Where it lives and what it costs to get it. The registry declares a route;
/// it never carries the bytes, because carrying a third party's dataset is
/// redistribution whatever the directory is called.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Obtain {
    /// A DOI, which is the only identifier in this field that is meant to
    /// outlive its host.
    #[serde(default)]
    pub doi: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    /// What a human has to do, for the corpora that cannot be fetched by a
    /// script: accept competition rules, email an author, sign an agreement.
    #[serde(default)]
    pub instructions: Option<String>,
    /// Whether obtaining it requires agreeing to terms in person. A fetcher
    /// must refuse rather than automate around it.
    #[serde(default)]
    pub requires_acceptance: bool,
}

/// How a copy can be checked against the copy this entry describes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Integrity {
    /// Where the manifest of per-file digests lives, if one exists.
    #[serde(default)]
    pub manifest: Option<String>,
    /// Lower case hex, 64 characters, checked rather than pattern-matched.
    #[serde(default)]
    pub sha256: Option<String>,
    /// What that digest covers: the manifest, one archive, a file list. A
    /// digest with no stated subject pins nothing a reader can act on.
    #[serde(default)]
    pub sha256_covers: Option<String>,
    /// The digest `stegobench score` computes over an unpacked copy of this
    /// corpus, written as `sha256:` and 64 hexadecimal characters.
    ///
    /// A different thing from [`Integrity::sha256`], which names whatever its
    /// publisher chose to hash: an archive, a manifest, a file list. This one
    /// is defined by the harness. It is taken over every sample's id and the
    /// digest that sample's own record states, in corpus order, so it survives
    /// the corpus being extracted from a shard and moved, and two people
    /// holding the same corpus compute the same value.
    ///
    /// **It is what makes a `named` run possible.** A result is marked `named`
    /// only when the corpus on disk matches a digest an independent registry
    /// entry declared in advance. Without that, naming a corpus would be the
    /// person running the benchmark asserting what they are measuring, and the
    /// one field that is supposed to be set by the harness rather than by them
    /// would be set by them.
    ///
    /// Absent is the ordinary state for a third party corpus nobody has
    /// computed it for, and such a corpus scores perfectly well as `custom`.
    #[serde(default)]
    pub records_sha256: Option<String>,
}

/// Enough to plan a run against it without downloading it first.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Properties {
    /// Distinct source images.
    #[serde(default)]
    pub base_images: Option<u64>,
    /// Every file, covers and stego images together, where the corpus ships
    /// both.
    #[serde(default)]
    pub total_images: Option<u64>,
    #[serde(default)]
    pub size_mb: Option<u64>,
    #[serde(default)]
    pub formats: Vec<String>,
    /// Whether each stego image has an identified clean twin. An unpaired
    /// corpus cannot answer the question this benchmark asks.
    #[serde(default)]
    pub paired: Option<bool>,
    /// What is known about the size when no number above is. Required in that
    /// case, so "we have not measured it" is written down rather than left as
    /// an empty field a reader mistakes for a small corpus. A planner sees no
    /// number and refuses; an invented one would let it proceed on a figure
    /// nobody checked.
    #[serde(default)]
    pub size_note: Option<String>,
}

impl CorpusEntry {
    /// Rules a TOML parser cannot express.
    ///
    /// Same shape as [`crate::registry::Entry::validate`]: every problem, not
    /// the first one, so a contributor fixes them in one pass rather than in
    /// one round trip each.
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut bad = Vec::new();

        if self.id.trim().is_empty() {
            bad.push("id is empty; it is the name `describe` looks up".into());
        } else if !self
            .id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        {
            bad.push(format!(
                "id {:?} must be lower case letters, digits and hyphens only, \
                 so it survives a shell, a URL and a file name unchanged",
                self.id
            ));
        }
        if self.name.trim().is_empty() {
            bad.push("name is empty; give the name its authors use".into());
        }
        if self.description.trim().is_empty() {
            bad.push(
                "description is empty; one or two sentences for somebody who \
                 has never heard of this corpus"
                    .into(),
            );
        }

        self.validate_licence(&mut bad);
        self.validate_obtain(&mut bad);
        self.validate_integrity(&mut bad);
        self.validate_properties(&mut bad);

        if bad.is_empty() {
            Ok(())
        } else {
            Err(bad)
        }
    }

    fn validate_licence(&self, bad: &mut Vec<String>) {
        let l = &self.licence;

        // The flag is a statement about the spdx field, so it cannot outlive
        // it. Set with no identifier to qualify, it reads as doubt about
        // nothing; set on an unverified entry it is redundant, because such an
        // entry may not name a licence at all.
        if l.spdx_version_inferred {
            if l.status != LicenceStatus::Verified {
                bad.push(
                    "licence.spdx_version_inferred is set but the status is not \
                     verified. An unverified entry names no licence, so there is no \
                     version to be inferred"
                        .into(),
                );
            }
            if l.spdx.as_ref().is_none_or(|s| s.trim().is_empty()) {
                bad.push(
                    "licence.spdx_version_inferred is set but no spdx identifier is \
                     given; the flag qualifies that field and means nothing without \
                     it"
                    .into(),
                );
            }
            if l.note.as_ref().is_none_or(|s| s.trim().is_empty()) {
                bad.push(
                    "licence.spdx_version_inferred is set but licence.note does not \
                     say where the version came from. The flag tells a machine the \
                     source did not state it; the note is what tells a person what \
                     to re-check"
                        .into(),
                );
            }
        }

        match l.status {
            LicenceStatus::Verified => {
                if l.spdx.as_ref().is_none_or(|s| s.trim().is_empty()) {
                    bad.push(
                        "licence.status is verified but no spdx identifier is \
                         given; name the licence that was read"
                            .into(),
                    );
                }
                if l.url.as_ref().is_none_or(|s| s.trim().is_empty()) {
                    bad.push(
                        "licence.status is verified but no url is given; link \
                         the licence text itself, not the dataset's landing \
                         page"
                            .into(),
                    );
                }
                match &l.verified_on {
                    Some(d) if is_iso_date(d) => {}
                    Some(d) => bad.push(format!(
                        "licence.verified_on {d:?} is not an ISO 8601 date \
                         (YYYY-MM-DD). Terms change, so a claim about them is \
                         only as good as its date"
                    )),
                    None => bad.push(
                        "licence.status is verified but no verified_on date is \
                         given. Terms change; an undated reading cannot be \
                         re-checked"
                            .into(),
                    ),
                }
                if l.source.as_ref().is_none_or(|s| s.trim().is_empty()) {
                    bad.push(
                        "licence.status is verified but licence.source does not \
                         say what was read. A mirror's metadata field is not a \
                         source; the rights holder's own terms are"
                            .into(),
                    );
                }
            }
            LicenceStatus::Unverified | LicenceStatus::NoneGranted => {
                if l.spdx.is_some() {
                    bad.push(format!(
                        "licence.status is {} but an spdx identifier is given. \
                         A licence name beside an unverified status is exactly \
                         the mislabelling this field exists to prevent; either \
                         verify it and record the source, or drop the name",
                        status_word(l.status)
                    ));
                }
                if l.note.as_ref().is_none_or(|s| s.trim().is_empty()) {
                    bad.push(format!(
                        "licence.status is {} but licence.note does not say \
                         why. Record what was looked for and where, so the \
                         next person starts from what has already been tried",
                        status_word(l.status)
                    ));
                }
            }
        }

        if l.redistribution_reason.trim().is_empty() {
            bad.push(
                "licence.redistribution_reason is empty. A redistribution \
                 answer with no reason cannot be told apart from one nobody \
                 checked"
                    .into(),
            );
        }
        if l.redistribution == Redistribution::Permitted && l.status != LicenceStatus::Verified {
            bad.push(format!(
                "licence.redistribution is permitted while the licence itself \
                 is {}. Republication cannot rest on terms nobody has read",
                status_word(l.status)
            ));
        }
        if l.share_alike && l.redistribution != Redistribution::Permitted {
            bad.push(
                "licence.share_alike is set but redistribution is not \
                 permitted, so the obligation describes something that may not \
                 happen. One of the two is wrong"
                    .into(),
            );
        }
    }

    fn validate_obtain(&self, bad: &mut Vec<String>) {
        let o = &self.obtain;
        let has = |v: &Option<String>| v.as_ref().is_some_and(|s| !s.trim().is_empty());
        if !has(&o.doi) && !has(&o.url) && !has(&o.instructions) {
            bad.push(
                "obtain declares no doi, url or instructions, so this entry \
                 names a corpus nobody can get. The registry points at data; \
                 it never carries it"
                    .into(),
            );
        }
        if let Some(doi) = &o.doi {
            if !doi.starts_with("10.") || !doi.contains('/') {
                bad.push(format!(
                    "obtain.doi {doi:?} is not a DOI. A DOI starts \"10.\" and \
                     carries a slash; write the identifier itself rather than a \
                     resolver link"
                ));
            }
        }
        if let Some(url) = &o.url {
            if !url.starts_with("https://") && !url.starts_with("http://") {
                bad.push(format!("obtain.url {url:?} is not an http(s) URL"));
            }
        }
    }

    fn validate_integrity(&self, bad: &mut Vec<String>) {
        let Some(i) = &self.integrity else {
            return;
        };
        if i.manifest.is_none() && i.sha256.is_none() && i.records_sha256.is_none() {
            bad.push(
                "the integrity block is present but empty; remove it rather \
                 than implying a corpus can be checked when it cannot"
                    .into(),
            );
        }
        if let Some(d) = &i.sha256 {
            // The same standard the registry holds an image digest to. A
            // placeholder or a truncated paste satisfies a shape test and
            // identifies nothing.
            if d.len() != SHA256_HEX_LEN || !d.bytes().all(|b| b.is_ascii_hexdigit()) {
                bad.push(format!(
                    "integrity.sha256 {d:?} is {} character(s); a sha256 digest \
                     is {SHA256_HEX_LEN} hexadecimal characters. A short or \
                     placeholder digest reads as pinned and pins nothing",
                    d.len()
                ));
            } else if d.bytes().any(|b| b.is_ascii_uppercase()) {
                bad.push(format!(
                    "integrity.sha256 {d:?} has upper case hex; digests are \
                     compared as text here, so case has to be settled"
                ));
            }
            if i.sha256_covers.as_ref().is_none_or(|s| s.trim().is_empty()) {
                bad.push(
                    "integrity.sha256 is given but sha256_covers does not say \
                     what it covers. A digest with no stated subject cannot be \
                     re-computed by anyone"
                        .into(),
                );
            }
        }

        // Held to a stricter shape than `sha256`, because this one is compared
        // as text against a value the harness computes and a mismatched prefix
        // would read as a corpus that changed rather than as a typo.
        if let Some(d) = &i.records_sha256 {
            let hex = d.strip_prefix("sha256:");
            let ok = hex.is_some_and(|h| {
                h.len() == SHA256_HEX_LEN
                    && h.bytes()
                        .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            });
            if !ok {
                bad.push(format!(
                    "integrity.records_sha256 {d:?} is not a corpus digest. It is \
                     written as \"sha256:\" followed by {SHA256_HEX_LEN} lower case \
                     hexadecimal characters, exactly as `stegobench score` prints \
                     it, because the two are compared as text"
                ));
            }
        }
    }

    fn validate_properties(&self, bad: &mut Vec<String>) {
        let p = &self.properties;
        if p.base_images.is_none()
            && p.total_images.is_none()
            && p.size_mb.is_none()
            && p.size_note.as_ref().is_none_or(|s| s.trim().is_empty())
        {
            bad.push(
                "properties declares no base_images, total_images or size_mb, \
                 so nothing can plan a run or check disk against this corpus. \
                 If the size genuinely is not established, say so in \
                 properties.size_note rather than leaving the fields empty"
                    .into(),
            );
        }
        // Zero is not a smaller number here, it is a different claim. A corpus
        // of no images would let a run report having scored one.
        for (field, value) in [
            ("base_images", p.base_images),
            ("total_images", p.total_images),
            ("size_mb", p.size_mb),
        ] {
            if value == Some(0) {
                bad.push(format!(
                    "properties.{field} is 0. Leave it out if it is unknown; \
                     zero states that the corpus is empty, and a run over it \
                     would report success having scored nothing"
                ));
            }
        }
        if let (Some(base), Some(total)) = (p.base_images, p.total_images) {
            if total < base {
                bad.push(format!(
                    "properties.total_images ({total}) is fewer than \
                     base_images ({base}); total counts every file, covers \
                     included"
                ));
            }
        }
    }

    /// A one-line summary for `stegobench list corpora`, in the same shape as
    /// [`crate::registry::Entry::summary`].
    pub fn summary(&self) -> String {
        // A trailing `?` on the version, because the listing is where people
        // actually look and a qualification only `describe` shows is one most
        // readers never see. It marks the VERSION as inferred, not the licence
        // as doubtful: the obligations are the same across versions.
        let licence = match self.licence.status {
            LicenceStatus::Verified => {
                let named = self
                    .licence
                    .spdx
                    .clone()
                    .unwrap_or_else(|| "verified".into());
                if self.licence.spdx_version_inferred {
                    format!("{named}?")
                } else {
                    named
                }
            }
            LicenceStatus::Unverified => "UNVERIFIED".into(),
            LicenceStatus::NoneGranted => "NO LICENCE".into(),
        };
        let republish = match self.licence.redistribution {
            Redistribution::Permitted => "republish: yes",
            Redistribution::Forbidden => "republish: no",
            Redistribution::Unknown => "republish: unknown",
        };
        let size = match (self.properties.base_images, self.properties.total_images) {
            (Some(b), _) => format!("{b} covers"),
            (None, Some(t)) => format!("{t} files"),
            (None, None) => match self.properties.size_mb {
                Some(mb) => format!("{mb} MB"),
                None => "size unestablished".into(),
            },
        };
        format!("{:<16} {:<16} {:<18} {republish}", self.id, licence, size)
    }
}

fn status_word(s: LicenceStatus) -> &'static str {
    match s {
        LicenceStatus::Verified => "verified",
        LicenceStatus::Unverified => "unverified",
        LicenceStatus::NoneGranted => "none-granted",
    }
}

/// `YYYY-MM-DD`, checked as a date rather than as a shape: a month of 13 or a
/// day of 00 is a typo that would otherwise sit in the record for years.
fn is_iso_date(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return false;
    }
    let part = |a: usize, b: usize| s[a..b].parse::<u32>().ok();
    let (Some(y), Some(m), Some(d)) = (part(0, 4), part(5, 7), part(8, 10)) else {
        return false;
    };
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let days = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=days).contains(&d)
}

/// Loads every `.toml` under `dir` as a corpus entry.
///
/// An invalid entry fails the load, exactly as an invalid tool does: a registry
/// that quietly drops a corpus reports a smaller world than it has, and the
/// person who added the file is the last to find out. The cost is that one
/// malformed file makes the whole registry unusable, which is the behaviour a
/// reader already predicts from the tool half, and a loud failure on the file
/// somebody just edited is easier to act on than a silent omission somebody
/// notices weeks later.
pub(crate) fn load_dir(dir: &Path) -> Result<BTreeMap<String, CorpusEntry>, RegistryError> {
    let mut out: BTreeMap<String, CorpusEntry> = BTreeMap::new();
    // A registry with no corpora directory is an ordinary state and loads as
    // an empty set. A corpora directory that is there and cannot be read is
    // not: `is_dir()` reported both as "no corpora", so a permission the user
    // lacked produced a registry that listed nothing and said nothing.
    match std::fs::metadata(dir) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(e) => {
            return Err(RegistryError::Read {
                path: dir.display().to_string(),
                source: e,
            })
        }
        Ok(meta) if !meta.is_dir() => return Ok(out),
        Ok(_) => {}
    }
    let mut stack = vec![(dir.to_path_buf(), 0usize)];
    while let Some((d, depth)) = stack.pop() {
        let read = std::fs::read_dir(&d).map_err(|e| RegistryError::Read {
            path: d.display().to_string(),
            source: e,
        })?;
        for item in read {
            // Not `flatten()`. An entry the filesystem cannot describe is a
            // corpus this walk did not see, and dropping it here is the silent
            // omission the paragraph above says the loud failure is worth
            // paying for.
            let item = item.map_err(|e| RegistryError::Read {
                path: d.display().to_string(),
                source: e,
            })?;
            let p = item.path();
            // Not `p.is_dir()`. That answers false for a directory it could not
            // stat, so a registry subdirectory behind a permission the user
            // lacks, or at the far end of a chain of links, is quietly walked
            // past and the corpora under it never appear at all.
            let meta = std::fs::metadata(&p).map_err(|e| RegistryError::Read {
                path: p.display().to_string(),
                source: e,
            })?;
            if meta.is_dir() {
                if depth + 1 > MAX_REGISTRY_DEPTH {
                    return Err(RegistryError::Read {
                        path: p.display().to_string(),
                        source: std::io::Error::other(format!(
                            "more than {MAX_REGISTRY_DEPTH} directories below the \
                             registry root. A registry is one directory of TOML \
                             files per kind, so this is either the wrong \
                             directory or a link that points back at one already \
                             visited"
                        )),
                    });
                }
                stack.push((p, depth + 1));
            } else if p.extension().is_some_and(|e| e == "toml") {
                let text = std::fs::read_to_string(&p).map_err(|e| RegistryError::Read {
                    path: p.display().to_string(),
                    source: e,
                })?;
                let entry: CorpusEntry =
                    toml::from_str(&text).map_err(|e| RegistryError::Parse {
                        path: p.display().to_string(),
                        source: e,
                    })?;
                entry
                    .validate()
                    .map_err(|problems| RegistryError::Invalid {
                        name: p.display().to_string(),
                        problems,
                    })?;
                // Two files claiming one id is a silent coin toss otherwise:
                // the directory walk visits them in filesystem order, so which
                // one wins would vary between machines and `describe` would be
                // reproducible only by luck.
                if let Some(first) = out.insert(entry.id.clone(), entry) {
                    return Err(RegistryError::Invalid {
                        name: p.display().to_string(),
                        problems: vec![format!(
                            "a second corpus claims the id {:?}; ids are how \
                             `describe` finds an entry, so two files cannot \
                             share one",
                            first.id
                        )],
                    });
                }
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIGEST: &str = "59710f7b5fbaeb7c3b1d4333e64654c1721a3ddb60b489d8e54d5d0e8b269bfb";

    /// A valid entry plus whatever the caller splices in.
    ///
    /// `extra` goes BEFORE the first table, because TOML assigns a bare key to
    /// the table above it and appending would silently make every top-level key
    /// part of `[properties]`.
    fn toml_for(extra: &str) -> String {
        format!(
            r#"
id = "example"
name = "Example Corpus"
description = "A corpus that exists only in this test."
{extra}

[licence]
status = "verified"
spdx = "CC-BY-4.0"
url = "https://creativecommons.org/licenses/by/4.0/legalcode"
verified_on = "2026-09-21"
source = "the deposit's own terms page"
redistribution = "permitted"
redistribution_reason = "CC BY permits derivatives and redistribution."
attribution_required = true

[obtain]
url = "https://example.org/corpus"

[properties]
base_images = 100
"#
        )
    }

    fn parse(extra: &str) -> CorpusEntry {
        toml::from_str(&toml_for(extra)).expect("parses")
    }

    fn problems(e: &CorpusEntry) -> Vec<String> {
        match e.validate() {
            Ok(()) => panic!("accepted an entry that should have been refused"),
            Err(p) => p,
        }
    }

    fn refused_for(e: &CorpusEntry, needle: &str) {
        let p = problems(e);
        assert!(
            p.iter().any(|x| x.contains(needle)),
            "refused for the wrong reason: {p:?}"
        );
    }

    #[test]
    fn a_fully_stated_corpus_is_valid() {
        assert_eq!(parse("").validate(), Ok(()));
    }

    #[test]
    fn an_id_that_would_not_survive_a_shell_is_refused() {
        let mut e = parse("");
        e.id = "Example Corpus".into();
        refused_for(&e, "lower case letters");
        e.id = String::new();
        refused_for(&e, "id is empty");
    }

    #[test]
    fn a_corpus_with_no_description_is_refused() {
        let mut e = parse("");
        e.description = "   ".into();
        refused_for(&e, "description is empty");
    }

    // ---- the licence rules, which are the point of the type ----

    #[test]
    fn a_verified_licence_must_name_what_was_read_and_when() {
        let mut e = parse("");
        e.licence.source = None;
        refused_for(&e, "what was read");

        let mut e = parse("");
        e.licence.verified_on = None;
        refused_for(&e, "no verified_on date");

        let mut e = parse("");
        e.licence.url = None;
        refused_for(&e, "no url is given");

        let mut e = parse("");
        e.licence.spdx = None;
        refused_for(&e, "no spdx identifier");
    }

    #[test]
    fn a_date_that_is_not_a_date_is_refused() {
        for bad in [
            "2026-13-01",
            "2026-00-10",
            "2026-02-30",
            "26-09-21",
            "2026-9-21",
            "yesterday",
            "2026-09-32",
        ] {
            let mut e = parse("");
            e.licence.verified_on = Some(bad.into());
            refused_for(&e, "ISO 8601 date");
        }
        // The leap year the rule has to get right, and the year that is not one.
        let mut e = parse("");
        e.licence.verified_on = Some("2024-02-29".into());
        assert_eq!(e.validate(), Ok(()));
        e.licence.verified_on = Some("2026-02-29".into());
        refused_for(&e, "ISO 8601 date");
    }

    /// The failure `cover-source-licensing.md` catalogues: a licence name
    /// written into a metadata field by somebody who did not read the terms.
    /// An unverified entry carrying an SPDX identifier is that, in our own
    /// schema, and it is refused rather than displayed with a caveat nobody
    /// reads.
    #[test]
    fn an_unverified_licence_may_not_carry_a_licence_name() {
        let mut e = parse("");
        e.licence.status = LicenceStatus::Unverified;
        e.licence.note = Some("the host is gone and no archive holds it".into());
        e.licence.redistribution = Redistribution::Unknown;
        e.licence.share_alike = false;
        refused_for(&e, "exactly the mislabelling");

        e.licence.spdx = None;
        e.licence.url = None;
        e.licence.verified_on = None;
        e.licence.source = None;
        assert_eq!(e.validate(), Ok(()));
    }

    #[test]
    fn an_unverified_or_ungranted_licence_must_say_why() {
        for status in [LicenceStatus::Unverified, LicenceStatus::NoneGranted] {
            let mut e = parse("");
            e.licence.status = status;
            e.licence.spdx = None;
            e.licence.redistribution = Redistribution::Forbidden;
            e.licence.share_alike = false;
            refused_for(&e, "does not say");
        }
    }

    #[test]
    fn republication_cannot_rest_on_terms_nobody_read() {
        let mut e = parse("");
        e.licence.status = LicenceStatus::Unverified;
        e.licence.spdx = None;
        e.licence.note = Some("nobody has read the terms".into());
        // redistribution stays "permitted" from the fixture.
        refused_for(&e, "Republication cannot rest");
    }

    #[test]
    fn a_redistribution_answer_without_a_reason_is_refused() {
        let mut e = parse("");
        e.licence.redistribution_reason = "  ".into();
        refused_for(&e, "cannot be told apart");
    }

    #[test]
    fn share_alike_on_a_corpus_that_may_not_be_republished_is_refused() {
        let mut e = parse("");
        e.licence.redistribution = Redistribution::Forbidden;
        e.licence.share_alike = true;
        refused_for(&e, "may not happen");
    }

    /// The distinction the whole licensing document turns on: two corpora that
    /// are equally usable and equally unpublishable, for different reasons, and
    /// a schema that can say so.
    #[test]
    fn used_but_not_republished_is_expressible_for_both_of_its_reasons() {
        let mut nd = parse("");
        nd.licence.spdx = Some("CC-BY-NC-ND-4.0".into());
        nd.licence.redistribution = Redistribution::Forbidden;
        nd.licence.redistribution_reason = "No-derivatives; a stego image is a derivative.".into();
        nd.licence.share_alike = false;
        assert_eq!(nd.validate(), Ok(()));
        assert!(!nd.licence.redistribution.allows_publishing());

        let mut none = parse("");
        none.licence.status = LicenceStatus::NoneGranted;
        none.licence.spdx = None;
        none.licence.url = None;
        none.licence.verified_on = None;
        none.licence.source = None;
        none.licence.note = Some("The organisers claimed rights; no grant was issued.".into());
        none.licence.redistribution = Redistribution::Forbidden;
        none.licence.redistribution_reason = "No grant exists to redistribute under.".into();
        none.licence.share_alike = false;
        assert_eq!(none.validate(), Ok(()));
        assert!(!none.licence.redistribution.allows_publishing());
        // And an unknown is not a yes.
        assert!(!Redistribution::Unknown.allows_publishing());
    }

    // ---- obtaining it ----

    #[test]
    fn a_corpus_nobody_can_get_is_refused() {
        let mut e = parse("");
        e.obtain = Obtain {
            doi: None,
            url: None,
            instructions: None,
            requires_acceptance: false,
        };
        refused_for(&e, "names a corpus nobody can get");
    }

    #[test]
    fn a_resolver_link_in_the_doi_field_is_refused() {
        let mut e = parse("");
        e.obtain.doi = Some("https://doi.org/10.17026/PT/DITX0A".into());
        refused_for(&e, "is not a DOI");
        e.obtain.doi = Some("10.17026/PT/DITX0A".into());
        assert_eq!(e.validate(), Ok(()));
    }

    #[test]
    fn a_url_that_is_not_a_url_is_refused() {
        let mut e = parse("");
        e.obtain.url = Some("example.org/corpus".into());
        refused_for(&e, "not an http(s) URL");
    }

    // ---- integrity ----

    #[test]
    fn a_digest_that_is_not_a_digest_is_refused() {
        for bad in [
            "REAL_DIGEST_AFTER_BUILD",
            "abc",
            "",
            &format!("{DIGEST}beef"),
            &format!("{}z", &DIGEST[..63]),
        ] {
            let mut e = parse("");
            e.integrity = Some(Integrity {
                manifest: None,
                sha256: Some(bad.to_string()),
                sha256_covers: Some("the archive".into()),
                records_sha256: None,
            });
            refused_for(&e, "hexadecimal characters");
        }
    }

    #[test]
    fn an_upper_case_digest_is_refused_because_it_is_compared_as_text() {
        let mut e = parse("");
        e.integrity = Some(Integrity {
            manifest: None,
            sha256: Some(DIGEST.to_uppercase()),
            sha256_covers: Some("the archive".into()),
            records_sha256: None,
        });
        refused_for(&e, "upper case hex");
    }

    #[test]
    fn a_digest_must_say_what_it_covers() {
        let mut e = parse("");
        e.integrity = Some(Integrity {
            manifest: None,
            sha256: Some(DIGEST.into()),
            sha256_covers: None,
            records_sha256: None,
        });
        refused_for(&e, "does not say");
        e.integrity = Some(Integrity {
            manifest: None,
            sha256: Some(DIGEST.into()),
            sha256_covers: Some("the published archive".into()),
            records_sha256: None,
        });
        assert_eq!(e.validate(), Ok(()));
    }

    #[test]
    fn an_empty_integrity_block_is_refused_rather_than_implying_a_check() {
        let mut e = parse("");
        e.integrity = Some(Integrity {
            manifest: None,
            sha256: None,
            sha256_covers: None,
            records_sha256: None,
        });
        refused_for(&e, "present but empty");
    }

    #[test]
    fn a_corpus_with_no_digest_at_all_is_still_valid_because_most_have_none() {
        let e = parse("");
        assert!(e.integrity.is_none());
        assert_eq!(e.validate(), Ok(()));
    }

    // ---- properties ----

    #[test]
    fn a_corpus_of_unstated_size_cannot_be_planned_against() {
        let mut e = parse("");
        e.properties = Properties::default();
        refused_for(&e, "plan a run");
    }

    /// An unmeasured corpus can still be registered, as long as the silence is
    /// written down. The planner still sees no number and still refuses; what
    /// it must never see is a figure somebody guessed to get past this check.
    #[test]
    fn an_unmeasured_size_is_allowed_only_when_it_says_so_in_words() {
        let mut e = parse("");
        e.properties = Properties::default();
        e.properties.size_note = Some("nobody here has downloaded it".into());
        assert_eq!(e.validate(), Ok(()));
        assert!(e.summary().contains("size unestablished"));
        e.properties.size_note = Some("   ".into());
        refused_for(&e, "plan a run");
    }

    /// Zero images is not a small corpus, it is an empty claim, and a run over
    /// it would report success having scored nothing.
    #[test]
    fn a_count_of_zero_is_refused_rather_than_read_as_a_small_corpus() {
        for field in ["base_images", "total_images", "size_mb"] {
            let mut e = parse("");
            e.properties = Properties::default();
            match field {
                "base_images" => e.properties.base_images = Some(0),
                "total_images" => e.properties.total_images = Some(0),
                _ => e.properties.size_mb = Some(0),
            }
            refused_for(&e, "zero states that the corpus is empty");
        }
    }

    #[test]
    fn a_total_smaller_than_the_cover_count_is_refused() {
        let mut e = parse("");
        e.properties.base_images = Some(100);
        e.properties.total_images = Some(50);
        refused_for(&e, "fewer than");
    }

    #[test]
    fn the_summary_names_the_licence_and_whether_it_can_be_republished() {
        let e = parse("");
        let s = e.summary();
        assert!(s.contains("example") && s.contains("CC-BY-4.0") && s.contains("republish: yes"));

        let mut u = parse("");
        u.licence.status = LicenceStatus::Unverified;
        u.licence.spdx = None;
        u.licence.note = Some("not established".into());
        u.licence.redistribution = Redistribution::Unknown;
        u.licence.share_alike = false;
        let s = u.summary();
        assert!(
            s.contains("UNVERIFIED") && s.contains("republish: unknown"),
            "got: {s}"
        );
    }

    // ---- loading ----

    #[test]
    fn a_missing_corpora_directory_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = load_dir(&dir.path().join("nothing-here")).expect("absent is empty");
        assert!(loaded.is_empty());
    }

    #[test]
    fn one_malformed_file_fails_the_whole_load_rather_than_being_skipped() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("good.toml"), toml_for("")).unwrap();
        std::fs::write(dir.path().join("bad.toml"), "id = \"broken\"\n").unwrap();
        let err = load_dir(dir.path()).expect_err("a malformed entry fails the load");
        assert!(matches!(err, RegistryError::Parse { .. }), "got: {err}");
    }

    #[test]
    fn an_invalid_entry_names_the_file_and_every_problem() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("bad.toml"),
            toml_for("").replace("base_images = 100", "base_images = 0"),
        )
        .unwrap();
        let err = load_dir(dir.path()).expect_err("zero images is refused");
        let text = err.to_string();
        assert!(
            text.contains("bad.toml") && text.contains("corpus is empty"),
            "got: {text}"
        );
    }

    #[test]
    fn two_files_claiming_one_id_are_refused_rather_than_racing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.toml"), toml_for("")).unwrap();
        std::fs::write(dir.path().join("b.toml"), toml_for("")).unwrap();
        let err = load_dir(dir.path()).expect_err("a duplicate id is refused");
        assert!(err.to_string().contains("second corpus claims the id"));
    }

    #[test]
    fn a_nested_directory_is_walked() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub/a.toml"), toml_for("")).unwrap();
        assert_eq!(load_dir(dir.path()).unwrap().len(), 1);
    }

    /// A name the walk cannot resolve is not evidence that no corpus is behind
    /// it. `is_dir` answers false for one, which sent it down the branch for
    /// ordinary files, where anything not ending in `.toml` is skipped without
    /// a word; a broken link named `pentimento-core.toml` would have taken that
    /// path and left the registry one corpus short of what it holds.
    #[cfg(unix)]
    #[test]
    fn a_name_the_walk_cannot_resolve_is_reported_rather_than_skipped() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.toml"), toml_for("")).unwrap();
        std::os::unix::fs::symlink(
            dir.path().join("was-here-once"),
            dir.path().join("dangling"),
        )
        .unwrap();
        let err = load_dir(dir.path()).expect_err("an unresolvable name is refused");
        assert!(err.to_string().contains("dangling"), "got: {err}");
    }

    /// A link pointing back at an ancestor is a directory tree with no bottom,
    /// and the walk follows links. Without the cap it descends until the
    /// operating system refuses to resolve any more of them, and because
    /// `is_dir` reports an unreadable path as "not a directory" the refusal
    /// arrived as an empty registry rather than as an error: the load reported
    /// success having found nothing.
    #[cfg(unix)]
    #[test]
    fn a_directory_link_pointing_at_its_own_parent_is_refused_rather_than_walked() {
        // No entry file anywhere: the duplicate-id check would otherwise stop
        // the walk on the second pass and hide that the walk itself never ends.
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::os::unix::fs::symlink(dir.path(), dir.path().join("sub/loop")).unwrap();
        let err = load_dir(dir.path()).expect_err("a link loop is refused");
        assert!(
            err.to_string()
                .contains("directories below the registry root"),
            "got: {err}"
        );
    }

    // ── The version-inferred flag ──────────────────────────────────────────
    //
    // REVEAL is why this exists. The DANS deposit says "CC-BY-SA" with no
    // version, so the `4.0` is inferred from the paper. Recording that only in
    // a prose note leaves the spdx field asserting more than its own `source`
    // supports, which is the mislabelled-mirror shape this schema exists to
    // refuse, and a note is invisible to anything reading the field.

    fn licensed(block: &str) -> CorpusEntry {
        let base = toml_for("");
        let start = base.find("[licence]").expect("has a licence block");
        let end = base.find("[obtain]").expect("has an obtain block");
        let replaced = format!("{}{}\n\n{}", &base[..start], block, &base[end..]);
        toml::from_str(&replaced).expect("parses")
    }

    const VERIFIED_INFERRED: &str = r#"[licence]
status = "verified"
spdx = "CC-BY-SA-4.0"
spdx_version_inferred = true
url = "https://creativecommons.org/licenses/by-sa/4.0/legalcode"
verified_on = "2026-09-21"
source = "the deposit's own terms, which name no version"
redistribution = "permitted"
redistribution_reason = "Share-alike permits redistribution of derivatives."
note = "The deposit names CC-BY-SA with no version; 4.0 comes from the paper."
"#;

    #[test]
    fn an_inferred_version_is_accepted_when_it_is_declared_and_explained() {
        licensed(VERIFIED_INFERRED).validate().expect("valid");
    }

    #[test]
    fn an_inferred_version_with_no_note_is_refused() {
        // The flag tells a machine the source did not state the version. The
        // note is what tells a person what to go and re-read.
        let without = VERIFIED_INFERRED
            .lines()
            .filter(|l| !l.starts_with("note = "))
            .collect::<Vec<_>>()
            .join("\n");
        let found = problems(&licensed(&without));
        assert!(
            found
                .iter()
                .any(|m| m.contains("does not say where the version")),
            "got {found:?}"
        );
    }

    #[test]
    fn the_flag_cannot_outlive_the_field_it_qualifies() {
        // Set with no spdx id, it expresses doubt about nothing.
        let block = r#"[licence]
status = "verified"
spdx_version_inferred = true
url = "https://creativecommons.org/licenses/by/4.0/legalcode"
verified_on = "2026-09-21"
source = "somewhere"
redistribution = "permitted"
redistribution_reason = "because"
note = "explained"
"#;
        let found = problems(&licensed(block));
        assert!(
            found.iter().any(|m| m.contains("means nothing without it")),
            "got {found:?}"
        );
    }

    #[test]
    fn the_flag_on_an_unverified_entry_is_refused() {
        // An unverified entry may not name a licence at all, so there is no
        // version for the flag to be about.
        let block = r#"[licence]
status = "unverified"
spdx_version_inferred = true
redistribution = "unknown"
redistribution_reason = "nobody has read the terms"
note = "the terms page is gone"
"#;
        let found = problems(&licensed(block));
        assert!(
            found
                .iter()
                .any(|m| m.contains("there is no version to be inferred")),
            "got {found:?}"
        );
    }

    #[test]
    fn the_listing_marks_an_inferred_version_where_people_look() {
        // `describe` showing it is not enough: most readers only ever see the
        // one-line listing.
        assert!(licensed(VERIFIED_INFERRED)
            .summary()
            .contains("CC-BY-SA-4.0?"));
    }

    #[test]
    fn a_stated_version_is_not_marked() {
        assert!(!parse("").summary().contains('?'));
    }
}
