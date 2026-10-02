// Author:  Daniel Iwugo
// Comment: Christ is King
// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Daniel Iwugo
//! Asking a tool both questions, and believing it only if it gets both right.
//!
//! A tool that answers "stego" to everything passes a detect-only check. One
//! that answers "clean" to everything passes a clear-only check. Either alone
//! is a control that cannot fail. So both fixtures are run and both must come
//! back correct, and a tool that fails either is reported as broken rather
//! than as present.
//!
//! This is the module that turns `doctor`'s "not verified" into an answer.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use stegobench_core::registry::Entry;

use crate::parsers::{self, Reading};

/// The outcome of asking one tool both questions.
#[derive(Debug, Clone, PartialEq)]
pub enum Verified {
    /// Correct on both fixtures.
    Passed,
    /// Wrong on at least one, with which and how.
    Failed(String),
    /// Could not be asked, which is not the same as being wrong.
    Skipped(String),
    /// It answered, and got the fixture wrong, and that is not a fault.
    ///
    /// A SUBJECT is a tool under evaluation. The entire question being asked
    /// of it is whether it detects things, so reporting "does not detect the
    /// fixture" as broken would be both wrong and prejudicial: it would mean
    /// doctor permanently describing the thing we are measuring as faulty,
    /// and it would confuse a real installation problem with the finding.
    ///
    /// Installed correctly and detects things are the same question for a
    /// reference tool and different questions for a subject. This is the
    /// distinction.
    Answered(String),
}

/// The reading, unless the tool died without answering, in which case the
/// death is the answer.
///
/// A TOOL THAT CRASHED HAS NOT SAID AN IMAGE IS CLEAN, AND FOR ONE REGISTERED
/// DETECTOR IT SAID SO ABOUT EVERY IMAGE IT WAS GIVEN.
///
/// zsteg dies under this sandbox with a Ruby traceback and exit 1, which the
/// SANDBOX comment below has recorded for some time. What nobody had noticed
/// is what the harness then did with it: the traceback goes to stderr, stdout
/// is empty, zsteg's parser finds no finding in either and returns
/// `Verdict(false)`, and `Verdict(false)` means clean. A forensic analyst
/// walking this build on 2026-10-02 was shown `clean` for four exhibits out
/// of six, with "6 of 6 answered, 0 errored" underneath and exit code 0, and
/// wrote them down as carrying no detector evidence. Four Ruby stack traces
/// had been rendered as evidence of absence.
///
/// It also explains how the two-sided self-test passed for a tool that cannot
/// process a single image here. `must_detect` passes because zsteg prints its
/// finding before it dies; `must_clear` passes BECAUSE the tool crashed, since
/// a crash and a clean verdict were the same value. A check that a crash can
/// satisfy is not a check.
///
/// The rule: a non-zero exit with no positive finding is a failure. A positive
/// finding is kept, because a tool that printed a hit and then died still
/// found something, and that is the shape zsteg has on a JPEG. Consistent with
/// the adapters, which already exit non-zero to report that they could not do
/// the work, after a run once exited zero and produced nothing for 2,000
/// images.
fn answered_or_died(reading: Reading, out: &std::process::Output, name: &str) -> Reading {
    if out.status.success() {
        return reading;
    }
    match reading {
        // It found something and then fell over. The finding stands; the fall
        // is in the raw output the reader is pointed at.
        found @ (Reading::Verdict(true) | Reading::Score(_)) => found,
        Reading::Failed(why) => Reading::Failed(why),
        Reading::Verdict(false) => {
            let code = out
                .status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "a signal".to_string());
            let last = String::from_utf8_lossy(&out.stderr);
            let last = last.lines().next().unwrap_or("").trim();
            Reading::Failed(format!(
                "{name} exited {code} without answering, so this is not a \
                 clean result: {}",
                clip_line(last)
            ))
        }
    }
}

/// One line of a tool's complaint, short enough to sit in a table cell's
/// footnote without becoming the output.
fn clip_line(line: &str) -> String {
    const MOST: usize = 120;
    if line.is_empty() {
        return "it printed nothing at all".to_string();
    }
    if line.chars().count() <= MOST {
        return line.to_string();
    }
    let kept: String = line.chars().take(MOST).collect();
    format!("{kept}...")
}

/// The flags every containerised run in this project is locked down with.
///
/// Public, and the ONE place they are written. `stegobench describe` prints a
/// command a reader can paste, and the only thing that makes that command
/// worth printing is that it is the command the harness runs. A second copy
/// of this list somewhere else is a copy that drifts, and a printed sandbox
/// weaker than the real one is worse than printing nothing.
///
/// THERE IS NO WRITABLE /tmp HERE, AND THAT WAS TRIED
///
/// zsteg writes a tempfile while identifying extracted data, and under
/// `--read-only` it dies on that with a Ruby traceback and exit 1, partway
/// through any image carrying trailing data. Adding `--tmpfs /tmp` removes
/// the traceback and lets it finish. It also turns zsteg into a detector that
/// answers stego to everything: with the tempfile available it identifies
/// random bits extracted from a CLEAN image as an OpenPGP key or an archive
/// and reports a hit. Its own two-sided self-test caught this immediately,
/// `must_clear` failing where `must_detect` still passed, which is the whole
/// reason that check has two sides.
///
/// So the strict sandbox is load-bearing for what zsteg measures, which is an
/// uncomfortable thing to be true and is recorded here rather than
/// rediscovered. Anybody loosening this must re-run every self-test and
/// expect the numbers to move. See DEFERRED.md.
pub const SANDBOX: &[&str] = &[
    "--network=none",
    "--cap-drop=ALL",
    "--security-opt",
    "no-new-privileges",
    "--read-only",
    "--memory=2g",
];

/// The stem every image is presented to a tool under.
///
/// WHY A TOOL IS NEVER TOLD WHAT IT IS LOOKING AT
/// ----------------------------------------------
/// A tool has to be told WHICH file to look at. It does not have to be told
/// what that file is, and the names on disk say it out loud: `lsb-0.4bpp.png`
/// against `clean.png` for the self-test fixtures, and one directory per
/// embedding arm in a corpus.
///
/// An adversarial researcher walked this build on 2026-10-02 with a plugin
/// that did no steganalysis whatsoever. Its whole decision was the basename:
///
/// ```sh
/// case "$b" in *lsb*|*appended*) echo 0.990 ;; *) echo 0.010 ;; esac
/// ```
///
/// It answered both fixtures correctly, earned the `verified` badge, and the
/// result document it then produced recorded `selftest: "passed"`. The
/// two-sided check was sound; the filenames gave the answer away. The same
/// trick works on a corpus, where the arm is a directory name.
///
/// So every image is presented under this stem, in a directory of its own, so
/// a label file sitting beside the image is out of reach as well. The
/// EXTENSION IS KEPT, because a real decoder dispatches on `.png` against
/// `.jpg` and a tool that cannot read its input is not a finding about the
/// tool.
///
/// Both self-test fixtures get the SAME stem in two DIFFERENT directories, and
/// the directories are fresh temporary ones, so their names are unpredictable
/// too. That is deliberate rather than a side effect: a path a plugin author
/// can predict is a path they can special case. The VERDICT stays
/// reproducible, because the only thing that differs between two runs of the
/// same tool over the same bytes is a string carrying no information about
/// them.
const PRESENTED_STEM: &str = "image";

/// The name one file is presented under: the neutral stem, its own extension.
fn presented_name(path: &Path) -> String {
    match path.extension().and_then(|e| e.to_str()) {
        Some(ext) if !ext.is_empty() => format!("{PRESENTED_STEM}.{ext}"),
        // A file with no extension is presented without one, which is what it
        // already had. Inventing one would change what a decoder dispatches on.
        _ => PRESENTED_STEM.to_string(),
    }
}

