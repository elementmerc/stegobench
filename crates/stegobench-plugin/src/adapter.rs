// Author:  Daniel Iwugo
// Comment: Christ is King
// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Daniel Iwugo
//! Finding the host adapter a registry entry names.
//!
//! WHY THE ENTRY DOES NOT HOLD THE ANSWER
//! --------------------------------------
//! A `host = true` entry declares its adapter the way the repository holds it,
//! `plugins/adapters/aletheia_one.py`. That is relative to the tree the
//! registry sits in, never to wherever the command happened to be typed.
//!
//! The obvious fix is to rewrite the entry at load time, and that is what this
//! replaced. It was wrong in two ways. An entry is a reflection of the TOML a
//! human wrote, and `describe` and `list --json` publish it, so a rewritten
//! path makes the published document disagree with the file it came from. For
//! the copy compiled into the binary it was worse than that: the adapters are
//! unpacked into a scratch directory, so the rewritten path changed on every
//! run and named a file that had already been deleted by the time the reader
//! saw it.
//!
//! So the entry stays as written, and the ROOTS it is resolved against travel
//! beside it. A resolution failure names the roots it looked under, because
//! the advice it used to give, run from the root of the clone, is useless to
//! somebody who installed a package and has no clone.

use std::fmt;
use std::path::{Path, PathBuf};

/// Where a relative `invoke.adapter` could not be found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotFound {
    declared: String,
    roots: Vec<PathBuf>,
}

impl fmt::Display for NotFound {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let declared = &self.declared;
        if Path::new(declared).is_absolute() {
            return write!(f, "adapter {declared} was not found");
        }
        if self.roots.is_empty() {
            return write!(
                f,
                "adapter {declared} was not found in the directory this ran \
                 from, and no registry tree was given to look under"
            );
        }
        let under = self
            .roots
            .iter()
            .map(|r| r.display().to_string())
            .collect::<Vec<_>>()
            .join(" or ");
        write!(
            f,
            "adapter {declared} was not found under {under}, nor in the \
             directory this ran from. A registry names its adapter relative to \
             the tree it sits in, so either that tree is not there or the entry \
             needs an absolute path"
        )
    }
}

impl std::error::Error for NotFound {}

/// The adapter this entry names, as an absolute path.
///
/// An absolute `declared` is used exactly as written and nothing is guessed
/// from it, because an entry that states a full path has already answered this
/// question.
///
/// A relative one is tried under each root in turn, in two shapes. The first,
/// `<root>/plugins/adapters/x.py`, is the registry's own layout and is what a
/// checkout and an install both use. The second, `<root>/x.py`, is the shape
/// the copy compiled into the binary takes: the adapters are unpacked side by
/// side into one scratch directory, where there is no `plugins/` to mirror.
///
/// The directory the command was typed in is tried last rather than not at
/// all, so a contributor standing in a checkout keeps working whether or not
/// a root was supplied.
pub fn resolve(declared: &str, roots: &[PathBuf]) -> Result<PathBuf, NotFound> {
    let named = Path::new(declared);
    let mut candidates: Vec<PathBuf> = Vec::new();
    if named.is_absolute() {
        candidates.push(named.to_path_buf());
    } else {
        for root in roots {
            candidates.push(root.join(named));
            if let Some(base) = named.file_name() {
                candidates.push(root.join(base));
            }
        }
        candidates.push(named.to_path_buf());
    }

    for candidate in candidates {
        if candidate.is_file() {
            // Canonicalised because the container route mounts this path and a
            // relative mount source is not a thing docker accepts. A file that
            // passed `is_file` and then cannot be canonicalised has moved
            // under us, which is the same finding as not being there.
            if let Ok(abs) = candidate.canonicalize() {
                return Ok(abs);
            }
        }
    }
    Err(NotFound {
        declared: declared.to_string(),
        roots: roots.to_vec(),
    })
}

