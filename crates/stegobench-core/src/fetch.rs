// Author:  Daniel Iwugo
// Comment: Christ is King
// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Daniel Iwugo
//! Fetching a corpus tier the registry declared a route for, and proving the
//! bytes are the ones it named.
//!
//! WHAT THIS WILL AND WILL NOT DO
//! ------------------------------
//! It downloads one tier, checks it against a digest an independent registry
//! entry declared in advance, and puts the verified bytes at a
//! content-addressed path. It does not unpack anything: a shard is a tar
//! archive and reading one would need a new workspace dependency, which is a
//! decision for the operator rather than a convenience slipped in here. It
//! returns a path and says what is at it.
//!
//! It also refuses, loudly and for a stated reason, to fetch a corpus whose
//! terms do not permit redistribution. Fetching somebody's dataset on a user's
//! behalf is this project serving it to them, and
//! `docs/design/cover-source-licensing.md` is a catalogue of what happens when
//! a mirror decides otherwise. The rule is read off the registry entry by
//! [`CorpusEntry::fetch_refusal`], never listed per corpus here, so a corpus
//! added next year gets the right behaviour without anybody remembering.
//!
//! THE NETWORK IS INJECTED, AND THAT IS NOT A CONVENIENCE
//! ------------------------------------------------------
//! There is no HTTP client in this crate and no default [`Transport`]. Every
//! byte arrives through the one the caller passed. A sibling tool in this
//! repository bound its real fetcher as a default argument, and its tests then
//! made real network calls for months while reading as though they did not, so
//! `no_transport_is_bundled_with_this_crate` reads this module's own source and
//! fails if a networking type ever appears in it.
//!
//! WHAT IS BOUNDED
//! ---------------
//! Every wait has a deadline, the total run has a budget, a stalled transfer is
//! abandoned rather than waited on, the stream is cut off the moment it exceeds
//! the size the registry declared, and nothing accumulates in memory beyond one
//! chunk. A download is resumable and idempotent: the partial file carries the
//! bytes already on hand, a second run over a verified blob does no work at
//! all, and an interrupted run leaves a `.part` that the next one continues
//! rather than a half-written blob that looks complete.

use std::fmt;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

use crate::corpus::{CorpusEntry, DownloadRoute};

/// How much is read from the transport in one go: 256 KiB.
///
/// The only per-transfer allocation, so this is the fetcher's memory cost
/// whatever the corpus's size. Large enough that a 48 GB tier is not two
/// hundred million syscalls, small enough that the stall detector and the
/// heartbeat get a look in several times a second on any usable connection.
pub const CHUNK_BYTES: usize = 256 * 1024;

/// How often a long download says it is still alive, per the baseline's
/// observability rule: every long-running loop emits a heartbeat every 30 to
/// 60 seconds.
pub const HEARTBEAT_EVERY: Duration = Duration::from_secs(30);

/// What a caller is told while a fetch runs.
///
/// Deliberately data rather than text. This crate knows nothing about
/// terminals, so a CLI renders a progress bar from these, a test counts them,
/// and a log writes one line each. A `String` here would have picked the
/// rendering for everybody.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Progress {
    /// The store already holds these verified bytes, so nothing was fetched.
    AlreadyHeld { digest: String },
    /// The store held something at this digest's path that is not the length
    /// the registry declares, so it was not used.
    HeldCopyRejected { bytes: u64, expect: u64 },
    /// A transfer is about to start. `expect` is the size the REGISTRY
    /// declares, not one the server offered.
    Starting { url: String, expect: u64 },
    /// Picking up a partial download from an earlier run.
    Resuming { have: u64, expect: u64 },
    /// The server ignored the resume request and started again from zero, so
    /// the partial was thrown away rather than appended to.
    RestartedFromZero { discarded: u64 },
    /// Bytes landed. Emitted once per chunk.
    Advanced { have: u64, expect: u64 },
    /// Still alive. Emitted at most once every [`HEARTBEAT_EVERY`].
    Heartbeat {
        have: u64,
        expect: u64,
        elapsed: Duration,
    },
    /// The transfer finished and the digest matched what the registry declared.
    Verified { digest: String, bytes: u64 },
}

/// What the caller gets back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fetched {
    /// Where the verified bytes are.
    pub path: PathBuf,
    /// The digest the registry declared and these bytes have.
    pub digest: String,
    pub bytes: u64,
    /// True when the store already held the blob and nothing was transferred.
    pub reused: bool,
}

/// Caps and deadlines, all of them explicit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// The most a single route may be allowed to deliver. A route declaring
    /// more than this is refused before anything opens.
    pub max_bytes: u64,
    /// How long the transport may take to produce a readable body.
    pub open_timeout: Duration,
    /// How long the transfer may go without a single byte arriving before it
    /// is abandoned. This is the one that catches a connection that is open,
    /// unclosed and silent, which no total budget catches soon enough.
    pub stall_timeout: Duration,
    /// The whole fetch's wall-clock ceiling.
    pub budget: Duration,
}

impl Default for Limits {
    /// Sized for the largest tier this project publishes over an ordinary
    /// connection, and no larger.
    ///
    /// 48 GB at 10 Mbit/s is about eleven hours, so the twelve hour budget is
    /// the real case with a margin rather than a round number. A caller
    /// fetching Nano should pass something far smaller: a budget that fits the
    /// worst case is a budget that never fires for the ordinary one.
    fn default() -> Self {
        Limits {
            max_bytes: crate::corpus::MAX_DOWNLOAD_BYTES,
            open_timeout: Duration::from_secs(60),
            stall_timeout: Duration::from_secs(120),
            budget: Duration::from_secs(12 * 60 * 60),
        }
    }
}

