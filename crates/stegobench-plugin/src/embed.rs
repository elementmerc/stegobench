// Author:  Daniel Iwugo
// Comment: Christ is King
// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Daniel Iwugo

//! Running an embedder over a cover the user chose.
//!
//! The registry has always described how to drive an embedder: the
//! `[roundtrip]` block carries the argv, the placeholders and the passphrase,
//! and `doctor` has used them since the beginning to prove a tool works. What
//! it could not do was run one over an image somebody actually has, so the six
//! registered embedders were a list you could read and not a thing you could
//! use.
//!
//! The execution machinery lives here rather than in the self-test because
//! both need it and there must be one of it. A container that runs with a
//! network in one place and without in the other is not a smaller problem for
//! being a copy-paste; it is the same tool measured two ways.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use stegobench_core::registry::Entry;

/// How the tool is reached, which decides both the argv and the paths.
///
/// A container sees the scratch directory at `/work`; a local program sees it
/// where it actually is. Substituting the wrong one produces a tool that exits
/// cleanly having written nothing, which is exactly the failure that is
/// hardest to attribute.
pub enum Reach {
    Container(String),
    Local(Vec<String>),
}

impl Reach {
    /// How this entry is run, or why it cannot be.
    pub fn of(entry: &Entry) -> Result<Reach, String> {
        match (entry.image.as_ref(), entry.binary.as_ref()) {
            (Some(image), _) => Ok(Reach::Container(image.reference.clone())),
            (None, Some(binary)) if !binary.command.is_empty() => {
                Ok(Reach::Local(binary.command.clone()))
            }
            _ => Err("no image and no binary command to run it with".into()),
        }
    }

    /// The prefix every path in the argv is written against.
    fn base(&self, work: &Path) -> String {
        match self {
            Reach::Container(_) => "/work".to_string(),
            Reach::Local(_) => work.display().to_string(),
        }
    }
}

/// A staged scratch directory and the one way to run a phase inside it.
///
/// Both the round trip and `stegobench embed` drive a tool through this, so
/// the sandbox flags, the mount, the user mapping and the deadline are stated
/// once. A caller supplies the argv and the substitutions; it does not get to
/// supply the isolation.
pub struct Session<'a> {
    reach: Reach,
    work: PathBuf,
    base: String,
    entrypoint: Option<&'a str>,
    uid_gid: String,
    timeout: Duration,
}

impl<'a> Session<'a> {
    pub fn new(
        reach: Reach,
        work: &Path,
        entrypoint: Option<&'a str>,
        timeout: Duration,
    ) -> Session<'a> {
        let base = reach.base(work);
        Session {
            reach,
            work: work.to_path_buf(),
            base,
            entrypoint,
            uid_gid: owner_of(work),
            timeout,
        }
    }

    /// The path prefix the tool will see, for building substitutions.
    pub fn base(&self) -> &str {
        &self.base
    }

    /// Run one phase, substituting `pairs` into every argument.
    ///
    /// Substitution is applied to the registry's own command words too,
    /// because a locally installed tool's `command` may itself carry a path
    /// placeholder.
    pub fn phase(&self, argv: &[String], pairs: &[(&str, &str)]) -> Result<(), String> {
        let subst = |a: &String| {
            let mut s = a.clone();
            for (from, to) in pairs {
                s = s.replace(from, to);
            }
            s
        };
        match &self.reach {
            Reach::Local(command) => {
                let mut run = Command::new(subst(&command[0]));
                run.args(command[1..].iter().map(&subst));
                run.args(argv.iter().map(&subst));
                // The scratch directory, so a tool that writes a stray file
                // beside its output leaves it there rather than in the user's
                // working directory.
                run.current_dir(&self.work);
                bounded(run, &command[0], self.timeout)
            }
            Reach::Container(image) => {
                let mut args: Vec<String> = vec![
                    "run".into(),
                    "--rm".into(),
                    "--network=none".into(),
                    "--cap-drop=ALL".into(),
                    "--security-opt".into(),
                    "no-new-privileges".into(),
                    "--memory=2g".into(),
                ];
                if !self.uid_gid.is_empty() {
                    args.push("--user".into());
                    args.push(self.uid_gid.clone());
                }
                args.push("-v".into());
                args.push(format!("{}:/work", self.work.display()));
                if let Some(ep) = self.entrypoint {
                    args.push("--entrypoint".into());
                    args.push(ep.to_string());
                }
                args.push(image.clone());
                args.extend(argv.iter().map(&subst));
                let mut run = Command::new("docker");
                run.args(&args);
                bounded(run, "the container", self.timeout)
            }
        }
    }
}

