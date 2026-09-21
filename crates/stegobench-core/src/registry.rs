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

use crate::corpus::CorpusEntry;

/// The size at or below which a tool ships inside the default image.
///
/// Operator's rule, 2026-09-18. It falls in a real gap in the measured data
/// rather than being a round number picked for looking tidy: the tools sit at
/// 118, 120, 287 and 512 MB, then jump to 836, 911, 1250, 1410, 8340 and 9090.
/// Nothing is near 750, so the line does not have to be argued about twice.
///
/// `bundled` is checked against this rather than set by hand, because a flag a
/// human maintains beside a number a machine measures drifts from it.
pub const BUNDLE_THRESHOLD_MB: u64 = 750;

/// A sha256 digest written as lowercase hexadecimal: 32 bytes, 64 characters.
const SHA256_HEX_LEN: usize = 64;

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
    /// How to invoke the tool, for the tools that do not speak the plugin
    /// protocol themselves.
    ///
    /// Two supported paths, both first class. A third party ships a container
    /// that answers `describe` and `run`, and needs nothing here. The classic
    /// tools predate any such idea by a decade and are driven by a declared
    /// argv plus a named parser, so the parsing lives in a small tested
    /// function rather than inside a general-purpose orchestrator. The zsteg
    /// parser alone had three bugs in one week while it lived in a 501 line
    /// script: the wrong marker, then the wrong stream, then an operator
    /// precedence error, and each was hard to see for the same reason.
    #[serde(default)]
    pub invoke: Option<Invoke>,
    #[serde(default)]
    pub selftest: Option<Selftest>,
    /// How to prove an EMBEDDER works, which is a different question.
    ///
    /// A detector is asked whether it can tell two images apart. An embedder
    /// cannot be asked that: the honest check is whether what goes in comes
    /// back out. Hide a known payload, extract it, compare the bytes.
    ///
    /// This matters more than it sounds. A corpus built by an embedder that
    /// silently wrote nothing would look exactly like an undetectable one, and
    /// every detector scored against it would appear to fail. The manifest
    /// already records samples changed for the same reason; this catches the
    /// case before a twenty hour build rather than after it.
    #[serde(default)]
    pub roundtrip: Option<Roundtrip>,
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
    NotEnoughDisk {
        name: String,
        needs_mb: u64,
        free_mb: u64,
    },
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
            PullRefusal::NotEnoughDisk {
                name,
                needs_mb,
                free_mb,
            } => write!(
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
            return Err(PullRefusal::NoNetwork {
                name: name.to_string(),
            });
        }
        let Some(size) = self.size_mb else {
            return Err(PullRefusal::UnknownSize {
                name: name.to_string(),
            });
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
    /// Size of the executable, for the toolkit image budget. A binary is
    /// always bundled: Stegcore's release build is 8.6 MB against a default
    /// image measured in hundreds, so the question does not arise.
    #[serde(default)]
    pub size_mb: Option<u64>,
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
        Emits {
            output: Output::Score,
            higher_means_stego: true,
        }
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

/// Prove an embedder by hiding something and getting it back.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Roundtrip {
    /// Argv to hide `{payload}` in `{cover}`, writing `{stego}`.
    pub embed_argv: Vec<String>,
    /// Argv to recover `{stego}` into `{recovered}`.
    pub extract_argv: Vec<String>,
    /// The cover to use, from the fixtures directory.
    pub cover: String,
    /// Substituted for `{passphrase}`. Not a secret: it protects a fixture
    /// that exists for three seconds inside a temporary directory, and a
    /// constant here keeps the check reproducible.
    #[serde(default = "default_passphrase")]
    pub passphrase: String,
    #[serde(default)]
    pub entrypoint: Option<String>,
}

fn default_passphrase() -> String {
    "stegobench-selftest".into()
}

