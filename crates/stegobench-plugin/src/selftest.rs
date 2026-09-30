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
    let Some(name) = absolute.file_name().and_then(|n| n.to_str()) else {
        return Reading::Failed("fixture has no usable filename".into());
    };
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
    parsers::parse(&invoke.parser, &text, &raw.stderr.clone())
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
    let file = fixture.display().to_string();
    let mut argv: Vec<String> = bin.command[1..].to_vec();
    argv.extend(invoke.argv.iter().map(|a| a.replace("{file}", &file)));

    let mut program_cmd = Command::new(&path);
    program_cmd.args(&argv);
    match crate::exec::captured(program_cmd, program, timeout) {
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
    let file = fixture.display().to_string();
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
             [invoke]\nargv = [\"{file}\"]\nparser = \"stegcore\"",
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

    use super::{roundtrip, Verified};

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
            Verified::Failed(r) => assert!(r.contains("did not come back"), "got {r}"),
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

    /// The payload. Short, recognisable, and not compressible into nothing,
    /// so a tool that silently wrote an empty file cannot pass by accident.
    pub const PAYLOAD: &[u8] =
        b"stegobench roundtrip fixture 2026: if you can read this, it survived.";

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