/// The owner of the scratch directory, as `--user` wants it.
///
/// A container writing as root leaves files the user cannot delete, which is
/// a worse outcome than the run failing.
fn owner_of(work: &Path) -> String {
    std::fs::metadata(work)
        .ok()
        .map(|m| {
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                format!("{}:{}", m.uid(), m.gid())
            }
            #[cfg(not(unix))]
            {
                let _ = m;
                String::new()
            }
        })
        .unwrap_or_default()
}

/// Run one phase and wait for it, but not for ever.
///
/// The waiting is shared with every other plugin invocation (see
/// [`crate::exec`]), because it is the part that goes wrong and it goes wrong
/// identically wherever it is written. What is local here is only what a
/// failed phase means: a non-zero exit is this phase failing, rather than an
/// answer for a parser to interpret.
pub fn bounded(command: Command, label: &str, timeout: Duration) -> Result<(), String> {
    let out = crate::exec::captured(command, label, timeout)?;
    if out.status.success() {
        return Ok(());
    }
    let tail = String::from_utf8_lossy(&out.stderr)
        .trim()
        .chars()
        .take(160)
        .collect::<String>();
    Err(format!("exit {}: {tail}", out.status.code().unwrap_or(-1)))
}

/// What one embed produced, and what it cost.
#[derive(Debug, Clone, PartialEq)]
pub struct Embedded {
    /// Where the stego image was written.
    pub stego: PathBuf,
    /// Its size in bytes, read back rather than assumed.
    pub bytes: u64,
    /// Whether the payload was recovered from it, where the entry declares a
    /// way to try. `None` when it declares none.
    ///
    /// Checked by default because an embedder that exits cleanly having
    /// written an image the payload is not actually in is the failure mode
    /// that produces a corpus of covers labelled stego, and every number
    /// measured on it is wrong in the direction nobody notices.
    pub recovered: Option<Result<(), String>>,
}

/// How long either phase is given before it is killed.
///
/// An embed is one image, so anything past this is a tool waiting on
/// something that is never coming: a passphrase prompt on a terminal nobody is
/// watching is the usual one, and it is exactly what a run driven from a
/// registry entry can provoke.
pub const PHASE_TIMEOUT: Duration = Duration::from_secs(300);

