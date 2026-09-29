// Author:  Daniel Iwugo
// Comment: Christ is King
// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Daniel Iwugo
//! Generates `stegobench(1)` plus one page per subcommand at build time,
//! from the same `Cli` definition the binary parses against.
//!
//! WHY `include!` AND NOT A NORMAL DEPENDENCY ON THE LIB TARGET
//! --------------------------------------------------------------
//! Cargo does not allow a package to depend on itself, including by path, so
//! `build.rs` cannot simply `use stegobench_cli::cli::Cli`. Textually
//! including `src/cli.rs` compiles the identical source into build.rs's own
//! throwaway binary instead, which keeps the single-source-of-truth property
//! that matters (the man pages describe exactly the command tree the shipped
//! binary parses) without asking Cargo for something it does not support.
//!
//! WHY A BUILD SCRIPT AND NOT AN XTASK
//! ------------------------------------
//! An xtask is a fine pattern when generation is occasional and manually
//! invoked. Man pages drift the moment someone adds a flag and forgets to
//! re-run a separate command, which is the exact failure this file exists to
//! close. A build script runs on every `cargo build`, so the pages are always
//! as fresh as the binary they describe, with no second step to forget.
//!
//! WHERE THE PAGES LAND
//! ---------------------
//! `$OUT_DIR` is per-build and hash-suffixed, so nothing outside Cargo can
//! find it reliably. The pages are written there AND mirrored to a fixed,
//! gitignored `target/man/` at the workspace root, which is what packaging
//! steps (and the toolkit Docker image, once it vendors this binary) should
//! read from.

use std::fs;
use std::path::PathBuf;

use clap::CommandFactory;

include!("src/cli.rs");

fn main() {
    println!("cargo:rerun-if-changed=src/cli.rs");

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR set by Cargo"));

    // Done FIRST, and never behind an early return: a missing man page is a
    // documentation problem, a missing built-in registry is a binary that
    // cannot find its own tools on a machine with no checkout.
    write_embedded_registry(&out_dir);
    write_embedded_fixtures(&out_dir);
    write_embedded_adapters(&out_dir);

    let man_dir = out_dir.join("man");
    // Emptied first, for the reason the mirror below is pruned: generation only
    // ever writes, so a page for a command that is gone or hidden would survive
    // in here and be copied out again on every build. This directory holds
    // nothing but pages this script wrote, so removing it costs nothing.
    let _ = fs::remove_dir_all(&man_dir);
    if let Err(e) = fs::create_dir_all(&man_dir) {
        // A man page that fails to generate must not fail the whole build:
        // it is documentation, not correctness, and a contributor without
        // write access to OUT_DIR (unusual, but sandboxes exist) should still
        // get a working binary.
        println!("cargo:warning=stegobench: could not create {man_dir:?}: {e}");
        return;
    }

    // `build()` is what propagates the globals (`--json`, `--registry`) down
    // into each subcommand. Without it the per-subcommand pages list neither,
    // although the binary accepts both on every subcommand, so the pages would
    // drift from the parser in the one way this whole file exists to prevent.
    let mut cmd = Cli::command();
    cmd.build();
    if let Err(e) = write_man_pages(&cmd, &man_dir) {
        println!("cargo:warning=stegobench: man page generation failed: {e}");
        return;
    }

    // Mirror to a fixed, predictable path for packaging and the Docker image.
    // Best effort: a workspace-relative target/ directory should always be
    // writable in a normal checkout, but this is convenience, not the build.
    if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
        let workspace_man = PathBuf::from(manifest_dir)
            .join("../../target/man")
            .to_path_buf();
        if fs::create_dir_all(&workspace_man).is_ok() {
            // Pruned before copying, because a copy only ever adds. A command
            // that is removed, or hidden, stops being generated and its page
            // would otherwise sit here until somebody deleted target/, and be
            // packaged and installed from a directory nobody re-reads. That is
            // how `stegobench-check.1` and `stegobench-scan.1` survived being
            // hidden. Only `.1` files are considered, and only those this build
            // did not just write, so anything else in the directory is left
            // alone.
            if let Ok(existing) = fs::read_dir(&workspace_man) {
                for item in existing.flatten() {
                    let path = item.path();
                    let is_page = path
                        .extension()
                        .and_then(|e| e.to_str())
                        .is_some_and(|e| e == "1");
                    if is_page && !man_dir.join(item.file_name()).exists() {
                        let _ = fs::remove_file(&path);
                    }
                }
            }
            let _ = copy_dir(&man_dir, &workspace_man);
        }
    }
}