/// Stages one file where a tool running on this machine can read it, under a
/// name that says nothing about it.
///
/// Returns the scratch directory, which has to stay alive for as long as the
/// tool runs, and the path inside it. The container route needs none of this:
/// a bind mount already renames the file for free.
///
/// A hard link first, because `score` takes this path too and a copy per image
/// is a copy of the whole corpus. It is no more exposure than before: this
/// route used to hand the tool the original path itself. Where the scratch
/// directory is on another filesystem the link fails and the bytes are copied,
/// and a failure to stage is reported rather than quietly becoming the
/// revealing name again.
fn presented_copy(file: &Path) -> Result<(tempfile::TempDir, PathBuf), String> {
    let absolute = file
        .canonicalize()
        .map_err(|e| format!("{} could not be resolved: {e}", file.display()))?;
    let dir = tempfile::tempdir().map_err(|e| format!("no scratch directory: {e}"))?;
    let staged = dir.path().join(presented_name(&absolute));
    if std::fs::hard_link(&absolute, &staged).is_err() {
        std::fs::copy(&absolute, &staged)
            .map_err(|e| format!("could not stage {}: {e}", absolute.display()))?;
    }
    Ok((dir, staged))
}

/// Runs one image through a containerised tool and parses what comes back.
///
/// The container is locked down the way every run in this project is: no
/// network, no capabilities, no new privileges, read-only root, and a memory
/// cap. A self-test is still running somebody else's code on our machine.
fn run_one(
    entry: &Entry,
    image: &str,
    fixture: &Path,
    timeout: Duration,
    adapter_roots: &[PathBuf],
    raw: &mut Raw,
) -> Reading {
    let Some(invoke) = &entry.invoke else {
        return Reading::Failed("entry declares no invoke block".into());
    };
    let Ok(absolute) = fixture.canonicalize() else {
        return Reading::Failed(format!("fixture {} not found", fixture.display()));
    };
    if absolute.file_name().is_none() {
        return Reading::Failed("fixture has no usable filename".into());
    }
    // The name the TOOL sees, which is not the name on disk. A bind mount makes
    // this free here: the container is handed a neutral stem and never learns
    // what the file was called. See [`PRESENTED_STEM`].
    let name = presented_name(&absolute);
    let name = name.as_str();
    // A writable working directory, when the tool writes beside its input.
    // The scratch directory is dropped when this function returns, so nothing
    // a tool leaves behind outlives the check.
    let scratch = if invoke.writable_workdir {
        match tempfile::tempdir() {
            Ok(d) => {
                if let Err(e) = std::fs::copy(&absolute, d.path().join(name)) {
                    return Reading::Failed(format!("could not stage the fixture: {e}"));
                }
                Some(d)
            }
            Err(e) => return Reading::Failed(format!("no scratch directory: {e}")),
        }
    } else {
        None
    };

    let mount = match &scratch {
        Some(d) => format!("{}:/work", d.path().display()),
        None => format!("{}:/work/{}:ro", absolute.display(), name),
    };
    let inner = format!("/work/{name}");

    let mut args: Vec<String> = vec!["run".into(), "--rm".into()];
    args.extend(SANDBOX.iter().map(|s| s.to_string()));
    args.push("-v".into());
    args.push(mount);

    // An adapter is mounted read-only beside the image it reads.
    let mut adapter_inner = String::new();
    if let Some(rel) = &invoke.adapter {
        let abs = match crate::adapter::resolve(rel, adapter_roots) {
            Ok(p) => p,
            Err(why) => return Reading::Failed(why.to_string()),
        };
        let Some(base) = abs.file_name().and_then(|n| n.to_str()) else {
            return Reading::Failed("adapter has no usable filename".into());
        };
        adapter_inner = format!("/adapter/{base}");
        args.push("-v".into());
        args.push(format!("{}:{}:ro", abs.display(), adapter_inner));
    }

    for kv in &invoke.env {
        args.push("-e".into());
        args.push(kv.clone());
    }

    // Several of these images set an entrypoint to the tool itself, which
    // would swallow the adapter's argv.
    if let Some(ep) = &invoke.entrypoint {
        args.push("--entrypoint".into());
        args.push(ep.clone());
    }

    args.push(image.into());
    args.extend(invoke.argv.iter().map(|a| {
        // `{files}` is honoured here as a batch of one, so a batched entry can
        // still be asked about a single image. The self-test depends on it:
        // `doctor` checks two fixtures one at a time, and an entry whose argv
        // only speaks the batch placeholder would otherwise be handed the
        // literal string `{files}` and fail for a reason that has nothing to do
        // with the detector.
        if a == "{files}" {
            return inner.clone();
        }
        a.replace("{file}", &inner)
            .replace("{adapter}", &adapter_inner)
    }));

    // Run as this user, so a tool writing into the scratch directory does not
    // leave root-owned files the cleanup then fails to remove.
    //
    // The ids come from the scratch directory's own metadata rather than from
    // libc: we created it, so it already carries them, and asking the
    // filesystem needs neither a dependency nor an unsafe block.
    #[cfg(unix)]
    if let Some(dir) = &scratch {
        use std::os::unix::fs::MetadataExt;
        if let Ok(meta) = std::fs::metadata(dir.path()) {
            args.insert(2, "--user".into());
            args.insert(3, format!("{}:{}", meta.uid(), meta.gid()));
        }
    }

    // Bounded, like every other invocation. A container that never answers
    // is the same stuck run as a binary that never answers, and it is the more
    // likely of the two: an image doing first-run work on a cold cache has no
    // way to say so.
    let mut docker = Command::new("docker");
    docker.args(&args);
    let out = match crate::exec::captured(docker, "the container", timeout) {
        Ok(out) => out,
        Err(e) => return Reading::Failed(e),
    };

    // Some tools answer in a file rather than on stdout. Reading an empty or
    // absent file as "found nothing" is how silence becomes a measurement, so
    // a missing file is a failure with its name.
    let text = match (&invoke.output_file, &scratch) {
        (Some(rel), Some(dir)) => {
            let path = dir.path().join(rel.trim_start_matches("/work/"));
            match std::fs::read_to_string(&path) {
                Ok(t) => t,
                Err(e) => {
                    return Reading::Failed(format!(
                        "{} wrote no {}: {e}. stderr: {}",
                        entry.name,
                        rel,
                        String::from_utf8_lossy(&out.stderr).trim()
                    ))
                }
            }
        }
        _ => String::from_utf8_lossy(&out.stdout).into_owned(),
    };

    raw.stdout = text.clone();
    raw.stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    // A batched entry speaks the keyed protocol whatever the batch size, so one
    // image still arrives as `<path>\t<answer>` and has to be read that way.
    // Reading it with the plain parser would see a line that is not a number
    // and report a working detector as broken, which is how `doctor` would
    // start lying about an entry the moment it declared a batch.
    if invoke.batch.is_some_and(|b| b > 1) {
        let keys = vec![inner.clone()];
        let one = parsers::parse_keyed(&invoke.parser, &text, &raw.stderr.clone(), &keys)
            .pop()
            .unwrap_or_else(|| Reading::Failed("the batch parser returned nothing".into()));
        return answered_or_died(one, &out, &entry.name);
    }
    answered_or_died(
        parsers::parse(&invoke.parser, &text, &raw.stderr.clone()),
        &out,
        &entry.name,
    )
}

