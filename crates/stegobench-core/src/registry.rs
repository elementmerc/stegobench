// Author:  Daniel Iwugo
// Comment: Christ is King
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
use std::net::{IpAddr, Ipv4Addr};
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

/// The platform names a registry entry may use.
///
/// Deliberately the values `std::env::consts::OS` produces rather than prettier
/// ones, because the check compares against exactly that. A name that reads
/// well and matches nothing is worse than no field.
pub const KNOWN_PLATFORMS: &[&str] = &["linux", "macos", "windows"];

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
    /// The operating systems this tool can run on.
    ///
    /// Empty means the entry does not say, which is the honest default and is
    /// what every entry written before this field existed means. It is NOT the
    /// same as "all three": an unstated platform is a question nobody has
    /// answered, and a run that fails on a Mac should be able to tell the user
    /// whether the tool cannot run there or merely is not installed.
    ///
    /// Only the binary route really needs it. A container is a Linux image
    /// wherever it runs, and on macOS and Windows the runtime supplies the
    /// Linux to run it in, so a containerised tool is available anywhere the
    /// runtime is. A locally installed program is whatever was built for that
    /// machine, and a Windows-only forensic tool is a real thing this registry
    /// has to be able to describe rather than quietly fail to find.
    #[serde(default)]
    pub platforms: Vec<String>,
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
    ///
    /// It is NOT a statement about this machine. The instance being scored
    /// normally runs on another host, so whether the image happens to be
    /// pulled here says nothing about whether a run can proceed, and the
    /// availability check deliberately does not ask.
    #[serde(default)]
    pub host: bool,
    /// The NAME of the environment variable carrying the service's address.
    ///
    /// A name, never a value, for the same reason `secrets` is names only: an
    /// address written into a registry entry is a default, and a default is
    /// scored against whatever answers on it, so a result can name one
    /// detector while measuring another. This field is how an entry can still
    /// say which variable a user is expected to export without carrying what
    /// goes in it, and `is_env_var_name` is what stops the address being
    /// written here instead: a URL is not the name of a variable, so the
    /// shortcut is refused rather than merely discouraged.
    ///
    /// Declaring it is what lets the harness tell two situations apart before
    /// a run starts: nobody has said where the service is, and the service is
    /// not answering. Without it the first is only discovered image by image,
    /// as the adapter's own refusal, once a corpus is already under way.
    ///
    /// Optional, because not every host adapter talks to a network service.
    /// Meaningless without `host`, and refused there rather than ignored.
    #[serde(default)]
    pub endpoint_env: Option<String>,
    /// Which built-in parser reads the output. Named rather than described,
    /// because these formats are quirky enough that a rule in TOML would be a
    /// small programming language nobody wants to debug.
    ///
    /// Checked against [`PARSERS`] at load. A name with a typo in it used to
    /// be accepted here and turn up once per image as "no built-in parser
    /// named ...", after the tool had already been launched.
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

/// How much of one invocation string is inspected for an address.
///
/// A registry entry is a small declarative file: the longest argv element and
/// the longest env entry in the shipped registry are both under 200 bytes. The
/// cap exists so that a pathological file cannot turn validation into a long
/// job, and a field over it is REFUSED rather than skipped, because skipping
/// the check is exactly how an address gets through one.
const INVOCATION_FIELD_MAX_BYTES: usize = 4096;

/// Characters that cannot appear inside a host, used to cut a string into
/// candidate tokens. Square brackets are absent on purpose: they delimit an
/// IPv6 literal in a URL and are part of the thing being read.
const TOKEN_SEPARATORS: &[char] = &[
    ' ', '\t', '\n', '\r', '"', '\'', '`', ',', ';', '=', '<', '>', '|', '\\', '(', ')', '{', '}',
];

/// A host-shaped token pulled out of a field.
struct HostToken {
    host: String,
    /// Whether it sat in the host slot of a URL. Only there is a bare number
    /// decoded as an address: `http://2130706433/` is 127.0.0.1 to every HTTP
    /// client, while a bare `2130706433` in an argv is a number.
    from_url: bool,
}