/// How to run a tool that does not speak the protocol.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Invoke {
    /// Argv inside the container. `{file}` is replaced by the mounted image
    /// and `{adapter}` by the mounted adapter, if one is declared.
    pub argv: Vec<String>,
    /// A small script mounted read-only into the container, for tools whose
    /// own command line throws away the number we need.
    ///
    /// Aletheia is the case that forces this to exist. Its CLI compares its
    /// own estimate to a threshold and prints a sentence, so driving it
    /// through the command line discards the statistic an ROC curve is made
    /// of. The adapter calls the same functions the CLI calls, unmodified,
    /// and prints the estimate instead of a verdict. Nothing is
    /// reimplemented; the detector stays Aletheia's.
    #[serde(default)]
    pub adapter: Option<String>,
    /// Override the container entrypoint, which several of these images set to
    /// the tool itself.
    #[serde(default)]
    pub entrypoint: Option<String>,
    /// Environment for the container, as `NAME=value`.
    ///
    /// This is where image-specific knowledge belongs. Aletheia installs its
    /// library at /opt/aletheia while the image's working directory is /data,
    /// so an import fails unless PYTHONPATH says otherwise. Putting that in
    /// the adapter would hardcode one image's layout into a script meant to
    /// outlive it; putting it here keeps the adapter about the detector and
    /// the entry about the container.
    ///
    /// Secrets never appear here. They are named in `secrets` and their values
    /// are read from the host environment at run time.
    #[serde(default)]
    pub env: Vec<String>,
    /// Give the tool a writable working directory with the image copied into
    /// it, rather than a read-only mount of the file.
    ///
    /// StegExpose is the case: it takes a directory, and it writes its CSV to
    /// a path you hand it. Given a read-only mount it prints nothing at all
    /// and exits zero, which is silence that looks exactly like "found
    /// nothing". Several other tools write beside their input the same way.
    #[serde(default)]
    pub writable_workdir: bool,
    /// Read the answer from this file inside the working directory instead of
    /// from stdout. Requires `writable_workdir`.
    #[serde(default)]
    pub output_file: Option<String>,
    /// Run the argv on the host rather than inside the container.
    ///
    /// For tools that are SERVICES rather than commands. StegaShield runs as
    /// a container serving HTTP, so scoring an image means posting it to a
    /// running instance; there is no command to run inside the image. The
    /// adapter runs here and asks the service a question.
    ///
    /// The image reference stays in the entry because it is still what
    /// identifies the subject in a result. It is never vendored: it is a third
    /// party's artefact, referenced by digest and pulled by whoever runs it.
    #[serde(default)]
    pub host: bool,
    /// Which built-in parser reads the output. Named rather than described,
    /// because these formats are quirky enough that a rule in TOML would be a
    /// small programming language nobody wants to debug.
    pub parser: String,
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
    /// The score above which this tool's output means "carrying something",
    /// for the self-test only.
    ///
    /// It has to be per tool because the outputs are not the same quantity.
    /// Aletheia's estimators return an embedding RATE, so a 0.4 bpp fixture
    /// scores near 0.4 and a fixed 0.5 would fail a perfectly good detector.
    /// StegExpose returns a fused statistic on its own scale. Picking one
    /// number for all of them would be measuring the threshold rather than
    /// the tool.
    ///
    /// This is NOT a calibrated operating point and must never be used as one:
    /// it is a smoke-test decision point against a deliberately loud fixture.
    /// Real thresholds come from a false-positive budget on a real corpus.
    #[serde(default = "default_threshold")]
    pub threshold: f64,
}

fn default_threshold() -> f64 {
    0.5
}

#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("cannot read {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{path} is not a valid registry entry: {source}")]
    Parse {
        path: String,
        #[source]
        source: toml::de::Error,
    },
    #[error("{name} is not usable:\n  {}", .problems.join("\n  "))]
    Invalid { name: String, problems: Vec<String> },
}

impl Entry {
    /// Rules a TOML parser cannot express.
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut bad = Vec::new();

        match (&self.image, &self.binary) {
            (Some(_), Some(_)) => {
                bad.push("declares both an image and a binary; it must be one or the other".into())
            }
            (None, None) => bad.push(
                "declares neither an image nor a binary; add an [image] block \
                 (a container reference pinned by digest) or a [binary] block \
                 (a locally installed executable) so the harness knows how to \
                 run this tool"
                    .into(),
            ),
            _ => {}
        }