/// Runs a binary plugin directly, with no container.
///
/// Our own tool and anything else already installed on the machine. There is
/// no isolation here and that is a deliberate limit: a binary entry says the
/// operator already trusts this program enough to have installed it, which is
/// a different statement from pulling a stranger's image.
fn run_binary(entry: &Entry, fixture: &Path, timeout: Duration, raw: &mut Raw) -> Reading {
    let (Some(bin), Some(invoke)) = (&entry.binary, &entry.invoke) else {
        return Reading::Failed("entry is not a runnable binary".into());
    };
    let Some(program) = bin.command.first() else {
        return Reading::Failed("binary.command is empty".into());
    };
    let Some(path) = crate::which(program) else {
        return Reading::Failed(format!("{program} is not on PATH"));
    };
    if !fixture.is_file() {
        return Reading::Failed(format!("fixture {} not found", fixture.display()));
    }
    // There is no mount namespace on this route, so the file is staged under a
    // neutral name instead. `_scratch` is held until this function returns,
    // which is after the tool has exited. See [`PRESENTED_STEM`].
    let (_scratch, staged) = match presented_copy(fixture) {
        Ok(pair) => pair,
        Err(why) => return Reading::Failed(why),
    };
    let file = staged.display().to_string();
    let mut argv: Vec<String> = bin.command[1..].to_vec();
    argv.extend(invoke.argv.iter().map(|a| {
        // `{files}` as a batch of one, exactly as the container path treats it.
        // The registry now refuses a batched binary entry outright, so nothing
        // should reach here declaring one; this is the second layer, because a
        // placeholder that survives into argv is handed to the tool as six
        // literal characters and the tool fails for a reason that has nothing
        // to do with the image it was asked about.
        if a == "{files}" {
            return file.clone();
        }
        a.replace("{file}", &file)
    }));

    let mut program_cmd = Command::new(&path);
    program_cmd.args(&argv);
    match crate::exec::captured(program_cmd, program, timeout) {
        Ok(out) => {
            raw.stdout = String::from_utf8_lossy(&out.stdout).into_owned();
            raw.stderr = String::from_utf8_lossy(&out.stderr).into_owned();
            answered_or_died(
                parsers::parse(&invoke.parser, &raw.stdout.clone(), &raw.stderr.clone()),
                &out,
                program,
            )
        }
        // Passed through rather than wrapped. The message already names the
        // tool and says what happened, and "could not run X: X gave no answer"
        // reports a timeout as a launch failure, which sends the reader to the
        // wrong problem.
        Err(e) => Reading::Failed(e),
    }
}

/// What the tool actually printed, kept so a reader can tell a detector that
/// found nothing from a harness that misread it.
///
/// Empty when the tool never ran, which is the honest answer: a container
/// that could not start printed nothing to keep. The failure itself is
/// already in the record's `error`.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Raw {
    pub stdout: String,
    pub stderr: String,
}

impl Raw {
    /// Whether there is anything here worth writing down.
    pub fn is_empty(&self) -> bool {
        self.stdout.is_empty() && self.stderr.is_empty()
    }
}

/// [`read_one`], and also what the tool printed while answering.
///
/// The raw text is an out-parameter rather than part of the return type so
/// that every path that fails BEFORE the tool runs stays exactly as it was:
/// those paths have no output to report and should not have to say so.
pub fn read_one_observed(
    entry: &Entry,
    fixture: &Path,
    timeout: Duration,
    adapter_roots: &[PathBuf],
    raw: &mut Raw,
) -> Reading {
    if entry.invoke.as_ref().is_some_and(|i| i.host) {
        return run_host_adapter(entry, fixture, timeout, adapter_roots, raw);
    }
    match (&entry.image, &entry.binary) {
        (Some(img), _) => run_one(
            entry,
            &img.reference.clone(),
            fixture,
            timeout,
            adapter_roots,
            raw,
        ),
        (_, Some(_)) => run_binary(entry, fixture, timeout, raw),
        _ => Reading::Failed("entry declares neither an image nor a binary".into()),
    }
}

/// Ask this tool about SEVERAL files in one invocation.
///
/// Returns one reading per input, at the same index, always: a batch that dies
/// produces a failure for every image in it rather than a short list the caller
/// would have to align itself.
///
/// WHY THIS EXISTS
/// ---------------
/// Starting a container and importing an interpreter measured 0.62 seconds per
/// image on the build box, against 0.1 seconds of actual work for zsteg. Nine
/// tenths of that run was overhead that scales with the corpus, and the Core
/// tier is hundreds of thousands of images, so it is days of pure startup.
///
/// Only an entry declaring `invoke.batch` above 1 is ever routed here, because
/// a tool that reads exactly one filename cannot be batched by being handed
/// more. See [`stegobench_core::registry::Invoke::batch`].
pub fn read_many_observed(
    entry: &Entry,
    files: &[PathBuf],
    timeout: Duration,
    adapter_roots: &[PathBuf],
    raw: &mut Raw,
) -> Vec<Reading> {
    let fail = |why: String| vec![Reading::Failed(why); files.len()];
    if files.is_empty() {
        return Vec::new();
    }
    let Some(invoke) = &entry.invoke else {
        return fail("entry declares no invoke block".into());
    };
    // A host adapter and a bare binary are both one-at-a-time today. Routed
    // back rather than refused, so declaring a batch never makes a working
    // entry stop working.
    if invoke.host || entry.image.is_none() {
        return files
            .iter()
            .map(|f| read_one_observed(entry, f, timeout, adapter_roots, &mut Raw::default()))
            .collect();
    }
    let Some(image) = &entry.image else {
        return fail("entry declares no image".into());
    };

    let mut args: Vec<String> = vec!["run".into(), "--rm".into()];
    args.extend(SANDBOX.iter().map(|s| s.to_string()));

    // Each image is mounted at a name this host chose, numbered by its position
    // in the batch. NOT at its own basename: every arm restarts its numbering
    // at 000000, so a batch spanning two arms would mount two different
    // photographs at one path and the second would shadow the first. The
    // numbered name is also the key the tool echoes back, so the mapping from
    // answer to image is a string this code constructed rather than a guess.
    //
    // It carries the same property [`PRESENTED_STEM`] exists for, by accident
    // rather than by design: a position tells a tool nothing about what it is
    // looking at. Do not put the basename back.
    let mut keys: Vec<String> = Vec::with_capacity(files.len());
    for (i, file) in files.iter().enumerate() {
        let Ok(absolute) = file.canonicalize() else {
            return fail(format!("{} not found", file.display()));
        };
        let ext = absolute
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| format!(".{e}"))
            .unwrap_or_default();
        let inner = format!("/work/{i:06}{ext}");
        args.push("-v".into());
        args.push(format!("{}:{}:ro", absolute.display(), inner));
        keys.push(inner);
    }

    let mut adapter_inner = String::new();
    if let Some(rel) = &invoke.adapter {
        let abs = match crate::adapter::resolve(rel, adapter_roots) {
            Ok(p) => p,
            Err(why) => return fail(why.to_string()),
        };
        let Some(base) = abs.file_name().and_then(|n| n.to_str()) else {
            return fail("adapter has no usable filename".into());
        };
        adapter_inner = format!("/adapter/{base}");
        args.push("-v".into());
        args.push(format!("{}:{}:ro", abs.display(), adapter_inner));
    }

    for kv in &invoke.env {
        args.push("-e".into());
        args.push(kv.clone());
    }
    if let Some(ep) = &invoke.entrypoint {
        args.push("--entrypoint".into());
        args.push(ep.clone());
    }
    args.push(image.reference.clone());

    // `{files}` stands alone and expands to one argument per image, which is
    // why the registry refuses it glued to other text.
    for a in &invoke.argv {
        if a == "{files}" {
            args.extend(keys.iter().cloned());
        } else {
            args.push(a.replace("{adapter}", &adapter_inner));
        }
    }

    let mut docker = Command::new("docker");
    docker.args(&args);
    let out = match crate::exec::captured(docker, "the container", timeout) {
        Ok(out) => out,
        // Every image in the batch shares the one invocation, so they share its
        // fate. Said per image so the records file still has a line each and a
        // resume does not have to reason about which of them were attempted.
        Err(e) => return fail(format!("the batch of {} failed: {e}", files.len())),
    };

    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    raw.stdout = stdout.clone();
    raw.stderr = stderr.clone();
    // Every image in the batch shares the one invocation, so a batch that
    // died without answering did not find all of them clean.
    parsers::parse_keyed(&invoke.parser, &stdout, &stderr, &keys)
        .into_iter()
        .map(|r| answered_or_died(r, &out, &entry.name))
        .collect()
}

