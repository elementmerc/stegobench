// Author:  Daniel Iwugo
// Comment: Christ is King
// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Daniel Iwugo
//! Finding the tool registry, from wherever the command was typed.
//!
//! THE DEFECT THIS EXISTS TO CLOSE
//! -------------------------------
//! The registry path used to default to the literal relative string
//! `plugins/registry`, which is only correct inside the source checkout. An
//! installed binary run from anywhere else could not find the registry, so
//! `list`, `describe`, `doctor`, `plan` and `score` all failed. They failed
//! politely, with an actionable message, which is exactly why it survived: the
//! first command a new user types failed every time and looked like a
//! configuration choice rather than a bug.
//!
//! THE ORDER, AND WHY IT IS THIS ORDER
//! -----------------------------------
//! 1. `--registry`, or `STEGOBENCH_REGISTRY`. Somebody said where. If it is
//!    not there, that is an ERROR and never a quiet fall-through: a typo in an
//!    environment variable that silently scored against a different registry
//!    would be undetectable in the result.
//! 2. `./plugins/registry`. The source checkout, kept first so a contributor
//!    editing a TOML file sees their edit rather than a copy installed months
//!    ago. Exactly that relative path, with no walk up the parent directories,
//!    because an ancestor walk is one more way to get a registry you did not
//!    mean.
//! 3. Beside the running executable. An unpacked release archive, and the
//!    `bin/../share` layout a package manager installs into.
//! 4. The per-user data directory for this platform.
//! 5. The system data directory for this platform.
//! 6. The copy compiled into this binary. A few kilobytes of TOML, so a fresh
//!    install works with no files at all.
//!
//! A candidate directory that EXISTS is selected, and a selected directory
//! that fails to load is a hard error naming it. Skipping a malformed registry
//! to try the next candidate would hide the broken file from the person who
//! just edited it and answer their question from somewhere else.
//!
//! WHY THE PLATFORM PATHS ARE COMPUTED HERE RATHER THAN BY A CRATE
//! ---------------------------------------------------------------
//! `dirs` and `directories` are the usual answer, and both would be a new
//! dependency for three environment variable reads and a join. This workspace
//! keeps a deliberately small dependency set (baseline Section 5) and every
//! dependency is attack surface that runs with the user's privileges. The
//! whole of what those crates would give us here is below, under test.

use std::path::{Path, PathBuf};

use stegobench_core::registry::Registry;

include!(concat!(env!("OUT_DIR"), "/embedded_registry.rs"));

/// The directory name a registry is installed under, below a data directory.
const INSTALL_SUBDIR: &str = "stegobench";

/// Where the registry that answered came from.
///
/// Carried alongside the registry rather than logged and dropped, because a
/// user with two registries on one machine needs to know which one answered
/// before they can trust the answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// `--registry`, or `STEGOBENCH_REGISTRY`.
    Explicit(PathBuf),
    /// `./plugins/registry`, relative to where the command was typed.
    WorkingDirectory(PathBuf),
    /// Beside the running executable, or in the `share` directory beside it.
    BesideExecutable(PathBuf),
    /// This platform's per-user data directory.
    UserData(PathBuf),
    /// This platform's system-wide data directory.
    SystemData(PathBuf),
    /// Compiled into this binary.
    BuiltIn,
}

impl Source {
    /// The directory it was read from, or `None` for the built-in copy.
    pub fn path(&self) -> Option<&Path> {
        match self {
            Source::Explicit(p)
            | Source::WorkingDirectory(p)
            | Source::BesideExecutable(p)
            | Source::UserData(p)
            | Source::SystemData(p) => Some(p),
            Source::BuiltIn => None,
        }
    }

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