/// Every host-shaped token in one string.
///
/// This is deliberately generous about what counts as a candidate, because a
/// token that is not an address costs nothing: it fails to parse and is
/// dropped. Missing one costs a published internal address.
fn hosts_in(text: &str) -> Vec<HostToken> {
    let mut found = Vec::new();
    for token in text.split(TOKEN_SEPARATORS) {
        if token.is_empty() {
            continue;
        }
        let (rest, from_url) = match token.split_once("://") {
            Some((_scheme, after)) => (after, true),
            None => (token, false),
        };
        // Userinfo, which is everything before the last `@` of an authority.
        let rest = rest.rsplit_once('@').map_or(rest, |(_, after)| after);
        // Path, query and fragment.
        let authority = rest
            .split(['/', '?', '#'])
            .next()
            .unwrap_or_default()
            .trim_end_matches('.');
        let host = if let Some(stripped) = authority.strip_prefix('[') {
            // `[::1]:3000`: the brackets exist precisely so the port colon can
            // be told from the address colons.
            match stripped.split_once(']') {
                Some((inside, _port)) => inside,
                None => stripped,
            }
        } else if authority.matches(':').count() == 1 {
            authority.split(':').next().unwrap_or_default()
        } else {
            // Either no port, or an unbracketed IPv6 literal such as `::1`,
            // which is what a hand-written argv tends to carry.
            authority
        };
        // An IPv6 zone identifier: `fe80::1%eth0`, and `%25eth0` once a URL has
        // escaped the percent sign. It says which interface, not which host.
        let host = host.split('%').next().unwrap_or_default();
        if !host.is_empty() {
            found.push(HostToken {
                host: host.to_ascii_lowercase(),
                from_url,
            });
        }
    }
    found
}

/// Why this address cannot appear in a registry entry, or `None` if it can.
///
/// A name is never resolved. `detector.internal` may well point at a private
/// address on the network the entry was written on, and looking it up would
/// make the answer depend on which machine ran the check and what DNS said
/// that minute, so two people would get different verdicts on one file. This
/// reads what is written down and nothing else.
fn unshippable_address(token: &HostToken) -> Option<&'static str> {
    if token.host == "localhost" || token.host.ends_with(".localhost") {
        return Some("a name that resolves only on the machine that runs it");
    }
    let ip = match token.host.parse::<IpAddr>() {
        Ok(ip) => ip,
        // A host slot in a URL is a host, however it is spelled, and every
        // HTTP client decodes these. `0x7f.1` and `2130706433` are both
        // 127.0.0.1 and neither one looks like an address to a reader.
        Err(_) if token.from_url => IpAddr::V4(inet_aton(&token.host)?),
        Err(_) => return None,
    };
    classify(ip)
}

fn classify(ip: IpAddr) -> Option<&'static str> {
    match ip {
        IpAddr::V4(v4) => {
            if v4.is_loopback() {
                Some("a loopback address, which is only ever this machine")
            } else if v4.is_unspecified() {
                Some("the unspecified address, which names no host at all")
            } else if v4.is_private() {
                Some("a private-network address, which means something different on every network")
            } else if v4.is_link_local() {
                Some("a link-local address, the range the cloud metadata service also sits in")
            } else {
                None
            }
        }
        IpAddr::V6(v6) => {
            let first = v6.segments()[0];
            if v6.is_loopback() {
                Some("a loopback address, which is only ever this machine")
            } else if v6.is_unspecified() {
                Some("the unspecified address, which names no host at all")
            // `::ffff:127.0.0.1` and the older `::127.0.0.1` are both the
            // loopback wearing a different spelling. Asked after the two
            // above, because `::1` also carries an IPv4 tail and it is not
            // 0.0.0.1.
            } else if let Some(v4) = v6.to_ipv4() {
                classify(IpAddr::V4(v4))
            } else if first & 0xfe00 == 0xfc00 {
                Some("a unique-local address, which means something different on every network")
            } else if first & 0xffc0 == 0xfe80 {
                Some("a link-local address, the range the cloud metadata service also sits in")
            } else {
                None
            }
        }
    }
}

/// The historical `inet_aton` spellings of an IPv4 address: one to four parts,
/// each decimal, octal or hexadecimal, with the last part filling whatever
/// bytes are left over.
///
/// Used only for a host taken from a URL. It is what browsers, curl and
/// Python's urllib all accept, so a check that reads only dotted quads has a
/// hole in it that anybody can walk through by accident.
fn inet_aton(host: &str) -> Option<Ipv4Addr> {
    let parts: Vec<&str> = host.split('.').collect();
    if parts.is_empty() || parts.len() > 4 {
        return None;
    }
    let mut values = Vec::with_capacity(parts.len());
    for part in &parts {
        let (digits, radix) =
            if let Some(hex) = part.strip_prefix("0x").or_else(|| part.strip_prefix("0X")) {
                (hex, 16)
            } else if part.len() > 1 && part.starts_with('0') {
                (&part[1..], 8)
            } else {
                (*part, 10)
            };
        if digits.is_empty() {
            return None;
        }
        values.push(u32::from_str_radix(digits, radix).ok()?);
    }
    // Every part but the last is one byte; the last fills the rest.
    let last = values.pop()?;
    if values.iter().any(|v| *v > 255) {
        return None;
    }
    let remaining_bytes = 4 - values.len();
    if remaining_bytes < 4 && last >= 1u32 << (8 * remaining_bytes) {
        return None;
    }
    let mut addr: u32 = 0;
    for (i, v) in values.iter().enumerate() {
        addr |= v << (8 * (3 - i));
    }
    Some(Ipv4Addr::from(addr | last))
}