        if let Some(img) = &self.image {
            // The single most common way a result becomes unreproducible.
            //
            // The digest itself is checked, not only the `@sha256:` marker: a
            // truncated or placeholder digest looks pinned to a substring test
            // and pins nothing, and a registry entry written ahead of the build
            // that produces the real digest is exactly how one gets written.
            match img.reference.split_once("@sha256:") {
                Some((repo, digest))
                    if !repo.is_empty()
                        && digest.len() == SHA256_HEX_LEN
                        && digest.bytes().all(|b| b.is_ascii_hexdigit()) =>
                {
                    // `ManifestV1::validate` and `CorpusEntry::validate` both
                    // refuse an uppercase digest on the grounds that digests
                    // are compared as text. This half of the registry accepted
                    // one, so the same paste passed here and failed there.
                    if digest.bytes().any(|b| b.is_ascii_uppercase()) {
                        bad.push(format!(
                            "image {:?} names an upper case digest; digests are \
                             compared as text here, so case has to be settled. \
                             `docker pull` reports lower case, which is the form \
                             to paste",
                            img.reference
                        ));
                    }
                }
                Some((_, digest)) => bad.push(format!(
                    "image {:?} names a digest of {} character(s), but a \
                     sha256 digest is {SHA256_HEX_LEN} hexadecimal \
                     characters. A short or placeholder digest reads as \
                     pinned and pins nothing; run `docker pull` and paste \
                     the digest it reports",
                    img.reference,
                    digest.len()
                )),
                None => bad.push(format!(
                    "image {:?} is not pinned by digest. A tag can move under \
                     you, so a result naming one cannot be reproduced; pull the \
                     image and add its @sha256:... digest to the reference",
                    img.reference
                )),
            }
        }

        if let Some(bin) = &self.binary {
            if bin.command.is_empty() {
                bad.push(
                    "binary.command is empty; set it to the argv that invokes \
                     this tool, for example [\"stegcore\", \"analyse\", \
                     \"--json\"]"
                        .into(),
                );
            } else if bin.version_args.is_empty() {
                bad.push(
                    "binary declares no version_args, so a run could not record \
                     which build it invoked"
                        .into(),
                );
            }
        }