    /// How it was found, for a person reading the terminal.
    pub fn how(&self) -> &'static str {
        match self {
            Source::Explicit(_) => "you named it, with --registry or STEGOBENCH_REGISTRY",
            Source::WorkingDirectory(_) => "found under the directory you are in",
            Source::BesideExecutable(_) => "found beside the stegobench executable",
            Source::UserData(_) => "found in your user data directory",
            Source::SystemData(_) => "found in the system data directory",
            Source::BuiltIn => "compiled into this binary, because no registry was found on disk",
        }
    }

    /// One line naming the registry that answered.
    ///
    /// The path is the answer. How it was found belongs in `--json` for
    /// somebody debugging two registries, not in front of everybody else.
    pub fn line(&self) -> String {
        match self.path() {
            Some(p) => format!("registry  {}", p.display()),
            None => "registry  built in".to_string(),
        }
    }
}

/// A registry, and where it came from.
#[derive(Debug)]
pub struct Resolved {
    pub registry: Registry,
    pub source: Source,
}

impl Resolved {
    /// Loads one named directory, and fails if it cannot be read.
    ///
    /// The path a caller names is never second-guessed: this is what
    /// `--registry` does, and what a test that wants a particular fixture
    /// wants.
    pub fn from_dir(dir: &Path) -> Result<Self, Error> {
        Ok(Resolved {
            registry: load(dir)?,
            source: Source::Explicit(dir.to_path_buf()),
        })
    }

    /// The copy compiled into this binary.
    pub fn built_in() -> Result<Self, Error> {
        Ok(Resolved {
            registry: embedded()?,
            source: Source::BuiltIn,
        })
    }

    /// The JSON block every command that reads a registry reports.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "source": self.source.tag(),
            "path": self.source.path().map(|p| p.display().to_string()),
            "how": self.source.how(),
            "tools": self.registry.entries.len(),
            "corpora": self.registry.corpora.len(),
        })
    }
}

/// Why no registry could be produced.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    // A named registry is never quietly swapped for another one, which is why
    // this is a failure rather than a fall back to the built-in copy. The
    // reader needs the path and the two ways out, not the reasoning.
    #[error(
        "{source}\n\
         {path} was named with --registry or STEGOBENCH_REGISTRY. Fix the \
         path, or drop the flag to let stegobench find one."
    )]
    Named {
        path: String,
        #[source]
        source: Box<stegobench_core::registry::RegistryError>,
    },
    #[error(
        "no registry found, and this build carries no built-in copy.\n\
         Looked for `plugins/registry` under: {searched}\n\
         Point --registry at a directory of tool descriptions, or run \
         stegobench from a checkout."
    )]
    NoneAnywhere { searched: String },
    #[error("the registry compiled into this binary is not loadable, which is a bug: {0}")]
    BuiltInBroken(String),
}

impl Error {
    /// Every one of these stops the command. `score` and `doctor` both read a
    /// registry before they do anything, so there is no partial state to
    /// report and a single code is honest.
    pub fn exit_code(&self) -> i32 {
        stegobench_core::exit::FAILURE
    }
}

fn load(dir: &Path) -> Result<Registry, Error> {
    Registry::load(dir).map_err(|source| Error::Named {
        path: dir.display().to_string(),
        source: Box::new(source),
    })
}

/// Works out which registry answers, in the order the module docstring sets
/// out, and loads it.
pub fn resolve(explicit: Option<&Path>) -> Result<Resolved, Error> {
    if let Some(dir) = explicit {
        return Resolved::from_dir(dir);
    }
    let candidates = search_path();
    for (dir, make) in &candidates {
        // `is_dir()` rather than `exists()`: a FILE called `plugins/registry`
        // is not a registry, and letting it be selected would turn a mistake
        // into a confusing load error from a path nobody typed.
        if dir.is_dir() {
            let registry = Registry::load(dir).map_err(|source| Error::Named {
                path: dir.display().to_string(),
                source: Box::new(source),
            })?;
            return Ok(Resolved {
                registry,
                source: make(dir.clone()),
            });
        }
    }
    if EMBEDDED_REGISTRY.is_empty() {
        return Err(Error::NoneAnywhere {
            searched: candidates
                .iter()
                .map(|(p, _)| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", "),
        });
    }
    Resolved::built_in()
}