/// The longest environment variable name a registry entry may declare.
///
/// The same 64 the `secrets` check uses, and for the same reason: no real
/// variable name is that long, so something over it almost certainly has a
/// value pasted into it.
const ENV_VAR_NAME_MAX: usize = 64;

/// Does this read as the NAME of an environment variable rather than a value?
///
/// Deliberately stricter than "contains no equals sign". The mistake this
/// catches is somebody writing the address itself where the variable's name
/// belongs, and `http://10.1.2.3:3000/api/analyze` carries no equals sign at
/// all. The shape below is what POSIX allows and what every shell exports.
fn is_env_var_name(name: &str) -> bool {
    if name.is_empty() || name.len() > ENV_VAR_NAME_MAX {
        return false;
    }
    let mut chars = name.chars();
    let first = chars.next().unwrap_or_default();
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

impl Entry {
    /// Every string that decides what actually gets executed, labelled the way
    /// the entry's own file spells it.
    ///
    /// `upstream` is included although nothing executes it: it is a URL, so
    /// reading it as one is safe, and a private address there would ship just
    /// as far. `notes` is excluded, because it is prose and a note explaining
    /// that this tool has no default endpoint would otherwise refuse the entry
    /// that explains it.
    fn invocation_fields(&self) -> Vec<(String, &str)> {
        let mut fields: Vec<(String, &str)> = Vec::new();
        if let Some(upstream) = &self.upstream {
            fields.push(("upstream".into(), upstream.as_str()));
        }
        if let Some(img) = &self.image {
            fields.push(("image.reference".into(), img.reference.as_str()));
        }
        if let Some(bin) = &self.binary {
            for (i, a) in bin.command.iter().enumerate() {
                fields.push((format!("binary.command[{i}]"), a.as_str()));
            }
            for (i, a) in bin.version_args.iter().enumerate() {
                fields.push((format!("binary.version_args[{i}]"), a.as_str()));
            }
        }
        if let Some(inv) = &self.invoke {
            for (i, a) in inv.argv.iter().enumerate() {
                fields.push((format!("invoke.argv[{i}]"), a.as_str()));
            }
            for (i, kv) in inv.env.iter().enumerate() {
                fields.push((format!("invoke.env[{i}]"), kv.as_str()));
            }
            if let Some(a) = &inv.adapter {
                fields.push(("invoke.adapter".into(), a.as_str()));
            }
            if let Some(e) = &inv.entrypoint {
                fields.push(("invoke.entrypoint".into(), e.as_str()));
            }
            if let Some(o) = &inv.output_file {
                fields.push(("invoke.output_file".into(), o.as_str()));
            }
        }
        if let Some(rt) = &self.roundtrip {
            for (i, a) in rt.embed_argv.iter().enumerate() {
                fields.push((format!("roundtrip.embed_argv[{i}]"), a.as_str()));
            }
            for (i, a) in rt.extract_argv.iter().enumerate() {
                fields.push((format!("roundtrip.extract_argv[{i}]"), a.as_str()));
            }
            if let Some(e) = &rt.entrypoint {
                fields.push(("roundtrip.entrypoint".into(), e.as_str()));
            }
        }
        fields
    }

    /// Refuses an entry that ships an address nobody outside one network can
    /// reach.
    ///
    /// `plugins/registry/detectors/stegashield.toml` carried
    /// `http://172.24.0.2:3000/api/analyze` as a default while the comment
    /// three lines above it explained why there must not be one. Two costs,
    /// and the smaller one is the leak: an entry that defaults to an address
    /// scores against whatever answers there, so a run can quietly measure a
    /// different service from the one the result names.
    fn check_no_shipped_address(&self, bad: &mut Vec<String>) {
        for (field, text) in self.invocation_fields() {
            if text.len() > INVOCATION_FIELD_MAX_BYTES {
                bad.push(format!(
                    "{}: {field} is {} bytes, over the {INVOCATION_FIELD_MAX_BYTES} byte \
                     limit this check reads, so it cannot be checked for a private \
                     address. Shorten it, or move what it carries into a script",
                    self.name,
                    text.len()
                ));
                continue;
            }
            for token in hosts_in(text) {
                if let Some(why) = unshippable_address(&token) {
                    bad.push(format!(
                        "{}: {field} names {}: {why}. A benchmark cannot ship \
                         an address: it scores against whatever answers there, and a \
                         reader of the result has no way to know what that was. Leave \
                         it out and have whoever runs it supply the address from the \
                         environment",
                        self.name, token.host
                    ));
                }
            }
        }
    }

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

        // A typo here is silent in the worst way: a tool declared for "MacOS"
        // or "darwin" matches nothing, so `doctor` reports it unsupported on
        // every machine in the world and the entry looks merely unlucky.
        for platform in &self.platforms {
            if !KNOWN_PLATFORMS.contains(&platform.as_str()) {
                bad.push(format!(
                    "platform {platform:?} is not one of {}. These are the \
                     names Rust's own std::env::consts::OS uses, because that \
                     is what the check compares against",
                    KNOWN_PLATFORMS.join(", ")
                ));
            }
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

            // A host entry runs the adapter and nothing else: there is no
            // command inside the image to fall back to, so an entry without
            // one names no program at all. Refused at load rather than
            // discovered as a self-test failure, because `list` and `describe`
            // would otherwise show a tool that can never be run.
            if !PARSERS.contains(&inv.parser.as_str()) {
                bad.push(format!(
                    "invoke.parser is {:?}, which is not a parser this build \
                     carries. The ones it does are: {}",
                    inv.parser,
                    PARSERS.join(", ")
                ));
            }

            if inv.host && inv.adapter.is_none() {
                bad.push(
                    "invoke.host is set but no adapter is declared, and a host \
                     invoke runs the adapter rather than anything in the image. \
                     Add adapter = \"plugins/adapters/....py\""
                        .into(),
                );
            }

            if let Some(name) = &inv.endpoint_env {
                if !inv.host {
                    bad.push(format!(
                        "invoke.endpoint_env names {name:?} but invoke.host is \
                         not set. Only a host adapter is handed this machine's \
                         environment, so the variable would be read by nothing; \
                         set host = true or drop the field"
                    ));
                }
                if !is_env_var_name(name) {
                    bad.push(format!(
                        "invoke.endpoint_env is {name:?}, which is not the name \
                         of an environment variable. This field names the \
                         variable that carries the service's address; the \
                         address itself must never be written here, because an \
                         address in a file is a default and a default is scored \
                         against whatever answers on it"
                    ));
                }
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

        self.check_no_shipped_address(&mut bad);

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
    ///
    /// Aligned against itself alone. A listing of several tools calls
    /// [`Self::cells`] and aligns them together, because a column wide enough
    /// for one entry is not wide enough for the next.
    pub fn summary(&self) -> String {
        crate::table::align(&[self.cells()]).remove(0)
    }

    /// The cells of this tool's listing row, unpadded.
    ///
    /// Padding is the listing's business rather than the entry's: the width a
    /// cell needs depends on the other rows being printed beside it, which an
    /// entry cannot know about itself.
    pub fn cells(&self) -> Vec<String> {
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
        // The route is named rather than left to be inferred from whether the
        // third column looks like a registry reference. It is the one thing on
        // this line that changes what the reader has to do next, and what the
        // run costs them in isolation.
        vec![
            self.name.clone(),
            self.licence.clone(),
            self.route().to_string(),
            format!("{how}{secrets}"),
        ]
    }

    /// `container` or `local`: how this tool gets run, in one word.
    pub fn route(&self) -> &'static str {
        match (&self.image, &self.binary) {
            (Some(_), _) => "container",
            (_, Some(_)) => "local",
            _ => "unset",
        }
    }
}

/// The subdirectory holding corpora rather than tools.
///
/// Corpora are a different type with different fields (see
/// [`crate::corpus`]), so the tool walk skips this directory and hands it to
/// the corpus loader instead. One registry to a user, two schemas underneath,
/// because a corpus has no image and a detector has no licence URL.
pub const CORPORA_DIR: &str = "corpora";

/// Every output parser a registry entry may name in `invoke.parser`.
///
/// The implementations live in `stegobench-plugin`, which is the layer above
/// this one and cannot be named from here. The list is therefore duplicated
/// on purpose, and held in line by a test beside those implementations that
/// asserts each of these names is handled and an invented one is not. The
/// duplication buys the check that matters: a typo is refused when the
/// registry loads rather than once per image, after the tool has been
/// launched, halfway through a corpus.
pub const PARSERS: &[&str] = &["zsteg", "stegexpose", "number", "stegcore"];

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
    /// Makes `invoke.adapter` absolute, against the tree the registry sits in.
    ///
    /// An entry names its adapter as `plugins/adapters/x.py`, and that is
    /// relative to the directory CONTAINING `plugins/`, never to wherever the
    /// command happened to be typed. Left relative it resolved against the
    /// current directory, which is the same defect the registry search and the
    /// fixture search were built to close, one layer further in.
    ///
    /// Measured 2026-09-29: from a directory that was not a checkout, `doctor`
    /// reported aletheia-rs and aletheia-spa BROKEN with "adapter
    /// plugins/adapters/aletheia_one.py not found". Both work. A scoring run
    /// would have failed the same way, which is the half that matters more:
    /// a self-test that cannot run is visible, and a detector that cannot be
    /// driven halfway through a corpus is an afternoon.
    ///
    /// A path that resolves nowhere is left exactly as written, so the refusal
    /// still quotes what the registry asked for rather than a path this
    /// function invented.
    fn absolutise_adapter(entry: &mut Entry, registry_dir: &Path) {
        let Some(invoke) = entry.invoke.as_mut() else {
            return;
        };
        let Some(rel) = invoke.adapter.as_deref() else {
            return;
        };
        let named = Path::new(rel);
        if named.is_absolute() {
            return;
        }
        // `<root>/plugins/registry` is the layout every shipped registry uses,
        // so `<root>` is two levels up. The current directory is tried second
        // rather than not at all, because a contributor running from the
        // checkout has always had it work and should not have to stop.
        let mut candidates = Vec::new();
        if let Some(root) = registry_dir.parent().and_then(Path::parent) {
            candidates.push(root.join(named));
        }
        candidates.push(named.to_path_buf());
        for candidate in candidates {
            if candidate.is_file() {
                if let Ok(abs) = candidate.canonicalize() {
                    invoke.adapter = Some(abs.display().to_string());
                }
                return;
            }
        }
    }

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
                    let mut entry: Entry =
                        toml::from_str(&text).map_err(|e| RegistryError::Parse {
                            path: p.display().to_string(),
                            source: e,
                        })?;
                    Self::absolutise_adapter(&mut entry, dir);
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
        Self::footprint_of(self.entries.values())
    }

    /// The same sum over any set of entries, so a filtered listing can report
    /// what IT costs.
    ///
    /// `list detectors` used to print this registry's whole footprint under
    /// seven of its thirteen tools, which reads as the cost of what is on the
    /// screen and is not. The shared-image rule still applies within whatever
    /// set is handed in: two detectors in one image are one image here.
    pub fn footprint_of<'a>(entries: impl Iterator<Item = &'a Entry>) -> Footprint {
        let mut seen: BTreeMap<&str, (u64, bool)> = BTreeMap::new();
        let mut binaries_mb = 0;
        let mut tools = 0usize;
        for e in entries {
            tools += 1;
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
            tools,
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

    /// A host invoke runs the adapter and nothing else, so an entry without
    /// one declares no program at all.
    #[test]
    fn a_host_invoke_without_an_adapter_is_refused() {
        let e = parse(
            "[image]\nreference = \"x@sha256:a\"\n\
             [invoke]\nhost = true\nargv = [\"{file}\"]\nparser = \"number\"",
        );
        let problems = e.validate().expect_err("a host entry with no adapter");
        assert!(
            problems
                .iter()
                .any(|p| p.contains("no adapter is declared")),
            "refused for the wrong reason: {problems:?}"
        );
    }

    #[test]
    fn a_host_invoke_with_an_adapter_and_an_endpoint_variable_is_valid() {
        let e = parse(&format!(
            "[image]\nreference = \"ghcr.io/x/y@sha256:{A_REAL_DIGEST}\"\nsize_mb = 1250\n\
             bundled = false\n\
             [invoke]\nhost = true\nadapter = \"plugins/adapters/x.py\"\n\
             argv = [\"{{adapter}}\", \"{{file}}\"]\nparser = \"number\"\n\
             endpoint_env = \"X_ENDPOINT\""
        ));
        assert_eq!(e.validate(), Ok(()));
        assert_eq!(
            e.invoke.unwrap().endpoint_env.as_deref(),
            Some("X_ENDPOINT")
        );
    }

    /// Only a host adapter is handed this machine's environment, so the field
    /// would be read by nothing. A field that silently does nothing is worse
    /// than one that is refused, because the entry looks configured.
    #[test]
    fn an_endpoint_variable_without_a_host_invoke_is_refused_rather_than_ignored() {
        let e = parse(&format!(
            "[image]\nreference = \"ghcr.io/x/y@sha256:{A_REAL_DIGEST}\"\n\
             [invoke]\nargv = [\"{{file}}\"]\nparser = \"number\"\n\
             endpoint_env = \"X_ENDPOINT\""
        ));
        let problems = e.validate().expect_err("endpoint_env without host");
        assert!(
            problems
                .iter()
                .any(|p| p.contains("invoke.host is not set")),
            "refused for the wrong reason: {problems:?}"
        );
    }

    /// The mistake the field exists to make impossible: the address written
    /// where the variable's name belongs. None of these carries an equals
    /// sign, so a "looks like a value" test of the `secrets` shape waves them
    /// all through.
    #[test]
    fn an_address_written_into_the_endpoint_variable_name_is_refused() {
        for written in [
            "http://10.1.2.3:3000/api/analyze",
            "10.1.2.3:3000",
            "detector.example.org",
            "",
            "X ENDPOINT",
            "9_LIVES",
        ] {
            let e = parse(&format!(
                "[image]\nreference = \"ghcr.io/x/y@sha256:{A_REAL_DIGEST}\"\n\
                 [invoke]\nhost = true\nadapter = \"plugins/adapters/x.py\"\n\
                 argv = [\"{{adapter}}\"]\nparser = \"number\"\n\
                 endpoint_env = \"{written}\""
            ));
            let problems = match e.validate() {
                Ok(()) => panic!("{written:?} was accepted as a variable name"),
                Err(problems) => problems,
            };
            assert!(
                problems
                    .iter()
                    .any(|p| p.contains("not the name of an environment variable")),
                "{written:?} was refused for the wrong reason: {problems:?}"
            );
        }
    }

    #[test]
    fn an_over_long_endpoint_variable_name_is_refused_too() {
        assert!(is_env_var_name(&"X".repeat(ENV_VAR_NAME_MAX)));
        assert!(!is_env_var_name(&"X".repeat(ENV_VAR_NAME_MAX + 1)));
    }

    #[test]
    fn ordinary_variable_names_are_accepted() {
        for name in ["X_ENDPOINT", "_PRIVATE", "A1", "STEGASHIELD_ENDPOINT"] {
            assert!(is_env_var_name(name), "{name} should be a valid name");
        }
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

/// An address a registry entry ships is an address a stranger's run reaches
/// for, so every spelling of one is worth a test here.
#[cfg(test)]
mod address_tests {
    use super::*;

    /// A valid host-adapter entry whose one env line is the caller's.
    fn with_env(kv: &str) -> Entry {
        toml::from_str(&format!(
            r#"
name = "x"
kind = "detector"
licence = "MIT"

[binary]
command = ["x"]
version_args = ["--version"]

[invoke]
host = true
adapter = "plugins/adapters/x.py"
argv = ["{{adapter}}", "{{file}}"]
env = ["{kv}"]
parser = "number"

[selftest]
must_detect = "a.png"
must_clear = "b.png"
"#
        ))
        .expect("parses")
    }

    fn refused(kv: &str) -> String {
        let problems = match with_env(kv).validate() {
            Ok(()) => String::new(),
            Err(problems) => problems.join("\n"),
        };
        assert!(
            problems.contains("cannot ship an address"),
            "{kv} was accepted, or refused for another reason: {problems}"
        );
        problems
    }

    fn accepted(kv: &str) {
        if let Err(problems) = with_env(kv).validate() {
            panic!("{kv} was refused: {problems:?}");
        }
    }

    #[test]
    fn every_unroutable_range_is_refused() {
        for address in [
            "127.0.0.1",
            "127.1.2.3",
            "10.0.0.5",
            "10.255.255.255",
            "172.16.0.1",
            "172.24.0.2",
            "172.31.255.254",
            "192.168.1.1",
            "169.254.1.1",
            "169.254.169.254",
            "0.0.0.0",
        ] {
            refused(&format!("ENDPOINT=http://{address}:3000/api/analyze"));
        }
    }

    /// `172.16.0.0/12` is the range a substring check gets wrong. `172.16.` as
    /// text misses `172.24.0.2`, which is the address that prompted this, and
    /// `172.` as text would refuse the public `172.217.x` that Google serves
    /// from. Both edges of the real range are tested, in both directions.
    #[test]
    fn the_twelve_bit_private_range_is_bounded_at_both_ends() {
        refused("ENDPOINT=http://172.16.0.0:8080/");
        refused("ENDPOINT=http://172.31.255.255:8080/");
        accepted("ENDPOINT=http://172.15.255.255:8080/");
        accepted("ENDPOINT=http://172.32.0.1:8080/");
        accepted("ENDPOINT=http://172.217.16.142:8080/");
    }

    #[test]
    fn a_public_address_or_a_hostname_passes() {
        accepted("ENDPOINT=http://93.184.216.34:3000/api/analyze");
        accepted("ENDPOINT=https://detector.example.org/api/analyze?v=2");
        accepted("PYTHONPATH=/opt/aletheia");
        accepted("OMP_NUM_THREADS=1");
    }

    /// Three IPv6 spellings of the same machine, and none of them contains a
    /// dot to hunt for.
    #[test]
    fn ipv6_loopback_is_refused_however_it_is_written() {
        refused("ENDPOINT=http://[::1]:3000/api/analyze");
        refused("ENDPOINT=http://[::ffff:127.0.0.1]:3000/");
        // The older IPv4-compatible spelling, which carries no `ffff` marker.
        refused("ENDPOINT=http://[::10.1.2.3]:3000/");
        refused("ENDPOINT=[::1]");
        refused("ENDPOINT=http://[fe80::1%25eth0]:3000/");
        refused("ENDPOINT=http://[fd00::2]:3000/");
        accepted("ENDPOINT=http://[2606:2800:220:1:248:1893:25c8:1946]:3000/");
    }

    /// The gap a dotted-quad check leaves. Every HTTP client reads all four of
    /// these as the loopback, and none of them looks like an address.
    #[test]
    fn a_numerically_encoded_host_in_a_url_is_decoded_rather_than_waved_through() {
        refused("ENDPOINT=http://2130706433/api/analyze");
        refused("ENDPOINT=http://0177.0.0.1/");
        refused("ENDPOINT=http://0x7f.0x0.0x0.0x1/");
        refused("ENDPOINT=http://127.1/");
        // 3232235777 is 192.168.0.1.
        refused("ENDPOINT=http://3232235777:8080/");
    }

    /// The same decoding outside a URL would refuse an ordinary number, and a
    /// check that fires on `--threads 2130706433` is one somebody turns off.
    #[test]
    fn a_bare_number_in_an_argument_is_a_number() {
        accepted("THREADS=2130706433");
        accepted("SEED=127.1");
    }

    #[test]
    fn a_host_hiding_behind_userinfo_or_a_long_path_is_still_found() {
        refused("ENDPOINT=http://user:pass@192.168.0.7:3000/api/analyze#top");
        refused("ENDPOINT=http://localhost:3000/a/b/c?d=e");
        refused("ENDPOINT=HTTP://LOCALHOST/");
        refused("ENDPOINT=http://api.localhost/");
    }

    /// Every field that decides what runs, not only `invoke.env`.
    #[test]
    fn argv_and_the_binary_command_are_checked_too() {
        let mut e = with_env("PYTHONPATH=/opt/x");
        e.invoke.as_mut().unwrap().argv =
            vec!["{adapter}".into(), "--api=http://10.1.2.3/x".into()];
        let problems = e.validate().expect_err("an address in argv is refused");
        assert!(
            problems.iter().any(|p| p.contains("invoke.argv[1]")),
            "the wrong field was named: {problems:?}"
        );

        let mut e = with_env("PYTHONPATH=/opt/x");
        e.binary.as_mut().unwrap().command = vec!["curl".into(), "http://127.0.0.1:9/".into()];
        let problems = e
            .validate()
            .expect_err("an address in the command is refused");
        assert!(
            problems.iter().any(|p| p.contains("binary.command[1]")),
            "the wrong field was named: {problems:?}"
        );
    }

    /// A refusal that does not say which entry, which line and which address
    /// sends the reader to grep for it, which is the moment the check stops
    /// being worth having.
    #[test]
    fn the_refusal_names_the_entry_the_field_and_the_address() {
        let problems = refused("STEGASHIELD_ENDPOINT=http://172.24.0.2:3000/api/analyze");
        assert!(problems.contains("x: "), "no entry name: {problems}");
        assert!(problems.contains("invoke.env[0]"), "no field: {problems}");
        assert!(problems.contains("172.24.0.2"), "no address: {problems}");
        assert!(
            problems.contains("private-network"),
            "no reason: {problems}"
        );
    }

    /// Skipping an over-long field would mean a file could opt out of the
    /// check by padding, so the entry is refused instead.
    #[test]
    fn a_field_too_long_to_read_is_refused_rather_than_skipped() {
        let mut e = with_env("PYTHONPATH=/opt/x");
        let padded = format!(
            "ENDPOINT=http://127.0.0.1/{}",
            "a".repeat(INVOCATION_FIELD_MAX_BYTES)
        );
        e.invoke.as_mut().unwrap().env = vec![padded];
        let problems = e.validate().expect_err("an unreadable field is refused");
        assert!(
            problems.iter().any(|p| p.contains("cannot be checked")),
            "got: {problems:?}"
        );
    }

    /// `upstream` is where somebody sends a reader to find the tool. An
    /// internal one is both unreachable and a disclosure.
    #[test]
    fn a_private_upstream_url_is_refused_although_nothing_runs_it() {
        let mut e = with_env("PYTHONPATH=/opt/x");
        e.upstream = Some("http://10.20.30.40:8080/registry/my-detector".into());
        let problems = e.validate().expect_err("a private upstream is refused");
        assert!(
            problems.iter().any(|p| p.contains("upstream")),
            "the wrong field was named: {problems:?}"
        );
    }

    /// The prose fields are excluded on purpose, and it has to stay that way:
    /// the comment explaining why there is no default endpoint would otherwise
    /// refuse the entry that carries it.
    #[test]
    fn a_note_explaining_the_rule_does_not_trip_the_rule() {
        let mut e = with_env("PYTHONPATH=/opt/x");
        e.notes = Some("There is no default, not even http://localhost:3000/.".into());
        assert_eq!(e.validate(), Ok(()));
    }

    /// The whole point is the shipped files, so the shipped files are the test.
    #[test]
    fn nothing_in_the_registry_this_repository_ships_carries_one() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/registry");
        let reg = Registry::load(&dir).unwrap_or_else(|e| {
            panic!("the shipped registry no longer loads: {e}");
        });
        assert!(
            reg.entries.len() >= 10,
            "only {} entries loaded from {}, so this test is not looking at the real registry",
            reg.entries.len(),
            dir.display()
        );
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

    /// A filtered listing reports what IT costs, not what the registry does.
    #[test]
    fn a_subset_is_summed_over_the_subset() {
        let r = reg(vec![
            ("a", "a@sha256:1", 100, true),
            ("b", "b@sha256:2", 200, true),
            ("c", "c@sha256:3", 400, true),
        ]);
        let whole = r.footprint();
        assert_eq!((whole.tools, whole.bundled_mb), (3, 700));

        let two = Registry::footprint_of(
            r.entries.values().filter(|e| e.name != "c"),
        );
        assert_eq!(
            (two.tools, two.bundled_mb, two.unique_images),
            (2, 300, 2),
            "a subset that reports the whole registry's cost is the bug this              guards"
        );
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

    /// The worked example in the README is the first registry entry anybody
    /// writes, because they copy it.
    ///
    /// It was wrong. `size_mb = 200` with `bundled = false` is refused, since
    /// the flag is derived from the size rather than chosen, so a newcomer
    /// following the documentation hit an error on their first attempt. A
    /// reviewer found it by trying the instructions rather than reading them.
    ///
    /// This parses the block out of the README itself, so the test is the
    /// document: editing the example without validating it fails here.
    #[test]
    fn the_readme_example_entry_is_one_the_registry_accepts() {
        let readme = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../README.md"),
        )
        .expect("the README is two levels up from this crate");

        let start = readme
            .find("name = \"my-detector\"")
            .expect("the README still carries a detector example");
        let block = &readme[start..];
        let end = block.find("```").expect("the example is a fenced block");
        // The example writes a digest as an ellipsis, because a real one is 64
        // characters of noise in the middle of an explanation.
        let body = block[..end].replace("sha256:...", &format!("sha256:{}", "a".repeat(64)));

        let entry: Entry = toml::from_str(&body).expect("the example parses as TOML");
        if let Err(problems) = entry.validate() {
            panic!("the README example is refused by the registry: {problems:?}");
        }
    }

    /// The same rule for the guide's worked example, for the same reason: the
    /// page walks a team who have never used this tool through writing their
    /// first entry, and they will copy the block rather than read it.
    #[test]
    fn the_http_guide_example_entry_is_one_the_registry_accepts() {
        let page = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../docs/guide/http-detector.md"),
        )
        .expect("the guide page is two levels up from this crate");

        let start = page
            .find("name = \"my-detector\"")
            .expect("the page still carries a detector example");
        let block = &page[start..];
        let end = block.find("```").expect("the example is a fenced block");
        let body = block[..end].replace("sha256:...", &format!("sha256:{}", "a".repeat(64)));

        let entry: Entry = toml::from_str(&body).expect("the example parses as TOML");
        if let Err(problems) = entry.validate() {
            panic!("the guide example is refused by the registry: {problems:?}");
        }
    }
}
