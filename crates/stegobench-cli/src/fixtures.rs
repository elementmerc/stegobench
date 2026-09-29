// Author:  Daniel Iwugo
// Comment: Christ is King
// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Daniel Iwugo
//! Finding the self-test fixtures, from wherever the command was typed.
//!
//! THE DEFECT THIS EXISTS TO CLOSE
//! -------------------------------
//! `--fixtures` used to default to the literal relative string `fixtures`,
//! which is only correct inside the source checkout. Measured on 2026-09-29,
//! with one binary on one machine reading one registry:
//!
//! ```text
//! inside a checkout   13 tool(s): 6 verified, 0 broken, 2 not installed
//! one directory out   13 tool(s): 0 verified, 6 broken, 3 not installed
//! ```
//!
//! Nothing was wrong with the six tools. The harness could not find its own
//! fixture images and reported that in the vocabulary of a tool failure, so
//! the first `doctor` a new user runs told them their installation was broken.
//! That is worse than refusing to check, because it sends somebody to debug a
//! detector that works.
//!
//! Two things fix it together, and neither is sufficient alone. `selftest`
//! now reports an absent fixture as "could not ask" rather than "answered
//! wrongly", which is the honest half. This module is the useful half: the
//! images travel inside the binary, so a fresh install can actually run the
//! check rather than politely declining to.
//!
//! THE ORDER, AND WHY IT MATCHES THE REGISTRY'S
//! --------------------------------------------
//! Deliberately the same order as `registry`, because a user who has learned
//! where one comes from should not have to learn a second rule for the other.
//!
//! 1. `--fixtures`, or `STEGOBENCH_FIXTURES`. Not there is an ERROR, never a
//!    quiet fall-through to a different set of images.
//! 2. `./fixtures`, the source checkout.
//! 3. Beside the running executable.
//! 4. The per-user data directory.
//! 5. The system data directory.
//! 6. The copy compiled into this binary, unpacked into a scratch directory
//!    that is removed when the command ends.

use std::path::{Path, PathBuf};

use crate::registry::{non_empty, system_data_dirs, user_data_dir};

include!(concat!(env!("OUT_DIR"), "/embedded_fixtures.rs"));

/// The directory name fixtures are installed under, below a data directory.
const INSTALL_SUBDIR: &str = "stegobench";

/// Where the fixtures that answered came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// `--fixtures`, or `STEGOBENCH_FIXTURES`.
    Explicit(PathBuf),
    /// `./fixtures`, relative to where the command was typed.
    WorkingDirectory(PathBuf),
    /// Beside the stegobench executable.
    BesideExecutable(PathBuf),
    /// This platform's per-user data directory.
    UserData(PathBuf),
    /// This platform's system-wide data directory.
    SystemData(PathBuf),
    /// Unpacked from the copy compiled into this binary.
    BuiltIn,
}

impl Source {
    /// A stable word for a script to match on.
    pub fn tag(&self) -> &'static str {
        match self {
            Source::Explicit(_) => "explicit",
            Source::WorkingDirectory(_) => "working-directory",
            Source::BesideExecutable(_) => "beside-executable",
            Source::UserData(_) => "user-data",
            Source::SystemData(_) => "system-data",
            Source::BuiltIn => "built-in",
        }
    }
}

/// A fixture directory, and whatever is keeping it alive.
///
/// The scratch directory for the built-in copy is owned here rather than
/// returned as a path, so it cannot be removed while a self-test is still
/// staging an image out of it.
#[derive(Debug)]
pub struct Fixtures {
    source: Source,
    dir: PathBuf,
    _scratch: Option<tempfile::TempDir>,
}

impl Fixtures {
    /// The directory the self-tests should read.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Where it came from.
    pub fn source(&self) -> &Source {
        &self.source
    }

    /// One line naming the fixtures that answered, for `doctor` to print above
    /// its table. The JSON block carries the tag and the path separately.
    pub fn line(&self) -> String {
        match &self.source {
            Source::BuiltIn => "fixtures  built in".to_string(),
            _ => format!("fixtures  {}", self.dir.display()),
        }
    }
}

