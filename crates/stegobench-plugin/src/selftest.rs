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
        "run".into(), "--rm".into(),
        "--network=none".into(),
        "--cap-drop=ALL".into(),
        "--security-opt".into(), "no-new-privileges".into(),
        "--read-only".into(),
        "--memory=2g".into(),
        "-v".into(), mount,
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
        a.replace("{file}", &inner).replace("{adapter}", &adapter_inner)
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
    let program = invoke.entrypoint.clone().unwrap_or_else(|| "python3".into());
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
        (Some(false), Some(true)) => Verified::Failed(
            "has both answers exactly backwards".into(),
        ),
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

/// Proving an embedder: what goes in must come back out.
pub mod roundtrip {
    use std::path::Path;
    use std::process::Command;

    use stegobench_core::registry::Entry;

    use super::Verified;

    /// The payload. Short, recognisable, and not compressible into nothing,
    /// so a tool that silently wrote an empty file cannot pass by accident.
    pub const PAYLOAD: &[u8] = b"stegobench roundtrip fixture 2026: if you can read this, it survived.";

    /// Hides the payload, recovers it, and compares the bytes.
    ///
    /// Byte comparison rather than a size check or a substring: a tool that
    /// returns a truncated or padded payload has not worked, and every weaker
    /// comparison has a way of passing when it should not.
    pub fn run(entry: &Entry, fixtures_dir: &Path) -> Verified {
        let Some(rt) = &entry.roundtrip else {
            return Verified::Skipped("no roundtrip declared".into());
        };
        let Some(image) = entry.image.as_ref().map(|i| i.reference.clone()) else {
            return Verified::Skipped("roundtrip currently covers containerised tools".into());
        };
        let cover_src = fixtures_dir.join(rt.cover.trim_start_matches("fixtures/"));
        if !cover_src.is_file() {
            return Verified::Failed(format!("cover {} not found", cover_src.display()));
        }

        let Ok(dir) = tempfile::tempdir() else {
            return Verified::Failed("no scratch directory".into());
        };
        let work = dir.path();
        let cover_name = cover_src.file_name().and_then(|n| n.to_str()).unwrap_or("cover");
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
        let stego_name = format!("/work/stego.{ext}");

        let subst = |a: &String| {
            a.replace("{cover}", &format!("/work/{cover_name}"))
                .replace("{payload}", "/work/payload.bin")
                .replace("{stego}", &stego_name)
                .replace("{recovered}", "/work/recovered.bin")
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

        let mut phase = |argv: &Vec<String>| -> Result<(), String> {
            let mut args: Vec<String> = vec![
                "run".into(), "--rm".into(),
                "--network=none".into(),
                "--cap-drop=ALL".into(),
                "--security-opt".into(), "no-new-privileges".into(),
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
            match Command::new("docker").args(&args).output() {
                Ok(o) if o.status.success() => Ok(()),
                Ok(o) => Err(format!(
                    "exit {}: {}",
                    o.status.code().unwrap_or(-1),
                    String::from_utf8_lossy(&o.stderr).trim().chars().take(160).collect::<String>()
                )),
                Err(e) => Err(format!("could not run the container: {e}")),
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
