// Author:  Daniel Iwugo
// Comment: Christ is King
// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Daniel Iwugo
//! `stegobench fetch`: the command, the transport it runs over, and the one
//! line that offers a real corpus to somebody holding a demonstration one.
//!
//! WHERE THE NETWORK LIVES, AND WHY IT LIVES HERE
//! ----------------------------------------------
//! `stegobench_core::fetch` holds the transfer, the resume, the digest check
//! and every refusal, and deliberately holds no networking code at all: a test
//! in that module reads its own source and fails if a networking type ever
//! appears in it. The transport is injected, always, and this file is where
//! the real one is written.
//!
//! WHAT CARRIES THE BYTES, AND WHY IT IS NOT A CRATE
//! -------------------------------------------------
//! The routes are HTTPS, and HTTPS means TLS, which the standard library does
//! not have. The two honest answers were a Rust HTTP client (one direct
//! dependency, roughly fifty transitive ones through a TLS stack) or the `curl`
//! already installed on every machine this tool runs on. This workspace keeps a
//! deliberately small dependency set (baseline Section 5) and a fetcher is the
//! one component whose dependencies handle bytes from a stranger, so the child
//! process won: it is a separate address space, it holds no credentials of
//! ours, and the bytes it hands back are checked against a digest an
//! independent registry entry declared in advance whatever it does.
//!
//! The cost is stated rather than hidden: `curl` has to be on PATH, and this
//! reports that as an unfit environment (exit 8) with the line to type rather
//! than failing halfway through a transfer.
//!
//! EVERY DEADLINE IS THE CHILD'S DEADLINE
//! --------------------------------------
//! `Read` carries no timeout, so a blocking read on a pipe cannot be
//! interrupted from this side. Every bound therefore goes to `curl` as a flag,
//! taken from the same [`Limits`] the core measures itself against rather than
//! from a second set invented here: the connect timeout from `open_timeout`,
//! `--max-time` from `budget`, and `--speed-limit 1 --speed-time` from
//! `stall_timeout`. When one fires, the child exits, the pipe closes, and the
//! core sees a short transfer and keeps the partial for the next run.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

use stegobench_core::corpus::CorpusEntry;
use stegobench_core::exit;
use stegobench_core::fetch::{Body, FetchError, Fetched, Limits, Progress, Transport};
use stegobench_core::registry::Registry;

/// The corpus a reader with nothing to score is pointed at.
///
/// Named rather than ranked. This is the corpus this harness was built to
/// score and the one whose figures the documentation quotes, so the offer is a
/// signpost to it, not a claim that it is the best entry in the registry. The
/// LINE is still generated from the registry entry, so what it offers follows
/// what that entry actually declares.
pub const OFFERED_CORPUS: &str = "pentimento-core";

/// Below this many images, a run is a demonstration rather than a measurement,
/// and the offer of a real corpus is the useful next line.
///
/// The shipped starter corpus is eighteen images. Pentimento's smallest tier is
/// 200 covers. Fifty sits between them and is well under anything somebody
/// would build by hand and quote.
pub const SMALL_RUN_IMAGES: u64 = 50;

/// How much header text is read before a response is treated as hostile.
const HEADER_CAP: usize = 64 * 1024;

/// How many header blocks one response may produce: redirect hops, plus the
/// informational `1xx` blocks a server may send first.
const MAX_HEADER_BLOCKS: usize = 12;

/// How often a terminal progress line is repainted.
const PAINT_EVERY: Duration = Duration::from_millis(200);

/// What went wrong, in the shape the command needs to report it.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The tool declines. `code` carries WHICH refusal it is, because a licence
    /// refusal and a missing download route are different facts and a caller
    /// that cannot tell them apart will chase the wrong one.
    #[error("{why}")]
    Refused { why: String, code: i32 },
    /// This machine has not got what it needs to fetch anything.
    #[error("{0}")]
    NoTransport(String),
    #[error(transparent)]
    Failed(FetchError),
}

impl Error {
    /// The process exit code, against the contract in `stegobench_core::exit`.
    ///
    /// Two of these differ from the shape somebody reading the error names
    /// might expect, and both differences are deliberate:
    ///
    /// * `Truncated` is a FAILURE (1) rather than a verification mismatch (5).
    ///   A short transfer is what a dropped connection looks like, the bytes on
    ///   hand are kept, and the next run resumes from them. Reporting it as a
    ///   mismatch would tell a caller the mirror is serving the wrong bytes
    ///   when the link merely dropped, and 5 is the code that says "these two
    ///   things are not about each other".
    /// * A missing download route is a pre-flight refusal (3) rather than a
    ///   licence refusal (7). Pentimento may be redistributed and asks nobody
    ///   to accept anything; what it has not got yet is a route. Calling that a
    ///   licence refusal would put a licence problem in front of a reader who
    ///   has none.
    pub fn exit_code(&self) -> i32 {
        match self {
            Error::Refused { code, .. } => *code,
            Error::NoTransport(_) => exit::ENVIRONMENT_UNFIT,
            Error::Failed(e) => match e {
                // Reached only if the core refused for a reason this file did
                // not anticipate. A refusal is a refusal, so it is reported as
                // one rather than as a failure somebody will retry forever.
                FetchError::Refused(_) => exit::PREFLIGHT_REFUSED,
                FetchError::UnknownTier { .. } | FetchError::TooLarge { .. } => exit::USAGE,
                FetchError::DigestMismatch { .. } | FetchError::Overlong { .. } => {
                    exit::VERIFY_MISMATCH
                }
                FetchError::Truncated { .. }
                | FetchError::Stalled { .. }
                | FetchError::OutOfTime { .. }
                | FetchError::BadResume { .. }
                | FetchError::Transport { .. }
                | FetchError::Store { .. } => exit::FAILURE,
            },
        }
    }
}

/// Why this corpus may not be fetched, and which refusal it is.
///
/// The RULE is not re-derived here: [`CorpusEntry::fetch_refusal`] decides, and
/// this only classifies the answer so the exit code carries the distinction.
/// Asked by the command before it looks for a transport at all, so a corpus
/// nobody may fetch is refused on a machine with no `curl` exactly as it is on
/// a machine with one.
pub fn refusal(entry: &CorpusEntry) -> Option<Error> {
    let why = entry.fetch_refusal()?;
    let code =
        if !entry.licence.redistribution.allows_publishing() || entry.obtain.requires_acceptance {
            exit::LICENCE_REFUSED
        } else {
            exit::PREFLIGHT_REFUSED
        };
    Some(Error::Refused { why, code })
}

