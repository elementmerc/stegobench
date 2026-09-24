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

use std::path::Path;
use std::process::Command;

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

/// Runs one image through a containerised tool and parses what comes back.
///
/// The container is locked down the way every run in this project is: no
/// network, no capabilities, no new privileges, read-only root, and a memory
/// cap. A self-test is still running somebody else's code on our machine.
fn run_one(entry: &Entry, image: &str, fixture: &Path) -> Reading {
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

    let mut args: Vec<String> = vec![
        "run".into(),
        "--rm".into(),
        "--network=none".into(),
        "--cap-drop=ALL".into(),
        "--security-opt".into(),
        "no-new-privileges".into(),
        "--read-only".into(),
        "--memory=2g".into(),
        "-v".into(),
        mount,
    ];

    // An adapter is mounted read-only beside the image it reads.
    let mut adapter_inner = String::new();
    if let Some(rel) = &invoke.adapter {
        let Ok(abs) = Path::new(rel).canonicalize() else {
            return Reading::Failed(format!("adapter {rel} not found"));
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

    let out = match Command::new("docker").args(&args).output() {
        Ok(out) => out,
        Err(e) => return Reading::Failed(format!("could not run the container: {e}")),
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

    parsers::parse(&invoke.parser, &text, &String::from_utf8_lossy(&out.stderr))
}

/// Runs a binary plugin directly, with no container.
///
/// Our own tool and anything else already installed on the machine. There is
/// no isolation here and that is a deliberate limit: a binary entry says the
/// operator already trusts this program enough to have installed it, which is
/// a different statement from pulling a stranger's image.
fn run_binary(entry: &Entry, fixture: &Path) -> Reading {
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

    match Command::new(&path).args(&argv).output() {
        Ok(out) => parsers::parse(
            &invoke.parser,
            &String::from_utf8_lossy(&out.stdout),
            &String::from_utf8_lossy(&out.stderr),
        ),
        Err(e) => Reading::Failed(format!("could not run {program}: {e}")),
    }
}

/// Dispatches to whichever kind of plugin this entry is.
fn run_any(entry: &Entry, fixture: &Path) -> Reading {
    // A host adapter wins over the image: the entry names an image to identify
    // the subject, but the thing to run is here, not in it.
    if entry.invoke.as_ref().is_some_and(|i| i.host) {
        return run_host_adapter(entry, fixture);
    }
    match (&entry.image, &entry.binary) {
        (Some(img), _) => run_one(entry, &img.reference.clone(), fixture),
        (_, Some(_)) => run_binary(entry, fixture),
        _ => Reading::Failed("entry declares neither an image nor a binary".into()),
    }
}

/// Runs an adapter on this machine, for tools that are services.
///
/// Environment is passed through from the host so the adapter can be told
/// where the service lives. Secrets are NOT handled here: a service consumes
/// its credentials when it starts, which is the operator's business, and this
/// only asks it a question.
fn run_host_adapter(entry: &Entry, fixture: &Path) -> Reading {
    let Some(invoke) = &entry.invoke else {
        return Reading::Failed("entry declares no invoke block".into());
    };
    let Some(rel) = &invoke.adapter else {
        return Reading::Failed("a host invoke needs an adapter".into());
    };
    if !fixture.is_file() {
        return Reading::Failed(format!("fixture {} not found", fixture.display()));
    }
    let adapter = match Path::new(rel).canonicalize() {
        Ok(p) => p.display().to_string(),
        Err(e) => return Reading::Failed(format!("adapter {rel} not found: {e}")),
    };
    let file = fixture.display().to_string();
    let program = invoke
        .entrypoint
        .clone()
        .unwrap_or_else(|| "python3".into());
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
    match cmd.output() {
        Ok(out) => parsers::parse(
            &invoke.parser,
            &String::from_utf8_lossy(&out.stdout),
            &String::from_utf8_lossy(&out.stderr),
        ),
        Err(e) => Reading::Failed(format!("could not run {program}: {e}")),
    }
}

/// Asks a tool both questions.
///
/// `fixtures_dir` holds the files the entry's selftest block names.
pub fn run(entry: &Entry, fixtures_dir: &Path) -> Verified {
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

    let threshold = test.threshold;
    let higher = entry.emits.higher_means_stego;

    let on_stego = run_any(entry, &detect_path);
    let on_clean = run_any(entry, &clear_path);

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
        match run(&e, Path::new(".")) {
            Verified::Failed(r) => assert!(r.contains("not on PATH"), "got {r}"),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn a_host_adapter_entry_is_dispatched_to_the_host_not_the_image() {
        // A service names an image to identify the subject, but there is
        // nothing to run inside it.
        let e = entry(
            "[image]\nreference = \"x@sha256:a\"\nsize_mb = 10\nbundled = true\n\
             [invoke]\nhost = true\nargv = [\"{adapter}\"]\nparser = \"number\"",
        );
        match run(&e, Path::new(".")) {
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
    fn a_missing_fixture_is_a_failure_with_the_path() {
        let e = entry(
            "[image]\nreference = \"x@sha256:a\"\nsize_mb = 10\nbundled = true\n\
             [invoke]\nargv = [\"true\"]\nparser = \"zsteg\"",
        );
        match run(&e, Path::new("/definitely/not/here")) {
            Verified::Failed(r) => assert!(r.contains("not found"), "got {r}"),
            other => panic!("expected Failed, got {other:?}"),
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
        let outcome = roundtrip::bounded(sleeper, "sh", Duration::from_millis(200));
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
    use std::io::Read;
    use std::path::Path;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    use stegobench_core::registry::Entry;

    use super::Verified;

    /// The payload. Short, recognisable, and not compressible into nothing,
    /// so a tool that silently wrote an empty file cannot pass by accident.
    pub const PAYLOAD: &[u8] =
        b"stegobench roundtrip fixture 2026: if you can read this, it survived.";

    /// How the tool is reached, which decides both the argv and the paths.
    ///
    /// A container sees the scratch directory at `/work`; a local program sees
    /// it where it actually is. Substituting the wrong one produces a tool
    /// that exits cleanly having written nothing, which is the failure this
    /// check exists to catch and would be blamed on the tool.
    enum Reach {
        Container(String),
        Local(Vec<String>),
    }

    /// How long either phase is given before it is killed.
    ///
    /// A self-test is a smoke test on one small fixture, so anything past this
    /// is a tool waiting on something that is never coming: a passphrase
    /// prompt on a terminal nobody is watching is the usual one, and it is
    /// exactly what a round trip driven from a registry entry can provoke.
    /// Without a bound, `doctor` hangs for ever and reports nothing at all.
    pub(crate) const PHASE_TIMEOUT: Duration = Duration::from_secs(300);

    /// Run one phase and wait for it, but not for ever.
    ///
    /// Standard output goes nowhere and standard error is drained on its own
    /// thread. Both matter: a tool that fills a pipe blocks on the write while
    /// the parent waits for it to exit, which is the deadlock a naive timeout
    /// introduces, and it would look exactly like the hang being fixed.
    pub(crate) fn bounded(
        mut command: Command,
        label: &str,
        timeout: Duration,
    ) -> Result<(), String> {
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let mut child = command
            .spawn()
            .map_err(|e| format!("could not run {label}: {e}"))?;
        let mut pipe = child.stderr.take();
        let (finished, drained) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(p) = pipe.as_mut() {
                let _ = p.read_to_end(&mut buf);
            }
            let _ = finished.send(buf);
        });

        let deadline = Instant::now() + timeout;
        let status = loop {
            match child.try_wait() {
                Ok(Some(s)) => break Some(s),
                Ok(None) => {}
                Err(e) => return Err(format!("could not wait for {label}: {e}")),
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            std::thread::sleep(Duration::from_millis(20));
        };

        // Bounded too, and for the same reason the wait above is. A tool that
        // forks leaves a grandchild holding the write end of this pipe, so it
        // stays open after the process we killed is gone, and joining on it
        // would reintroduce the hang one line after fixing it. What we lose by
        // not waiting is the tail of a message from a tool that has already
        // failed; what we would lose by waiting is the whole check.
        let stderr = drained
            .recv_timeout(Duration::from_secs(2))
            .unwrap_or_default();
        let tail = String::from_utf8_lossy(&stderr)
            .trim()
            .chars()
            .take(160)
            .collect::<String>();
        match status {
            Some(s) if s.success() => Ok(()),
            Some(s) => Err(format!("exit {}: {tail}", s.code().unwrap_or(-1))),
            None => Err(format!(
                "no answer in {}s and was killed, which usually means it is \
                 waiting on something nobody is going to type: {tail}",
                timeout.as_secs()
            )),
        }
    }

    /// Hides the payload, recovers it, and compares the bytes.
    ///
    /// Byte comparison rather than a size check or a substring: a tool that
    /// returns a truncated or padded payload has not worked, and every weaker
    /// comparison has a way of passing when it should not.
    pub fn run(entry: &Entry, fixtures_dir: &Path) -> Verified {
        let Some(rt) = &entry.roundtrip else {
            return Verified::Skipped("no roundtrip declared".into());
        };
        // A binary entry is a program the operator installed themselves, which
        // is a different statement from pulling a stranger's image, and it gets
        // the different level of isolation the registry already describes: it
        // runs as the user, unsandboxed. Refusing to check it would not make
        // that safer, it would only mean nobody knows whether it works.
        let reach = match (entry.image.as_ref(), entry.binary.as_ref()) {
            (Some(image), _) => Reach::Container(image.reference.clone()),
            (None, Some(binary)) if !binary.command.is_empty() => {
                Reach::Local(binary.command.clone())
            }
            _ => {
                return Verified::Skipped(
                    "no image and no binary command to run the round trip with".into(),
                )
            }
        };
        let cover_src = fixtures_dir.join(rt.cover.trim_start_matches("fixtures/"));
        if !cover_src.is_file() {
            return Verified::Failed(format!("cover {} not found", cover_src.display()));
        }

        let Ok(dir) = tempfile::tempdir() else {
            return Verified::Failed("no scratch directory".into());
        };
        let work = dir.path();
        let cover_name = cover_src
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("cover");
        if std::fs::copy(&cover_src, work.join(cover_name)).is_err() {
            return Verified::Failed("could not stage the cover".into());
        }
        if std::fs::write(work.join("payload.bin"), PAYLOAD).is_err() {
            return Verified::Failed("could not stage the payload".into());
        }

        // The stego file inherits the cover's extension. Several of these
        // tools infer the format from the name and refuse anything else:
        // outguess answers "Unknown data type" to a file called .out and
        // exits 1, which looks like a broken tool rather than a bad filename.
        let ext = Path::new(cover_name)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("bin");
        let base = match &reach {
            Reach::Container(_) => "/work".to_string(),
            Reach::Local(_) => work.display().to_string(),
        };
        let stego_name = format!("{base}/stego.{ext}");

        let subst = |a: &String| {
            a.replace("{cover}", &format!("{base}/{cover_name}"))
                .replace("{payload}", &format!("{base}/payload.bin"))
                .replace("{stego}", &stego_name)
                .replace("{recovered}", &format!("{base}/recovered.bin"))
                .replace("{passphrase}", &rt.passphrase)
        };

        let uid_gid = std::fs::metadata(work)
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
            .unwrap_or_default();

        let local_phase = |command: &Vec<String>, argv: &Vec<String>| -> Result<(), String> {
            let mut run = Command::new(&command[0]);
            run.args(command[1..].iter().map(&subst));
            run.args(argv.iter().map(&subst));
            // The scratch directory, so a tool that writes a stray file beside
            // its output leaves it there rather than in the user's cwd.
            run.current_dir(work);
            bounded(run, &command[0], PHASE_TIMEOUT)
        };

        let container_phase = |image: &String, argv: &Vec<String>| -> Result<(), String> {
            let mut args: Vec<String> = vec![
                "run".into(),
                "--rm".into(),
                "--network=none".into(),
                "--cap-drop=ALL".into(),
                "--security-opt".into(),
                "no-new-privileges".into(),
                "--memory=2g".into(),
            ];
            if !uid_gid.is_empty() {
                args.push("--user".into());
                args.push(uid_gid.clone());
            }
            args.push("-v".into());
            args.push(format!("{}:/work", work.display()));
            if let Some(ep) = &rt.entrypoint {
                args.push("--entrypoint".into());
                args.push(ep.clone());
            }
            args.push(image.clone());
            args.extend(argv.iter().map(&subst));
            let mut run = Command::new("docker");
            run.args(&args);
            bounded(run, "the container", PHASE_TIMEOUT)
        };

        let phase = |argv: &Vec<String>| -> Result<(), String> {
            match &reach {
                Reach::Container(image) => container_phase(image, argv),
                Reach::Local(command) => local_phase(command, argv),
            }
        };

        if let Err(e) = phase(&rt.embed_argv) {
            return Verified::Failed(format!("embed failed: {e}"));
        }
        // A tool can exit zero having written nothing, which is the silent
        // failure this whole check exists to catch.
        match std::fs::metadata(work.join(format!("stego.{ext}"))) {
            Ok(m) if m.len() > 0 => {}
            _ => return Verified::Failed("embed exited cleanly but wrote no stego file".into()),
        }
        if let Err(e) = phase(&rt.extract_argv) {
            return Verified::Failed(format!("extract failed: {e}"));
        }
        match std::fs::read(work.join("recovered.bin")) {
            Ok(got) if got == PAYLOAD => Verified::Passed,
            Ok(got) => Verified::Failed(format!(
                "recovered {} bytes, expected {}: what went in did not come back",
                got.len(),
                PAYLOAD.len()
            )),
            Err(e) => Verified::Failed(format!("extract wrote no payload: {e}")),
        }
    }
}
