// Author:  Daniel Iwugo
// Comment: Christ is King
// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Daniel Iwugo
//! Schemas and the corpus model for stegobench.
//!
//! This crate holds the formats other people are meant to adopt, and the rules
//! about them that the type system cannot express. Scoring and plugin execution
//! live elsewhere, so that the definition of a result never depends on how a
//! result was obtained.
//!
//! The IO it does do is reading: a registry of TOML files, a corpus directory
//! on somebody's disk, and, behind the injected trait boundaries in
//! [`fetch`], a corpus tier being downloaded. There is no network client in
//! here and no default transport; every byte a fetch moves arrives through
//! something the caller passed in.

pub mod corpus;
pub mod fetch;
pub mod header;
pub mod manifest;
pub mod registry;
pub mod result;
pub mod run;
pub mod samples;

pub use corpus::{ArchiveFormat, CorpusEntry, DownloadRoute, LicenceStatus, Redistribution};
pub use fetch::{BlobStore, FetchError, Fetched, FileStore, Limits, Progress, Transport};
pub use header::{Format, Geometry, HeaderError, Shape};
pub use manifest::{ManifestV1, MANIFEST_SCHEMA_ID};
pub use registry::{Entry, Kind, Registry};
pub use result::{Result1, RESULT_SCHEMA_ID};
pub use run::{RunV1, RUN_SCHEMA_ID};
pub use samples::{Role, Sample, SampleError, Samples};

/// Process exit codes, which are part of the CLI's contract and are documented
/// in the man page. A caller, human or otherwise, distinguishes "I refused" from
/// "I broke", and an agent that cannot tell them apart retries the refusal.
pub mod exit {
    /// Everything worked.
    pub const OK: i32 = 0;
    /// Something failed, with a diagnostic path printed.
    pub const FAILURE: i32 = 1;
    /// Bad flags, unknown plugin name.
    pub const USAGE: i32 = 2;
    /// The governor says this run does not fit on this machine. A refusal,
    /// not a breakage: retrying it unchanged will refuse again.
    pub const PREFLIGHT_REFUSED: i32 = 3;
    /// A detector or embedder failed beyond the error tolerance.
    pub const PLUGIN_FAILED: i32 = 4;
    /// A digest did not match what was claimed.
    pub const VERIFY_MISMATCH: i32 = 5;
    /// A document did not match its schema.
    pub const SCHEMA_INVALID: i32 = 6;
    /// The corpus licence does not permit this operation. Also a refusal.
    pub const LICENCE_REFUSED: i32 = 7;
    /// A dependency needed for the requested work is missing or broken.
    pub const ENVIRONMENT_UNFIT: i32 = 8;
    /// Interrupted; state written, safe to resume.
    pub const INTERRUPTED: i32 = 130;
}

/// The harness version, stamped into every result so a number can be traced to
/// the code that produced it.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