/// Fetch one tier, rendering progress as it goes.
///
/// The transport is a parameter and there is no default, for the reason the
/// core module gives at length: a sibling tool in this repository bound its
/// real fetcher as a default argument and its tests made real network calls for
/// weeks while reading as though they did not.
pub fn run(
    entry: &CorpusEntry,
    tier: &str,
    transport: &dyn Transport,
    store: &dyn stegobench_core::fetch::BlobStore,
    limits: Limits,
    renderer: &mut Renderer<'_>,
) -> Result<Fetched, Error> {
    if let Some(refused) = refusal(entry) {
        return Err(refused);
    }
    let outcome = stegobench_core::fetch::fetch(entry, tier, transport, store, limits, &mut |p| {
        renderer.on(p)
    });
    renderer.finish();
    outcome.map_err(|e| match e {
        // The core reached a refusal this file's pre-flight did not, which
        // means the two disagree. The refusal still stands; it is classified
        // the same way rather than demoted to a generic failure.
        FetchError::Refused(why) => Error::Refused {
            why,
            code: refusal(entry).map_or(exit::PREFLIGHT_REFUSED, |r| r.exit_code()),
        },
        other => Error::Failed(other),
    })
}

/// The caps and deadlines for one run, from the defaults the core publishes.
///
/// Only the two a user can reasonably decide are exposed. Inventing a second
/// set of timeouts here is how a transport ends up waiting longer than the
/// transfer it belongs to is allowed to take.
pub fn limits(max_bytes: Option<u64>, budget_minutes: u64) -> Limits {
    let base = Limits::default();
    Limits {
        max_bytes: max_bytes.unwrap_or(base.max_bytes),
        // A zero-minute budget would refuse every transfer before its first
        // byte, which is a confusing way to report a mistyped flag.
        budget: Duration::from_secs(budget_minutes.max(1).saturating_mul(60)),
        ..base
    }
}

/// Where verified bytes are kept when nobody says.
///
/// A content-addressed store under this platform's per-user data directory, so
/// a second `fetch` of the same tier from a different working directory costs
/// nothing. Falls back to a path under the working directory only when there is
/// no home to put it in.
pub fn default_dest() -> PathBuf {
    match crate::registry::user_data_dir() {
        Some(dir) => dir.join("stegobench").join("corpora"),
        None => PathBuf::from("corpora").join("downloads"),
    }
}

/// What to say after a fetch that worked.
///
/// The bytes are verified and they are still a tar archive: the core does not
/// unpack, so the next line a reader needs is the one that does. Both `next`
/// lines carry angle-bracket placeholders rather than invented paths, for the
/// reason `needs.rs` gives: a reader who pastes one unchanged gets a shell
/// error rather than a wrong result.
pub fn describe_success(
    entry: &CorpusEntry,
    tier: &str,
    fetched: &Fetched,
    progress_error: Option<String>,
) -> (serde_json::Value, String) {
    // Serialised rather than printed from the Debug spelling, so the word here
    // is the word the registry file and the schema use. A JSON field that says
    // `targz` where every other document says `tar-gz` is a field a script
    // matches on and misses.
    let archive = entry.route(tier).and_then(|r| {
        serde_json::to_value(r.archive)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
    });
    let unpack = match archive.as_deref() {
        Some("tar") => Some(format!(
            "tar -xf {} -C <a directory>",
            fetched.path.display()
        )),
        Some("tar-gz") => Some(format!(
            "tar -xzf {} -C <a directory>",
            fetched.path.display()
        )),
        Some("tar-zst") => Some(format!(
            "tar --zstd -xf {} -C <a directory>",
            fetched.path.display()
        )),
        Some("zip") => Some(format!("unzip {} -d <a directory>", fetched.path.display())),
        _ => None,
    };

    let mut human = String::new();
    if fetched.reused {
        human.push_str(&format!(
            "{} {tier} was already here, and the copy is the length its \
             registry entry declares. Nothing was downloaded.\n",
            entry.id
        ));
    } else {
        human.push_str(&format!(
            "{} {tier} is here, and its digest is the one the registry \
             declared before the download started.\n",
            entry.id
        ));
    }
    human.push_str(&format!("  path    {}\n", fetched.path.display()));
    human.push_str(&format!("  bytes   {}\n", human_bytes(fetched.bytes)));
    human.push_str(&format!("  digest  {}\n", fetched.digest));
    if let Some(unpack) = &unpack {
        human.push_str(&format!(
            "\nThese are archive bytes, not a directory. Unpack them, then \
             score them:\n  {unpack}\n  stegobench score --corpus <that \
             directory> --detector <name> --corpus-id {}\n",
            entry.id
        ));
    }
    if let Some(e) = &progress_error {
        human.push_str(&format!(
            "\nProgress could not be written to this terminal ({e}). The \
             download itself was unaffected.\n"
        ));
    }
    human.pop();

    let json = serde_json::json!({
        "ok": true,
        "corpus": entry.id,
        "tier": tier,
        "path": fetched.path.display().to_string(),
        "digest": fetched.digest,
        "bytes": fetched.bytes,
        "reused": fetched.reused,
        "archive": archive,
        "unpack": unpack,
        "progress_error": progress_error,
    });
    (json, human)
}

/// One line offering a real corpus, or nothing.
///
/// THE RULE THIS OBEYS: a command printed here is a command that runs. The
/// offered corpus declares no download route today, so the line offers
/// `describe` rather than a `fetch` that would refuse. The day a route lands in
/// the registry entry, the same code prints the `fetch` line with that route's
/// own size, because both halves are read off the entry rather than written
/// down here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    /// The command, exactly as typed.
    pub run: String,
    /// What typing it gets you.
    pub why: String,
}

impl Offer {
    /// The listing row: the command, a gutter, and what it gets you.
    pub fn line(&self) -> String {
        format!("{}    {}", self.run, self.why)
    }

    /// The command alone, for prose.
    ///
    /// The row above is two columns separated by a gutter, and a gutter is
    /// four spaces in the middle of a sentence. Spliced after "Next: " it also
    /// ended the sentence on the description's dangling tail, "to quote a
    /// number from". A sentence takes the command; the reason is already the
    /// sentence it sits in.
    pub fn command(&self) -> &str {
        &self.run
    }
}

pub fn offer(reg: &Registry) -> Option<Offer> {
    let entry = reg.corpora.get(OFFERED_CORPUS)?;
    let covers = entry
        .properties
        .base_images
        .map(|n| format!("{} covers", commas(n)))
        .unwrap_or_else(|| "a corpus".to_string());

    // The smallest tier with a route, because the offer is for somebody who has
    // nothing yet and 48 GB is not an introduction.
    let smallest = entry.download.iter().min_by_key(|r| r.size_bytes);
    if let (Some(route), None) = (smallest, entry.fetch_refusal()) {
        let size = human_bytes(route.size_bytes);
        let what = match route.covers {
            Some(n) => format!("{} covers, {size}", commas(n)),
            None => size,
        };
        return Some(Offer {
            run: format!("stegobench fetch {} --tier {}", entry.id, route.tier),
            why: what,
        });
    }
    Some(Offer {
        run: format!("stegobench describe {}", entry.id),
        // No route yet, so this says where to get it rather than pretending it
        // can be fetched.
        why: format!("{covers} with their licences attached, to quote a number from"),
    })
}

