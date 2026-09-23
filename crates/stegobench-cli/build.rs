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
    let man_dir = out_dir.join("man");
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
            let _ = copy_dir(&man_dir, &workspace_man);
        }
    }
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
    for sub in cmd.get_subcommands() {
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