/// Hide `payload` in `cover`, writing the result to `out`.
///
/// Uses the entry's `[roundtrip]` argv, which is the only description the
/// registry holds of how to drive the tool. The passphrase is the entry's own
/// unless the caller overrides it.
pub fn run(
    entry: &Entry,
    cover: &Path,
    payload: &Path,
    out: &Path,
    passphrase: Option<&str>,
    extract_check: bool,
) -> Result<Embedded, String> {
    let rt = entry
        .roundtrip
        .as_ref()
        .ok_or_else(|| format!("{} declares no way to embed with it", entry.name))?;
    let reach = Reach::of(entry)?;

    let dir = tempfile::tempdir().map_err(|e| format!("no scratch directory: {e}"))?;
    let work = dir.path();

    // The staged cover keeps the user's extension, because several of these
    // tools infer the format from the name and refuse anything else: outguess
    // answers "Unknown data type" to a file called .out and exits 1, which
    // reads as a broken tool rather than a bad filename.
    let ext = cover
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("bin")
        .to_string();
    let cover_name = format!("cover.{ext}");
    std::fs::copy(cover, work.join(&cover_name))
        .map_err(|e| format!("could not stage {}: {e}", cover.display()))?;
    std::fs::copy(payload, work.join("payload.bin"))
        .map_err(|e| format!("could not stage {}: {e}", payload.display()))?;

    let session = Session::new(reach, work, rt.entrypoint.as_deref(), PHASE_TIMEOUT);
    let base = session.base().to_string();
    let pass = passphrase.unwrap_or(&rt.passphrase);
    let stego_in = format!("{base}/stego.{ext}");
    let cover_in = format!("{base}/{cover_name}");
    let payload_in = format!("{base}/payload.bin");
    let recovered_in = format!("{base}/recovered.bin");
    let pairs: Vec<(&str, &str)> = vec![
        ("{cover}", cover_in.as_str()),
        ("{payload}", payload_in.as_str()),
        ("{stego}", stego_in.as_str()),
        ("{recovered}", recovered_in.as_str()),
        ("{passphrase}", pass),
    ];

    session
        .phase(&rt.embed_argv, &pairs)
        .map_err(|e| format!("embed failed: {e}"))?;

    // A tool can exit zero having written nothing.
    let staged = work.join(format!("stego.{ext}"));
    let bytes = match std::fs::metadata(&staged) {
        Ok(m) if m.len() > 0 => m.len(),
        _ => return Err("the embedder exited cleanly and wrote no stego file".into()),
    };

    let recovered = if extract_check && !rt.extract_argv.is_empty() {
        Some(match session.phase(&rt.extract_argv, &pairs) {
            Err(e) => Err(format!("extract failed: {e}")),
            Ok(()) => match (
                std::fs::read(work.join("recovered.bin")),
                std::fs::read(payload),
            ) {
                (Ok(got), Ok(want)) if got == want => Ok(()),
                (Ok(got), Ok(want)) => Err(format!(
                    "recovered {} bytes and the payload is {}: what went in \
                     did not come back",
                    got.len(),
                    want.len()
                )),
                (Err(e), _) => Err(format!("extract wrote no payload: {e}")),
                (_, Err(e)) => Err(format!("could not re-read the payload: {e}")),
            },
        })
    } else {
        None
    };

    if let Some(parent) = out.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("could not make {}: {e}", parent.display()))?;
        }
    }
    // Copy rather than rename: the scratch directory is very often on another
    // filesystem, and a rename across one fails with a message about devices
    // that says nothing about what the user did.
    std::fs::copy(&staged, out).map_err(|e| format!("could not write {}: {e}", out.display()))?;

    Ok(Embedded {
        stego: out.to_path_buf(),
        bytes,
        recovered,
    })
}

#[cfg(all(test, unix))]
mod tests {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    use stegobench_core::registry::Entry;