/// A number with thousands separators, so 10000 reads as a size rather than a
/// serial number.
fn commas(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Bytes as a person reads them, in the decimal units a download page uses.
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: &[(u64, &str)] = &[
        (1_000_000_000_000, "TB"),
        (1_000_000_000, "GB"),
        (1_000_000, "MB"),
        (1_000, "kB"),
    ];
    for (scale, name) in UNITS {
        if bytes >= *scale {
            return format!("{:.1} {name}", bytes as f64 / *scale as f64);
        }
    }
    format!("{bytes} bytes")
}

// ── progress ────────────────────────────────────────────────────────────────

/// Turns the core's typed [`Progress`] values into something a person reads.
///
/// WHY IT DOES NOT SCROLL
///
/// `Progress::Advanced` fires once per 256 KiB chunk, which is four thousand
/// times for a 1 GB tier. A line each would bury every message that matters. On
/// a terminal the moving figure is repainted over itself with a carriage
/// return, throttled so a fast link does not spend its time formatting; with
/// output redirected there are no carriage returns at all, only the milestones
/// and the heartbeat the baseline asks of any long-running loop.
pub struct Renderer<'a> {
    out: &'a mut dyn Write,
    interactive: bool,
    started: Instant,
    last_paint: Instant,
    open_line: bool,
    /// The first write that failed. Kept rather than dropped: a swallowed error
    /// is how a run reports nothing and looks finished.
    failed: Option<std::io::Error>,
}

impl<'a> Renderer<'a> {
    pub fn new(out: &'a mut dyn Write, interactive: bool) -> Self {
        let now = Instant::now();
        Renderer {
            out,
            interactive,
            started: now,
            // Far enough back that the first chunk paints immediately rather
            // than after the throttle interval.
            last_paint: now - PAINT_EVERY,
            open_line: false,
            failed: None,
        }
    }

    /// The first write error, if progress could not be shown.
    pub fn write_error(&self) -> Option<&std::io::Error> {
        self.failed.as_ref()
    }

    /// The progress callback. Every arm of [`Progress`] is answered, so a new
    /// one added upstream is a compile error here rather than silence.
    pub fn on(&mut self, p: Progress) {
        match p {
            Progress::AlreadyHeld { digest } => {
                self.line(&format!("already here, verified: {digest}"))
            }
            Progress::HeldCopyRejected { bytes, expect } => self.line(&format!(
                "the copy here is {} against the {} the registry declares, so \
                 it is not used. Fetching it again",
                human_bytes(bytes),
                human_bytes(expect)
            )),
            Progress::Starting { url, expect } => {
                self.line(&format!("fetching {} from {url}", human_bytes(expect)))
            }
            Progress::Resuming { have, expect } => self.line(&format!(
                "resuming at {} of {}",
                human_bytes(have),
                human_bytes(expect)
            )),
            Progress::RestartedFromZero { discarded } => self.line(&format!(
                "the server would not resume, so {} already here were \
                 discarded and the transfer starts again",
                human_bytes(discarded)
            )),
            Progress::Advanced { have, expect } => {
                if !self.interactive || self.last_paint.elapsed() < PAINT_EVERY {
                    return;
                }
                self.last_paint = Instant::now();
                let text = format!(
                    "  {} of {}  {}%  {}",
                    human_bytes(have),
                    human_bytes(expect),
                    percent(have, expect),
                    rate(have, self.started.elapsed())
                );
                self.paint(&text);
            }
            Progress::Heartbeat {
                have,
                expect,
                elapsed,
            } => {
                // Printed as a permanent line even on a terminal. It is the
                // thirty-second proof of life, and a line somebody can paste
                // into a bug report is worth more than one more repaint.
                self.line(&format!(
                    "still going: {} of {}  {}%  {} elapsed  {}",
                    human_bytes(have),
                    human_bytes(expect),
                    percent(have, expect),
                    duration(elapsed),
                    rate(have, elapsed)
                ));
            }
            Progress::Verified { digest, bytes } => {
                self.line(&format!("{} verified against {digest}", human_bytes(bytes)))
            }
        }
    }

    /// Closes any repainted line, so whatever prints next starts cleanly.
    pub fn finish(&mut self) {
        if self.open_line {
            self.write("\n");
            self.open_line = false;
        }
    }

    fn paint(&mut self, text: &str) {
        // Trailing spaces, because a shorter line painted over a longer one
        // leaves the tail of the old one on screen.
        self.write(&format!("\r{text}          "));
        self.open_line = true;
    }

    fn line(&mut self, text: &str) {
        self.finish();
        self.write(&format!("{text}\n"));
    }

    fn write(&mut self, text: &str) {
        if self.failed.is_some() {
            return;
        }
        if let Err(e) = self
            .out
            .write_all(text.as_bytes())
            .and_then(|()| self.out.flush())
        {
            self.failed = Some(e);
        }
    }
}

fn percent(have: u64, expect: u64) -> u64 {
    if expect == 0 {
        return 0;
    }
    have.saturating_mul(100) / expect
}

fn rate(have: u64, elapsed: Duration) -> String {
    let seconds = elapsed.as_secs_f64();
    if seconds < 0.5 || have == 0 {
        return "measuring".to_string();
    }
    format!("{}/s", human_bytes((have as f64 / seconds) as u64))
}

fn duration(d: Duration) -> String {
    let s = d.as_secs();
    if s < 60 {
        return format!("{s}s");
    }
    if s < 3600 {
        return format!("{}m{:02}s", s / 60, s % 60);
    }
    format!("{}h{:02}m", s / 3600, (s % 3600) / 60)
}

// ── the transport ───────────────────────────────────────────────────────────

/// Bytes over HTTPS, carried by the `curl` already on the machine.
pub struct Curl {
    program: PathBuf,
    limits: Limits,
}

impl Curl {
    /// Find `curl`, or say what is missing and how to fix it.
    pub fn find(limits: Limits) -> Result<Self, Error> {
        match stegobench_plugin::which("curl") {
            Some(program) => Ok(Curl { program, limits }),
            None => Err(Error::NoTransport(
                "curl is not on PATH, and it is what carries the bytes. It \
                 ships with macOS and with Windows 10 and later; on Debian and \
                 Ubuntu it is `sudo apt install curl`. Nothing was downloaded."
                    .into(),
            )),
        }
    }

    /// The exact argument list, built where a test can read it.
    ///
    /// Three of these are load-bearing rather than tidy. `--proto =https` and
    /// `--proto-redir =https` mean a redirect cannot walk the transfer down to
    /// plain HTTP or sideways into `file://`. `--url` means a URL beginning
    /// with a dash is a URL rather than a flag. And every deadline is here
    /// because a blocking read on a pipe has none.
    ///
    /// `--include` rather than `--show-headers`, which is the same flag under
    /// the name curl 8.10 gave it: the old spelling is still accepted by the
    /// new curl and the new one is not accepted by anything older.
    fn argv(&self, url: &str, from: u64, connect: Duration) -> Vec<String> {
        let mut args: Vec<String> = vec![
            "--silent".into(),
            "--show-error".into(),
            "--location".into(),
            "--max-redirs".into(),
            "5".into(),
            "--proto".into(),
            "=https".into(),
            "--proto-redir".into(),
            "=https".into(),
            "--tlsv1.2".into(),
            "--include".into(),
            "--connect-timeout".into(),
            seconds(connect),
            "--max-time".into(),
            seconds(self.limits.budget),
            "--speed-limit".into(),
            "1".into(),
            "--speed-time".into(),
            seconds(self.limits.stall_timeout),
        ];
        if from > 0 {
            args.push("--range".into());
            args.push(format!("{from}-"));
        }
        args.push("--url".into());
        args.push(url.to_string());
        args
    }
}