/// Ask this tool about one file, dispatching on whichever kind of plugin the
/// entry is.
///
/// Public because it is what a run does, once per item: `doctor` asks about
/// two fixtures and `score` asks about a corpus, and they must ask the same
/// way or the self-test stops predicting anything about the run.
pub fn read_one(
    entry: &Entry,
    fixture: &Path,
    timeout: Duration,
    adapter_roots: &[PathBuf],
) -> Reading {
    read_one_observed(entry, fixture, timeout, adapter_roots, &mut Raw::default())
}

/// Runs an adapter on this machine, for tools that are services.
///
/// Environment is passed through from the host so the adapter can be told
/// where the service lives. Secrets are NOT handled here: a service consumes
/// its credentials when it starts, which is the operator's business, and this
/// only asks it a question.
fn run_host_adapter(
    entry: &Entry,
    fixture: &Path,
    timeout: Duration,
    adapter_roots: &[PathBuf],
    raw: &mut Raw,
) -> Reading {
    let Some(invoke) = &entry.invoke else {
        return Reading::Failed("entry declares no invoke block".into());
    };
    let Some(rel) = &invoke.adapter else {
        return Reading::Failed("a host invoke needs an adapter".into());
    };
    if !fixture.is_file() {
        return Reading::Failed(format!("fixture {} not found", fixture.display()));
    }
    let adapter = match crate::adapter::resolve(rel, adapter_roots) {
        Ok(p) => p.display().to_string(),
        Err(why) => return Reading::Failed(why.to_string()),
    };
    // Same reason as the binary route: the adapter runs on this machine and
    // would otherwise be handed a path that names the arm. See
    // [`PRESENTED_STEM`].
    let (_scratch, staged) = match presented_copy(fixture) {
        Ok(pair) => pair,
        Err(why) => return Reading::Failed(why),
    };
    let file = staged.display().to_string();
    let program = invoke
        .entrypoint
        .clone()
        .unwrap_or_else(|| crate::DEFAULT_HOST_ENTRYPOINT.into());
    let argv: Vec<String> = invoke
        .argv
        .iter()
        .map(|a| a.replace("{adapter}", &adapter).replace("{file}", &file))
        .collect();

    let mut cmd = Command::new(&program);
    cmd.args(&argv);
    for kv in &invoke.env {
        if let Some((k, v)) = kv.split_once('=') {
            cmd.env(k, v);
        }
    }
    match crate::exec::captured(cmd, &program, timeout) {
        Ok(out) => {
            raw.stdout = String::from_utf8_lossy(&out.stdout).into_owned();
            raw.stderr = String::from_utf8_lossy(&out.stderr).into_owned();
            parsers::parse(&invoke.parser, &raw.stdout.clone(), &raw.stderr.clone())
        }
        // Passed through rather than wrapped. The message already names the
        // tool and says what happened, and "could not run X: X gave no answer"
        // reports a timeout as a launch failure, which sends the reader to the
        // wrong problem.
        Err(e) => Reading::Failed(e),
    }
}

/// Asks a tool both questions.
///
/// `fixtures_dir` holds the files the entry's selftest block names, and
/// `adapter_roots` the trees a relative `invoke.adapter` is resolved against.
pub fn run(entry: &Entry, fixtures_dir: &Path, adapter_roots: &[PathBuf]) -> Verified {
    // An embedder cannot be asked to tell two images apart. The honest check
    // is whether what goes in comes back out.
    if entry.kind == stegobench_core::registry::Kind::Embedder {
        return roundtrip::run(entry, fixtures_dir);
    }
    let Some(test) = &entry.selftest else {
        return Verified::Skipped("no selftest declared".into());
    };
    if entry.invoke.is_none() {
        return Verified::Skipped(
            "no invoke block yet, so this tool cannot be driven by the host".into(),
        );
    }

    let detect_path = fixtures_dir.join(strip_prefix(&test.must_detect));
    let clear_path = fixtures_dir.join(strip_prefix(&test.must_clear));

    // A fixture this harness cannot find is not a fault in the tool, and
    // saying so in the vocabulary of a tool failure is how `doctor` came to
    // report six working detectors as BROKEN. Measured 2026-09-29: the same
    // binary, on the same machine, with the same registry, answered
    // "6 verified, 0 broken" inside a checkout and "0 verified, 6 broken" one
    // directory outside it, because the fixture paths resolved against the
    // directory the command was typed in.
    //
    // Could not ask and answered wrongly are different findings, and a reader
    // who cannot tell them apart will go and debug a detector that is fine.
    for path in [&detect_path, &clear_path] {
        if !path.is_file() {
            return Verified::Skipped(format!(
                "fixture {} was not found, so this tool was never asked. \
                 Name the directory with --fixtures <DIR>",
                path.display()
            ));
        }
    }

    // The same distinction for the adapter script. Resolved against the roots
    // the registry was read from, so reaching here means it genuinely is not on
    // this machine rather than that the command was typed somewhere unexpected.
    if let Some(declared) = entry.invoke.as_ref().and_then(|i| i.adapter.as_deref()) {
        if let Err(why) = crate::adapter::resolve(declared, adapter_roots) {
            return Verified::Skipped(format!("{why}, so this tool was never asked"));
        }
    }

    let threshold = test.threshold;
    let higher = entry.emits.higher_means_stego;

    // A self-test asks the same question a run asks, with the same deadline,
    // so that passing here predicts something about scoring a corpus.
    let on_stego = read_one(
        entry,
        &detect_path,
        crate::exec::ITEM_TIMEOUT,
        adapter_roots,
    );
    let on_clean = read_one(entry, &clear_path, crate::exec::ITEM_TIMEOUT, adapter_roots);

    let subject = entry.maintainer == stegobench_core::registry::Maintainer::Subject;

    match (
        on_stego.says_stego(higher, threshold),
        on_clean.says_stego(higher, threshold),
    ) {
        (Some(true), Some(false)) => Verified::Passed,
        // It responded to both, which is all a subject has to do to be
        // installed. What it answered is the measurement's business.
        (Some(_), Some(_)) if subject => Verified::Answered(
            "reachable and answering; it did not separate the fixtures, which \
             is a result rather than a fault"
                .into(),
        ),
        // Both halves are named separately, because "says yes to everything"
        // and "says no to everything" are different faults with different
        // fixes, and a single "failed" would hide which one it is.
        (Some(true), Some(true)) => Verified::Failed(
            "says stego on the clean fixture too, so it answers yes to everything".into(),
        ),
        (Some(false), Some(false)) => Verified::Failed(
            "says clean on the stego fixture too, so it answers no to everything".into(),
        ),
        (Some(false), Some(true)) => Verified::Failed("has both answers exactly backwards".into()),
        // A failure to answer at all IS an installation problem, for a
        // subject as much as anything else.
        (None, _) => Verified::Failed(format!("could not read the stego fixture: {on_stego:?}")),
        (_, None) => Verified::Failed(format!("could not read the clean fixture: {on_clean:?}")),
    }
}