/// What went wrong, in the user's words.
#[derive(Debug)]
pub enum Error {
    /// A directory was named and is not there. Never falls through.
    NamedButMissing(PathBuf),
    /// The built-in copy could not be unpacked.
    CouldNotUnpack(String),
    /// This binary was built without fixtures and none were found on disk.
    NoneAnywhere,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NamedButMissing(p) => write!(
                f,
                "no fixture directory at {}.\n\
                 It was named with --fixtures or STEGOBENCH_FIXTURES, so nothing \
                 else was tried: a typo that quietly self-tested against a \
                 different set of images would not show up in the result.",
                p.display()
            ),
            Error::CouldNotUnpack(why) => write!(
                f,
                "the fixtures compiled into this binary could not be unpacked: {why}.\n\
                 Self-tests need the images on disk. Point --fixtures at a copy, \
                 or run `stegobench doctor --no-selftest` to skip the checks."
            ),
            Error::NoneAnywhere => write!(
                f,
                "this build carries no self-test fixtures and none were found on disk.\n\
                 Point --fixtures at the `fixtures` directory from the source tree, \
                 or run `stegobench doctor --no-selftest`."
            ),
        }
    }
}

/// A directory that would be accepted, and what finding it there would mean.
///
/// The same shape as `registry`'s, so the two modules stay readable side by
/// side.
type Candidate = (PathBuf, fn(PathBuf) -> Source);

/// Every directory that would be accepted, in order.
///
/// Consumed by [`resolve`] rather than merely mirroring it. The two used to
/// build this list separately, which meant a sixth location added here would
/// have been tested and never searched.
pub fn search_path() -> Vec<Candidate> {
    let mut out: Vec<Candidate> = vec![(PathBuf::from("fixtures"), Source::WorkingDirectory)];
    for dir in beside_executable() {
        out.push((dir, Source::BesideExecutable));
    }
    if let Some(d) = user_data_dir() {
        out.push((d.join(INSTALL_SUBDIR).join("fixtures"), Source::UserData));
    }
    for d in system_data_dirs() {
        out.push((d.join(INSTALL_SUBDIR).join("fixtures"), Source::SystemData));
    }
    out
}

fn beside_executable() -> Vec<PathBuf> {
    let Ok(exe) = std::env::current_exe() else {
        return Vec::new();
    };
    let Some(dir) = exe.parent() else {
        return Vec::new();
    };
    vec![
        dir.join("fixtures"),
        dir.join("..")
            .join("share")
            .join(INSTALL_SUBDIR)
            .join("fixtures"),
    ]
}

/// Finds the fixtures, unpacking the built-in copy only if nothing is on disk.
pub fn resolve(explicit: Option<&Path>) -> Result<Fixtures, Error> {
    resolve_with(explicit, non_empty("STEGOBENCH_FIXTURES").as_deref())
}

/// [`resolve`], with the environment's answer handed in rather than read.
///
/// The seam exists so the precedence between `--fixtures` and
/// `STEGOBENCH_FIXTURES`, and the treatment of an empty variable, are testable
/// without `set_var`, which is process-global and would reach into every test
/// running beside it.
pub fn resolve_with(explicit: Option<&Path>, from_env: Option<&str>) -> Result<Fixtures, Error> {
    let named = explicit
        .map(PathBuf::from)
        .or_else(|| from_env.filter(|v| !v.is_empty()).map(PathBuf::from));
    if let Some(p) = named {
        if p.is_dir() {
            return Ok(Fixtures {
                source: Source::Explicit(p.clone()),
                dir: p,
                _scratch: None,
            });
        }
        return Err(Error::NamedButMissing(p));
    }

    // `is_dir()` rather than `exists()`: a FILE called `fixtures` is not a
    // fixture directory, and selecting it would turn a mistake into a
    // confusing read error from a path nobody typed.
    for (p, make) in search_path() {
        if !p.is_dir() {
            continue;
        }
        return Ok(Fixtures {
            source: make(p.clone()),
            dir: p,
            _scratch: None,
        });
    }

    unpack_built_in()
}