        // The bundling flag is derived, not decided. Letting it be set freely
        // would mean the default image's contents drift from the sizes that
        // justify them, and nobody would notice until a pull took nine minutes.
        if let Some(img) = &self.image {
            if let Some(size) = img.size_mb {
                let should = size <= BUNDLE_THRESHOLD_MB;
                if img.bundled != should {
                    bad.push(format!(
                        "bundled is {} but the image is {} MB, and the rule is \
                         bundled at or below {} MB",
                        img.bundled, size, BUNDLE_THRESHOLD_MB
                    ));
                }
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

        if let Some(inv) = &self.invoke {
            if inv.output_file.is_some() && !inv.writable_workdir {
                bad.push(
                    "invoke.output_file needs writable_workdir, or there is \
                     nowhere for the tool to write it"
                        .into(),
                );
            }
        }

        // env is for layout, not credentials. A token pasted here would be
        // written into a file that is committed, which is the failure the
        // secrets field exists to prevent, so it is refused in both places.
        if let Some(inv) = &self.invoke {
            for kv in &inv.env {
                let looks_secret = ["TOKEN", "SECRET", "KEY", "PASSWORD", "LICENCE", "LICENSE"]
                    .iter()
                    .any(|k| {
                        kv.split('=')
                            .next()
                            .is_some_and(|n| n.to_uppercase().contains(k))
                    });
                if looks_secret {
                    bad.push(format!(
                        "invoke.env entry {:?} names a credential. Declare it in \
                         secrets instead, so its value is read from the \
                         environment and never committed",
                        kv.split('=').next().unwrap_or(kv)
                    ));
                }
            }
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
            (_, Some(b)) => b
                .command
                .first()
                .cloned()
                .unwrap_or_else(|| "binary".into()),
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

/// The subdirectory holding corpora rather than tools.
///
/// Corpora are a different type with different fields (see
/// [`crate::corpus`]), so the tool walk skips this directory and hands it to
/// the corpus loader instead. One registry to a user, two schemas underneath,
/// because a corpus has no image and a detector has no licence URL.
pub const CORPORA_DIR: &str = "corpora";

/// Everything registered, keyed by name so the order is stable.
#[derive(Debug, Default)]
pub struct Registry {
    pub entries: BTreeMap<String, Entry>,
    /// Registered datasets, keyed by id.
    pub corpora: BTreeMap<String, CorpusEntry>,
}

impl Registry {
    /// Loads every `.toml` under `dir`, recursively, plus the corpora in
    /// `dir/corpora`.
    ///
    /// An invalid entry fails the load rather than being skipped: a registry
    /// that quietly drops a tool reports a smaller world than it has, and the
    /// person who added the file is the last to find out. Corpora load the
    /// same way, deliberately, so the behaviour a reader predicts from one
    /// half holds for the other.
    pub fn load(dir: &Path) -> Result<Self, RegistryError> {
        let mut reg = Registry {
            corpora: crate::corpus::load_dir(&dir.join(CORPORA_DIR))?,
            ..Registry::default()
        };
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            let read = std::fs::read_dir(&d).map_err(|e| RegistryError::Read {
                path: d.display().to_string(),
                source: e,
            })?;
            for item in read.flatten() {
                let p = item.path();
                if p.is_dir() {
                    if p.file_name().is_some_and(|n| n == CORPORA_DIR) {
                        continue;
                    }
                    stack.push(p);
                } else if p.extension().is_some_and(|e| e == "toml") {
                    let text = std::fs::read_to_string(&p).map_err(|e| RegistryError::Read {
                        path: p.display().to_string(),
                        source: e,
                    })?;
                    let entry: Entry = toml::from_str(&text).map_err(|e| RegistryError::Parse {
                        path: p.display().to_string(),
                        source: e,
                    })?;
                    entry
                        .validate()
                        .map_err(|problems| RegistryError::Invalid {
                            name: p.display().to_string(),
                            problems,
                        })?;
                    // Two files claiming one name is a silent coin toss
                    // otherwise: the walk visits them in filesystem order, so
                    // which one wins varies between machines and `describe`
                    // would be reproducible only by luck. A name is also how a
                    // result identifies what produced it.
                    if let Some(first) = reg.entries.insert(entry.name.clone(), entry) {
                        return Err(RegistryError::Invalid {
                            name: p.display().to_string(),
                            problems: vec![format!(
                                "a second tool claims the name {:?}; a name is \
                                 how `describe` finds an entry and how a result \
                                 says what produced it, so two files cannot \
                                 share one",
                                first.name
                            )],
                        });
                    }
                }
            }
        }
        // A corpus id and a tool name share one `describe` namespace, so a
        // collision would make one of them unreachable by the vocabulary the
        // user is told to use.
        for id in reg.corpora.keys() {
            if reg.entries.contains_key(id) {
                return Err(RegistryError::Invalid {
                    name: id.clone(),
                    problems: vec!["is registered both as a tool and as a corpus. \
                         `describe` takes one name for both, so one of the two \
                         would be unreachable"
                        .into()],
                });
            }
        }
        Ok(reg)
    }

    pub fn of_kind(&self, kind: Kind) -> Vec<&Entry> {
        self.entries.values().filter(|e| e.kind == kind).collect()
    }

    /// How much disk the registry actually costs, counting each image once.
    ///
    /// Summing per entry is wrong and wrong by a lot. `aletheia-spa` and
    /// `aletheia-rs` are two tools in one 8.3 GB image, so a per-entry total
    /// reported 30 GB of on-demand tools where the truth is 21.8 GB. Telling
    /// somebody to free nine gigabytes they do not need is not a rounding
    /// error, it is a wrong answer to the only question they asked.
    pub fn footprint(&self) -> Footprint {
        let mut seen: BTreeMap<&str, (u64, bool)> = BTreeMap::new();
        let mut binaries_mb = 0;
        for e in self.entries.values() {
            match (&e.image, &e.binary) {
                (Some(img), _) => {
                    // Keyed on the reference, so two tools sharing an image
                    // are one entry here however they are named.
                    seen.insert(
                        img.reference.as_str(),
                        (img.size_mb.unwrap_or(0), img.bundled),
                    );
                }
                (_, Some(bin)) => binaries_mb += bin.size_mb.unwrap_or(0),
                _ => {}
            }
        }
        let bundled_images: u64 = seen.values().filter(|(_, b)| *b).map(|(s, _)| s).sum();
        Footprint {
            bundled_mb: bundled_images + binaries_mb,
            on_demand_mb: seen.values().filter(|(_, b)| !*b).map(|(s, _)| s).sum(),
            unique_images: seen.len(),
            tools: self.entries.len(),
        }
    }
}

/// What the registry costs on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Footprint {
    /// Sum of the distinct bundled images plus every binary.
    ///
    /// This is an UPPER BOUND on the default image, not its size. The bundled
    /// tools share base layers (steghide and outguess are both Debian, and the
    /// Java tools share a JRE), so the built image is smaller than the sum of
    /// its parts. Reporting the sum as the image size would be a different
    /// bookkeeping error in the opposite direction.
    pub bundled_mb: u64,
    /// Sum of the distinct images not in the default image.
    pub on_demand_mb: u64,
    /// Distinct images, which is fewer than the tool count whenever one image
    /// provides several tools.
    pub unique_images: usize,
    pub tools: usize,
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

    const A_REAL_DIGEST: &str = "59710f7b5fbaeb7c3b1d4333e64654c1721a3ddb60b489d8e54d5d0e8b269bfb";

    #[test]
    fn a_container_entry_pinned_by_digest_is_valid() {
        let e = parse(&format!(
            "[image]\nreference = \"ghcr.io/x/y@sha256:{A_REAL_DIGEST}\""
        ));
        assert_eq!(e.validate(), Ok(()));
    }

    #[test]
    fn a_mutable_tag_is_refused() {
        let e = parse("[image]\nreference = \"ghcr.io/x/y:latest\"");
        assert!(e.validate().unwrap_err()[0].contains("not pinned by digest"));
    }

    /// The two halves of the registry have to hold one standard. `ManifestV1`
    /// and `CorpusEntry` both refuse an uppercase digest because digests are
    /// compared as text; this half accepted one, so the same paste passed in
    /// one file and failed in another with nothing to explain the difference.
    #[test]
    fn an_upper_case_image_digest_is_refused_as_it_is_everywhere_else() {
        let e = parse(&format!(
            "[image]\nreference = \"ghcr.io/x/y@sha256:{}\"",
            A_REAL_DIGEST.to_uppercase()
        ));
        let problems = e.validate().expect_err("upper case digest was accepted");
        assert!(
            problems.iter().any(|p| p.contains("upper case")),
            "refused for the wrong reason: {problems:?}"
        );
    }

    /// `@sha256:` as a substring is not a pin. A placeholder waiting on a
    /// build, a digest truncated in a paste, or one with a non-hex character
    /// in it all satisfy a `contains` test and identify no image at all.
    #[test]
    fn a_digest_that_is_not_a_digest_is_refused() {
        for bad in [
            "ghcr.io/x/y@sha256:abc",
            "ghcr.io/x/y@sha256:REAL_DIGEST_AFTER_BUILD",
            "ghcr.io/x/y@sha256:",
            &format!("ghcr.io/x/y@sha256:{A_REAL_DIGEST}beef"),
            &format!("ghcr.io/x/y@sha256:{}z", &A_REAL_DIGEST[..63]),
        ] {
            let e = parse(&format!("[image]\nreference = \"{bad}\""));
            let problems = match e.validate() {
                Ok(()) => panic!("{bad} was accepted as pinned"),
                Err(problems) => problems,
            };
            assert!(
                problems
                    .iter()
                    .any(|p| p.contains("hexadecimal characters")),
                "{bad} was refused for the wrong reason: {problems:?}"
            );
        }
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
mod load_tests {
    use super::*;

    const TOOL: &str = r#"
name = "steghide"
kind = "embedder"
licence = "GPL-2.0-only"

[binary]
command = ["steghide"]
version_args = ["--version"]

[selftest]
must_detect = "a.png"
must_clear = "b.png"
"#;

    const CORPUS: &str = r#"
id = "steghide"
name = "A corpus named like a tool"
description = "Only exists to collide with one."

[licence]
status = "unverified"
note = "a fixture, so nothing was read"
redistribution = "unknown"
redistribution_reason = "a fixture"

[obtain]
url = "https://example.org/x"

[properties]
base_images = 10
"#;

    fn registry_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("embedders")).unwrap();
        std::fs::write(dir.path().join("embedders/steghide.toml"), TOOL).unwrap();
        std::fs::create_dir(dir.path().join(CORPORA_DIR)).unwrap();
        dir
    }

    /// A corpus file is a different schema, so the tool walk must not try to
    /// parse it. If it did, adding the first corpus would break every command
    /// that loads the registry.
    #[test]
    fn corpora_are_loaded_as_corpora_and_not_parsed_as_tools() {
        let dir = registry_dir();
        std::fs::write(
            dir.path().join(CORPORA_DIR).join("c.toml"),
            CORPUS.replace("steghide", "example-corpus"),
        )
        .unwrap();
        let reg = Registry::load(dir.path()).expect("loads");
        assert_eq!(reg.entries.len(), 1);
        assert_eq!(reg.corpora.len(), 1);
        assert!(reg.corpora.contains_key("example-corpus"));
    }

    /// One `describe` namespace, so one name cannot mean two things.
    #[test]
    fn a_corpus_id_colliding_with_a_tool_name_is_refused() {
        let dir = registry_dir();
        std::fs::write(dir.path().join(CORPORA_DIR).join("c.toml"), CORPUS).unwrap();
        let err = Registry::load(dir.path()).expect_err("the collision is refused");
        assert!(
            err.to_string().contains("both as a tool and as a corpus"),
            "got: {err}"
        );
    }

    /// Filesystem order decided the winner before this, so two entries sharing
    /// a name resolved differently on different machines.
    #[test]
    fn two_tool_files_claiming_one_name_are_refused_rather_than_racing() {
        let dir = registry_dir();
        std::fs::write(dir.path().join("embedders/copy.toml"), TOOL).unwrap();
        let err = Registry::load(dir.path()).expect_err("the duplicate is refused");
        assert!(
            err.to_string().contains("second tool claims the name"),
            "got: {err}"
        );
    }

    #[test]
    fn a_registry_with_no_corpora_directory_still_loads_its_tools() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("steghide.toml"), TOOL).unwrap();
        let reg = Registry::load(dir.path()).expect("loads");
        assert_eq!(reg.entries.len(), 1);
        assert!(reg.corpora.is_empty());
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
        let e = img(Some(100))
            .pull_preflight("aletheia", false, false, 999_999)
            .unwrap_err();
        assert!(e.to_string().contains("no network"));
    }