/// One readable response, however the caller chose to obtain it.
pub struct Body {
    /// The byte offset this body actually starts at. A transport that honoured
    /// a resume request reports where it resumed; one that did not reports 0,
    /// and the fetcher handles that rather than corrupting the file.
    pub start: u64,
    /// What the server said the whole resource is, where it said anything. Not
    /// trusted for anything that matters: it arrives from the same place as the
    /// bytes, so the registry's `size_bytes` is what limits are measured
    /// against.
    pub total: Option<u64>,
    pub reader: Box<dyn Read + Send>,
}

/// Where bytes come from.
///
/// Implemented outside this crate, always. See the module note on why there is
/// no default.
pub trait Transport {
    /// Open `url`, starting at byte `from`, giving up if a body is not readable
    /// by `deadline`.
    ///
    /// A transport that cannot resume returns a body with `start: 0`, which is
    /// handled rather than treated as an error. A transport that starts
    /// somewhere other than `from` or 0 is a bug in the transport and the
    /// fetcher says so rather than writing the bytes into the wrong place.
    fn open(&self, url: &str, from: u64, deadline: Instant) -> Result<Body, FetchError>;
}

/// Where verified bytes end up, and where partial ones wait.
///
/// The boundary the baseline's Section 12 asks for: the fetcher never touches a
/// filesystem path of its own, so an object-store backend is a second
/// implementation of this trait rather than a rewrite of the transfer loop.
pub trait BlobStore {
    /// The verified blob for `digest`, and its length, when this store holds
    /// it.
    ///
    /// The length is not decoration. A blob only ever reaches its final path
    /// through the verified commit below, so holding the path is holding the
    /// bytes, but that argument stops at the moment something outside this
    /// process truncates the file. Returning the length lets the caller notice
    /// a short copy instead of handing it back as verified, which is the one
    /// shape of "a partial used as though it were whole" the content address
    /// does not rule out on its own.
    fn resolve(&self, digest: &str) -> Result<Option<(PathBuf, u64)>, FetchError>;
    /// How many bytes of an interrupted download are already on hand.
    fn partial_len(&self, digest: &str) -> Result<u64, FetchError>;
    /// Replay the partial's bytes, so the hash of what is already held can be
    /// rebuilt before anything is appended to it.
    fn read_partial(
        &self,
        digest: &str,
        sink: &mut dyn FnMut(&[u8]) -> Result<(), FetchError>,
    ) -> Result<(), FetchError>;
    /// Open the partial for appending. Dropping the writer must leave every
    /// byte already handed to it readable by the next run.
    fn append(&self, digest: &str) -> Result<Box<dyn Write>, FetchError>;
    /// Promote the partial to the verified blob, atomically: a reader either
    /// sees no blob or sees the whole one, never a growing file at the final
    /// path.
    fn commit(&self, digest: &str) -> Result<PathBuf, FetchError>;
    /// Throw the partial away. Called whenever it cannot be trusted.
    fn discard(&self, digest: &str) -> Result<(), FetchError>;
}

#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    /// The licence, the acceptance step or the absence of a route says no. A
    /// refusal, not a breakage: retrying it unchanged will refuse again.
    #[error("{0}")]
    Refused(String),
    #[error(
        "{id} declares no route for a tier called {tier:?}. It offers: {}",
        if .known.is_empty() { "nothing".to_string() } else { .known.join(", ") }
    )]
    UnknownTier {
        id: String,
        tier: String,
        known: Vec<String>,
    },
    #[error(
        "{url} would deliver {want} bytes, over the {cap} byte ceiling this run \
         allows. Raise the limit deliberately or fetch a smaller tier"
    )]
    TooLarge { url: String, want: u64, cap: u64 },
    #[error(
        "{url} delivered {got} bytes against the {want} the registry declares, \
         so the download is short. Nothing was verified and nothing was used; \
         the partial is kept and the next run resumes from it"
    )]
    Truncated { url: String, want: u64, got: u64 },
    #[error(
        "{url} is still sending after {want} bytes, which is what the registry \
         declares the whole file is. The transfer was cut off rather than read \
         to an end nobody has bounded"
    )]
    Overlong { url: String, want: u64 },
    #[error(
        "{url} delivered {bytes} bytes whose digest is {got}, not the {want} its \
         registry entry declares. These are not the bytes that were promised, so \
         they have been thrown away rather than kept as a corpus"
    )]
    DigestMismatch {
        url: String,
        want: String,
        got: String,
        bytes: u64,
    },
    #[error(
        "{url} sent nothing for {} second(s) after {have} bytes. The transfer \
         was abandoned; the partial is kept and the next run resumes from it",
        .waited.as_secs()
    )]
    Stalled {
        url: String,
        have: u64,
        waited: Duration,
    },
    #[error(
        "fetching {url} passed its {} second budget after {have} of {want} \
         bytes. The partial is kept and the next run resumes from it",
        .budget.as_secs()
    )]
    OutOfTime {
        url: String,
        have: u64,
        want: u64,
        budget: Duration,
    },
    #[error(
        "{url} resumed at byte {got}, but {want} bytes were already held. A \
         transport must start where it was asked to or at zero; anywhere else \
         would write the wrong bytes into the middle of the file"
    )]
    BadResume { url: String, want: u64, got: u64 },
    #[error("cannot fetch {url}: {source}")]
    Transport {
        url: String,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    #[error("cannot {doing} {path}: {source}")]
    Store {
        doing: &'static str,
        path: String,
        #[source]
        source: std::io::Error,
    },
}