/// Compiles `plugins/registry` into the binary as the last-resort fallback.
///
/// WHY GENERATED AND NOT A HAND-WRITTEN LIST OF `include_str!` CALLS
/// -----------------------------------------------------------------
/// A hand-written list drifts the first time somebody adds a detector, and it
/// drifts silently: the binary keeps working, it just stops knowing about the
/// new tool when run outside a checkout. Walking the directory here means the
/// built-in copy is whatever the repository holds, so an addition is embedded
/// by adding the file and nothing else.
///
/// The paths are made absolute before they reach `include_str!`, because that
/// macro resolves a relative path against the file it appears in, and the file
/// it appears in is in `$OUT_DIR`.
///
/// WHAT HAPPENS WHEN THE DIRECTORY IS NOT THERE
/// ---------------------------------------------
/// An empty table is written and a build warning is printed. The binary then
/// reports plainly that it has no built-in registry rather than pretending to
/// one, which is the honest behaviour for a source tree that does not carry
/// the registry (a crate published on its own would be that case).
fn write_embedded_registry(out_dir: &std::path::Path) {
    let manifest_dir = PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR set by Cargo"),
    );
    let root = manifest_dir.join("../../plugins/registry");
    println!("cargo:rerun-if-changed={}", root.display());

    let dest = out_dir.join("embedded_registry.rs");
    let mut files: Vec<(String, PathBuf)> = Vec::new();
    if root.is_dir() {
        collect_toml(&root, &root, &mut files).unwrap_or_else(|e| {
            // A registry that is there and cannot be read is a broken build,
            // not a build without a registry. Failing here is the loud half of
            // the fail-loud rule: shipping a binary that silently lost half its
            // tools is the outcome this refuses.
            panic!(
                "stegobench: could not read the registry at {}: {e}",
                root.display()
            )
        });
        files.sort_by(|a, b| a.0.cmp(&b.0));
    } else {
        println!(
            "cargo:warning=stegobench: no registry at {}, so this binary is \
             built with no built-in fallback registry",
            root.display()
        );
    }

    let mut body = String::from(
        "// Generated by build.rs from plugins/registry. Do not edit.\n\
         pub static EMBEDDED_REGISTRY: &[(&str, &str)] = &[\n",
    );
    for (rel, abs) in &files {
        println!("cargo:rerun-if-changed={}", abs.display());
        body.push_str(&format!(
            "    ({:?}, include_str!({:?})),\n",
            rel,
            abs.display().to_string()
        ));
    }
    body.push_str("];\n");
    fs::write(&dest, body)
        .unwrap_or_else(|e| panic!("stegobench: could not write {}: {e}", dest.display()));
}

/// Compiles `plugins/adapters` into the binary, for the reason the fixtures are.
///
/// An entry with `invoke.host = true` names its adapter as a path relative to
/// the repository root, and the built-in registry exists precisely for the
/// machine that has no repository. Without the adapters beside it, the fall
/// back registry carries three detectors that can never run, and the failure
/// arrives as "run from the root of the clone" at somebody who installed from
/// a package and has no clone to run from.
///
/// Test modules are left out: `test_*.py` is pytest's, not a plugin's, and
/// shipping it in the binary would grow every install for nobody's benefit.
fn write_embedded_adapters(out_dir: &std::path::Path) {
    let manifest_dir = PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR set by Cargo"),
    );
    let root = manifest_dir.join("../../plugins/adapters");
    println!("cargo:rerun-if-changed={}", root.display());

    let dest = out_dir.join("embedded_adapters.rs");
    let mut files: Vec<(String, PathBuf)> = Vec::new();
    if root.is_dir() {
        let entries = fs::read_dir(&root).unwrap_or_else(|e| {
            panic!(
                "stegobench: could not read the adapters at {}: {e}",
                root.display()
            )
        });
        for item in entries {
            let path = item
                .unwrap_or_else(|e| panic!("stegobench: could not read an adapter entry: {e}"))
                .path();
            if !path.is_file() {
                continue;
            }
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.starts_with("test_") {
                    continue;
                }
                files.push((name.to_string(), path.clone()));
            }
        }
        files.sort_by(|a, b| a.0.cmp(&b.0));
    } else {
        println!(
            "cargo:warning=stegobench: no adapters at {}, so this binary cannot \
             run a host entry from its built-in registry",
            root.display()
        );
    }

    let mut body = String::from(
        "// Generated by build.rs from plugins/adapters/. Do not edit.\n\
         pub static EMBEDDED_ADAPTERS: &[(&str, &[u8])] = &[\n",
    );
    for (name, abs) in &files {
        println!("cargo:rerun-if-changed={}", abs.display());
        body.push_str(&format!(
            "    ({:?}, include_bytes!({:?})),\n",
            name,
            abs.display().to_string()
        ));
    }
    body.push_str("];\n");
    fs::write(&dest, body)
        .unwrap_or_else(|e| panic!("stegobench: could not write {}: {e}", dest.display()));
}