    /// A stand-in embedder, so every branch is reachable without installing
    /// one of the six real tools or pulling a container.
    fn fake_tool(dir: &Path, body: &str) -> String {
        let path = dir.join("tool.sh");
        let mut f = std::fs::File::create(&path).expect("script");
        write!(f, "#!/bin/sh\nset -e\n{body}\n").expect("written");
        let mut perms = f.metadata().expect("metadata").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).expect("chmod");
        path.display().to_string()
    }

    fn entry(command: &str) -> Entry {
        toml::from_str(&format!(
            "name = \"x\"\nkind = \"embedder\"\nlicence = \"MIT\"\n\
             [binary]\ncommand = [\"{command}\"]\n\
             [roundtrip]\ncover = \"fixtures/clean.png\"\n\
             embed_argv = [\"embed\", \"{{cover}}\", \"{{payload}}\", \"{{stego}}\"]\n\
             extract_argv = [\"extract\", \"{{stego}}\", \"{{recovered}}\"]\n\
             [selftest]\nmust_detect = \"fixtures/a.png\"\nmust_clear = \"fixtures/b.png\"\n"
        ))
        .expect("parses")
    }

    /// cover, payload and out, staged in a scratch directory.
    fn staged(dir: &Path) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
        let cover = dir.join("cover.png");
        let payload = dir.join("p.bin");
        std::fs::write(&cover, b"\x89PNG\r\n\x1a\ncover bytes").expect("cover");
        std::fs::write(&payload, b"the payload").expect("payload");
        (cover, payload, dir.join("out.png"))
    }

    #[test]
    fn a_working_embedder_writes_the_file_and_proves_the_payload_is_in_it() {
        let dir = tempfile::tempdir().unwrap();
        let tool = fake_tool(
            dir.path(),
            "case \"$1\" in\n  embed) cat \"$2\" \"$3\" > \"$4\" ;;\n  \
             extract) tail -c 11 \"$2\" > \"$3\" ;;\nesac",
        );
        let (cover, payload, out) = staged(dir.path());
        let done = super::run(&entry(&tool), &cover, &payload, &out, None, true).expect("embedded");
        assert!(out.is_file(), "no stego file was written");
        assert!(done.bytes > 0);
        assert_eq!(done.recovered, Some(Ok(())));
    }

    #[test]
    fn an_embedder_that_writes_nothing_is_caught_rather_than_reported_as_working() {
        // The failure this check exists for: exit 0, no file. Without it the
        // caller gets a success and no stego image.
        let dir = tempfile::tempdir().unwrap();
        let tool = fake_tool(dir.path(), "exit 0");
        let (cover, payload, out) = staged(dir.path());
        let err = super::run(&entry(&tool), &cover, &payload, &out, None, true)
            .expect_err("should have refused");
        assert!(err.contains("wrote no stego file"), "got {err}");
        assert!(!out.exists(), "a file was written anyway");
    }

    #[test]
    fn a_payload_that_does_not_come_back_is_reported_against_the_written_file() {
        // An image is produced and the payload is not in it, which is how a
        // cover ends up labelled stego. The file is still written, because
        // the user may want to look at it; the answer says it failed.
        let dir = tempfile::tempdir().unwrap();
        let tool = fake_tool(
            dir.path(),
            "case \"$1\" in\n  embed) cp \"$2\" \"$4\" ;;\n  \
             extract) printf 'something else' > \"$3\" ;;\nesac",
        );
        let (cover, payload, out) = staged(dir.path());
        let done = super::run(&entry(&tool), &cover, &payload, &out, None, true).expect("ran");
        match done.recovered {
            Some(Err(e)) => assert!(e.contains("did not come back"), "got {e}"),
            other => panic!("the bad round trip was not reported: {other:?}"),
        }
    }

    #[test]
    fn no_verify_skips_the_check_and_says_nothing_about_the_payload() {
        let dir = tempfile::tempdir().unwrap();
        let tool = fake_tool(
            dir.path(),
            "case \"$1\" in\n  embed) cp \"$2\" \"$4\" ;;\nesac",
        );
        let (cover, payload, out) = staged(dir.path());
        let done = super::run(&entry(&tool), &cover, &payload, &out, None, false).expect("ran");
        assert_eq!(done.recovered, None, "a check ran that was not asked for");
    }

    #[test]
    fn an_entry_with_no_roundtrip_block_says_it_cannot_embed() {
        let e: Entry = toml::from_str(
            "name = \"x\"\nkind = \"embedder\"\nlicence = \"MIT\"\n\
             [binary]\ncommand = [\"true\"]\n",
        )
        .expect("parses");
        let dir = tempfile::tempdir().unwrap();
        let (cover, payload, out) = staged(dir.path());
        let err = super::run(&e, &cover, &payload, &out, None, true).expect_err("refused");
        assert!(err.contains("declares no way to embed"), "got {err}");
    }

    #[test]
    fn the_output_directory_is_made_rather_than_the_write_failing() {
        let dir = tempfile::tempdir().unwrap();
        let tool = fake_tool(
            dir.path(),
            "case \"$1\" in\n  embed) cp \"$2\" \"$4\" ;;\nesac",
        );
        let (cover, payload, _) = staged(dir.path());
        let out = dir.path().join("a/b/c/out.png");
        super::run(&entry(&tool), &cover, &payload, &out, None, false).expect("embedded");
        assert!(out.is_file(), "the nested path was not created");
    }
}
