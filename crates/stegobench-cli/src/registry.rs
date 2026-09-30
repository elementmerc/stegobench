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
include!(concat!(env!("OUT_DIR"), "/embedded_adapters.rs"));

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
            // Resolved, for the reason `fixtures::Fixtures::line` gives: a
            // relative path is not somewhere a reader can go, and these two
            // lines are read as a pair.
            Some(p) => format!("registry  {}", crate::resolved_path(p).display()),
            None => "registry  built in".to_string(),
        }
    }
}

/// A registry, and where it came from.
#[derive(Debug)]
pub struct Resolved {
    pub registry: Registry,
    pub source: Source,
    /// Holds the temporary directory the built-in adapters were written to,
    /// so they outlive the resolution and are removed when the command ends.
    /// `None` for every registry read from disk, which has its adapters beside
    /// it already.
    _adapters: Option<tempfile::TempDir>,
    /// The trees a relative `invoke.adapter` is resolved against.
    ///
    /// Carried here rather than written into the entries, because an entry is
    /// a reflection of the TOML a human wrote and `describe` publishes it. See
    /// `stegobench_plugin::adapter`.
    adapter_roots: Vec<PathBuf>,
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
            _adapters: None,
            adapter_roots: stegobench_plugin::adapter::roots_for_registry(dir),
        })
    }

    /// The copy compiled into this binary.
    pub fn built_in() -> Result<Self, Error> {
        let registry = embedded()?;
        let adapters = unpack_built_in_adapters(&registry)?;
        // The scratch directory itself, because the adapters are unpacked into
        // it side by side under their plain names rather than under the
        // `plugins/adapters/` the entries declare.
        let adapter_roots = adapters
            .as_ref()
            .map(|d| vec![d.path().to_path_buf()])
            .unwrap_or_default();
        Ok(Resolved {
            registry,
            source: Source::BuiltIn,
            _adapters: adapters,
            adapter_roots,
        })
    }

    /// The trees a relative `invoke.adapter` is resolved against.
    pub fn adapter_roots(&self) -> &[PathBuf] {
        &self.adapter_roots
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
    #[error("{source}{}", registry_footer(.path, *.explicit, .source))]
    Named {
        path: String,
        /// Whether this directory was named on the command line or found.
        explicit: bool,
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

/// The line under a registry refusal, which is not the same line every time.
///
/// "Fix the path" is advice for the one refusal the path itself caused. Under
/// a malformed entry it is wrong twice over: the path was right, and dropping
/// the flag loads a different registry in which the entry being repaired does
/// not appear at all. Every other case gets the registry named and nothing
/// else, the same way the commands that succeed name it.
fn registry_footer(
    path: &str,
    explicit: bool,
    source: &stegobench_core::registry::RegistryError,
) -> String {
    use stegobench_core::registry::RegistryError;
    match source {
        RegistryError::Read { .. } if explicit => format!(
            "\n{path} was named with --registry or STEGOBENCH_REGISTRY. Fix \
             the path, or drop the flag to let stegobench find one."
        ),
        _ => format!("\nregistry  {path}"),
    }
}

fn load(dir: &Path) -> Result<Registry, Error> {
    Registry::load(dir).map_err(|source| Error::Named {
        path: dir.display().to_string(),
        explicit: true,
        source: Box::new(source),
    })
}

/// Works out which registry answers, in the order the module docstring sets
/// out, and loads it.
pub fn resolve(explicit: Option<&Path>) -> Result<Resolved, Error> {
    resolve_with(explicit, non_empty("STEGOBENCH_REGISTRY").as_deref())
}

/// [`resolve`], with the environment's answer handed in rather than read.
///
/// The seam exists so the precedence between `--registry` and
/// `STEGOBENCH_REGISTRY`, and the treatment of an empty variable, are testable
/// without `set_var`. Setting a variable is process-global, and this suite runs
/// its tests on several threads at once, so a test that set one would be
/// reaching into every other test running beside it.
pub fn resolve_with(explicit: Option<&Path>, from_env: Option<&str>) -> Result<Resolved, Error> {
    // `--registry` wins, and an empty variable is not an answer: `non_empty`
    // has already discarded one, and a caller handing in `Some("")` means the
    // same thing and is treated the same way.
    let named = explicit
        .map(Path::to_path_buf)
        .or_else(|| from_env.filter(|v| !v.is_empty()).map(PathBuf::from));
    if let Some(dir) = named {
        return Resolved::from_dir(&dir);
    }
    let candidates = search_path();
    for (dir, make) in &candidates {
        // `is_dir()` rather than `exists()`: a FILE called `plugins/registry`
        // is not a registry, and letting it be selected would turn a mistake
        // into a confusing load error from a path nobody typed.
        if dir.is_dir() {
            let registry = Registry::load(dir).map_err(|source| Error::Named {
                path: dir.display().to_string(),
                explicit: false,
                source: Box::new(source),
            })?;
            return Ok(Resolved {
                registry,
                source: make(dir.clone()),
                _adapters: None,
                adapter_roots: stegobench_plugin::adapter::roots_for_registry(dir),
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
pub(crate) fn system_data_dirs() -> Vec<PathBuf> {
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
pub fn non_empty(key: &str) -> Option<String> {
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

/// Writes the built-in adapters into a scratch directory for this run.
///
/// WHY THE BUILT-IN REGISTRY NEEDS THIS AT ALL
/// -------------------------------------------
/// A `host = true` entry names its adapter the way the repository holds it,
/// `plugins/adapters/aletheia_one.py`, which is relative to the tree the
/// registry sits in. The built-in copy has no tree: it exists for the machine
/// with NO checkout and no installed registry, so its three entries that
/// declare an adapter would name a file that is not there and fail with advice
/// ("run from the root of the clone") that a reader who installed a package
/// cannot act on.
///
/// The bytes are therefore carried in the binary beside the TOML and written
/// to a temporary directory held for the life of the command, which is the
/// same shape the self-test fixtures use. Nothing is left behind, and no
/// cache has to be invalidated when an adapter changes, because the copy is
/// only ever as old as the binary.
///
/// The ENTRIES ARE NOT TOUCHED. The directory becomes this resolution's only
/// adapter root, and `stegobench_plugin::adapter` matches the declared path
/// against it by file name. Rewriting the entries is what this used to do, and
/// it put a scratch path that changed every run, and was deleted before the
/// reader saw it, into `describe` and `list --json`.
///
/// Returns `None` when no entry declares an adapter, so a registry that needs
/// no adapters writes nothing at all.
fn unpack_built_in_adapters(registry: &Registry) -> Result<Option<tempfile::TempDir>, Error> {
    let wanted: Vec<String> = registry
        .entries
        .values()
        .filter_map(|e| e.invoke.as_ref().and_then(|i| i.adapter.clone()))
        .collect();
    if wanted.is_empty() {
        return Ok(None);
    }

    let scratch = tempfile::Builder::new()
        .prefix("stegobench-adapters-")
        .tempdir()
        .map_err(|e| Error::BuiltInBroken(format!("could not unpack the adapters: {e}")))?;

    let mut written: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for (name, bytes) in EMBEDDED_ADAPTERS {
        // The names come from a directory listing at build time, so they carry
        // no separators. Checked anyway, because this writes files.
        if name.contains('/') || name.contains('\\') || name.contains("..") {
            return Err(Error::BuiltInBroken(format!(
                "built-in adapter {name:?} is not a plain file name"
            )));
        }
        let dest = scratch.path().join(name);
        std::fs::write(&dest, bytes).map_err(|e| {
            Error::BuiltInBroken(format!("could not write the built-in adapter {name}: {e}"))
        })?;
        written.insert(*name);
    }

    // An entry naming an adapter this binary does not carry is a broken build
    // rather than a user's problem, and saying so here beats a file-not-found
    // from three layers down at the moment a detector was meant to run.
    for rel in &wanted {
        let base = Path::new(rel)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if !written.contains(base) {
            return Err(Error::BuiltInBroken(format!(
                "an entry names the adapter {rel}, which is not compiled into this binary"
            )));
        }
    }

    Ok(Some(scratch))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shipped() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugins/registry")
    }

    /// The built-in registry is for the machine with no checkout, and until
    /// this landed its three host entries named an adapter by a path relative
    /// to wherever the user happened to be standing. Measured from a scratch
    /// directory, all three reported BROKEN with advice to run from the root
    /// of a clone the reader does not have.
    #[test]
    fn every_built_in_adapter_resolves_against_the_roots_this_resolution_carries() {
        let resolved = Resolved::built_in().expect("the built-in registry loads");
        let declared: Vec<&str> = resolved
            .registry
            .entries
            .values()
            .filter_map(|e| e.invoke.as_ref().and_then(|i| i.adapter.as_deref()))
            .collect();
        assert!(
            !declared.is_empty(),
            "no entry declares an adapter, so this test proves nothing; if that \
             is now true, delete it rather than leaving it passing vacuously"
        );
        for path in declared {
            // The ENTRY is untouched: still the relative path the TOML wrote.
            // Rewriting it is what put a scratch directory into `describe`.
            assert!(
                Path::new(path).is_relative(),
                "{path} was rewritten into the entry, which is what makes \
                 `describe --json` differ between two runs of one binary"
            );
            let found = stegobench_plugin::adapter::resolve(path, resolved.adapter_roots())
                .unwrap_or_else(|why| panic!("{why}"));
            assert!(found.is_file(), "{} is not there", found.display());
        }
    }

    /// The temporary directory lives on the `Resolved`, so a caller that keeps
    /// only the roots would be holding a directory whose files had been
    /// deleted. Both halves are asserted: the adapters are there while the
    /// resolution is, and gone once it is dropped.
    #[test]
    fn the_unpacked_adapters_live_exactly_as_long_as_the_resolution() {
        let resolved = Resolved::built_in().expect("the built-in registry loads");
        let declared = resolved
            .registry
            .entries
            .values()
            .find_map(|e| e.invoke.as_ref().and_then(|i| i.adapter.clone()))
            .expect("at least one entry declares an adapter");
        let found = stegobench_plugin::adapter::resolve(&declared, resolved.adapter_roots())
            .expect("it resolves while the resolution is alive");
        assert!(found.is_file());

        let roots: Vec<PathBuf> = resolved.adapter_roots().to_vec();
        drop(resolved);
        assert!(
            !found.exists(),
            "{} outlived its resolution, so the scratch directory is leaking",
            found.display()
        );
        // NOT "resolving now fails". Resolution falls back to the working
        // directory on purpose, so that a contributor standing in a checkout
        // is served by the tree they are standing in. Asserting a failure here
        // therefore asserts the absence of that fallback, and its outcome
        // depends on where the test was run from: it passed under `cargo test`,
        // whose working directory is the package, and failed every time the
        // same binary was run from the root of the repository, where
        // `plugins/adapters` is real. What the lifetime actually promises is
        // that the scratch copy is gone, so that is what is checked.
        if let Ok(after) = stegobench_plugin::adapter::resolve(&declared, &roots) {
            assert_ne!(
                after, found,
                "the scratch copy is still being handed out after its \
                 resolution was dropped"
            );
        }
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
        // Compared LITERALLY, adapter path included. This used to need a
        // carve-out, because `Registry::load` rewrote `invoke.adapter` to an
        // absolute path and the built-in copy, parsed from strings compiled
        // into the binary, had no directory to rewrite against. Nothing
        // rewrites an entry any more, so the two are the same bytes and the
        // carve-out that hid a whole field from this comparison is gone.
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

    /// `STEGOBENCH_REGISTRY` used to be bound with clap's `env`, which reads
    /// a variable that is set but empty as a flag supplied without its value
    /// and so refused EVERY command, `--help` included. It is read by this
    /// resolver now, and the whole of that behaviour is asserted here through
    /// the seam rather than by setting a process-global variable in a suite
    /// that runs its tests on several threads at once.
    #[test]
    fn the_environment_names_a_registry_only_when_it_has_something_in_it() {
        let tmp = tempfile::tempdir().expect("tmp");
        let from_env = tmp.path().join("from-env");
        let from_flag = tmp.path().join("from-flag");
        for dir in [&from_env, &from_flag] {
            std::fs::create_dir_all(dir).expect("dirs");
        }

        // The variable is read.
        let r = resolve_with(None, Some(&from_env.display().to_string())).expect("loads");
        assert_eq!(r.source.path(), Some(from_env.as_path()));

        // The flag beats it.
        let r =
            resolve_with(Some(&from_flag), Some(&from_env.display().to_string())).expect("loads");
        assert_eq!(
            r.source.path(),
            Some(from_flag.as_path()),
            "the environment overrode an explicit --registry"
        );

        // An empty value is not an answer. If it were read as one, this would
        // try to load "" and fail rather than falling through to the search.
        let empty = resolve_with(None, Some(""));
        assert!(
            empty.is_err() || empty.expect("ok").source.path() != Some(Path::new("")),
            "an empty variable was read as a registry path"
        );

        // A path that IS named and is not there is an error, never a quiet
        // fall back to some other registry.
        let missing = resolve_with(None, Some(&tmp.path().join("nope").display().to_string()));
        assert!(
            missing.is_err(),
            "a named registry that is absent fell back"
        );
    }

    /// The roots a registry hands its adapters, for both layouts that exist.
    #[test]
    fn a_resolved_registry_carries_the_roots_its_adapters_need() {
        let tmp = tempfile::tempdir().expect("tmp");
        let registry = tmp
            .path()
            .join("prefix")
            .join("stegobench")
            .join("registry");
        std::fs::create_dir_all(&registry).expect("dirs");
        let resolved = Resolved::from_dir(&registry).expect("an empty registry loads");
        let roots = resolved.adapter_roots();
        assert!(
            roots.contains(&tmp.path().join("prefix")),
            "the checkout layout root is missing: {roots:?}"
        );
        assert!(
            roots.contains(&tmp.path().join("prefix").join("stegobench")),
            "the install layout root is missing: {roots:?}"
        );
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

    /// "Fix the path" is advice for one refusal only.
    ///
    /// Every malformed entry used to end on it, and on an offer to drop the
    /// flag, which would load a different registry in which the entry being
    /// repaired does not appear at all. The path was right every time; the
    /// line above it had already said what was wrong.
    #[test]
    fn only_a_path_refusal_tells_the_reader_to_fix_the_path() {
        let tmp = tempfile::tempdir().expect("tmp");
        let dir = tmp.path().join("reg");
        std::fs::create_dir_all(dir.join("detectors")).expect("dirs");
        std::fs::write(
            dir.join("detectors/broken.toml"),
            "name = \"broken\"\nkind = \"detector\"\nlicence = \"MIT\"\nthis is not toml\n",
        )
        .expect("write");

        let bad_entry = resolve_with(Some(&dir), None).expect_err("a malformed entry refuses");
        let text = bad_entry.to_string();
        assert!(
            !text.contains("Fix the path") && !text.contains("drop the flag"),
            "a malformed entry was blamed on the path:\n{text}"
        );
        assert!(
            text.contains(&format!("registry  {}", dir.display())),
            "the refusal does not name the registry it read:\n{text}"
        );

        // And the refusal the advice was written for still carries it.
        let missing = tmp.path().join("nope");
        let bad_path = resolve_with(Some(&missing), None).expect_err("a missing path refuses");
        let text = bad_path.to_string();
        assert!(
            text.contains("Fix the path") && text.contains("drop the flag"),
            "the path advice went missing from the refusal it answers:\n{text}"
        );
    }
}