    #[test]
    fn the_disk_check_keeps_a_margin_for_extraction() {
        // 9.1 GB of Aletheia needs more than 9.1 GB free, because the image is
        // unpacked as well as downloaded, and filling the disk halfway through
        // leaves a partial layer cache rather than a clean failure.
        let rich = img(Some(9090));
        assert!(rich
            .pull_preflight("aletheia-rich", false, true, 9100)
            .is_err());
        assert_eq!(
            rich.pull_preflight("aletheia-rich", false, true, 11000),
            Ok(())
        );
    }

    #[test]
    fn the_refusal_says_how_much_to_free_rather_than_just_no() {
        let e = img(Some(1000))
            .pull_preflight("hstego", false, true, 500)
            .unwrap_err();
        // 1000 + 20% margin = 1200 needed, 500 free, so 700 to free.
        assert!(e.to_string().contains("700"), "got: {e}");
    }

    #[test]
    fn an_image_with_no_declared_size_is_refused_rather_than_pulled_blind() {
        let e = img(None)
            .pull_preflight("mystery", false, true, 999_999)
            .unwrap_err();
        assert!(e.to_string().contains("declares no size"));
    }
}

#[cfg(test)]
mod bundling_tests {
    use super::*;

    fn entry_with(size: u64, bundled: bool) -> Entry {
        let mut e: Entry = toml::from_str(
            r#"name = "x"
kind = "detector"
licence = "MIT"
[selftest]
must_detect = "a.png"
must_clear = "b.png"
"#,
        )
        .unwrap();
        e.image = Some(Image {
            reference: format!("x@sha256:{}", "a".repeat(SHA256_HEX_LEN)),
            needs_network: false,
            size_mb: Some(size),
            bundled,
        });
        e
    }

