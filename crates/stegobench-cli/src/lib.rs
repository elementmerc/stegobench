// Author:  Daniel Iwugo
// Comment: Christ is King
// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Daniel Iwugo
//! Library half of the `stegobench` command, split out purely so `build.rs`
//! can import the command definition and generate man pages from the same
//! source the binary parses against. There is no other reason for a lib
//! target here; the behaviour lives in the binary.

pub mod cli;
pub mod fetch;
pub mod fixtures;
pub mod help_topics;
pub mod metrics;
pub mod needs;
pub mod registry;
pub mod report;
pub mod score;