/// Registry entries write `fixtures/clean.png`; the directory is passed
/// separately, so the prefix is dropped rather than doubled.
fn strip_prefix(name: &str) -> &str {
    name.strip_prefix("fixtures/").unwrap_or(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// As in `availability`: these tests are about the verdict, not about
    /// where an adapter lives, so they ask with no roots and get the same
    /// working-directory fallback the signature without roots had.
    fn run(entry: &Entry, fixtures_dir: &Path) -> Verified {
        super::run(entry, fixtures_dir, &[])
    }

    fn entry(extra: &str) -> Entry {
        toml::from_str(&format!(
            "name = \"x\"\nkind = \"detector\"\nlicence = \"MIT\"\n{extra}\n\
             [selftest]\nmust_detect = \"fixtures/a.png\"\nmust_clear = \"fixtures/b.png\"\n"
        ))
        .expect("parses")
    }

    /// The second layer under rung 5's `{files}` finding. The registry now
    /// refuses a batched binary entry outright, so this entry cannot be written
    /// in a TOML file any more; it is built here deliberately, because the
    /// defence is that a placeholder must never be handed to a tool as literal
    /// text whatever route it arrived by.
    #[cfg(unix)]
    #[test]
    fn a_binary_handed_the_batch_placeholder_still_receives_a_path() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().expect("tmp");
        // Prints the argument it was given, so the test reads what the tool saw
        // rather than what the harness believes it sent.
        let script = tmp.path().join("echoer.sh");
        std::fs::write(
            &script,
            "#!/bin/sh\n[ \"$1\" = \"--version\" ] && { echo v1; exit 0; }\n\
             case \"$1\" in *.png) echo 0.9 ;; *) echo 0.1 ;; esac\n",
        )
        .unwrap();
        let mut perms = std::fs::metadata(&script).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script, perms).unwrap();

        let e = entry(&format!(
            "[binary]\ncommand = [{:?}]\nversion_args = [\"--version\"]\n\
             [emits]\noutput = \"score\"\nhigher_means_stego = true\n\
             [invoke]\nargv = [\"{{files}}\"]\nparser = \"number\"",
            script.display().to_string()
        ));
        let fixture = tmp.path().join("a.png");
        std::fs::write(&fixture, b"\x89PNG\r\n\x1a\n").unwrap();

        // 0.9 is the branch the script takes when it was given the path. The
        // literal placeholder would take the other branch, so the assertion
        // distinguishes a substituted argument from an unsubstituted one rather
        // than merely checking that something came back.
        assert_eq!(
            read_one(&e, &fixture, Duration::from_secs(5), &[]),
            Reading::Score(0.9),
            "the tool was handed the placeholder instead of the image"
        );
    }

    /// A stand-in detector, written to a scratch directory and made runnable.
    ///
    /// It answers `--version` first, because a registry entry has to declare
    /// `version_args` and the probe is run before anything else.
    #[cfg(unix)]
    fn stand_in(dir: &Path, name: &str, body: &str) -> Entry {
        use std::os::unix::fs::PermissionsExt;
        let script = dir.join(name);
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\n[ \"$1\" = \"--version\" ] && {{ echo 'stand-in 1.0'; exit 0; }}\n\
                 {body}\n"
            ),
        )
        .expect("script");
        let mut perms = std::fs::metadata(&script).expect("meta").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script, perms).expect("chmod");
        entry(&format!(
            "[binary]\ncommand = [{:?}]\nversion_args = [\"--version\"]\n\
             [emits]\noutput = \"score\"\nhigher_means_stego = true\n\
             [invoke]\nargv = [\"{{file}}\"]\nparser = \"number\"",
            script.display().to_string()
        ))
    }

    /// The two fixtures `entry` names, with the stego one carrying a marker in
    /// its bytes so an honest stand-in has something to find.
    #[cfg(unix)]
    fn two_fixtures(dir: &Path) -> &Path {
        std::fs::write(dir.join("a.png"), b"\x89PNG\r\n\x1a\nPAYLOAD").expect("stego fixture");
        std::fs::write(dir.join("b.png"), b"\x89PNG\r\n\x1a\nnothing").expect("clean fixture");
        dir
    }

    /// Rung 5, 2026-10-02. The researcher's plugin did no steganalysis at all:
    /// its entire decision was the basename, `*lsb*` or `*appended*` meant
    /// 0.990 and everything else meant 0.010. It passed the two-sided check,
    /// earned the `verified` badge, and the result document it produced
    /// recorded `selftest: "passed"`.
    ///
    /// Named after the real fixtures rather than the test's own, because the
    /// public fixture names are the information that was leaking.
    #[cfg(unix)]
    #[test]
    fn a_detector_that_keys_on_the_fixture_name_can_no_longer_pass() {
        let tmp = tempfile::tempdir().expect("tmp");
        let e = stand_in(
            tmp.path(),
            "cheat.sh",
            "b=$(basename \"$1\")\n\
             case \"$b\" in *lsb*|*appended*|*a.png) echo 0.990 ;; *) echo 0.010 ;; esac",
        );
        let fx = tempfile::tempdir().expect("fixtures");
        match run(&e, two_fixtures(fx.path())) {
            Verified::Failed(why) => assert!(why.contains("no to everything"), "got {why}"),
            other => panic!("a name-keyed plugin was verified: {other:?}"),
        }
    }

    /// The other half, and the half that matters more: a plugin that actually
    /// reads the bytes must be unaffected.
    #[cfg(unix)]
    #[test]
    fn a_detector_that_reads_the_bytes_still_passes() {
        let tmp = tempfile::tempdir().expect("tmp");
        let e = stand_in(
            tmp.path(),
            "honest.sh",
            "if grep -q PAYLOAD \"$1\"; then echo 0.9; else echo 0.1; fi",
        );
        let fx = tempfile::tempdir().expect("fixtures");
        match run(&e, two_fixtures(fx.path())) {
            Verified::Passed => {}
            other => panic!("an honest plugin stopped passing: {other:?}"),
        }
    }

    /// The verdict is reproducible although the path is not.
    ///
    /// The staging directory is deliberately unpredictable, so this is the
    /// property that makes that safe: two runs of one tool over the same bytes
    /// reach the same answer, for the honest tool and the cheat alike.
    #[cfg(unix)]
    #[test]
    fn two_runs_reach_the_same_verdict_although_the_path_differs() {
        let tmp = tempfile::tempdir().expect("tmp");
        let honest = stand_in(
            tmp.path(),
            "honest.sh",
            "if grep -q PAYLOAD \"$1\"; then echo 0.9; else echo 0.1; fi",
        );
        let cheat = stand_in(
            tmp.path(),
            "cheat.sh",
            "case \"$(basename \"$1\")\" in *a.png) echo 0.9 ;; *) echo 0.1 ;; esac",
        );
        let fx = tempfile::tempdir().expect("fixtures");
        let dir = two_fixtures(fx.path());
        for e in [&honest, &cheat] {
            assert_eq!(run(e, dir), run(e, dir), "the same tool answered twice");
        }
    }

    /// The tool is handed a name that says nothing, and an extension that says
    /// everything it legitimately needs.
    ///
    /// Asserted through what the TOOL saw rather than through what this code
    /// believes it sent, which is the only version of this assertion worth
    /// having.
    #[cfg(unix)]
    #[test]
    fn a_tool_is_handed_a_neutral_stem_and_its_own_extension() {
        let tmp = tempfile::tempdir().expect("tmp");
        let e = stand_in(
            tmp.path(),
            "reporter.sh",
            "case \"$(basename \"$1\")\" in image.png) echo 0.9 ;; *) echo 0.1 ;; esac",
        );
        let fixture = tmp.path().join("lsb-0.4bpp.png");
        std::fs::write(&fixture, b"\x89PNG\r\n\x1a\n").expect("fixture");
        assert_eq!(
            read_one(&e, &fixture, Duration::from_secs(5), &[]),
            Reading::Score(0.9),
            "the tool saw a name other than the neutral one"
        );
    }

    /// A label sitting beside the image is out of reach too.
    ///
    /// Pentimento writes a JSON sidecar next to every image, and a detector
    /// that reads it is reading the answer. Staging the image alone in a fresh
    /// directory closes that without anybody having to remember.
    #[cfg(unix)]
    #[test]
    fn a_label_file_beside_the_image_cannot_be_read() {
        let tmp = tempfile::tempdir().expect("tmp");
        let e = stand_in(
            tmp.path(),
            "peeker.sh",
            "if [ -f \"$(dirname \"$1\")/lsb-0.4bpp.json\" ]; then echo 0.9; else echo 0.1; fi",
        );
        let fixture = tmp.path().join("lsb-0.4bpp.png");
        std::fs::write(&fixture, b"\x89PNG\r\n\x1a\n").expect("fixture");
        std::fs::write(tmp.path().join("lsb-0.4bpp.json"), b"{\"arm\":\"lsb\"}").expect("sidecar");
        assert_eq!(
            read_one(&e, &fixture, Duration::from_secs(5), &[]),
            Reading::Score(0.1),
            "the tool could still read the label beside the image"
        );
    }

    /// The third route, which the researcher did not reach and which leaks
    /// exactly the same thing.
    ///
    /// A `host = true` entry runs its adapter on this machine, so like the
    /// binary route it has no mount to rename the file for it. StegaShield is
    /// registered this way and is a subject, which is the entry a flattering
    /// result would most want.
    #[cfg(unix)]
    #[test]
    fn a_host_adapter_is_handed_the_neutral_name_too() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().expect("tmp");
        let adapter = tmp.path().join("adapter.sh");
        std::fs::write(
            &adapter,
            "#!/bin/sh\ncase \"$(basename \"$1\")\" in image.png) echo 0.9 ;; *) echo 0.1 ;; esac\n",
        )
        .expect("adapter");
        let mut perms = std::fs::metadata(&adapter).expect("meta").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&adapter, perms).expect("chmod");

        let e = entry(&format!(
            "[image]\nreference = \"x@sha256:a\"\nsize_mb = 1\nbundled = true\n\
             [emits]\noutput = \"score\"\nhigher_means_stego = true\n\
             [invoke]\nhost = true\nentrypoint = \"sh\"\nadapter = {:?}\n\
             argv = [\"{{adapter}}\", \"{{file}}\"]\nparser = \"number\"",
            adapter.display().to_string()
        ));
        let fixture = tmp.path().join("lsb-0.4bpp.png");
        std::fs::write(&fixture, b"\x89PNG\r\n\x1a\n").expect("fixture");
        assert_eq!(
            read_one(&e, &fixture, Duration::from_secs(5), &[]),
            Reading::Score(0.9),
            "the adapter saw a name other than the neutral one"
        );
    }

    #[test]
    fn the_presented_name_keeps_the_extension_and_drops_everything_else() {
        assert_eq!(presented_name(Path::new("/x/lsb-0.4bpp.png")), "image.png");
        assert_eq!(presented_name(Path::new("/x/clean.JPG")), "image.JPG");
        assert_eq!(presented_name(Path::new("/x/appended")), "image");
        // The container route builds its mount from exactly this, so the two
        // fixtures arrive at one path inside two different containers and are
        // indistinguishable by name.
        assert_eq!(
            presented_name(Path::new("/x/a.png")),
            presented_name(Path::new("/y/b.png"))
        );
    }

    #[test]
    fn a_tool_with_no_invoke_block_is_skipped_not_failed() {
        // Skipped and Failed must stay distinct: "we have not taught the host
        // to drive this yet" is not "this tool is broken".
        let e = entry("[image]\nreference = \"x@sha256:a\"\nsize_mb = 10\nbundled = true");
        assert!(matches!(run(&e, Path::new(".")), Verified::Skipped(_)));
    }

    #[test]
    fn a_binary_tool_is_now_run_rather_than_skipped() {
        // It used to be skipped as "containerised tools only". Binaries run
        // directly now, so a missing one is a FAILURE naming the program
        // rather than a silent pass: the whole point of the check is that a
        // tool we cannot run is not a tool that works.
        let e = entry(
            "[binary]\ncommand = [\"definitely-not-real-xyzzy\"]\nversion_args = [\"-v\"]\n\
             [invoke]\nargv = [\"/work\"]\nwritable_workdir = true\nparser = \"stegcore\"",
        );
        let fx = fixtures_present();
        match run(&e, fx.path()) {
            Verified::Failed(r) => assert!(r.contains("not on PATH"), "got {r}"),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    /// A directory holding the two images `entry` declares.
    ///
    /// The tests below are about resolving the TOOL, so the fixtures have to
    /// be real: since 2026-09-29 an absent fixture short circuits `run` before
    /// the tool is reached, which is the point of that change. Passing a
    /// directory with nothing in it would make these tests pass for the wrong
    /// reason and stop covering what they are named for.
    fn fixtures_present() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("scratch directory");
        for name in ["a.png", "b.png"] {
            std::fs::write(dir.path().join(name), b"not really a png").expect("fixture written");
        }
        dir
    }

    #[test]
    fn a_host_adapter_entry_is_dispatched_to_the_host_not_the_image() {
        // A service names an image to identify the subject, but there is
        // nothing to run inside it.
        let e = entry(
            "[image]\nreference = \"x@sha256:a\"\nsize_mb = 10\nbundled = true\n\
             [invoke]\nhost = true\nargv = [\"{adapter}\"]\nparser = \"number\"",
        );
        let fx = fixtures_present();
        match run(&e, fx.path()) {
            Verified::Failed(r) => assert!(r.contains("adapter"), "got {r}"),
            other => panic!("expected Failed about the adapter, got {other:?}"),
        }
    }

    #[test]
    fn the_fixtures_prefix_is_not_doubled() {
        assert_eq!(strip_prefix("fixtures/clean.png"), "clean.png");
        assert_eq!(strip_prefix("clean.png"), "clean.png");
    }

    #[test]
    fn a_missing_fixture_is_skipped_with_the_path_and_never_called_a_failure() {
        // Renamed and inverted on 2026-09-29. It used to assert `Failed`, and
        // that assertion was the bug: `doctor` reported "0 verified, 6 broken"
        // from outside a checkout and "6 verified, 0 broken" inside one, with
        // the same binary and the same tools, because a fixture this harness
        // could not find was reported in the vocabulary of a tool failure.
        //
        // The distinction is the whole point, so it is asserted both ways:
        // the path is still named, and the verdict is no longer an accusation.
        let e = entry(
            "[image]\nreference = \"x@sha256:a\"\nsize_mb = 10\nbundled = true\n\
             [invoke]\nargv = [\"true\"]\nparser = \"zsteg\"",
        );
        match run(&e, Path::new("/definitely/not/here")) {
            Verified::Skipped(r) => {
                assert!(r.contains("not found"), "got {r}");
                assert!(r.contains("/definitely/not/here"), "the path is named: {r}");
                assert!(r.contains("--fixtures"), "the way out is named: {r}");
            }
            other => panic!("expected Skipped, got {other:?}"),
        }
    }

    /// The round trip payload has to stay inside the band that was measured,
    /// and a doc comment is not a mechanism.
    ///
    /// Below 96 bytes outguess silently drops the tail bits of the payload and
    /// still exits 0 on both halves, so a shorter payload turns this check into
    /// a coin flip that reports a working tool as broken. Above 212 bytes
    /// fixtures/clean.jpg cannot carry it and outguess refuses outright. The
    /// constant's own comment explains both ends; this is what stops the next
    /// reader shortening it back.
    #[test]
    fn the_roundtrip_payload_stays_inside_the_measured_band() {
        assert_eq!(
            roundtrip::PAYLOAD.len(),
            128,
            "the round trip payload must be 128 bytes: read the comment on \
             roundtrip::PAYLOAD before changing it"
        );
    }
}