/// Every root a registry read from `dir` resolves its adapters against.
///
/// Two, because two layouts are both real. A checkout keeps the registry at
/// `<root>/plugins/registry`, so the tree holding `plugins/` is two levels up.
/// An installed copy keeps it at `<prefix>/stegobench/registry` beside
/// `<prefix>/stegobench/plugins/adapters`, so there it is one level up. Trying
/// both costs two `is_file` calls and means neither layout has to be declared
/// anywhere.
pub fn roots_for_registry(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(up) = dir.parent().and_then(Path::parent) {
        out.push(up.to_path_buf());
    }
    if let Some(up) = dir.parent() {
        if !out.contains(&up.to_path_buf()) {
            out.push(up.to_path_buf());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(path: &Path) {
        std::fs::create_dir_all(path.parent().expect("has a parent")).expect("dirs");
        std::fs::write(path, b"#!/usr/bin/env python3\n").expect("written");
    }

    /// The checkout layout, which is the one every shipped registry uses.
    #[test]
    fn a_registry_two_levels_under_the_tree_finds_its_adapter() {
        let tmp = tempfile::tempdir().expect("tmp");
        let root = tmp.path().join("checkout");
        let registry = root.join("plugins").join("registry");
        std::fs::create_dir_all(&registry).expect("dirs");
        let adapter = root.join("plugins").join("adapters").join("one.py");
        touch(&adapter);

        let roots = roots_for_registry(&registry);
        let found = resolve("plugins/adapters/one.py", &roots).expect("found");
        assert_eq!(found, adapter.canonicalize().expect("canonical"));
    }

    /// The install layout: registry and adapters side by side under a prefix,
    /// which is what a package would lay down and what the working directory
    /// resolution never handled.
    #[test]
    fn a_registry_one_level_under_the_tree_finds_its_adapter_too() {
        let tmp = tempfile::tempdir().expect("tmp");
        let prefix = tmp.path().join("usr-share").join("stegobench");
        let registry = prefix.join("registry");
        std::fs::create_dir_all(&registry).expect("dirs");
        let adapter = prefix.join("plugins").join("adapters").join("one.py");
        touch(&adapter);

        let roots = roots_for_registry(&registry);
        let found = resolve("plugins/adapters/one.py", &roots).expect("found");
        assert_eq!(found, adapter.canonicalize().expect("canonical"));
    }

    /// The shape the copy compiled into the binary unpacks into: plain names
    /// in one scratch directory, with no `plugins/` to mirror.
    #[test]
    fn a_flat_root_matches_on_the_file_name_alone() {
        let tmp = tempfile::tempdir().expect("tmp");
        let adapter = tmp.path().join("one.py");
        touch(&adapter);

        let found = resolve("plugins/adapters/one.py", &[tmp.path().to_path_buf()])
            .expect("the flat shape resolves");
        assert_eq!(found, adapter.canonicalize().expect("canonical"));
    }

    /// An entry that states a full path has answered the question itself, and
    /// nothing may be guessed from a root that happens to hold the same name.
    #[test]
    fn an_absolute_adapter_is_used_as_written_and_no_root_can_shadow_it() {
        let tmp = tempfile::tempdir().expect("tmp");
        let real = tmp.path().join("elsewhere").join("one.py");
        touch(&real);
        let decoy_root = tmp.path().join("decoy");
        touch(&decoy_root.join("one.py"));

        let declared = real.display().to_string();
        let found = resolve(&declared, std::slice::from_ref(&decoy_root)).expect("used as given");
        assert_eq!(found, real.canonicalize().expect("canonical"));

        // And an absolute path that is not there is not quietly rescued by a
        // root holding the same file name.
        let missing = tmp.path().join("nowhere").join("one.py");
        let why = resolve(&missing.display().to_string(), &[decoy_root])
            .expect_err("an absolute path that is not there");
        assert!(
            why.to_string().ends_with("was not found"),
            "an absolute path should not be explained with roots: {why}"
        );
    }

    /// The failure a stranger meets. It names where it looked and says nothing
    /// about a clone, because the reader who most needs it has not got one.
    #[test]
    fn the_refusal_names_the_roots_and_never_mentions_a_clone() {
        let tmp = tempfile::tempdir().expect("tmp");
        let why = resolve("plugins/adapters/one.py", &[tmp.path().to_path_buf()])
            .expect_err("nothing is there");
        let text = why.to_string();
        assert!(
            text.contains(&tmp.path().display().to_string()),
            "the refusal does not say where it looked: {text}"
        );
        assert!(
            !text.contains("clone"),
            "the refusal still sends a packaged install to a clone: {text}"
        );
    }

    /// With no root at all the working directory is still tried, so a
    /// contributor standing in a checkout is not broken by this indirection.
    #[test]
    fn with_no_roots_the_working_directory_is_still_tried() {
        let why =
            resolve("plugins/adapters/definitely-not-here.py", &[]).expect_err("nothing is there");
        assert!(
            why.to_string().contains("no registry tree was given"),
            "{why}"
        );
    }
}