/// Every place a registry is looked for, in order, without touching the disk.
///
/// Split out from `resolve` so the order itself is testable: the ordering is
/// the security-relevant part of this module, and a test that had to create
/// directories in five places to check it would not be written.
type Candidate = (PathBuf, fn(PathBuf) -> Source);

pub fn search_path() -> Vec<Candidate> {
    let mut out: Vec<Candidate> = Vec::new();
    out.push((
        PathBuf::from("plugins").join("registry"),
        Source::WorkingDirectory,
    ));
    for dir in beside_executable() {
        out.push((dir, Source::BesideExecutable));
    }
    if let Some(dir) = user_data_dir() {
        out.push((dir.join(INSTALL_SUBDIR).join("registry"), Source::UserData));
    }
    for dir in system_data_dirs() {
        out.push((
            dir.join(INSTALL_SUBDIR).join("registry"),
            Source::SystemData,
        ));
    }
    out
}

/// The two layouts a release archive and a package manager produce.
///
/// An empty list when the executable's own path cannot be established, which
/// happens on some sandboxes: that is a reason to look elsewhere, not to fail.
fn beside_executable() -> Vec<PathBuf> {
    let Ok(exe) = std::env::current_exe() else {
        return Vec::new();
    };
    let Some(dir) = exe.parent() else {
        return Vec::new();
    };
    vec![
        // An unpacked release archive, kept whole.
        dir.join("plugins").join("registry"),
        dir.join("registry"),
        // <prefix>/bin/stegobench installed beside <prefix>/share/stegobench.
        dir.join("..")
            .join("share")
            .join(INSTALL_SUBDIR)
            .join("registry"),
    ]
}

/// This platform's per-user data directory, by its own convention.
///
/// Public because `fetch` puts its content-addressed corpus store under the
/// same directory, and two implementations of this would disagree the first
/// time somebody set XDG_DATA_HOME.
pub fn user_data_dir() -> Option<PathBuf> {
    if cfg!(target_os = "windows") {
        return non_empty("APPDATA").map(PathBuf::from);
    }
    if cfg!(target_os = "macos") {
        return non_empty("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"));
    }
    // The XDG base directory specification, which is what Linux and the other
    // unixes follow. A relative XDG_DATA_HOME is ignored rather than resolved
    // against the current directory: the specification says it must be
    // absolute, and honouring a relative one would make the answer depend on
    // where the command was typed.
    match non_empty("XDG_DATA_HOME").map(PathBuf::from) {
        Some(p) if p.is_absolute() => Some(p),
        _ => non_empty("HOME").map(|h| PathBuf::from(h).join(".local/share")),
    }
}

/// The system-wide data directories, most specific first.
fn system_data_dirs() -> Vec<PathBuf> {
    if cfg!(target_os = "windows") {
        return non_empty("PROGRAMDATA")
            .map(|p| vec![PathBuf::from(p)])
            .unwrap_or_default();
    }
    vec![
        PathBuf::from("/usr/local/share"),
        PathBuf::from("/usr/share"),
    ]
}

/// An environment variable that is set AND not empty.
///
/// An empty `HOME` is the case that matters: joining onto it produces a
/// relative path that silently resolves against the current directory, which
/// is the "a registry you did not mean" failure this module is trying to avoid.
fn non_empty(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.is_empty())
}