/// The round trip against a LOCAL program rather than a container.
///
/// Stegcore is registered as an embedder and is a binary rather than an image,
/// so without these the one entry that takes this path is unproven on every
/// machine that does not have the program installed, which is every machine in
/// CI.
#[cfg(all(test, unix))]
mod local_roundtrip_tests {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    use stegobench_core::registry::Entry;

    use super::{read_one_observed, roundtrip, Raw, Verified};
    use crate::parsers::Reading;
    use std::time::Duration;

    /// A stand-in embedder: `embed` concatenates a marker and the payload,
    /// `extract` gives the payload back. Enough to exercise every branch of
    /// the runner without installing a real tool.
    fn fake_tool(dir: &Path, name: &str, body: &str) -> String {
        let path = dir.join(name);
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
             [binary]\ncommand = [{command}]\n\
             [roundtrip]\ncover = \"fixtures/clean.png\"\n\
             embed_argv = [\"embed\", \"{{cover}}\", \"{{payload}}\", \"{{stego}}\"]\n\
             extract_argv = [\"extract\", \"{{stego}}\", \"{{recovered}}\"]\n\
             [selftest]\nmust_detect = \"fixtures/a.png\"\nmust_clear = \"fixtures/b.png\"\n"
        ))
        .expect("parses")
    }

    fn fixtures(dir: &Path) -> &Path {
        std::fs::write(dir.join("clean.png"), b"\x89PNG\r\n\x1a\ncover bytes").expect("cover");
        dir
    }

    /// A detector entry whose program is the given script.
    fn detector(command: &str) -> Entry {
        toml::from_str(&format!(
            "name = \"crasher\"\nkind = \"detector\"\nlicence = \"MIT\"\n\
             [binary]\ncommand = [{command}]\nversion_args = [\"-v\"]\n\
             [emits]\noutput = \"verdict\"\n\
             [invoke]\nargv = [\"{{file}}\"]\nparser = \"zsteg\"\n\
             [selftest]\nmust_detect = \"a.png\"\nmust_clear = \"b.png\"\n"
        ))
        .expect("parses")
    }

    /// A TOOL THAT CRASHED HAS NOT CALLED AN IMAGE CLEAN.
    ///
    /// A forensic analyst was shown `clean` for four exhibits out of six on
    /// 2026-10-02, with exit code 0 and "0 errored" underneath, and every one
    /// of the four was a Ruby traceback from a detector that could not start.
    /// The parser found no finding in an empty stdout and returned the verdict
    /// that means clean, which is the same value a working detector returns
    /// when it genuinely found nothing.
    #[test]
    fn a_detector_that_exits_nonzero_saying_nothing_has_not_found_an_image_clean() {
        let tmp = tempfile::tempdir().expect("tmp");
        // Exactly zsteg's shape under this sandbox: nothing on stdout, a
        // complaint on stderr, exit 1.
        let tool = fake_tool(
            tmp.path(),
            "crash.sh",
            "case \"$1\" in -v) echo 1.0; exit 0;; esac\n\
             echo \"tempfile.rb:159: Read-only file system (Errno::EROFS)\" >&2\n\
             exit 1",
        );
        let e = detector(&format!("{tool:?}"));
        let img = tmp.path().join("exhibit.png");
        std::fs::write(&img, b"\x89PNG\r\n\x1a\nnot really a png").expect("image");
        let mut raw = Raw::default();
        let reading = read_one_observed(&e, &img, Duration::from_secs(30), &[], &mut raw);
        match reading {
            Reading::Failed(why) => {
                assert!(
                    why.contains("exited 1"),
                    "the exit code belongs in it: {why}"
                );
                assert!(
                    why.contains("not a clean result"),
                    "the whole point is that it is NOT clean: {why}"
                );
            }
            other => panic!(
                "a crash was read as an answer about the image: {other:?}. This is \
                 the defect that put four stack traces in front of a forensic \
                 analyst as evidence of absence"
            ),
        }
    }

    /// The other half, and it is not symmetric: zsteg on a JPEG prints its
    /// finding to stderr and THEN dies, so a tool that found something before
    /// falling over has still found something.
    #[test]
    fn a_detector_that_reports_a_hit_and_then_dies_keeps_the_hit() {
        let tmp = tempfile::tempdir().expect("tmp");
        let tool = fake_tool(
            tmp.path(),
            "hit-then-crash.sh",
            "case \"$1\" in -v) echo 1.0; exit 0;; esac\n\
             echo \"[?] 29 bytes of extra data after image end\"\n\
             echo \"then it fell over\" >&2\n\
             exit 1",
        );
        let e = detector(&format!("{tool:?}"));
        let img = tmp.path().join("exhibit.png");
        std::fs::write(&img, b"\x89PNG\r\n\x1a\nnot really a png").expect("image");
        let mut raw = Raw::default();
        match read_one_observed(&e, &img, Duration::from_secs(30), &[], &mut raw) {
            Reading::Verdict(true) => {}
            other => panic!("a finding printed before the crash was thrown away: {other:?}"),
        }
    }

    #[test]
    fn a_local_tool_that_returns_the_payload_passes() {
        let tmp = tempfile::tempdir().expect("tmp");
        // A fixed-length marker rather than the cover's own bytes, so extract
        // knows the offset without being handed the cover, which is the
        // position a real extractor is in.
        let tool = fake_tool(
            tmp.path(),
            "tool.sh",
            "case \"$1\" in\n\
             embed) { printf 'STEGO:'; cat \"$3\"; } > \"$4\" ;;\n\
             extract) tail -c +7 \"$2\" > \"$3\" ;;\n\
             esac",
        );
        let e = entry(&format!("{tool:?}"));
        let fix = tempfile::tempdir().expect("fixtures");
        match roundtrip::run(&e, fixtures(fix.path())) {
            Verified::Passed => {}
            other => panic!("expected Passed, got {other:?}"),
        }
    }

    #[test]
    fn a_local_tool_that_writes_nothing_fails_rather_than_passing_quietly() {
        // Exiting zero having written no stego file is the silent failure the
        // whole check exists to catch, and it is easier to hit locally than in
        // a container because there is no image to be missing first.
        let tmp = tempfile::tempdir().expect("tmp");
        let tool = fake_tool(tmp.path(), "quiet.sh", "exit 0");
        let e = entry(&format!("{tool:?}"));
        let fix = tempfile::tempdir().expect("fixtures");
        match roundtrip::run(&e, fixtures(fix.path())) {
            Verified::Failed(r) => assert!(r.contains("wrote no stego file"), "got {r}"),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn a_local_tool_that_returns_the_wrong_bytes_fails() {
        // A tool that truncates or pads has not worked, and every comparison
        // weaker than byte equality has a way of passing when it should not.
        let tmp = tempfile::tempdir().expect("tmp");
        let tool = fake_tool(
            tmp.path(),
            "lossy.sh",
            "case \"$1\" in\n\
             embed) cat \"$2\" \"$3\" > \"$4\" ;;\n\
             extract) printf 'not the payload' > \"$3\" ;;\n\
             esac",
        );
        let e = entry(&format!("{tool:?}"));
        let fix = tempfile::tempdir().expect("fixtures");
        match roundtrip::run(&e, fixtures(fix.path())) {
            // Was `contains("did not come back")`. The size now comes from
            // PAYLOAD rather than being written out, so shortening the payload
            // cannot leave this assertion quietly passing against a stale
            // number.
            Verified::Failed(r) => {
                assert!(r.contains("did not survive the round trip"), "got {r}");
                assert!(
                    r.contains(&format!("{} bytes went in", roundtrip::PAYLOAD.len())),
                    "the payload size: {r}"
                );
                assert!(r.contains("15 came back"), "what came back: {r}");
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn a_tool_that_is_not_installed_fails_naming_the_program() {
        // Distinct from "wrote nothing": the reader has to know whether to
        // install something or to report a bug against the tool.
        let e = entry("\"definitely-not-installed-anywhere\"");
        let fix = tempfile::tempdir().expect("fixtures");
        match roundtrip::run(&e, fixtures(fix.path())) {
            Verified::Failed(r) => {
                assert!(r.contains("embed failed"), "got {r}");
                assert!(r.contains("definitely-not-installed-anywhere"), "got {r}");
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn a_tool_that_never_answers_is_killed_rather_than_waited_on_for_ever() {
        // The usual cause is a passphrase prompt on a terminal nobody is
        // watching, which a round trip driven from a registry entry provokes
        // easily. Unbounded, `doctor` hangs and reports nothing at all.
        use std::time::Duration;
        let mut sleeper = std::process::Command::new("sh");
        sleeper.args(["-c", "sleep 30"]);
        let started = std::time::Instant::now();
        let outcome = crate::embed::bounded(sleeper, "sh", Duration::from_millis(200));
        assert!(started.elapsed() < Duration::from_secs(10), "it waited");
        match outcome {
            Err(why) => assert!(why.contains("no answer in"), "got {why}"),
            Ok(()) => panic!("a tool that never answered was reported as fine"),
        }
    }

    #[test]
    fn an_entry_with_neither_an_image_nor_a_binary_is_skipped() {
        // Skipped and Failed stay distinct: nothing to run is not the same as
        // something that ran and did not work.
        let e: Entry = toml::from_str(
            "name = \"x\"\nkind = \"embedder\"\nlicence = \"MIT\"\n\
             [roundtrip]\ncover = \"fixtures/clean.png\"\n\
             embed_argv = [\"e\"]\nextract_argv = [\"x\"]\n\
             [selftest]\nmust_detect = \"fixtures/a.png\"\nmust_clear = \"fixtures/b.png\"\n",
        )
        .expect("parses");
        let fix = tempfile::tempdir().expect("fixtures");
        match roundtrip::run(&e, fixtures(fix.path())) {
            Verified::Skipped(r) => assert!(r.contains("no image and no binary"), "got {r}"),
            other => panic!("expected Skipped, got {other:?}"),
        }
    }
}

/// Proving an embedder: what goes in must come back out.
pub mod roundtrip {
    use std::path::Path;

    use stegobench_core::registry::Entry;

    use super::Verified;

    /// The payload. 128 bytes, and THE LENGTH IS LOAD BEARING: do not shorten
    /// it, and do not tidy it back into a one line string.
    ///
    /// Recognisable and not compressible into nothing, so a tool that silently
    /// wrote an empty file cannot pass by accident. That much was always true
    /// and is not the part that is easy to get wrong.
    ///
    /// The length is. OutGuess 0.4 walks the cover with a stride scaled to how
    /// much payload is left (`iterator_adapt` in its own source), and nothing
    /// bounds that walk against the end of the bitmap. With a short payload the
    /// stride is coarse enough to run off the end part way through the final
    /// byte. The bits that fall off are never written, `steg_embedchunk` returns
    /// success anyway, and extraction reads past the end, so the payload comes
    /// back the RIGHT LENGTH with a wrong bit in its last byte and both halves
    /// exit 0. outguess is deterministic, so a payload that lands on that case
    /// fails every single time, for ever, and reports a working tool as broken.
    /// A permanent red gets believed, which is worse than a flaky one.
    ///
    /// Measured 2026-10-01 against the digest the registry pins, 40 random
    /// payloads per cell across three covers including fixtures/clean.jpg: bits
    /// were dropped at 32, 48, 64 and 69 bytes (16 of 40 on one cover) and never
    /// once at 96, 128, 192 or 256. A further 750 round trips at 128 bytes, 250
    /// on each of the three covers, dropped nothing and recovered every byte.
    ///
    /// The other end of the band is capacity. fixtures/clean.jpg carries 4622
    /// usable bits of which outguess will use 1697, so it refuses anything over
    /// 212 bytes on it. 128 bytes is 1056 bits, 62% of that, which leaves the
    /// fixture room to be replaced. The usable band is roughly 96 to 212 bytes
    /// and this sits in the middle of it on purpose.
    ///
    /// To change it, re-measure. Embed the candidate and check that outguess
    /// prints `Bits embedded: N` with N equal to (4 + payload length) * 8. A
    /// short count means bits were dropped, whether or not the recovered bytes
    /// happen to come back right that time.
    pub const PAYLOAD: &[u8] = b"stegobench roundtrip fixture 2026: if you can \
        read this, it survived. 128 bytes on purpose: do not shorten it, read \
        the comment.";

    /// Hides the payload, recovers it, and compares the bytes.
    ///
    /// Byte comparison rather than a size check or a substring: a tool that
    /// returns a truncated or padded payload has not worked, and every weaker
    /// comparison has a way of passing when it should not.
    ///
    /// The running itself is [`crate::embed`], which is also what
    /// `stegobench embed` drives, so this check exercises the same code path
    /// a user does. A self-test passing against a sandbox the real command
    /// does not use is a self-test measuring the wrong thing.
    pub fn run(entry: &Entry, fixtures_dir: &Path) -> Verified {
        let Some(rt) = &entry.roundtrip else {
            return Verified::Skipped("no roundtrip declared".into());
        };
        let cover_src = fixtures_dir.join(rt.cover.trim_start_matches("fixtures/"));
        if !cover_src.is_file() {
            // Skipped, not Failed: an absent fixture is this harness failing
            // to find its own file, and reporting it as a round trip failure
            // blames the embedder for it.
            return Verified::Skipped(format!(
                "cover {} was not found, so no round trip was attempted. \
                 Name the directory with --fixtures <DIR>",
                cover_src.display()
            ));
        }
        let Ok(dir) = tempfile::tempdir() else {
            return Verified::Failed("no scratch directory".into());
        };
        let payload = dir.path().join("payload.bin");
        if std::fs::write(&payload, PAYLOAD).is_err() {
            return Verified::Failed("could not stage the payload".into());
        }
        let ext = cover_src
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("bin");
        let out = dir.path().join(format!("stego.{ext}"));

        match crate::embed::run(entry, &cover_src, &payload, &out, None, true) {
            // A tool with no image and no binary is not a failing tool.
            Err(e) if e.starts_with("no image and no binary") => Verified::Skipped(e),
            Err(e) => Verified::Failed(e),
            Ok(done) => match done.recovered {
                Some(Ok(())) => Verified::Passed,
                Some(Err(e)) => Verified::Failed(e),
                None => Verified::Skipped("no extract argv to check the embed with".into()),
            },
        }
    }
}
