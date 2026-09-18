// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Daniel Iwugo
//! The registry: tools as declarative data rather than as code.
//!
//! WHY DATA AND NOT A TRAIT IMPL
//! -----------------------------
//! lm-eval-harness did not win LLM evaluation by being the best written. It
//! won because adding a benchmark is a config file, so strangers contributed
//! hundreds of them without reading the codebase. Every steganalysis benchmark
//! to date makes you write code against its internals, and that is the gap.
//!
//! So a detector is a TOML file. Ours is a TOML file exactly like a stranger's,
//! and nothing in the harness special-cases a name.
//!
//! TWO KINDS, BECAUSE NOT EVERY TOOL HAS AN IMAGE
//! ----------------------------------------------
//! Most tools are containers, pinned by digest. Some are a binary already on
//! the machine: Stegcore is one, and its published image could not be resolved
//! without a token scope nobody should need in order to run a benchmark.
//! Requiring an image would have made "contribute a detector" mean "publish a
//! container first", which is a barrier at exactly the wrong end.
//!
//! Both kinds are pinned. A container is pinned by its digest and a binary by
//! the SHA256 of the executable actually invoked, which is a stronger claim
//! than a tag: a tag can move under you and a hash cannot.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// A registered tool.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub name: String,
    pub kind: Kind,
    #[serde(default)]
    pub upstream: Option<String>,
    pub licence: String,
    /// Who maintains the tool. `Us` marks our own work, which the leaderboard
    /// uses to keep our entries out of the ranking: a benchmark run by a
    /// participant is not trusted, and the answer to that is mechanism rather
    /// than a promise.
    #[serde(default)]
    pub maintainer: Maintainer,
    /// How to run it. Exactly one of these must be present.
    #[serde(default)]
    pub image: Option<Image>,
    #[serde(default)]
    pub binary: Option<Binary>,
    #[serde(default)]
    pub emits: Emits,
    #[serde(default)]
    pub accepts: Accepts,
    #[serde(default)]
    pub cost: Cost,
    /// Environment variables the tool needs that hold secrets. The registry
    /// records the NAME only. A value never appears in this file, in a log, in
    /// a manifest or in a result: see `secret_names`.
    #[serde(default)]
    pub secrets: Vec<String>,
    #[serde(default)]
    pub selftest: Option<Selftest>,
    #[serde(default)]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Detector,
    Embedder,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Maintainer {
    /// Somebody else's tool.
    #[default]
    ThirdParty,
    /// Ours. Never ranked against the field.
    Us,
    /// The subject of an evaluation, which is a different thing again: it is
    /// here so its own numbers can be reproduced by the people who wrote it.
    Subject,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Image {
    /// Full reference including the digest, for example
    /// `ghcr.io/x/y@sha256:...`. A tag is refused by [`Entry::validate`].
    pub reference: String,
    #[serde(default)]
    pub needs_network: bool,
    /// Uncompressed size, measured rather than guessed. This is what decides
    /// whether a tool is bundled, and what the pre-flight checks disk against.
    #[serde(default)]
    pub size_mb: Option<u64>,
    /// Whether the tool ships inside the default toolkit image.
    ///
    /// Measured 2026-09-18, the whole set is 22.8 GB, and most of that is two
    /// tools: Aletheia is 8.3 GB and its rich-model variant 9.1 GB, because
    /// they carry a scientific Python stack and Octave. Bundling everything
    /// would mean a first-time user waits for nine gigabytes of Octave in
    /// order to run a Ruby script that reads a PNG header.
    ///
    /// So the default image carries the small tools, which are the ones a CTF
    /// player or a first run actually reaches for, and the large ones are
    /// fetched on demand with the pre-flight in [`Image::pull_preflight`].
    #[serde(default)]
    pub bundled: bool,
}

/// Why a pull cannot proceed.
#[derive(Debug, Clone, PartialEq)]
pub enum PullRefusal {
    /// The image is not present and nothing can fetch it.
    NoNetwork { name: String },
    /// There is not enough room, with the numbers so the user can act.
    NotEnoughDisk { name: String, needs_mb: u64, free_mb: u64 },
    /// Size is unknown, so the disk check cannot be made honestly.
    UnknownSize { name: String },
}

impl std::fmt::Display for PullRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PullRefusal::NoNetwork { name } => write!(
                f,
                "{name} is not installed and there is no network to fetch it. \
                 Pull it on a connected machine, or run without it."
            ),
            PullRefusal::NotEnoughDisk { name, needs_mb, free_mb } => write!(
                f,
                "{name} needs {needs_mb} MB and {free_mb} MB is free. Free up \
                 {} MB, or choose a tool that is already installed.",
                needs_mb.saturating_sub(*free_mb)
            ),
            PullRefusal::UnknownSize { name } => write!(
                f,
                "{name} declares no size, so the disk check cannot be made. \
                 Add size_mb to its registry entry rather than pulling blind."
            ),
        }
    }
}