/// Builds a registry from the TOML compiled into this binary.
///
/// WHY THIS PARSES THE FILES ITSELF
/// --------------------------------
/// `Registry::load` takes a directory, and there is no directory here. The
/// alternative is unpacking the embedded copy into a temporary directory at
/// every invocation, which puts a filesystem write on the path of a command
/// that is meant to work when nothing is installed.
///
/// The duplication that costs is the validation: this repeats the checks the
/// directory loader makes rather than sharing them. It is held in line by
/// `the_built_in_registry_is_the_registry_on_disk`, which loads both and
/// compares them.
fn embedded() -> Result<Registry, Error> {
    let mut reg = Registry::default();
    for (path, text) in EMBEDDED_REGISTRY {
        let in_corpora = path.starts_with(&format!("{}/", stegobench_core::registry::CORPORA_DIR));
        if in_corpora {
            let entry: stegobench_core::corpus::CorpusEntry =
                toml::from_str(text).map_err(|e| Error::BuiltInBroken(format!("{path}: {e}")))?;
            entry.validate().map_err(|problems| {
                Error::BuiltInBroken(format!("{path}: {}", problems.join("; ")))
            })?;
            if reg.corpora.insert(entry.id.clone(), entry).is_some() {
                return Err(Error::BuiltInBroken(format!(
                    "{path}: a second corpus claims that id"
                )));
            }
        } else {
            let entry: stegobench_core::registry::Entry =
                toml::from_str(text).map_err(|e| Error::BuiltInBroken(format!("{path}: {e}")))?;
            entry.validate().map_err(|problems| {
                Error::BuiltInBroken(format!("{path}: {}", problems.join("; ")))
            })?;
            if reg.entries.insert(entry.name.clone(), entry).is_some() {
                return Err(Error::BuiltInBroken(format!(
                    "{path}: a second tool claims that name"
                )));
            }
        }
    }
    for id in reg.corpora.keys() {
        if reg.entries.contains_key(id) {
            return Err(Error::BuiltInBroken(format!(
                "{id} is registered both as a tool and as a corpus"
            )));
        }
    }
    Ok(reg)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shipped() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugins/registry")
    }

    /// The whole point of embedding. A binary with a built-in copy that has
    /// drifted from the repository answers `list detectors` with a world that
    /// no longer exists, and nothing on the machine would say so.
    #[test]
    fn the_built_in_registry_is_the_registry_on_disk() {
        let disk = Registry::load(&shipped()).expect("the shipped registry loads");
        let built_in = embedded().expect("the built-in registry loads");
        assert_eq!(
            disk.entries.keys().collect::<Vec<_>>(),
            built_in.entries.keys().collect::<Vec<_>>(),
            "the built-in registry names different tools from plugins/registry"
        );
        assert_eq!(
            disk.corpora.keys().collect::<Vec<_>>(),
            built_in.corpora.keys().collect::<Vec<_>>(),
            "the built-in registry names different corpora from plugins/registry"
        );
        for (name, entry) in &disk.entries {
            assert_eq!(
                Some(entry),
                built_in.entries.get(name),
                "{name} differs between the built-in registry and plugins/registry"
            );
        }
        for (id, entry) in &disk.corpora {
            assert_eq!(
                Some(entry),
                built_in.corpora.get(id),
                "{id} differs between the built-in registry and plugins/registry"
            );
        }
    }

    /// A check on the build script rather than on the loader: a walk that
    /// missed a subdirectory would still produce a self-consistent registry.
    #[test]
    fn every_toml_file_in_the_registry_is_embedded() {
        let mut on_disk: Vec<String> = Vec::new();
        let mut stack = vec![shipped()];
        while let Some(d) = stack.pop() {
            for item in std::fs::read_dir(&d)
                .expect("the registry directory reads")
                .flatten()
            {
                let p = item.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().is_some_and(|e| e == "toml") {
                    on_disk.push(
                        p.strip_prefix(shipped())
                            .expect("under the registry root")
                            .components()
                            .map(|c| c.as_os_str().to_string_lossy().into_owned())
                            .collect::<Vec<_>>()
                            .join("/"),
                    );
                }
            }
        }
        on_disk.sort();
        let mut embedded_paths: Vec<String> = EMBEDDED_REGISTRY
            .iter()
            .map(|(p, _)| p.to_string())
            .collect();
        embedded_paths.sort();
        assert_eq!(
            on_disk, embedded_paths,
            "build.rs embedded a different set of files from the one in plugins/registry"
        );
        assert!(
            !embedded_paths.is_empty(),
            "nothing was embedded, so an installed binary has no fallback registry"
        );
    }

    /// The ordering is the part of this module that decides whether somebody
    /// gets a registry they did not mean, so it is pinned rather than trusted.
    #[test]
    fn the_working_directory_is_searched_before_anything_installed() {
        let path = search_path();
        assert_eq!(
            path.first().map(|(p, _)| p.clone()),
            Some(PathBuf::from("plugins").join("registry")),
            "the checkout is no longer searched first, so an editor's changes \
             can be answered from an installed copy"
        );
        assert!(
            path.len() >= 2,
            "only {} candidate(s), so nothing installed is searched at all",
            path.len()
        );
    }

    /// No candidate may be a bare relative path other than the documented
    /// checkout one. A relative path resolves against wherever the command was
    /// typed, which is exactly the defect this module closes.
    #[test]
    fn no_installed_candidate_is_relative() {
        for (p, _) in search_path().into_iter().skip(1) {
            assert!(
                p.is_absolute(),
                "{} is relative, so it means a different directory depending on \
                 where the command was typed",
                p.display()
            );
        }
    }

    /// A named path that is not there is an error, never a silent fall-through
    /// to the built-in copy. A typo in STEGOBENCH_REGISTRY that quietly scored
    /// against a different registry would not be visible in the result.
    #[test]
    fn a_named_registry_that_is_missing_is_an_error_not_a_fallback() {
        let missing = PathBuf::from("/nonexistent/stegobench/registry");
        let err = resolve(Some(&missing)).expect_err("a missing named registry fails");
        assert!(
            matches!(err, Error::Named { .. }),
            "got {err:?}, which is not the named-path failure"
        );
        let text = err.to_string();
        // The path the user named, and the two ways out. Not the reasoning.
        assert!(
            text.contains(&missing.display().to_string()),
            "the message does not name the path that was asked for: {text}"
        );
        assert!(
            text.contains("--registry") && text.contains("drop the flag"),
            "the message does not say what to do next: {text}"
        );
    }

    #[test]
    fn a_named_registry_that_is_there_is_used_as_given() {
        let resolved = resolve(Some(&shipped())).expect("the shipped registry resolves");
        assert_eq!(resolved.source, Source::Explicit(shipped()));
        assert!(!resolved.registry.entries.is_empty());
    }

    /// The built-in copy has to be reachable without a filesystem, because
    /// that is the case it exists for.
    #[test]
    fn the_built_in_registry_resolves_and_says_so() {
        let resolved = Resolved::built_in().expect("the built-in registry loads");
        assert_eq!(resolved.source, Source::BuiltIn);
        assert_eq!(resolved.source.path(), None);
        assert_eq!(resolved.source.tag(), "built-in");
        assert_eq!(resolved.source.line(), "registry  built in");
        assert!(!resolved.registry.entries.is_empty());
    }

    #[test]
    fn the_reported_source_names_the_directory_that_answered() {
        let resolved = Resolved::from_dir(&shipped()).expect("loads");
        let json = resolved.to_json();
        assert_eq!(json["source"], "explicit");
        assert_eq!(
            json["path"].as_str().expect("a path"),
            shipped().display().to_string()
        );
        assert!(json["tools"].as_u64().expect("a count") > 0);
        assert!(resolved.source.line().contains("plugins/registry"));
    }

    #[test]
    fn an_empty_environment_variable_is_not_a_path() {
        assert_eq!(non_empty("STEGOBENCH_A_VARIABLE_NOBODY_SETS"), None);
    }

    /// Every variant reports a tag and a sentence, so a variant added later
    /// without one fails here rather than printing an empty line.
    #[test]
    fn every_source_variant_describes_itself() {
        let p = PathBuf::from("/x");
        for source in [
            Source::Explicit(p.clone()),
            Source::WorkingDirectory(p.clone()),
            Source::BesideExecutable(p.clone()),
            Source::UserData(p.clone()),
            Source::SystemData(p.clone()),
            Source::BuiltIn,
        ] {
            assert!(!source.tag().is_empty());
            assert!(source.how().len() > 10, "{:?} has no explanation", source);
            assert!(!source.line().is_empty());
        }
    }
}