/// Writes the compiled-in images into a scratch directory for this run.
fn unpack_built_in() -> Result<Fixtures, Error> {
    if EMBEDDED_FIXTURES.is_empty() {
        return Err(Error::NoneAnywhere);
    }
    let scratch = tempfile::Builder::new()
        .prefix("stegobench-fixtures-")
        .tempdir()
        .map_err(|e| Error::CouldNotUnpack(e.to_string()))?;
    for (name, bytes) in EMBEDDED_FIXTURES {
        // The names come from a directory listing at build time, so they carry
        // no separators. Checked anyway, because this writes files: a name
        // that escaped the scratch directory would be writing wherever it
        // pointed, and the cost of the check is one comparison per image.
        if name.contains('/') || name.contains('\\') || name.contains("..") {
            return Err(Error::CouldNotUnpack(format!(
                "built-in fixture {name:?} is not a plain file name"
            )));
        }
        std::fs::write(scratch.path().join(name), bytes)
            .map_err(|e| Error::CouldNotUnpack(format!("writing {name}: {e}")))?;
    }
    Ok(Fixtures {
        source: Source::BuiltIn,
        dir: scratch.path().to_path_buf(),
        _scratch: Some(scratch),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_binary_carries_its_own_fixtures() {
        // The whole point of the module. A build that lost them would make
        // every self-test on a fresh install decline instead of running, which
        // is quieter than the bug it replaced and just as useless.
        assert!(
            !EMBEDDED_FIXTURES.is_empty(),
            "no fixtures were compiled in; build.rs found none beside the workspace"
        );
    }

    #[test]
    fn every_built_in_fixture_has_a_plain_name_and_some_bytes() {
        for (name, bytes) in EMBEDDED_FIXTURES {
            assert!(!name.contains('/'), "{name} carries a separator");
            assert!(!name.contains(".."), "{name} carries a parent reference");
            assert!(!bytes.is_empty(), "{name} is empty");
        }
    }

    #[test]
    fn unpacking_writes_every_image_and_removes_them_afterwards() {
        let dir;
        {
            let f = unpack_built_in().expect("the built-in copy unpacks");
            assert_eq!(f.source(), &Source::BuiltIn);
            dir = f.dir().to_path_buf();
            for (name, bytes) in EMBEDDED_FIXTURES {
                let written = std::fs::read(dir.join(name)).expect("fixture written");
                assert_eq!(&written, bytes, "{name} round trips byte for byte");
            }
        }
        // Dropping the handle takes the scratch directory with it, so a long
        // lived process does not accumulate one copy per doctor run.
        assert!(!dir.exists(), "the scratch directory outlived its handle");
    }

    #[test]
    fn a_named_directory_that_is_absent_is_an_error_not_a_fall_through() {
        let missing = Path::new("definitely-not-a-fixture-directory-xyzzy");
        match resolve(Some(missing)) {
            Err(Error::NamedButMissing(p)) => assert_eq!(p, missing),
            other => panic!("expected NamedButMissing, got {other:?}"),
        }
    }

    #[test]
    fn a_named_directory_that_exists_is_taken_as_given() {
        let tmp = tempfile::tempdir().expect("scratch");
        let f = resolve(Some(tmp.path())).expect("an existing directory resolves");
        assert_eq!(f.dir(), tmp.path());
        assert_eq!(f.source().tag(), "explicit");
    }

    #[test]
    fn the_search_path_starts_at_the_checkout_and_ends_at_the_system() {
        let path = search_path();
        assert_eq!(path.first().unwrap().0, Path::new("fixtures"));
        assert!(
            path.len() > 1,
            "nothing but the working directory was searched"
        );
    }

    /// `STEGOBENCH_FIXTURES` used to be bound with clap's `env`, where a
    /// variable that is set but empty refused every command. It is read by
    /// this resolver now, and the behaviour is asserted through the seam
    /// rather than by setting a process-global variable in a suite that runs
    /// its tests on several threads at once.
    #[test]
    fn the_environment_names_a_fixture_directory_only_when_it_has_something_in_it() {
        let tmp = tempfile::tempdir().expect("scratch");
        let from_env = tmp.path().join("from-env");
        let from_flag = tmp.path().join("from-flag");
        for dir in [&from_env, &from_flag] {
            std::fs::create_dir_all(dir).expect("dirs");
        }

        let f = resolve_with(None, Some(&from_env.display().to_string())).expect("resolves");
        assert_eq!(f.dir(), from_env);
        assert_eq!(f.source().tag(), "explicit");

        let f = resolve_with(Some(&from_flag), Some(&from_env.display().to_string()))
            .expect("resolves");
        assert_eq!(
            f.dir(),
            from_flag,
            "the environment overrode an explicit --fixtures"
        );

        // A named directory that is not there is an ERROR, never a quiet
        // fall-through to a different set of images.
        let missing = resolve_with(None, Some(&tmp.path().join("nope").display().to_string()));
        assert!(
            matches!(missing, Err(Error::NamedButMissing(_))),
            "a named fixture directory that is absent fell through"
        );

        // An empty value is not an answer, so this falls through to the
        // search and finds something rather than refusing on a path of "".
        resolve_with(None, Some("")).expect("an empty variable is ignored");
    }

    #[test]
    fn the_line_names_the_directory_or_says_built_in() {
        let f = unpack_built_in().expect("unpacks");
        assert_eq!(f.line(), "fixtures  built in");
        let tmp = tempfile::tempdir().expect("scratch");
        let g = resolve(Some(tmp.path())).expect("resolves");
        assert!(g.line().starts_with("fixtures  "));
        assert!(g.line().contains(&tmp.path().display().to_string()));
    }
}