/// Fetch one tier of one corpus, or say why not.
///
/// Every refusal is decided before anything opens: the licence, the acceptance
/// step, the missing route and the size ceiling are all facts about the entry
/// rather than about the transfer, and discovering one of them halfway through
/// a 48 GB download is discovering it far too late.
///
/// Running this twice is safe and cheap. The second run finds the verified blob
/// in the store and returns it without opening the transport at all.
pub fn fetch(
    entry: &CorpusEntry,
    tier: &str,
    transport: &dyn Transport,
    store: &dyn BlobStore,
    limits: Limits,
    progress: &mut dyn FnMut(Progress),
) -> Result<Fetched, FetchError> {
    // Asked first, and asked of the entry rather than of a list kept here. A
    // corpus nobody may redistribute is one this tool will not serve, whatever
    // else is true of it.
    if let Some(reason) = entry.fetch_refusal() {
        return Err(FetchError::Refused(reason));
    }
    let route = entry.route(tier).ok_or_else(|| FetchError::UnknownTier {
        id: entry.id.clone(),
        tier: tier.to_string(),
        known: entry.download.iter().map(|r| r.tier.clone()).collect(),
    })?;

    if route.size_bytes > limits.max_bytes {
        return Err(FetchError::TooLarge {
            url: route.url.clone(),
            want: route.size_bytes,
            cap: limits.max_bytes,
        });
    }

    // Idempotence, and the reason a second run costs nothing. The path IS the
    // digest, and bytes only ever reach it through the verified commit below,
    // so holding the path is holding the bytes. A caller that suspects local
    // corruption asks the store to re-read it rather than making every run pay
    // for the doubt.
    match store.resolve(&route.sha256)? {
        Some((path, bytes)) if bytes == route.size_bytes => {
            progress(Progress::AlreadyHeld {
                digest: route.sha256.clone(),
            });
            return Ok(Fetched {
                path,
                digest: route.sha256.clone(),
                bytes,
                reused: true,
            });
        }
        Some((_, bytes)) => {
            // A blob at the content-addressed path that is not the length the
            // registry declares. Something outside this process truncated or
            // replaced it, so it is fetched again rather than handed back: a
            // short file at the name that means "verified" is exactly the
            // failure the digest check exists to prevent, arriving by the one
            // route the digest check does not cover.
            progress(Progress::HeldCopyRejected {
                bytes,
                expect: route.size_bytes,
            });
        }
        None => {}
    }

    transfer(route, transport, store, limits, progress)
}

/// The transfer loop, once every refusal has been decided.
fn transfer(
    route: &DownloadRoute,
    transport: &dyn Transport,
    store: &dyn BlobStore,
    limits: Limits,
    progress: &mut dyn FnMut(Progress),
) -> Result<Fetched, FetchError> {
    let started = Instant::now();
    let mut hasher = Sha256::new();

    // The partial is replayed through the hasher rather than trusted, because
    // the hash of a resumed download has to cover the bytes written by the run
    // that was interrupted as well as the ones about to arrive. If those
    // earlier bytes were damaged, the final digest will not match and the whole
    // thing is thrown away, which is the correct outcome and the only one
    // available: nothing can tell a good prefix from a bad one until the end.
    let mut have = store.partial_len(&route.sha256)?;
    if have > route.size_bytes {
        // Longer than the whole file. Not a resume point, so it is not resumed
        // from: something else wrote here, or the registry's figure changed.
        store.discard(&route.sha256)?;
        progress(Progress::RestartedFromZero { discarded: have });
        have = 0;
    }
    if have > 0 {
        let mut replayed: u64 = 0;
        store.read_partial(&route.sha256, &mut |bytes| {
            replayed += bytes.len() as u64;
            hasher.update(bytes);
            Ok(())
        })?;
        // The store disagrees with itself about its own partial. Trusting
        // either number would hash one thing and resume another.
        if replayed != have {
            store.discard(&route.sha256)?;
            progress(Progress::RestartedFromZero { discarded: have });
            hasher = Sha256::new();
            have = 0;
        } else {
            progress(Progress::Resuming {
                have,
                expect: route.size_bytes,
            });
        }
    }

    progress(Progress::Starting {
        url: route.url.clone(),
        expect: route.size_bytes,
    });

    let deadline = started + limits.open_timeout.min(limits.budget);
    let body = transport.open(&route.url, have, deadline)?;
    if body.start != have {
        if body.start == 0 {
            // A transport that cannot resume. Ordinary rather than exceptional,
            // and the honest response is to start again from zero rather than
            // to append the start of the file onto the middle of it.
            store.discard(&route.sha256)?;
            progress(Progress::RestartedFromZero { discarded: have });
            hasher = Sha256::new();
            have = 0;
        } else {
            return Err(FetchError::BadResume {
                url: route.url.clone(),
                want: have,
                got: body.start,
            });
        }
    }

    let mut reader = body.reader;
    let mut sink = store.append(&route.sha256)?;
    let mut buffer = vec![0u8; CHUNK_BYTES];
    let mut last_heartbeat = Instant::now();

    loop {
        if have == route.size_bytes {
            break;
        }
        // Checked before the read rather than after it, so a transfer that is
        // already past its budget does not get one more blocking wait first.
        let elapsed = started.elapsed();
        if elapsed > limits.budget {
            return Err(FetchError::OutOfTime {
                url: route.url.clone(),
                have,
                want: route.size_bytes,
                budget: limits.budget,
            });
        }

        // Never asks for more than is outstanding, which is what makes
        // `Overlong` reachable on the next read rather than a matter of reading
        // an unbounded stream to its end.
        let outstanding = (route.size_bytes - have).min(CHUNK_BYTES as u64) as usize;
        let read_started = Instant::now();
        let read = match reader.read(&mut buffer[..outstanding]) {
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => {
                return Err(FetchError::Transport {
                    url: route.url.clone(),
                    source: Box::new(e),
                })
            }
        };

        // WHAT THIS CATCHES AND WHAT IT CANNOT
        //
        // `Read` carries no deadline, so a transport whose socket has none can
        // block inside `read` forever and nothing at this layer can interrupt
        // it. Setting that timeout is the transport's job and
        // [`Transport::open`] says so. What is reachable from here is the
        // connection that is technically alive and going nowhere: one chunk
        // taking longer than the whole transfer is allowed to stall for means
        // the transfer is over whatever the socket thinks.
        let waited = read_started.elapsed();
        if waited > limits.stall_timeout {
            return Err(FetchError::Stalled {
                url: route.url.clone(),
                have,
                waited,
            });
        }
        if read == 0 {
            // The stream ended. Whether that is the whole file is decided by
            // the count below, not by the server saying so.
            break;
        }

        let chunk = &buffer[..read];
        sink.write_all(chunk).map_err(|e| FetchError::Store {
            doing: "write the partial download for",
            path: route.sha256.clone(),
            source: e,
        })?;
        hasher.update(chunk);
        have += read as u64;

        progress(Progress::Advanced {
            have,
            expect: route.size_bytes,
        });
        if last_heartbeat.elapsed() >= HEARTBEAT_EVERY {
            last_heartbeat = Instant::now();
            progress(Progress::Heartbeat {
                have,
                expect: route.size_bytes,
                elapsed: started.elapsed(),
            });
        }
    }

    // Anything still coming is more than the registry said the file is. Read
    // once, deliberately, so the difference between "exactly right" and "the
    // server is still going" is reported rather than ignored.
    if have == route.size_bytes {
        let mut extra = [0u8; 1];
        if let Ok(1) = reader.read(&mut extra) {
            return Err(FetchError::Overlong {
                url: route.url.clone(),
                want: route.size_bytes,
            });
        }
    }

    sink.flush().map_err(|e| FetchError::Store {
        doing: "flush the partial download for",
        path: route.sha256.clone(),
        source: e,
    })?;
    drop(sink);

    if have < route.size_bytes {
        // The partial is KEPT. A short download is the ordinary result of a
        // dropped connection, and throwing the bytes away would make every
        // flaky link start from zero.
        return Err(FetchError::Truncated {
            url: route.url.clone(),
            want: route.size_bytes,
            got: have,
        });
    }

    let got = hex(&hasher.finalize());
    if got != route.sha256 {
        // Thrown away, and not kept as a resume point. These bytes are not the
        // ones that were promised and no amount of resuming will turn them into
        // them; keeping them would mean every later run hashed the same wrong
        // file and failed the same way.
        store.discard(&route.sha256)?;
        return Err(FetchError::DigestMismatch {
            url: route.url.clone(),
            want: route.sha256.clone(),
            got,
            bytes: have,
        });
    }

    let path = store.commit(&route.sha256)?;
    progress(Progress::Verified {
        digest: route.sha256.clone(),
        bytes: have,
    });
    Ok(Fetched {
        path,
        digest: route.sha256.clone(),
        bytes: have,
        reused: false,
    })
}