    #[test]
    fn a_small_tool_must_be_bundled() {
        assert_eq!(entry_with(512, true).validate(), Ok(()));
        assert!(entry_with(512, false).validate().is_err());
    }

    #[test]
    fn a_large_tool_must_not_be() {
        // openstego at 911 MB and stegosuite at 836 MB were both bundled until
        // the rule was written down, which is exactly the drift this prevents.
        assert_eq!(entry_with(911, false).validate(), Ok(()));
        assert!(entry_with(911, true).validate().is_err());
    }

    #[test]
    fn the_boundary_is_inclusive_and_stated() {
        assert_eq!(entry_with(BUNDLE_THRESHOLD_MB, true).validate(), Ok(()));
        assert_eq!(
            entry_with(BUNDLE_THRESHOLD_MB + 1, false).validate(),
            Ok(())
        );
    }

    #[test]
    fn the_refusal_names_both_numbers_so_it_can_be_acted_on() {
        let e = entry_with(836, true).validate().unwrap_err();
        assert!(
            e[0].contains("836") && e[0].contains("750"),
            "got: {:?}",
            e[0]
        );
    }
}

#[cfg(test)]
mod footprint_tests {
    use super::*;

    fn reg(entries: Vec<(&str, &str, u64, bool)>) -> Registry {
        let mut r = Registry::default();
        for (name, reference, size, bundled) in entries {
            let mut e: Entry = toml::from_str(
                r#"name = "x"
kind = "detector"
licence = "MIT"
[selftest]
must_detect = "a.png"
must_clear = "b.png"
"#,
            )
            .unwrap();
            e.name = name.into();
            e.image = Some(Image {
                reference: reference.into(),
                needs_network: false,
                size_mb: Some(size),
                bundled,
            });
            r.entries.insert(name.into(), e);
        }
        r
    }

