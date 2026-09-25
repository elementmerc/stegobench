// Author:  Daniel Iwugo
// Comment: Christ is King
// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Daniel Iwugo
//
// `cargo package` can only carry files that live inside the crate directory,
// so the LICENSE at the repository root is not one of them. Without a copy in
// each crate, all four would reach crates.io declaring AGPL-3.0-or-later and
// carrying none of its text, which for a copyleft licence is the part that
// does the work: Section 13 of the AGPL is a term a reader has to be able to
// read.
//
// This test is the gate that keeps the copies there and keeps them identical.
// It fails if a copy is deleted, if one drifts from the root text, or if a
// manifest grows an `include` or `exclude` key that would leave the copy out
// of the package even though the file is still on disk.

use std::fs;
use std::path::{Path, PathBuf};

/// Every crate `cargo package` is ever run against.
const PUBLISHED_CRATES: [&str; 4] = [
    "stegobench-core",
    "stegobench-metrics",
    "stegobench-plugin",
    "stegobench-cli",
];

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("the crate directory is two levels below the workspace root")
        .to_path_buf()
}

fn read(path: &Path) -> Vec<u8> {
    fs::read(path).unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()))
}

#[test]
fn every_published_crate_carries_the_licence_text() {
    let root = workspace_root();
    let canonical = read(&root.join("LICENSE"));
    assert!(
        canonical.len() > 10_000,
        "the root LICENSE is {} bytes, which is too short to be the AGPL text",
        canonical.len()
    );

    for name in PUBLISHED_CRATES {
        let path = root.join("crates").join(name).join("LICENSE");
        assert!(
            path.exists(),
            "{name} has no LICENSE of its own, so `cargo package -p {name}` would \
             publish it with a licence field and no licence text. Copy the root \
             LICENSE to {}",
            path.display()
        );
        assert_eq!(
            read(&path),
            canonical,
            "{name}'s LICENSE has drifted from the one at the repository root. \
             Two licence texts under one project is a question nobody should \
             have to answer; copy the root LICENSE over {}",
            path.display()
        );
    }
}

#[test]
fn no_manifest_filters_the_licence_out_of_its_package() {
    let root = workspace_root();

    for name in PUBLISHED_CRATES {
        let manifest_path = root.join("crates").join(name).join("Cargo.toml");
        let manifest: toml::Value =
            toml::from_str(&fs::read_to_string(&manifest_path).unwrap_or_else(|error| {
                panic!("cannot read {}: {error}", manifest_path.display())
            }))
            .unwrap_or_else(|error| panic!("cannot parse {}: {error}", manifest_path.display()));

        let package = manifest
            .get("package")
            .and_then(toml::Value::as_table)
            .unwrap_or_else(|| panic!("{} has no [package] section", manifest_path.display()));

        if let Some(include) = package.get("include") {
            let listed = include
                .as_array()
                .unwrap_or_else(|| panic!("{name}'s `include` is not a list"))
                .iter()
                .any(|entry| {
                    entry
                        .as_str()
                        .is_some_and(|value| value.contains("LICENSE"))
                });
            assert!(
                listed,
                "{name}'s manifest has an `include` list that does not name LICENSE, \
                 so the licence text would be dropped from the package"
            );
        }

        if let Some(exclude) = package.get("exclude") {
            let dropped = exclude
                .as_array()
                .unwrap_or_else(|| panic!("{name}'s `exclude` is not a list"))
                .iter()
                .any(|entry| {
                    entry
                        .as_str()
                        .is_some_and(|value| value.contains("LICENSE"))
                });
            assert!(
                !dropped,
                "{name}'s manifest excludes LICENSE from the package"
            );
        }
    }
}