/// Compiles `fixtures/` into the binary, for the same reason the registry is.
///
/// A self-test fixture is named by a registry entry as `fixtures/clean.png`,
/// and that path used to be resolved against the directory the command was
/// typed in. Measured on 2026-09-29, from a directory that was not a checkout:
/// `doctor` reported six tools BROKEN that the identical binary reported
/// VERIFIED one directory later. The tools were fine. The harness could not
/// find its own fixtures and said so in the vocabulary of a tool failure.
///
/// Reporting somebody's working installation as broken is worse than refusing
/// to check it, so the files travel with the binary. Six images, 344 KB.
fn write_embedded_fixtures(out_dir: &std::path::Path) {
    let manifest_dir = PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR set by Cargo"),
    );
    let root = manifest_dir.join("../../fixtures");
    println!("cargo:rerun-if-changed={}", root.display());

    let dest = out_dir.join("embedded_fixtures.rs");
    let mut files: Vec<(String, PathBuf)> = Vec::new();
    if root.is_dir() {
        let entries = fs::read_dir(&root).unwrap_or_else(|e| {
            panic!(
                "stegobench: could not read the fixtures at {}: {e}",
                root.display()
            )
        });
        for item in entries {
            let path = item
                .unwrap_or_else(|e| panic!("stegobench: could not read a fixture entry: {e}"))
                .path();
            if path.is_file() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    files.push((name.to_string(), path.clone()));
                }
            }
        }
        files.sort_by(|a, b| a.0.cmp(&b.0));
    } else {
        println!(
            "cargo:warning=stegobench: no fixtures at {}, so this binary cannot \
             self-test a tool without --fixtures",
            root.display()
        );
    }

    let mut body = String::from(
        "// Generated by build.rs from fixtures/. Do not edit.\n\
         pub static EMBEDDED_FIXTURES: &[(&str, &[u8])] = &[\n",
    );
    for (name, abs) in &files {
        println!("cargo:rerun-if-changed={}", abs.display());
        body.push_str(&format!(
            "    ({:?}, include_bytes!({:?})),\n",
            name,
            abs.display().to_string()
        ));
    }
    body.push_str("];\n");
    fs::write(&dest, body)
        .unwrap_or_else(|e| panic!("stegobench: could not write {}: {e}", dest.display()));
}

/// Every `.toml` under `dir`, keyed by its path relative to `root` with `/`
/// separators so the generated table reads the same on every platform.
fn collect_toml(
    root: &std::path::Path,
    dir: &std::path::Path,
    out: &mut Vec<(String, PathBuf)>,
) -> std::io::Result<()> {
    for item in fs::read_dir(dir)? {
        let item = item?;
        let path = item.path();
        let meta = fs::metadata(&path)?;
        if meta.is_dir() {
            collect_toml(root, &path, out)?;
        } else if path.extension().is_some_and(|e| e == "toml") {
            let rel = path
                .strip_prefix(root)
                .map_err(std::io::Error::other)?
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            out.push((rel, path));
        }
    }
    Ok(())
}

fn write_man_pages(cmd: &clap::Command, dir: &std::path::Path) -> std::io::Result<()> {
    // stegobench(1): the root page.
    let root_page = dir.join(format!("{}.1", cmd.get_name()));
    let mut buf: Vec<u8> = Vec::new();
    clap_mangen::Man::new(cmd.clone()).render(&mut buf)?;
    fs::write(&root_page, &buf)?;

    // One page per subcommand group, named stegobench-<name>.1, which is the
    // convention getopt-driven tools (git, cargo) already use, so `man
    // stegobench-doctor` is guessable rather than needing to be looked up.
    //
    // Hidden subcommands are skipped. `check` and `scan` are signposts that
    // exist only to explain that this tool does not examine your own images,
    // and they are hidden from `--help` for that reason. A man page is
    // documentation too, so shipping one for them would advertise the very
    // thing the signpost exists to deny.
    for sub in cmd.get_subcommands().filter(|s| !s.is_hide_set()) {
        let name = format!("{}-{}", cmd.get_name(), sub.get_name());
        let page = dir.join(format!("{name}.1"));
        let mut buf: Vec<u8> = Vec::new();
        // clap_builder 4.6's `Command::name` takes `impl Into<Str>`, and `Str`
        // has no `From<String>` in this version, only `From<&'static str>`.
        // `Box::leak` is safe here specifically because build.rs is a
        // short-lived, single-purpose process that exits right after this
        // function returns; it would be the wrong call in the shipped binary.
        let name: &'static str = Box::leak(name.into_boxed_str());
        clap_mangen::Man::new(sub.clone().name(name)).render(&mut buf)?;
        fs::write(&page, &buf)?;
    }
    Ok(())
}

fn copy_dir(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            fs::copy(entry.path(), to.join(entry.file_name()))?;
        }
    }
    Ok(())
}