    #[test]
    fn two_tools_in_one_image_are_counted_once() {
        // The real case: aletheia-spa and aletheia-rs are one 8.3 GB image.
        // Counting per entry said 16.7 GB and would have told somebody to
        // free eight gigabytes they do not need.
        let r = reg(vec![
            ("aletheia-spa", "aletheia@sha256:a", 8340, false),
            ("aletheia-rs", "aletheia@sha256:a", 8340, false),
        ]);
        let f = r.footprint();
        assert_eq!(f.on_demand_mb, 8340);
        assert_eq!(f.unique_images, 1);
        assert_eq!(f.tools, 2);
    }

    #[test]
    fn distinct_images_still_add_up() {
        let r = reg(vec![
            ("a", "a@sha256:1", 100, true),
            ("b", "b@sha256:2", 200, true),
        ]);
        assert_eq!(r.footprint().bundled_mb, 300);
    }

    #[test]
    fn bundled_and_on_demand_are_kept_apart() {
        let r = reg(vec![
            ("small", "s@sha256:1", 500, true),
            ("large", "l@sha256:2", 9000, false),
        ]);
        let f = r.footprint();
        assert_eq!((f.bundled_mb, f.on_demand_mb), (500, 9000));
    }

    #[test]
    fn a_binary_counts_toward_the_bundle_because_it_is_compiled_in() {
        let mut r = reg(vec![("img", "i@sha256:1", 100, true)]);
        let mut e: Entry = toml::from_str(
            r#"name = "stegcore"
kind = "detector"
licence = "AGPL-3.0-or-later"
[binary]
command = ["stegcore"]
version_args = ["--version"]
size_mb = 9

[selftest]
must_detect = "a.png"
must_clear = "b.png"
"#,
        )
        .unwrap();
        e.name = "stegcore".into();
        r.entries.insert("stegcore".into(), e);
        assert_eq!(r.footprint().bundled_mb, 109);
    }
}