fn seconds(d: Duration) -> String {
    // Never zero: curl reads 0 as "no limit", which would turn a deadline into
    // its opposite.
    d.as_secs().max(1).to_string()
}

impl Transport for Curl {
    fn open(&self, url: &str, from: u64, deadline: Instant) -> Result<Body, FetchError> {
        let fail = |why: String| FetchError::Transport {
            url: url.to_string(),
            source: why.into(),
        };

        // Checked here as well as in the registry's own validation, because
        // this is the last point before a process is spawned and the cost of
        // the second check is one comparison.
        if !url.starts_with("https://") {
            return Err(fail(
                "only https routes are fetched. A plain http route cannot be \
                 checked against the certificate its registry entry trusts"
                    .into(),
            ));
        }
        let now = Instant::now();
        if deadline <= now {
            return Err(fail(
                "the deadline for opening this transfer had already passed \
                 before it began"
                    .into(),
            ));
        }
        let connect = deadline
            .saturating_duration_since(now)
            .min(self.limits.open_timeout);

        let mut child = Command::new(&self.program)
            .args(self.argv(url, from, connect))
            .stdin(Stdio::null())
            // Inherited on purpose: curl's own diagnosis of a TLS failure or a
            // refused connection is better than anything this could write, and
            // stderr is where this tool's diagnostics go anyway.
            .stderr(Stdio::inherit())
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|e| fail(format!("{} would not start: {e}", self.program.display())))?;

        let Some(stdout) = child.stdout.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(fail("the transfer produced no readable output".into()));
        };
        // Wrapped before anything else can fail, so every path from here kills
        // the child rather than leaving it running against a closed pipe.
        let mut piped = Piped {
            child: Some(child),
            stdout,
        };

        let mut pending: Vec<u8> = Vec::new();
        let mut head = read_head(&mut piped, &mut pending).map_err(&fail)?;
        let mut blocks = 1;
        while (100..200).contains(&head.status) || (300..400).contains(&head.status) {
            blocks += 1;
            if blocks > MAX_HEADER_BLOCKS {
                return Err(fail(format!(
                    "the server sent more than {MAX_HEADER_BLOCKS} header \
                     blocks without a body"
                )));
            }
            head = read_head(&mut piped, &mut pending).map_err(&fail)?;
        }

        let (start, total) = match head.status {
            200 => (0, head.value("content-length").and_then(|v| v.parse().ok())),
            206 => {
                let range = head.value("content-range");
                let start = range
                    .as_deref()
                    .and_then(content_range_start)
                    // A 206 with no usable Content-Range: the server says it
                    // honoured the request, so it is taken at its word and the
                    // core's own resume check catches it if the bytes say
                    // otherwise.
                    .unwrap_or(from);
                (start, range.as_deref().and_then(content_range_total))
            }
            416 => {
                return Err(fail(
                    "the server says the resume point is past the end of the \
                     file. Delete the partial download and run this again"
                        .into(),
                ))
            }
            status => {
                return Err(fail(format!(
                    "the server answered {status} rather than serving the file"
                )))
            }
        };

        Ok(Body {
            start,
            total,
            reader: Box::new(std::io::Cursor::new(pending).chain(piped)),
        })
    }
}

/// The child's stdout, with the child attached so that dropping the body ends
/// the transfer.
///
/// Without this, an abandoned fetch leaves a `curl` running against a pipe
/// nobody reads: it blocks on a full buffer and stays until the shell closes.
struct Piped {
    child: Option<Child>,
    stdout: ChildStdout,
}

impl Read for Piped {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.stdout.read(buf)
    }
}

impl Drop for Piped {
    fn drop(&mut self) {
        let Some(mut child) = self.child.take() else {
            return;
        };
        // A child that has already exited is the ordinary case, and killing one
        // is an error on some platforms. Both outcomes end the same way, with
        // the process reaped rather than left as a zombie, so neither is
        // reported: there is nobody left to report it to.
        match child.try_wait() {
            Ok(Some(_)) => {}
            _ => {
                let _ = child.kill();
            }
        }
        let _ = child.wait();
    }
}

/// One block of response headers.
#[derive(Debug)]
struct Head {
    status: u16,
    headers: Vec<(String, String)>,
}

impl Head {
    fn value(&self, name: &str) -> Option<String> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
    }
}

/// Reads one header block, leaving whatever came after it in `pending`.
///
/// Bounded at [`HEADER_CAP`], because a response that never sends a blank line
/// is otherwise an unbounded read into memory, and the thing on the other end
/// is a stranger.
fn read_head(reader: &mut dyn Read, pending: &mut Vec<u8>) -> Result<Head, String> {
    let mut buffer = [0u8; 4096];
    loop {
        if let Some((end, skip)) = find_blank_line(pending) {
            let block = String::from_utf8_lossy(&pending[..end]).to_string();
            pending.drain(..end + skip);
            return parse_head(&block);
        }
        if pending.len() > HEADER_CAP {
            return Err(format!(
                "the response sent more than {HEADER_CAP} bytes of headers \
                 without ending them"
            ));
        }
        match reader.read(&mut buffer) {
            Ok(0) => {
                return Err("the transfer ended before any response headers arrived. \
                     curl's own message above says why"
                    .into())
            }
            Ok(n) => pending.extend_from_slice(&buffer[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(format!("could not read the response: {e}")),
        }
    }
}

/// Where the headers end, and how many bytes the terminator took.
fn find_blank_line(bytes: &[u8]) -> Option<(usize, usize)> {
    let crlf = window(bytes, b"\r\n\r\n").map(|i| (i, 4));
    let lf = window(bytes, b"\n\n").map(|i| (i, 2));
    match (crlf, lf) {
        (Some(a), Some(b)) => Some(if a.0 <= b.0 { a } else { b }),
        (a, b) => a.or(b),
    }
}

fn window(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.len() > hay.len() {
        return None;
    }
    (0..=hay.len() - needle.len()).find(|&i| &hay[i..i + needle.len()] == needle)
}

fn parse_head(block: &str) -> Result<Head, String> {
    let mut lines = block.lines();
    let status_line = lines
        .next()
        .ok_or_else(|| "the response began with no status line".to_string())?;
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| format!("{status_line:?} is not an HTTP status line"))?;
    let headers = lines
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            Some((name.trim().to_lowercase(), value.trim().to_string()))
        })
        .collect();
    Ok(Head { status, headers })
}