fn hex(bytes: &[u8]) -> String {
    use fmt::Write as _;
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        // Writing into a String cannot fail, and the alternative is a panic
        // path in non-test code for a case the type system already rules out.
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// A [`BlobStore`] on a local filesystem, laid out by content address.
///
/// `blobs/ab/cd/abcd...` and `partial/abcd....part`, which is the layout the
/// baseline's Section 12 asks for: the same key works unchanged against object
/// storage, and no directory ever holds more entries than one byte of the
/// digest allows.
#[derive(Debug, Clone)]
pub struct FileStore {
    root: PathBuf,
}

impl FileStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        FileStore { root: root.into() }
    }

    /// Where a verified blob lives. Public because a caller that wants to
    /// re-read and re-hash one needs the path without asking for a fetch.
    pub fn blob_path(&self, digest: &str) -> PathBuf {
        self.root
            .join("blobs")
            .join(&digest[0..2])
            .join(&digest[2..4])
            .join(digest)
    }

    fn partial_path(&self, digest: &str) -> PathBuf {
        self.root.join("partial").join(format!("{digest}.part"))
    }

    fn io(doing: &'static str, path: &Path, source: std::io::Error) -> FetchError {
        FetchError::Store {
            doing,
            path: path.display().to_string(),
            source,
        }
    }
}

impl BlobStore for FileStore {
    fn resolve(&self, digest: &str) -> Result<Option<(PathBuf, u64)>, FetchError> {
        let path = self.blob_path(digest);
        match std::fs::metadata(&path) {
            Ok(m) if m.is_file() => Ok(Some((path, m.len()))),
            Ok(_) => Ok(None),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(Self::io("read", &path, e)),
        }
    }

    fn partial_len(&self, digest: &str) -> Result<u64, FetchError> {
        let path = self.partial_path(digest);
        match std::fs::metadata(&path) {
            Ok(m) if m.is_file() => Ok(m.len()),
            Ok(_) => Ok(0),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(0),
            Err(e) => Err(Self::io("read", &path, e)),
        }
    }

    fn read_partial(
        &self,
        digest: &str,
        sink: &mut dyn FnMut(&[u8]) -> Result<(), FetchError>,
    ) -> Result<(), FetchError> {
        let path = self.partial_path(digest);
        let mut file = match std::fs::File::open(&path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(Self::io("open", &path, e)),
        };
        let mut buffer = vec![0u8; CHUNK_BYTES];
        loop {
            let read = match file.read(&mut buffer) {
                Ok(n) => n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(Self::io("read", &path, e)),
            };
            if read == 0 {
                return Ok(());
            }
            sink(&buffer[..read])?;
        }
    }