impl Image {
    /// Decides whether fetching this is safe, before anything is downloaded.
    ///
    /// A twenty per cent margin is kept above the image size, because an image
    /// is extracted as well as downloaded and a disk that fills mid-pull
    /// leaves a partial layer cache rather than a clean failure.
    pub fn pull_preflight(
        &self,
        name: &str,
        installed: bool,
        network: bool,
        free_mb: u64,
    ) -> Result<(), PullRefusal> {
        if installed {
            return Ok(());
        }
        if !network {
            return Err(PullRefusal::NoNetwork { name: name.to_string() });
        }
        let Some(size) = self.size_mb else {
            return Err(PullRefusal::UnknownSize { name: name.to_string() });
        };
        let needs = size + size / 5;
        if free_mb < needs {
            return Err(PullRefusal::NotEnoughDisk {
                name: name.to_string(),
                needs_mb: needs,
                free_mb,
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Binary {
    /// Argv. The first element is looked up on PATH unless it is absolute.
    pub command: Vec<String>,
    /// Arguments that make it print its version, so a run can record what it
    /// actually invoked rather than what was installed when this was written.
    #[serde(default)]
    pub version_args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Emits {
    /// `score` for a number, `verdict` for a yes or no.
    ///
    /// A verdict cannot produce an ROC curve, which is why several tools here
    /// are driven through their library rather than their CLI: Aletheia's
    /// command line compares its own estimate to a threshold and prints a
    /// sentence, throwing away the number a comparison needs.
    pub output: Output,
    /// Whether a higher number means more likely to be carrying something.
    #[serde(default = "default_true")]
    pub higher_means_stego: bool,
}

fn default_true() -> bool {
    true
}

impl Default for Emits {
    fn default() -> Self {
        Emits { output: Output::Score, higher_means_stego: true }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Output {
    Score,
    Verdict,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Accepts {
    #[serde(default)]
    pub formats: Vec<String>,
    #[serde(default)]
    pub max_pixels: Option<u64>,
}

/// What one image costs, which is what the governor multiplies by the run size.
///
/// Declared values are estimates and are replaced by measured ones once a run
/// has been seen on this machine. They exist so that `stegobench plan` can
/// refuse before the damage: on 2026-09-17 a run became 128 Octave workers on
/// 16 cores and produced 298 OOM kills, for a measured 1.2x speedup, and
/// nothing in the tooling could have said so in advance.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Cost {
    #[serde(default)]
    pub seconds_per_image: Option<f64>,
    #[serde(default)]
    pub peak_rss_mb: Option<u64>,
    #[serde(default)]
    pub cores_per_worker: Option<u32>,
}

/// Fixtures that prove the tool is installed and working.
///
/// **Both directions are mandatory.** A tool that answers "stego" to everything
/// passes a detect-only check, and a tool that answers "clean" to everything
/// passes a clear-only one. This project has produced four separate controls
/// that could not fail, including a rich-model extraction that ran over 2,000
/// images, exited zero and produced nothing because an Octave package was
/// missing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Selftest {
    pub must_detect: String,
    pub must_clear: String,
}

#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("cannot read {path}: {source}")]
    Read { path: String, #[source] source: std::io::Error },
    #[error("{path} is not a valid registry entry: {source}")]
    Parse { path: String, #[source] source: toml::de::Error },
    #[error("{name} is not usable:\n  {}", .problems.join("\n  "))]
    Invalid { name: String, problems: Vec<String> },
}

impl Entry {
    /// Rules a TOML parser cannot express.
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut bad = Vec::new();

        match (&self.image, &self.binary) {
            (Some(_), Some(_)) => bad.push(
                "declares both an image and a binary; it must be one or the other".into(),
            ),
            (None, None) => bad.push("declares neither an image nor a binary".into()),
            _ => {}
        }

        if let Some(img) = &self.image {
            // The single most common way a result becomes unreproducible.
            if !img.reference.contains("@sha256:") {
                bad.push(format!(
                    "image {:?} is not pinned by digest. A tag can move under \
                     you, so a result naming one cannot be reproduced",
                    img.reference
                ));
            }
        }

        if let Some(bin) = &self.binary {
            if bin.command.is_empty() {
                bad.push("binary.command is empty".into());
            } else if bin.version_args.is_empty() {
                bad.push(
                    "binary declares no version_args, so a run could not record \
                     which build it invoked"
                        .into(),
                );
            }
        }

        if self.selftest.is_none() {
            bad.push(
                "declares no selftest. Both a must_detect and a must_clear \
                 fixture are required, because a check that cannot fail is \
                 worse than no check"
                    .into(),
            );
        }

        // A secret that looks like it holds a value rather than naming one.
        for s in &self.secrets {
            if s.contains('=') || s.len() > 64 {
                bad.push(format!(
                    "secrets entry {s:?} looks like a value. This field names \
                     environment variables; a value must never be written here"
                ));
            }
        }

        if bad.is_empty() {
            Ok(())
        } else {
            Err(bad)
        }
    }

    /// The environment variable names this tool needs supplied at run time.
    pub fn secret_names(&self) -> &[String] {
        &self.secrets
    }

    /// A one-line summary for `stegobench list`.
    pub fn summary(&self) -> String {
        let how = match (&self.image, &self.binary) {
            (Some(i), _) => i.reference.split('@').next().unwrap_or("image").to_string(),
            (_, Some(b)) => b.command.first().cloned().unwrap_or_else(|| "binary".into()),
            _ => "unconfigured".into(),
        };
        let secrets = if self.secrets.is_empty() {
            String::new()
        } else {
            format!("  needs {}", self.secrets.join(", "))
        };
        format!("{:<16} {:<10} {how}{secrets}", self.name, self.licence)
    }
}

/// Everything registered, keyed by name so the order is stable.
#[derive(Debug, Default)]
pub struct Registry {
    pub entries: BTreeMap<String, Entry>,
}

impl Registry {
    /// Loads every `.toml` under `dir`, recursively.
    ///
    /// An invalid entry fails the load rather than being skipped: a registry
    /// that quietly drops a tool reports a smaller world than it has, and the
    /// person who added the file is the last to find out.
    pub fn load(dir: &Path) -> Result<Self, RegistryError> {
        let mut reg = Registry::default();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            let read = std::fs::read_dir(&d).map_err(|e| RegistryError::Read {
                path: d.display().to_string(),
                source: e,
            })?;
            for item in read.flatten() {
                let p = item.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().is_some_and(|e| e == "toml") {
                    let text = std::fs::read_to_string(&p).map_err(|e| RegistryError::Read {
                        path: p.display().to_string(),
                        source: e,
                    })?;
                    let entry: Entry =
                        toml::from_str(&text).map_err(|e| RegistryError::Parse {
                            path: p.display().to_string(),
                            source: e,
                        })?;
                    entry.validate().map_err(|problems| RegistryError::Invalid {
                        name: p.display().to_string(),
                        problems,
                    })?;
                    reg.entries.insert(entry.name.clone(), entry);
                }
            }
        }
        Ok(reg)
    }

    pub fn of_kind(&self, kind: Kind) -> Vec<&Entry> {
        self.entries.values().filter(|e| e.kind == kind).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A valid entry plus whatever the caller adds.
    ///
    /// `extra` is spliced BEFORE the [selftest] table, because TOML assigns a
    /// bare key to the table above it: putting it after made every top-level
    /// key in a test silently become part of [selftest], which is how two of
    /// these tests were asserting on a parse error rather than on behaviour.
    fn minimal(extra: &str) -> String {
        format!(
            r#"
name = "x"
kind = "detector"
licence = "MIT"
{extra}

[selftest]
must_detect = "a.png"
must_clear = "b.png"
"#
        )
    }

    fn parse(extra: &str) -> Entry {
        toml::from_str(&minimal(extra)).expect("parses")
    }

    #[test]
    fn a_container_entry_pinned_by_digest_is_valid() {
        let e = parse("[image]\nreference = \"ghcr.io/x/y@sha256:abc\"");
        assert_eq!(e.validate(), Ok(()));
    }

    #[test]
    fn a_mutable_tag_is_refused() {
        let e = parse("[image]\nreference = \"ghcr.io/x/y:latest\"");
        assert!(e.validate().unwrap_err()[0].contains("not pinned by digest"));
    }

    #[test]
    fn a_binary_entry_is_valid_when_it_can_report_its_version() {
        let e = parse("[binary]\ncommand = [\"stegcore\"]\nversion_args = [\"--version\"]");
        assert_eq!(e.validate(), Ok(()));
    }

    #[test]
    fn a_binary_that_cannot_report_its_version_is_refused() {
        let e = parse("[binary]\ncommand = [\"stegcore\"]");
        assert!(e.validate().unwrap_err()[0].contains("version_args"));
    }

    #[test]
    fn declaring_both_an_image_and_a_binary_is_refused() {
        let e = parse(
            "[image]\nreference = \"x@sha256:a\"\n[binary]\ncommand = [\"y\"]\nversion_args = [\"-v\"]",
        );
        assert!(e.validate().unwrap_err()[0].contains("one or the other"));
    }

    #[test]
    fn an_entry_without_a_selftest_is_refused() {
        let e: Entry = toml::from_str(
            r#"name = "x"
kind = "detector"
licence = "MIT"
[binary]
command = ["y"]
version_args = ["-v"]"#,
        )
        .unwrap();
        assert!(e.validate().unwrap_err()[0].contains("cannot fail"));
    }

    #[test]
    fn a_secret_that_looks_like_a_value_is_refused() {
        // The field names variables. Somebody pasting a token here is the
        // failure this catches, and it is worth catching loudly.
        // The fixture is deliberately short. A realistic-looking token here
        // would trip the repository's own secret-content gate on every commit,
        // and teaching that gate to ignore this file is a worse trade than
        // testing the rule with a value that is obviously not one.
        let e = parse(
            "secrets = [\"NAME=value\"]\n\
             [binary]\ncommand = [\"y\"]\nversion_args = [\"-v\"]\n",
        );
        assert!(e.validate().unwrap_err()[0].contains("looks like a value"));
    }

    #[test]
    fn an_over_long_secret_name_is_refused_too() {
        // The other half of the rule: no environment variable is 65 characters
        // long, so something that is probably has a value pasted into it.
        let long = "X".repeat(70);
        let e = parse(&format!(
            "secrets = [\"{long}\"]\n[binary]\ncommand = [\"y\"]\nversion_args = [\"-v\"]\n"
        ));
        assert!(e.validate().unwrap_err()[0].contains("looks like a value"));
    }

    #[test]
    fn secrets_are_names_only_and_are_reported_as_such() {
        let e = parse(
            "secrets = [\"STEGASHIELD_LICENCE\"]\n\
             [binary]\ncommand = [\"y\"]\nversion_args = [\"-v\"]",
        );
        assert_eq!(e.validate(), Ok(()));
        assert_eq!(e.secret_names(), ["STEGASHIELD_LICENCE"]);
    }
}

#[cfg(test)]
mod preflight_tests {
    use super::*;

    fn img(size: Option<u64>) -> Image {
        Image {
            reference: "x@sha256:a".into(),
            needs_network: false,
            size_mb: size,
            bundled: false,
        }
    }

    #[test]
    fn an_installed_image_needs_nothing_checked() {
        // Offline with a full disk is fine when the bytes are already here,
        // which is the normal case for a bundled tool.
        assert_eq!(img(None).pull_preflight("x", true, false, 0), Ok(()));
    }

    #[test]
    fn a_missing_image_offline_is_refused_with_advice() {
        let e = img(Some(100)).pull_preflight("aletheia", false, false, 999_999).unwrap_err();
        assert!(e.to_string().contains("no network"));
    }

    #[test]
    fn the_disk_check_keeps_a_margin_for_extraction() {
        // 9.1 GB of Aletheia needs more than 9.1 GB free, because the image is
        // unpacked as well as downloaded, and filling the disk halfway through
        // leaves a partial layer cache rather than a clean failure.
        let rich = img(Some(9090));
        assert!(rich.pull_preflight("aletheia-rich", false, true, 9100).is_err());
        assert_eq!(rich.pull_preflight("aletheia-rich", false, true, 11000), Ok(()));
    }

    #[test]
    fn the_refusal_says_how_much_to_free_rather_than_just_no() {
        let e = img(Some(1000)).pull_preflight("hstego", false, true, 500).unwrap_err();
        // 1000 + 20% margin = 1200 needed, 500 free, so 700 to free.
        assert!(e.to_string().contains("700"), "got: {e}");
    }

    #[test]
    fn an_image_with_no_declared_size_is_refused_rather_than_pulled_blind() {
        let e = img(None).pull_preflight("mystery", false, true, 999_999).unwrap_err();
        assert!(e.to_string().contains("declares no size"));
    }
}