/// `bytes 1000-4095/4096` to 1000.
fn content_range_start(value: &str) -> Option<u64> {
    value
        .trim()
        .strip_prefix("bytes ")?
        .split('-')
        .next()?
        .trim()
        .parse()
        .ok()
}

/// `bytes 1000-4095/4096` to 4096. A `*` length is no answer, not zero.
fn content_range_total(value: &str) -> Option<u64> {
    value.rsplit('/').next()?.trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};
    use stegobench_core::fetch::FileStore;

    /// The offer is rendered two ways and they are not interchangeable.
    ///
    /// Found by running the binary rather than by reading it: the epilogue
    /// after a small run said "Next: stegobench describe pentimento-core
    /// 10,000 covers with their licences attached, to quote a number from",
    /// with the listing gutter sitting in the middle of the sentence and the
    /// sentence ending on the description's dangling tail. A listing row is
    /// not a clause.
    #[test]
    fn the_prose_form_of_an_offer_is_a_command_and_not_a_listing_row() {
        let offer = offer(&shipped()).expect("the shipped registry makes an offer");

        assert!(
            !offer.command().contains("  "),
            "a run of spaces in a sentence: {:?}",
            offer.command()
        );
        assert!(
            offer.command().starts_with("stegobench "),
            "the prose form has to be the command: {:?}",
            offer.command()
        );
        let sentence = format!("Next: {}", offer.command());
        assert!(
            !sentence.contains("  "),
            "the epilogue reads as two columns: {sentence:?}"
        );

        // And the listing row keeps its gutter, so this cannot be satisfied by
        // flattening both forms into one.
        assert!(
            offer.line().contains("    "),
            "the listing row lost its gutter: {:?}",
            offer.line()
        );
    }

    fn shipped() -> Registry {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugins/registry");
        Registry::load(&dir).expect("the shipped registry loads")
    }

    fn digest_of(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(bytes);
        format!("{:x}", h.finalize())
    }

    /// A corpus that exists only in this test, with a route serving `bytes`.
    fn entry_with_route(bytes: &[u8]) -> CorpusEntry {
        let toml_text = format!(
            r#"
id = "example"
name = "Example"
description = "A corpus that exists only in this test."

[licence]
status = "verified"
spdx = "CC0-1.0"
url = "https://creativecommons.org/publicdomain/zero/1.0/legalcode"
verified_on = "2026-09-28"
source = "the test that built it"
redistribution = "permitted"
redistribution_reason = "CC0 grants it."
attribution_required = false
share_alike = false

[obtain]
url = "https://example.org/corpus"
requires_acceptance = false

[[download]]
tier = "nano"
url = "https://example.org/nano.tar"
sha256 = "{}"
size_bytes = {}
archive = "tar"
covers = 8

[properties]
base_images = 8
"#,
            digest_of(bytes),
            bytes.len()
        );
        toml::from_str(&toml_text).expect("the test corpus parses")
    }

    /// A transport that serves fixed bytes and records what it was asked for.
    struct Canned {
        bytes: Vec<u8>,
        calls: RefCell<Vec<(String, u64)>>,
    }

    impl Transport for Canned {
        fn open(&self, url: &str, from: u64, _deadline: Instant) -> Result<Body, FetchError> {
            self.calls.borrow_mut().push((url.to_string(), from));
            let tail = self.bytes[(from as usize).min(self.bytes.len())..].to_vec();
            Ok(Body {
                start: from,
                total: Some(self.bytes.len() as u64),
                reader: Box::new(std::io::Cursor::new(tail)),
            })
        }
    }

    /// A transport that refuses everything. Proves a path never opened one.
    struct Refusing;
    impl Transport for Refusing {
        fn open(&self, url: &str, _from: u64, _d: Instant) -> Result<Body, FetchError> {
            Err(FetchError::Transport {
                url: url.to_string(),
                source: "this test forbids any transfer".into(),
            })
        }
    }

    fn fetch_into(
        dir: &Path,
        entry: &CorpusEntry,
        tier: &str,
        transport: &dyn Transport,
    ) -> (Result<Fetched, Error>, String) {
        let store = FileStore::new(dir);
        let mut buffer: Vec<u8> = Vec::new();
        let out = {
            let mut renderer = Renderer::new(&mut buffer, false);
            run(
                entry,
                tier,
                transport,
                &store,
                limits(None, 60),
                &mut renderer,
            )
        };
        (out, String::from_utf8(buffer).expect("progress is text"))
    }

    /// THE TEST THE SIBLING TOOL DID NOT HAVE, at this layer.
    ///
    /// The bytes served exist nowhere but in this test, so a default or real
    /// transport reaching the network instead would land something else and the
    /// digest would not match. It proves the injected transport carried every
    /// byte rather than merely that a fetch succeeded.
    #[test]
    fn every_byte_came_from_the_injected_transport() {
        let bytes: Vec<u8> = (0u8..=255).cycle().take(9_001).collect();
        let dir = tempfile::tempdir().unwrap();
        let entry = entry_with_route(&bytes);
        let transport = Canned {
            bytes: bytes.clone(),
            calls: RefCell::new(Vec::new()),
        };

        let (out, progress) = fetch_into(dir.path(), &entry, "nano", &transport);
        let out = out.expect("a well-formed fetch was refused");

        assert_eq!(std::fs::read(&out.path).unwrap(), bytes);
        assert_eq!(
            transport.calls.borrow().as_slice(),
            &[("https://example.org/nano.tar".to_string(), 0)]
        );
        assert!(progress.contains("verified"), "{progress}");
    }

    /// The other half of that guarantee: nothing in the command's own path may
    /// build a transport of its own, so a test cannot be given a stub and quietly
    /// use something else.
    #[test]
    fn the_run_path_builds_no_transport_of_its_own() {
        let source = include_str!("fetch.rs");
        let start = source
            .find("pub fn run(")
            .expect("the run function is in this file");
        let body = &source[start..];
        let end = body.find("\n}\n").expect("the function ends");
        let body = &body[..end];
        // Spelled in halves so this file is not its own haystack.
        for forbidden in [concat!("Cu", "rl"), concat!("Com", "mand::new")] {
            assert!(
                !body.contains(forbidden),
                "{forbidden} appears inside `run`, which is how a test suite \
                 starts making real network calls while reading as though it \
                 does not"
            );
        }
    }

    /// A refusal is decided from the registry entry, before a transport is
    /// consulted at all: the refusing transport here would surface its own
    /// error instead if the order were wrong.
    #[test]
    fn a_corpus_that_may_not_be_redistributed_is_refused_as_a_licence_refusal() {
        let reg = shipped();
        let entry = reg.corpora.get("bossbase").expect("bossbase is registered");
        let dir = tempfile::tempdir().unwrap();
        let (out, _) = fetch_into(dir.path(), entry, "core", &Refusing);
        let err = out.expect_err("a corpus nobody may redistribute was fetched");
        assert_eq!(err.exit_code(), exit::LICENCE_REFUSED);
        assert!(
            err.to_string().contains("may not be redistributed"),
            "{err}"
        );
    }

    /// The distinction the exit code exists to carry. Pentimento may be
    /// redistributed and asks nobody to accept anything; what it has not got is
    /// a route, and that is a pre-flight refusal rather than a licence one.
    #[test]
    fn a_corpus_with_no_route_is_never_reported_as_a_licence_refusal() {
        let reg = shipped();
        let entry = reg
            .corpora
            .get(OFFERED_CORPUS)
            .expect("the offered corpus is registered");
        match refusal(entry) {
            None => assert!(
                !entry.download.is_empty(),
                "the corpus was not refused and declares no route"
            ),
            Some(e) => {
                assert_eq!(
                    e.exit_code(),
                    exit::PREFLIGHT_REFUSED,
                    "a missing route was reported as {}: {e}",
                    e.exit_code()
                );
                assert!(e.to_string().contains("no download route"), "{e}");
            }
        }
    }

    #[test]
    fn a_tier_nobody_declared_is_a_usage_error_and_names_the_ones_that_exist() {
        let bytes = b"x".to_vec();
        let dir = tempfile::tempdir().unwrap();
        let entry = entry_with_route(&bytes);
        let (out, _) = fetch_into(dir.path(), &entry, "core", &Refusing);
        let err = out.expect_err("an unknown tier was accepted");
        assert_eq!(err.exit_code(), exit::USAGE);
        assert!(err.to_string().contains("nano"), "{err}");
    }

    /// Bytes that are not the declared ones exit 5, and a link that dropped
    /// exits 1. The two are different events and a caller retries only one.
    #[test]
    fn a_wrong_download_and_a_dropped_one_are_not_the_same_exit_code() {
        let promised: Vec<u8> = (0u8..=255).cycle().take(4_096).collect();
        let served: Vec<u8> = vec![7u8; 4_096];
        let dir = tempfile::tempdir().unwrap();
        let entry = entry_with_route(&promised);
        let wrong = Canned {
            bytes: served,
            calls: RefCell::new(Vec::new()),
        };
        let (out, _) = fetch_into(dir.path(), &entry, "nano", &wrong);
        let err = out.expect_err("bytes with the wrong digest were accepted");
        assert_eq!(err.exit_code(), exit::VERIFY_MISMATCH);

        let short = Canned {
            bytes: promised[..1_000].to_vec(),
            calls: RefCell::new(Vec::new()),
        };
        let dir = tempfile::tempdir().unwrap();
        let (out, _) = fetch_into(dir.path(), &entry, "nano", &short);
        let err = out.expect_err("a short download was accepted");
        assert_eq!(
            err.exit_code(),
            exit::FAILURE,
            "a dropped connection was reported as a verification mismatch, \
             which tells a caller the mirror is serving wrong bytes"
        );
    }

    /// A partial is kept and the next run resumes from it, and nothing that
    /// looks verified is left behind in between.
    #[test]
    fn an_interrupted_fetch_leaves_something_resumable_and_nothing_verified() {
        let bytes: Vec<u8> = (0u8..=255).cycle().take(8_192).collect();
        let dir = tempfile::tempdir().unwrap();
        let entry = entry_with_route(&bytes);
        let short = Canned {
            bytes: bytes[..3_000].to_vec(),
            calls: RefCell::new(Vec::new()),
        };
        assert!(fetch_into(dir.path(), &entry, "nano", &short).0.is_err());

        let store = FileStore::new(dir.path());
        let digest = &entry.download[0].sha256;
        assert_eq!(
            stegobench_core::fetch::BlobStore::resolve(&store, digest).unwrap(),
            None,
            "an interrupted fetch left something at the path that means verified"
        );

        let whole = Canned {
            bytes: bytes.clone(),
            calls: RefCell::new(Vec::new()),
        };
        let (out, progress) = fetch_into(dir.path(), &entry, "nano", &whole);
        assert_eq!(std::fs::read(&out.unwrap().path).unwrap(), bytes);
        assert_eq!(whole.calls.borrow()[0].1, 3_000, "the resume started again");
        assert!(progress.contains("resuming"), "{progress}");
    }

    #[test]
    fn a_second_run_over_a_verified_blob_opens_nothing() {
        let bytes = b"already here".to_vec();
        let dir = tempfile::tempdir().unwrap();
        let entry = entry_with_route(&bytes);
        let transport = Canned {
            bytes: bytes.clone(),
            calls: RefCell::new(Vec::new()),
        };
        assert!(fetch_into(dir.path(), &entry, "nano", &transport).0.is_ok());

        let (out, progress) = fetch_into(dir.path(), &entry, "nano", &Refusing);
        assert!(out.expect("a held blob was fetched again").reused);
        assert!(progress.contains("already here"), "{progress}");
    }

    /// Redirected output must not carry a repainted line per chunk: the whole
    /// point of the throttle is that a log stays readable.
    #[test]
    fn progress_with_output_redirected_never_scrolls_and_never_repaints() {
        let bytes: Vec<u8> = (0u8..=255).cycle().take(600_000).collect();
        let dir = tempfile::tempdir().unwrap();
        let entry = entry_with_route(&bytes);
        let transport = Canned {
            bytes,
            calls: RefCell::new(Vec::new()),
        };
        let (_, progress) = fetch_into(dir.path(), &entry, "nano", &transport);
        assert!(!progress.contains('\r'), "{progress:?}");
        assert!(
            progress.lines().count() <= 4,
            "a redirected run printed {} lines:\n{progress}",
            progress.lines().count()
        );
    }

    #[test]
    fn progress_on_a_terminal_repaints_one_line_rather_than_printing_many() {
        let bytes: Vec<u8> = (0u8..=255).cycle().take(600_000).collect();
        let dir = tempfile::tempdir().unwrap();
        let entry = entry_with_route(&bytes);
        let store = FileStore::new(dir.path());
        let transport = Canned {
            bytes,
            calls: RefCell::new(Vec::new()),
        };
        let mut buffer: Vec<u8> = Vec::new();
        {
            let mut renderer = Renderer::new(&mut buffer, true);
            run(
                &entry,
                "nano",
                &transport,
                &store,
                limits(None, 60),
                &mut renderer,
            )
            .expect("the fetch failed");
        }
        let progress = String::from_utf8(buffer).unwrap();
        assert!(progress.contains('\r'), "{progress:?}");
        assert!(
            progress.matches('\n').count() <= 4,
            "a terminal run printed {} lines",
            progress.matches('\n').count()
        );
    }

    /// A terminal that cannot be written to is recorded rather than ignored.
    #[test]
    fn a_progress_write_that_fails_is_kept_rather_than_swallowed() {
        struct Broken;
        impl Write for Broken {
            fn write(&mut self, _b: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "nobody is reading",
                ))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut out = Broken;
        let mut renderer = Renderer::new(&mut out, false);
        renderer.on(Progress::Starting {
            url: "https://example.org/nano.tar".into(),
            expect: 10,
        });
        assert_eq!(
            renderer.write_error().map(|e| e.kind()),
            Some(std::io::ErrorKind::BrokenPipe)
        );
    }

    // ── the offer ───────────────────────────────────────────────────────────

    /// THE RULE: a command printed here is a command that runs.
    ///
    /// Parsed by the real command tree, and then checked against the registry
    /// it names, so the day a route lands the offer either becomes a fetch that
    /// would not be refused or stays the describe line.
    #[test]
    fn the_offer_names_a_command_that_runs() {
        let reg = shipped();
        let offer = offer(&reg).expect("the offered corpus is registered");
        let words: Vec<&str> = offer.run.split_whitespace().collect();
        assert_eq!(words[0], "stegobench");
        let parsed =
            <crate::cli::Cli as clap::Parser>::try_parse_from(&words).unwrap_or_else(|e| {
                panic!("the offer does not parse as a command: {e}\n{}", offer.run)
            });

        match parsed.command {
            Some(crate::cli::Command::Describe { name, .. }) => {
                assert!(reg.corpora.contains_key(&name), "{name} is not registered");
            }
            Some(crate::cli::Command::Fetch { corpus, tier, .. }) => {
                let entry = reg.corpora.get(&corpus).expect("the corpus is registered");
                assert!(
                    entry.route(&tier).is_some(),
                    "the offer names a tier {tier} that {corpus} does not declare"
                );
                assert!(
                    refusal(entry).is_none(),
                    "the offer prints a fetch that would be refused: {}",
                    refusal(entry).map(|e| e.to_string()).unwrap_or_default()
                );
            }
            _ => panic!("the offer is neither a describe nor a fetch: {}", offer.run),
        }
    }

    /// Today's shape, asserted rather than assumed: no route, so no fetch line.
    #[test]
    fn a_corpus_with_no_route_is_offered_as_a_describe_rather_than_a_fetch() {
        let reg = shipped();
        let entry = reg.corpora.get(OFFERED_CORPUS).unwrap();
        if !entry.download.is_empty() {
            return;
        }
        let offer = offer(&reg).unwrap();
        assert!(
            !offer.run.contains("fetch"),
            "a corpus with no route was offered as a fetch: {}",
            offer.line()
        );
        assert!(offer.line().contains("covers"), "{}", offer.line());
    }

    /// The other branch, over a registry built for it: a route that exists is
    /// offered as the fetch command, with that route's own size.
    #[test]
    fn a_corpus_with_a_route_is_offered_as_the_fetch_command_it_supports() {
        let bytes: Vec<u8> = vec![1u8; 2_000_000];
        let mut entry = entry_with_route(&bytes);
        entry.id = OFFERED_CORPUS.to_string();
        let mut reg = shipped();
        reg.corpora.insert(OFFERED_CORPUS.to_string(), entry);

        let offer = offer(&reg).expect("the corpus is registered");
        assert_eq!(
            offer.run,
            format!("stegobench fetch {OFFERED_CORPUS} --tier nano")
        );
        assert!(offer.why.contains("8 covers"), "{}", offer.why);
        assert!(offer.why.contains("2.0 MB"), "{}", offer.why);
    }

    #[test]
    fn a_registry_without_the_offered_corpus_offers_nothing_rather_than_guessing() {
        let mut reg = shipped();
        reg.corpora.remove(OFFERED_CORPUS);
        assert_eq!(offer(&reg), None);
    }

    // ── the transport, without a network ────────────────────────────────────

    fn curl() -> Curl {
        Curl {
            program: PathBuf::from("/nonexistent/curl"),
            limits: limits(None, 10),
        }
    }

    #[test]
    fn the_transport_refuses_anything_that_is_not_https_without_spawning() {
        let out = curl().open(
            "http://example.org/x.tar",
            0,
            Instant::now() + Duration::from_secs(5),
        );
        match out {
            Err(e) => assert!(e.to_string().contains("https"), "{e}"),
            Ok(_) => panic!("a plain http route was accepted"),
        }
    }

    #[test]
    fn a_deadline_that_has_already_passed_opens_nothing() {
        let out = curl().open("https://example.org/x.tar", 0, Instant::now());
        assert!(out.is_err());
    }

    /// The flags are the security posture, so they are asserted rather than
    /// trusted to stay there.
    #[test]
    fn the_transport_pins_the_protocol_and_every_deadline() {
        let args = curl().argv("https://example.org/x.tar", 0, Duration::from_secs(30));
        let joined = args.join(" ");
        assert!(joined.contains("--proto =https"), "{joined}");
        assert!(joined.contains("--proto-redir =https"), "{joined}");
        assert!(joined.contains("--connect-timeout 30"), "{joined}");
        assert!(joined.contains("--max-time 600"), "{joined}");
        assert!(joined.contains("--speed-time 120"), "{joined}");
        // The URL is a value of --url, never a bare argument that could be read
        // as a flag.
        assert_eq!(
            args[args.len() - 2..],
            ["--url".to_string(), "https://example.org/x.tar".to_string()]
        );
        assert!(!joined.contains("--range"), "a fresh fetch asked to resume");
    }

    #[test]
    fn a_resume_asks_for_the_bytes_that_are_missing() {
        let args = curl().argv("https://example.org/x.tar", 1_000, Duration::from_secs(5));
        let at = args.iter().position(|a| a == "--range").expect("a range");
        assert_eq!(args[at + 1], "1000-");
    }

    #[test]
    fn no_deadline_is_ever_passed_to_curl_as_zero_which_would_mean_forever() {
        let short = Curl {
            program: PathBuf::from("/nonexistent/curl"),
            limits: Limits {
                budget: Duration::from_millis(1),
                stall_timeout: Duration::from_millis(1),
                ..Limits::default()
            },
        };
        let args = short.argv("https://example.org/x.tar", 0, Duration::from_millis(1));
        assert!(!args.iter().any(|a| a == "0"), "{args:?}");
    }

    // ── response headers ────────────────────────────────────────────────────

    fn head_of(text: &str) -> (Head, Vec<u8>) {
        let mut reader = std::io::Cursor::new(text.as_bytes().to_vec());
        let mut pending = Vec::new();
        let head = read_head(&mut reader, &mut pending).expect("the headers parse");
        (head, pending)
    }

    #[test]
    fn a_plain_response_starts_at_zero_and_leaves_the_body_untouched() {
        let (head, pending) = head_of("HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\nbody");
        assert_eq!(head.status, 200);
        assert_eq!(head.value("content-length").as_deref(), Some("4"));
        assert_eq!(pending, b"body");
    }

    #[test]
    fn a_partial_response_reports_where_the_server_actually_resumed() {
        let (head, _) =
            head_of("HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 1000-4095/4096\r\n\r\n");
        assert_eq!(head.status, 206);
        let range = head.value("content-range").unwrap();
        assert_eq!(content_range_start(&range), Some(1_000));
        assert_eq!(content_range_total(&range), Some(4_096));
    }

    #[test]
    fn an_unknown_content_range_length_is_no_answer_rather_than_zero() {
        assert_eq!(content_range_total("bytes 0-99/*"), None);
    }

    #[test]
    fn headers_that_never_end_are_cut_off_rather_than_read_into_memory() {
        let flood = vec![b'x'; HEADER_CAP + 4_096];
        let mut reader = std::io::Cursor::new(flood);
        let mut pending = Vec::new();
        let err = read_head(&mut reader, &mut pending).expect_err("an endless header was read");
        assert!(err.contains("without ending them"), "{err}");
    }

    #[test]
    fn a_response_that_ends_before_its_headers_says_so() {
        let mut reader = std::io::Cursor::new(Vec::new());
        let mut pending = Vec::new();
        let err = read_head(&mut reader, &mut pending).expect_err("nothing was read as a header");
        assert!(err.contains("ended before"), "{err}");
    }

    #[test]
    fn a_status_line_that_is_not_one_is_refused() {
        let mut reader = std::io::Cursor::new(b"not http at all\r\n\r\n".to_vec());
        let mut pending = Vec::new();
        assert!(read_head(&mut reader, &mut pending).is_err());
    }

    // ── formatting ──────────────────────────────────────────────────────────

    #[test]
    fn sizes_read_the_way_a_download_page_writes_them() {
        assert_eq!(human_bytes(0), "0 bytes");
        assert_eq!(human_bytes(999), "999 bytes");
        assert_eq!(human_bytes(1_500), "1.5 kB");
        assert_eq!(human_bytes(1_000_000_000), "1.0 GB");
        assert_eq!(human_bytes(48_000_000_000), "48.0 GB");
    }

    #[test]
    fn counts_read_as_sizes_rather_than_serial_numbers() {
        assert_eq!(commas(6), "6");
        assert_eq!(commas(200), "200");
        assert_eq!(commas(10_000), "10,000");
        assert_eq!(commas(344_357), "344,357");
    }

    #[test]
    fn a_percentage_of_nothing_is_not_a_division_by_zero() {
        assert_eq!(percent(0, 0), 0);
        assert_eq!(percent(1, 4), 25);
    }

    #[test]
    fn a_rate_is_not_claimed_before_there_is_anything_to_measure() {
        assert_eq!(rate(0, Duration::from_secs(10)), "measuring");
        assert_eq!(rate(1_000_000, Duration::from_millis(10)), "measuring");
        assert_eq!(rate(2_000_000, Duration::from_secs(2)), "1.0 MB/s");
    }

    #[test]
    fn an_elapsed_time_reads_as_a_time() {
        assert_eq!(duration(Duration::from_secs(9)), "9s");
        assert_eq!(duration(Duration::from_secs(61)), "1m01s");
        assert_eq!(duration(Duration::from_secs(3_700)), "1h01m");
    }

    #[test]
    fn a_successful_fetch_tells_the_reader_the_bytes_are_still_an_archive() {
        let bytes = b"tarry bytes".to_vec();
        let dir = tempfile::tempdir().unwrap();
        let entry = entry_with_route(&bytes);
        let transport = Canned {
            bytes: bytes.clone(),
            calls: RefCell::new(Vec::new()),
        };
        let (out, _) = fetch_into(dir.path(), &entry, "nano", &transport);
        let fetched = out.unwrap();
        let (json, human) = describe_success(&entry, "nano", &fetched, None);
        assert_eq!(json["ok"], true);
        assert_eq!(json["digest"], fetched.digest);
        assert!(human.contains("tar -xf"), "{human}");
        assert!(human.contains("stegobench score --corpus"), "{human}");
    }

    /// The whole of what a person sees, in one place, so a change to any of it
    /// is visible in a diff rather than only in a terminal somebody happened to
    /// be watching. Run with `--nocapture` to read it.
    #[test]
    fn what_a_person_sees_from_end_to_end() {
        let bytes: Vec<u8> = (0u8..=255).cycle().take(900_000).collect();
        let dir = tempfile::tempdir().unwrap();
        let entry = entry_with_route(&bytes);
        let transport = Canned {
            bytes: bytes.clone(),
            calls: RefCell::new(Vec::new()),
        };
        let (out, progress) = fetch_into(dir.path(), &entry, "nano", &transport);
        let (_, human) = describe_success(&entry, "nano", out.as_ref().unwrap(), None);
        println!("--- a fetch that worked ---\n{progress}{human}");

        let reg = shipped();
        let refused = refusal(reg.corpora.get("bossbase").unwrap()).unwrap();
        println!(
            "\n--- a corpus the terms forbid (exit {}) ---\n{refused}",
            refused.exit_code()
        );
        let pentimento = refusal(reg.corpora.get(OFFERED_CORPUS).unwrap());
        if let Some(e) = &pentimento {
            println!(
                "\n--- the offered corpus today (exit {}) ---\n{e}",
                e.exit_code()
            );
        }
        println!("\n--- the offer ---\n  {}", offer(&reg).unwrap().line());

        // The assertions, so this is a test rather than a print.
        assert!(progress.contains("fetching"), "{progress}");
        assert!(human.contains("digest"), "{human}");
    }

    /// Every archive word the registry can hold gets an unpack line, and the
    /// word in the JSON is the word the registry file uses. Spelled out because
    /// the Debug spelling of these variants is not the serialised one, and a
    /// document saying `targz` where everything else says `tar-gz` is a field a
    /// script matches on and misses.
    #[test]
    fn every_archive_the_registry_can_declare_is_answered_in_its_own_vocabulary() {
        let bytes = b"bytes".to_vec();
        let fetched = Fetched {
            path: PathBuf::from("/tmp/blob"),
            digest: digest_of(&bytes),
            bytes: 5,
            reused: false,
        };
        for (word, expect) in [
            ("tar", Some("tar -xf")),
            ("tar-gz", Some("tar -xzf")),
            ("tar-zst", Some("tar --zstd -xf")),
            ("zip", Some("unzip")),
            ("none", None),
        ] {
            let mut entry = entry_with_route(&bytes);
            entry.download[0].archive = serde_json::from_value(serde_json::json!(word))
                .unwrap_or_else(|e| panic!("{word} is not an archive the registry knows: {e}"));
            let (json, human) = describe_success(&entry, "nano", &fetched, None);
            assert_eq!(json["archive"], word);
            match expect {
                Some(command) => assert!(
                    human.contains(command),
                    "{word} produced no {command} line:\n{human}"
                ),
                None => assert!(
                    !human.contains("Unpack"),
                    "a single file was reported as an archive to unpack:\n{human}"
                ),
            }
        }
    }

    #[test]
    fn a_progress_failure_is_reported_beside_the_result_rather_than_lost() {
        let bytes = b"x".to_vec();
        let entry = entry_with_route(&bytes);
        let fetched = Fetched {
            path: PathBuf::from("/tmp/blob"),
            digest: digest_of(&bytes),
            bytes: 1,
            reused: false,
        };
        let (json, human) =
            describe_success(&entry, "nano", &fetched, Some("broken pipe".to_string()));
        assert_eq!(json["progress_error"], "broken pipe");
        assert!(human.contains("broken pipe"), "{human}");
    }
}