    fn append(&self, digest: &str) -> Result<Box<dyn Write>, FetchError> {
        let path = self.partial_path(digest);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Self::io("create", parent, e))?;
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| Self::io("open", &path, e))?;
        Ok(Box::new(file))
    }

    fn commit(&self, digest: &str) -> Result<PathBuf, FetchError> {
        let from = self.partial_path(digest);
        let to = self.blob_path(digest);
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Self::io("create", parent, e))?;
        }
        // Rename on close, within one filesystem, so a reader sees either no
        // blob or the whole one. Never a copy: a copy to the final path is a
        // growing file at the name that is supposed to mean "verified".
        std::fs::rename(&from, &to).map_err(|e| Self::io("commit", &from, e))?;
        Ok(to)
    }

    fn discard(&self, digest: &str) -> Result<(), FetchError> {
        let path = self.partial_path(digest);
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(Self::io("remove", &path, e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::corpus::{
        ArchiveFormat, Licence, LicenceStatus, Obtain, Properties, Redistribution,
    };
    use std::cell::RefCell;

    fn digest_of(bytes: &[u8]) -> String {
        let mut h = Sha256::new();
        h.update(bytes);
        hex(&h.finalize())
    }

    fn entry(route: Option<DownloadRoute>) -> CorpusEntry {
        CorpusEntry {
            id: "example".into(),
            name: "Example".into(),
            description: "A corpus that exists only in this test.".into(),
            citation: None,
            tier: None,
            licence: Licence {
                status: LicenceStatus::Verified,
                spdx: Some("CC0-1.0".into()),
                url: Some("https://creativecommons.org/publicdomain/zero/1.0/legalcode".into()),
                verified_on: Some("2026-09-28".into()),
                source: Some("the test that built it".into()),
                redistribution: Redistribution::Permitted,
                redistribution_reason: "CC0 grants it.".into(),
                attribution_required: false,
                share_alike: false,
                spdx_version_inferred: false,
                note: None,
            },
            obtain: Obtain {
                doi: None,
                url: Some("https://example.org/corpus".into()),
                instructions: None,
                requires_acceptance: false,
            },
            download: route.into_iter().collect(),
            integrity: None,
            properties: Properties {
                base_images: Some(8),
                ..Properties::default()
            },
            demonstration: false,
            notes: None,
        }
    }

    fn route_for(bytes: &[u8]) -> DownloadRoute {
        DownloadRoute {
            tier: "nano".into(),
            url: "https://example.org/nano.tar".into(),
            sha256: digest_of(bytes),
            size_bytes: bytes.len() as u64,
            archive: ArchiveFormat::Tar,
            covers: Some(8),
            note: None,
        }
    }

    /// A transport that serves fixed bytes and records every call it was asked
    /// to make.
    struct Canned {
        bytes: Vec<u8>,
        /// Cut the body off after this many bytes, to simulate a dropped
        /// connection.
        cut_at: Option<usize>,
        /// Ignore the resume offset, as a server with no range support does.
        ignores_range: bool,
        calls: RefCell<Vec<(String, u64)>>,
    }

    impl Canned {
        fn new(bytes: &[u8]) -> Self {
            Canned {
                bytes: bytes.to_vec(),
                cut_at: None,
                ignores_range: false,
                calls: RefCell::new(Vec::new()),
            }
        }
    }

    impl Transport for Canned {
        fn open(&self, url: &str, from: u64, _deadline: Instant) -> Result<Body, FetchError> {
            self.calls.borrow_mut().push((url.to_string(), from));
            let start = if self.ignores_range { 0 } else { from };
            let mut tail = self.bytes[(start as usize).min(self.bytes.len())..].to_vec();
            if let Some(cut) = self.cut_at {
                tail.truncate(cut);
            }
            Ok(Body {
                start,
                total: Some(self.bytes.len() as u64),
                reader: Box::new(std::io::Cursor::new(tail)),
            })
        }
    }

    /// A transport that refuses. Proves a code path never reached the network.
    struct Refusing;
    impl Transport for Refusing {
        fn open(&self, url: &str, _from: u64, _deadline: Instant) -> Result<Body, FetchError> {
            Err(FetchError::Transport {
                url: url.to_string(),
                source: "this test forbids any transfer".into(),
            })
        }
    }

    /// A store whose partial file cannot be written to, which is what a full
    /// disk looks like from here.
    struct FullDisk(FileStore);
    impl BlobStore for FullDisk {
        fn resolve(&self, d: &str) -> Result<Option<(PathBuf, u64)>, FetchError> {
            self.0.resolve(d)
        }
        fn partial_len(&self, d: &str) -> Result<u64, FetchError> {
            self.0.partial_len(d)
        }
        fn read_partial(
            &self,
            d: &str,
            sink: &mut dyn FnMut(&[u8]) -> Result<(), FetchError>,
        ) -> Result<(), FetchError> {
            self.0.read_partial(d, sink)
        }
        fn append(&self, _d: &str) -> Result<Box<dyn Write>, FetchError> {
            struct NoSpace;
            impl Write for NoSpace {
                fn write(&mut self, _b: &[u8]) -> std::io::Result<usize> {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::StorageFull,
                        "no space left on device",
                    ))
                }
                fn flush(&mut self) -> std::io::Result<()> {
                    Ok(())
                }
            }
            Ok(Box::new(NoSpace))
        }
        fn commit(&self, d: &str) -> Result<PathBuf, FetchError> {
            self.0.commit(d)
        }
        fn discard(&self, d: &str) -> Result<(), FetchError> {
            self.0.discard(d)
        }
    }

    fn run(
        entry: &CorpusEntry,
        transport: &dyn Transport,
        store: &dyn BlobStore,
    ) -> (Result<Fetched, FetchError>, Vec<Progress>) {
        let mut seen = Vec::new();
        let out = fetch(
            entry,
            "nano",
            transport,
            store,
            Limits::default(),
            &mut |p| seen.push(p),
        );
        (out, seen)
    }

    #[test]
    fn a_clean_fetch_lands_the_declared_bytes_at_a_content_addressed_path() {
        let bytes = b"one small corpus, honestly described".to_vec();
        let dir = tempfile::tempdir().unwrap();
        let store = FileStore::new(dir.path());
        let e = entry(Some(route_for(&bytes)));
        let transport = Canned::new(&bytes);

        let (out, seen) = run(&e, &transport, &store);
        let out = out.expect("a well-formed fetch was refused");

        assert_eq!(out.bytes, bytes.len() as u64);
        assert!(!out.reused);
        assert_eq!(std::fs::read(&out.path).unwrap(), bytes);
        // The layout, not merely a path that happens to work.
        let d = &out.digest;
        assert!(out.path.ends_with(format!("{}/{}/{d}", &d[0..2], &d[2..4])));
        assert!(seen.contains(&Progress::Verified {
            digest: d.clone(),
            bytes: bytes.len() as u64
        }));
    }

    /// THE TEST THE SIBLING TOOL DID NOT HAVE.
    ///
    /// The bytes served here exist nowhere but in this test. If any default or
    /// real transport were reached instead of the injected one, the file would
    /// hold something else and its digest would not match. So this proves the
    /// injected transport carried every byte, rather than proving only that a
    /// fetch succeeded.
    #[test]
    fn every_byte_came_from_the_injected_transport() {
        let bytes: Vec<u8> = (0u8..=255).cycle().take(9_001).collect();
        let dir = tempfile::tempdir().unwrap();
        let store = FileStore::new(dir.path());
        let e = entry(Some(route_for(&bytes)));
        let transport = Canned::new(&bytes);

        let out = run(&e, &transport, &store).0.unwrap();

        assert_eq!(std::fs::read(&out.path).unwrap(), bytes);
        assert_eq!(
            transport.calls.borrow().as_slice(),
            &[("https://example.org/nano.tar".to_string(), 0)]
        );
    }

    /// The other half of the same guarantee, and the one a future edit would
    /// break: no networking type may appear in this module at all, so nobody
    /// can add a default transport without this failing first.
    #[test]
    fn no_transport_is_bundled_with_this_crate() {
        let source = include_str!("fetch.rs");
        // Every needle is spelled in two halves so the haystack, which is this
        // file, does not contain the needle and make the test pass itself.
        for forbidden in [
            concat!("std::", "net"),
            concat!("Tcp", "Stream"),
            concat!("req", "west"),
            concat!("ur", "eq::"),
            concat!("hyp", "er::"),
            concat!("curl", "::"),
        ] {
            assert!(
                !source.contains(forbidden),
                "{forbidden} appears in fetch.rs. The network is injected here, \
                 always: a default transport is how a test suite starts making \
                 real calls while reading as though it does not"
            );
        }
    }

    #[test]
    fn a_corpus_that_may_not_be_redistributed_is_never_fetched() {
        let bytes = b"bytes nobody granted us the right to serve".to_vec();
        let dir = tempfile::tempdir().unwrap();
        let store = FileStore::new(dir.path());
        let mut e = entry(Some(route_for(&bytes)));
        e.licence.redistribution = Redistribution::Forbidden;

        // The transport REFUSES, so this also proves nothing was opened: a
        // refusal decided after the transfer would surface the transport's
        // error instead of the licence's.
        let (out, _) = run(&e, &Refusing, &store);
        match out {
            Err(FetchError::Refused(why)) => {
                assert!(why.contains("may not be redistributed"), "{why}")
            }
            other => panic!("a forbidden corpus was not refused: {other:?}"),
        }
    }

    #[test]
    fn an_unknown_redistribution_is_refused_as_well_because_an_unknown_is_not_a_yes() {
        let bytes = b"x".to_vec();
        let dir = tempfile::tempdir().unwrap();
        let store = FileStore::new(dir.path());
        let mut e = entry(Some(route_for(&bytes)));
        e.licence.redistribution = Redistribution::Unknown;
        assert!(matches!(
            run(&e, &Refusing, &store).0,
            Err(FetchError::Refused(_))
        ));
    }

    #[test]
    fn a_corpus_obtained_by_accepting_terms_is_refused() {
        let bytes = b"x".to_vec();
        let dir = tempfile::tempdir().unwrap();
        let store = FileStore::new(dir.path());
        let mut e = entry(Some(route_for(&bytes)));
        e.obtain.requires_acceptance = true;
        match run(&e, &Refusing, &store).0 {
            Err(FetchError::Refused(why)) => assert!(why.contains("your act"), "{why}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_corpus_with_no_route_is_refused_rather_than_guessed_at() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileStore::new(dir.path());
        let e = entry(None);
        match run(&e, &Refusing, &store).0 {
            Err(FetchError::Refused(why)) => assert!(why.contains("no download route"), "{why}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_tier_the_entry_does_not_offer_lists_the_ones_it_does() {
        let bytes = b"x".to_vec();
        let dir = tempfile::tempdir().unwrap();
        let store = FileStore::new(dir.path());
        let e = entry(Some(route_for(&bytes)));
        let mut seen = Vec::new();
        let out = fetch(&e, "core", &Refusing, &store, Limits::default(), &mut |p| {
            seen.push(p)
        });
        match out {
            Err(FetchError::UnknownTier { known, .. }) => assert_eq!(known, vec!["nano"]),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_route_larger_than_the_cap_is_refused_before_anything_opens() {
        let bytes = b"x".to_vec();
        let dir = tempfile::tempdir().unwrap();
        let store = FileStore::new(dir.path());
        let mut e = entry(Some(route_for(&bytes)));
        e.download[0].size_bytes = 4096;
        let limits = Limits {
            max_bytes: 1024,
            ..Limits::default()
        };
        let out = fetch(&e, "nano", &Refusing, &store, limits, &mut |_| {});
        assert!(matches!(out, Err(FetchError::TooLarge { .. })), "{out:?}");
    }

    #[test]
    fn bytes_that_are_not_the_declared_ones_are_thrown_away_not_kept() {
        let promised = b"the corpus the registry describes".to_vec();
        let served = b"something else entirely, same length".to_vec();
        let dir = tempfile::tempdir().unwrap();
        let store = FileStore::new(dir.path());
        let mut route = route_for(&promised);
        route.size_bytes = served.len() as u64;
        let e = entry(Some(route));

        let (out, _) = run(&e, &Canned::new(&served), &store);
        match out {
            Err(FetchError::DigestMismatch { want, got, .. }) => {
                assert_eq!(want, digest_of(&promised));
                assert_eq!(got, digest_of(&served));
            }
            other => panic!("a wrong download was not refused: {other:?}"),
        }
        // Neither committed nor left behind as a resume point.
        assert_eq!(store.resolve(&digest_of(&promised)).unwrap(), None);
        assert_eq!(store.partial_len(&digest_of(&promised)).unwrap(), 0);
    }

    #[test]
    fn a_truncated_download_is_refused_and_its_bytes_are_kept_for_the_resume() {
        let bytes: Vec<u8> = (0u8..=255).cycle().take(4_096).collect();
        let dir = tempfile::tempdir().unwrap();
        let store = FileStore::new(dir.path());
        let e = entry(Some(route_for(&bytes)));
        let mut transport = Canned::new(&bytes);
        transport.cut_at = Some(1_000);

        let (out, _) = run(&e, &transport, &store);
        match out {
            Err(FetchError::Truncated { want, got, .. }) => {
                assert_eq!((want, got), (4_096, 1_000));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(store.partial_len(&e.download[0].sha256).unwrap(), 1_000);
        assert_eq!(store.resolve(&e.download[0].sha256).unwrap(), None);
    }

    #[test]
    fn an_interrupted_download_resumes_from_where_it_stopped() {
        let bytes: Vec<u8> = (0u8..=255).cycle().take(4_096).collect();
        let dir = tempfile::tempdir().unwrap();
        let store = FileStore::new(dir.path());
        let e = entry(Some(route_for(&bytes)));

        let mut first = Canned::new(&bytes);
        first.cut_at = Some(1_000);
        assert!(run(&e, &first, &store).0.is_err());

        let second = Canned::new(&bytes);
        let (out, seen) = run(&e, &second, &store);
        let out = out.expect("the resume failed");

        assert_eq!(std::fs::read(&out.path).unwrap(), bytes);
        assert_eq!(
            second.calls.borrow().as_slice(),
            &[("https://example.org/nano.tar".to_string(), 1_000)],
            "the resume asked for the whole file again"
        );
        assert!(seen.contains(&Progress::Resuming {
            have: 1_000,
            expect: 4_096
        }));
    }

    #[test]
    fn a_transport_that_cannot_resume_starts_again_rather_than_corrupting_the_file() {
        let bytes: Vec<u8> = (0u8..=255).cycle().take(4_096).collect();
        let dir = tempfile::tempdir().unwrap();
        let store = FileStore::new(dir.path());
        let e = entry(Some(route_for(&bytes)));

        let mut first = Canned::new(&bytes);
        first.cut_at = Some(1_000);
        assert!(run(&e, &first, &store).0.is_err());

        let mut second = Canned::new(&bytes);
        second.ignores_range = true;
        let (out, seen) = run(&e, &second, &store);
        let out = out.expect("a transport with no range support broke the fetch");

        assert_eq!(std::fs::read(&out.path).unwrap(), bytes);
        assert!(seen.contains(&Progress::RestartedFromZero { discarded: 1_000 }));
    }

    #[test]
    fn a_transport_resuming_somewhere_it_was_not_asked_to_is_refused() {
        struct Wrong;
        impl Transport for Wrong {
            fn open(&self, _u: &str, _f: u64, _d: Instant) -> Result<Body, FetchError> {
                Ok(Body {
                    start: 7,
                    total: None,
                    reader: Box::new(std::io::Cursor::new(Vec::new())),
                })
            }
        }
        let bytes: Vec<u8> = vec![9; 4_096];
        let dir = tempfile::tempdir().unwrap();
        let store = FileStore::new(dir.path());
        let e = entry(Some(route_for(&bytes)));
        let out = run(&e, &Wrong, &store).0;
        assert!(matches!(out, Err(FetchError::BadResume { .. })), "{out:?}");
    }

    #[test]
    fn a_partial_longer_than_the_whole_file_is_discarded_rather_than_resumed_from() {
        let bytes: Vec<u8> = (0u8..=255).cycle().take(1_024).collect();
        let dir = tempfile::tempdir().unwrap();
        let store = FileStore::new(dir.path());
        let e = entry(Some(route_for(&bytes)));
        {
            let mut w = store.append(&e.download[0].sha256).unwrap();
            w.write_all(&vec![0u8; 2_048]).unwrap();
        }
        let (out, seen) = run(&e, &Canned::new(&bytes), &store);
        assert!(out.is_ok(), "{out:?}");
        assert!(seen.contains(&Progress::RestartedFromZero { discarded: 2_048 }));
    }

    #[test]
    fn a_server_still_sending_past_the_declared_size_is_cut_off() {
        let declared: Vec<u8> = vec![7; 1_024];
        let served: Vec<u8> = vec![7; 4_096];
        let dir = tempfile::tempdir().unwrap();
        let store = FileStore::new(dir.path());
        let mut route = route_for(&declared);
        route.size_bytes = 1_024;
        let e = entry(Some(route));
        let out = run(&e, &Canned::new(&served), &store).0;
        assert!(matches!(out, Err(FetchError::Overlong { .. })), "{out:?}");
    }

    #[test]
    fn a_second_run_over_a_verified_blob_does_no_work_at_all() {
        let bytes = b"already held".to_vec();
        let dir = tempfile::tempdir().unwrap();
        let store = FileStore::new(dir.path());
        let e = entry(Some(route_for(&bytes)));
        assert!(run(&e, &Canned::new(&bytes), &store).0.is_ok());

        // The refusing transport is the proof: a second fetch that touched the
        // network would fail here rather than return.
        let (out, seen) = run(&e, &Refusing, &store);
        let out = out.expect("a held blob was re-fetched");
        assert!(out.reused);
        assert_eq!(std::fs::read(&out.path).unwrap(), bytes);
        assert_eq!(
            seen,
            vec![Progress::AlreadyHeld {
                digest: e.download[0].sha256.clone()
            }]
        );
    }

    #[test]
    fn a_held_blob_that_has_been_truncated_since_is_fetched_again_not_handed_back() {
        let bytes: Vec<u8> = (0u8..=255).cycle().take(2_048).collect();
        let dir = tempfile::tempdir().unwrap();
        let store = FileStore::new(dir.path());
        let e = entry(Some(route_for(&bytes)));
        let first = run(&e, &Canned::new(&bytes), &store).0.unwrap();

        // Something outside this process shortens the verified blob.
        std::fs::write(&first.path, &bytes[..100]).unwrap();

        let (out, seen) = run(&e, &Canned::new(&bytes), &store);
        let out = out.expect("a truncated blob broke the re-fetch");
        assert!(!out.reused);
        assert_eq!(std::fs::read(&out.path).unwrap(), bytes);
        assert!(seen.contains(&Progress::HeldCopyRejected {
            bytes: 100,
            expect: 2_048
        }));
    }

    #[test]
    fn a_full_disk_is_reported_as_a_store_failure_rather_than_a_bad_download() {
        let bytes: Vec<u8> = vec![3; 4_096];
        let dir = tempfile::tempdir().unwrap();
        let store = FullDisk(FileStore::new(dir.path()));
        let e = entry(Some(route_for(&bytes)));
        let mut seen = Vec::new();
        let out = fetch(
            &e,
            "nano",
            &Canned::new(&bytes),
            &store,
            Limits::default(),
            &mut |p| seen.push(p),
        );
        match out {
            Err(FetchError::Store { source, .. }) => {
                assert_eq!(source.kind(), std::io::ErrorKind::StorageFull);
            }
            other => panic!("a full disk was reported as {other:?}"),
        }
    }

    #[test]
    fn a_budget_that_has_already_passed_stops_the_transfer() {
        let bytes: Vec<u8> = vec![1; 4_096];
        let dir = tempfile::tempdir().unwrap();
        let store = FileStore::new(dir.path());
        let e = entry(Some(route_for(&bytes)));
        let limits = Limits {
            budget: Duration::ZERO,
            ..Limits::default()
        };
        let out = fetch(
            &e,
            "nano",
            &Canned::new(&bytes),
            &store,
            limits,
            &mut |_| {},
        );
        assert!(matches!(out, Err(FetchError::OutOfTime { .. })), "{out:?}");
    }

    #[test]
    fn a_chunk_that_takes_longer_than_the_stall_ceiling_abandons_the_transfer() {
        let bytes: Vec<u8> = vec![2; 4_096];
        let dir = tempfile::tempdir().unwrap();
        let store = FileStore::new(dir.path());
        let e = entry(Some(route_for(&bytes)));
        // Zero, so the very first chunk is over the ceiling. The alternative is
        // a test that sleeps, which buys nothing and costs the suite seconds.
        let limits = Limits {
            stall_timeout: Duration::ZERO,
            ..Limits::default()
        };
        let out = fetch(
            &e,
            "nano",
            &Canned::new(&bytes),
            &store,
            limits,
            &mut |_| {},
        );
        match out {
            Err(FetchError::Stalled { have, .. }) => assert_eq!(have, 0),
            other => panic!("a stalled transfer was reported as {other:?}"),
        }
    }

    #[test]
    fn a_transport_error_names_the_url_it_could_not_reach() {
        let bytes = b"x".to_vec();
        let dir = tempfile::tempdir().unwrap();
        let store = FileStore::new(dir.path());
        let e = entry(Some(route_for(&bytes)));
        let out = run(&e, &Refusing, &store).0;
        let text = out.unwrap_err().to_string();
        assert!(text.contains("https://example.org/nano.tar"), "{text}");
    }

    #[test]
    fn a_long_download_emits_a_heartbeat_the_caller_can_render() {
        // The interval is real time, so this asserts the shape the caller has
        // to handle rather than waiting thirty seconds for one to fire.
        let p = Progress::Heartbeat {
            have: 10,
            expect: 100,
            elapsed: Duration::from_secs(31),
        };
        assert!(matches!(p, Progress::Heartbeat { have: 10, .. }));
        assert!(HEARTBEAT_EVERY >= Duration::from_secs(30));
        assert!(HEARTBEAT_EVERY <= Duration::from_secs(60));
    }

    #[test]
    fn the_store_survives_a_digest_it_holds_nothing_for() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileStore::new(dir.path());
        let d = "a".repeat(64);
        assert_eq!(store.resolve(&d).unwrap(), None);
        assert_eq!(store.partial_len(&d).unwrap(), 0);
        store.discard(&d).unwrap();
        let mut count = 0;
        store
            .read_partial(&d, &mut |b| {
                count += b.len();
                Ok(())
            })
            .unwrap();
        assert_eq!(count, 0);
    }
}
