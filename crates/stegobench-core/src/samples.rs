// Author:  Daniel Iwugo
// Comment: Christ is King
// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Daniel Iwugo
//! Enumerating the scorable items of an unpacked corpus directory.
//!
//! WHY THIS IS NOT IN `corpus.rs`
//! ------------------------------
//! [`crate::corpus`] holds `CorpusEntry`, which is the registry's *description*
//! of a dataset: its licence, where to download it, how many covers it claims.
//! This module reads the bytes on somebody's disk. They share a noun and
//! nothing else, and putting a filesystem walk into the file that currently
//! does no IO at all would blur the one boundary the crate's own documentation
//! promises (see `lib.rs`: the definition of a result never depends on how a
//! result was obtained).
//!
//! WHAT A SAMPLE IS
//! ----------------
//! The shipped reader, `load_pentimento.py`, defines it: an image and a JSON
//! record that share a basename, so `000123.png` and `000123.json`. That holds
//! inside a tar shard and inside a folder somebody extracted, which is the only
//! layout this module reads. Tar is deliberately out of scope; a reader that
//! wanted it would need a new workspace dependency, and that is a decision for
//! the operator rather than a convenience to be slipped in.
//!
//! WHY THE ID IS A CORPUS-RELATIVE STEM
//! ------------------------------------
//! The id is the record's path relative to the corpus root with the extension
//! removed, POSIX separators, so `wow-0200/000123`. Three properties are being
//! bought:
//!
//! - **It is not the basename.** Every arm restarts its numbering at `000000`,
//!   so `000123` names thirty-nine different images across a Core release. The
//!   shipped loader learned this the same way and says so in its own comments.
//! - **It does not contain the absolute path.** A resumable run keys on the id,
//!   so a corpus moved from `/scratch` to `/data` between the interruption and
//!   the resume has to produce the same keys, or the resume silently rescores
//!   everything.
//! - **It is byte-identical on every machine.** Names are required to be UTF-8
//!   and free of control characters, and the ordering is `str` comparison,
//!   which is byte-wise over UTF-8 and therefore the same everywhere. Directory
//!   iteration order is not: it varies with the filesystem, and a benchmark
//!   whose item order depends on which disk it was run from cannot be resumed
//!   or diffed.
//!
//! WHAT A HALF SAMPLE DOES
//! -----------------------
//! It stops the walk with a named basename. An image with no record, or a
//! record with no image, is a corpus defect: a truncated download, a partial
//! extraction, or the wrong directory. Skipping it quietly is how a run scores
//! fewer items than the corpus holds and still exits zero, which is the exact
//! shape of failure this project keeps finding in other people's numbers.

use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The largest per-sample JSON record this will read, in bytes.
///
/// A Pentimento record is a flat object of a few dozen scalar fields and runs
/// to roughly one kilobyte. 256 KiB is two orders of magnitude of headroom and
/// still small enough that a file which is secretly a gigabyte of nested arrays
/// is refused before it is parsed rather than after it has exhausted the
/// machine.
pub const MAX_RECORD_BYTES: u64 = 256 * 1024;

/// How far below the corpus root the walk will descend.
///
/// A release is flat, or one directory per arm, so real depth is zero or one.
/// The cap exists for the pathological input rather than the real one; symlinks
/// are refused outright (see [`SampleError::Symlink`]), so this is the belt to
/// that braces.
pub const MAX_DEPTH: usize = 32;

/// How many entries a single directory may hold.
///
/// One of the two places the walk holds a collection proportional to its input:
/// a directory's file names are read and sorted before any of its samples are
/// emitted, because filesystem order is not an order. A shard extracts to 2,000
/// entries and the whole cover tier to 20,000, so the cap is roughly fifty
/// times the largest real case. Worst case it costs this many names in memory;
/// the corresponding figure for the corpus as a whole is not bounded by
/// anything here, and deliberately is not held.
pub const MAX_ENTRIES_PER_DIR: usize = 1_048_576;

/// How many directories one enumeration will visit.
///
/// The other bounded collection. A depth-first walk queues a directory's
/// subdirectories before descending, so the queue is proportional to the tree's
/// breadth rather than to its samples. A Core release has 39 arm directories,
/// so this is four orders of magnitude of headroom, and it is a cap rather than
/// an unbounded queue because [`MAX_DEPTH`] alone bounds nothing: a single level
/// of a million directories is shallow.
pub const MAX_DIRECTORIES: usize = 1_000_000;

/// How many samples one enumeration may yield.
///
/// A Core release is 344,357 pairs over 39 arms, under a million samples. A
/// directory tree that offers a hundred million is not a corpus, and the run
/// that was about to start on it would not have finished either.
pub const MAX_SAMPLES: u64 = 100_000_000;

/// File extensions treated as the image half of a sample.
///
/// The shipped Python loader treats every non-`.json` member as an image, which
/// is safe inside a tar shard because a shard holds nothing else. A directory
/// on somebody's disk holds plenty else, so the harness names the formats it
/// will score instead of guessing. A record beside an image in some other
/// format is then reported as a half sample, which is loud, rather than
/// silently dropped, which is not.
///
/// Matched without regard to case, because a case-insensitive filesystem hands
/// back whatever case the file was created with and `000123.PNG` is the same
/// image as `000123.png`.
pub const IMAGE_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "tif", "tiff", "bmp", "pgm", "ppm", "pnm", "webp",
];

/// Which side of the measurement an item is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// Carries a payload.
    Stego,
    /// Does not. A cover, or the clean half of a pair.
    Clean,
}

/// One scorable item: an image, and what the record beside it says about it.
///
/// Deliberately thin. Pairing and split discipline are judged by whoever writes
/// the `result-v1` document, not here, because this has no way to know which
/// arm or which split the caller asked for. What it owes that caller is enough
/// to make the judgement, which is [`Sample::cover`] and [`Sample::split`].
#[derive(Debug, Clone, PartialEq)]
pub struct Sample {
    /// Record path relative to the corpus root, extension removed, POSIX
    /// separators. Stable across runs, machines and a move of the corpus.
    pub id: String,
    /// Where the image actually is, rooted at whatever path the caller opened.
    pub image: PathBuf,
    pub role: Role,
    /// The cover this derives from, as the record's `source_png` names it.
    ///
    /// `None` for a cover-tier row, which is its own cover, and for a JPEG-arm
    /// row carrying only `source_jpeg`: that route needs the clean-JPEG pool's
    /// manifest to resolve, which is not in this directory, and inventing the
    /// join from positional names is how half an arm gets attributed to the
    /// wrong photographer.
    pub cover: Option<String>,
    /// `train` or `test`, where the record carries it.
    ///
    /// Cover rows do. Arm rows do not, because the split is a property of the
    /// cover and is inherited through [`Sample::cover`]; a caller enforcing
    /// by-cover discipline joins rather than reads it here.
    pub split: Option<String>,
    /// The name the record gives for its own image, where it gives one.
    ///
    /// NOT the name on disk, and the difference is the whole reason this field
    /// exists. The packer renames every member to its position in the tier, so
    /// a cover whose record says `file = "09710.png"` is extracted as
    /// `000123.png`, while a stego row made from it still names `09710.png`
    /// under `source_png`. A caller joining stego rows to covers by the name on
    /// disk therefore joins nothing at all on a packed release, and a check
    /// that cannot look reports clean.
    pub declared_name: Option<String>,
    /// The digest the record states for its own image, exactly as written.
    ///
    /// A claim, not a measurement: nothing here opens the image to check it.
    /// It is carried so a caller can identify a corpus by what its manifest
    /// says, which is the only identification available for a directory
    /// somebody extracted from a shard and then moved.
    pub digest: Option<String>,
    /// What the record says about the arm this sample belongs to.
    pub arm: ArmInfo,
}

/// The arm fields, reported exactly as the record writes them.
///
/// Every one of these is a string or a number out of somebody's corpus, and
/// none of it is mapped onto this crate's own vocabulary here. A record saying
/// `jpeg-dct` means the same thing as `result-v1`'s `jpeg`, and deciding that
/// is a judgement about two projects' vocabularies rather than a fact about a
/// file, so it belongs with the caller that writes the result document.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ArmInfo {
    /// The published arm name, such as `wow-0200`.
    ///
    /// The packer writes this deliberately in the form the release publishes
    /// rather than the form the builder used, because the two once differed and
    /// a reader grouping by this field got an arm that matched nothing.
    pub name: Option<String>,
    /// The embedding tool, such as `wow`. `clean` on the synthesised clean arms.
    pub tool: Option<String>,
    /// How much was hidden, in whatever unit [`ArmInfo::rate_unit`] states.
    pub rate: Option<f64>,
    /// The unit, which is load-bearing: `0.4` means bits per pixel for the
    /// spatial schemes and a fraction of a reported capacity for the JPEG ones,
    /// and a reader comparing the two numbers is comparing nothing.
    pub rate_unit: Option<String>,
    /// `spatial`, `jpeg-dct`, `container` or whatever else a corpus writes.
    pub domain: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum SampleError {
    #[error("cannot read {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "{path} is not a directory; point at an unpacked corpus, not at a shard or beside one"
    )]
    NotADirectory { path: String },
    #[error("{path} is not valid JSON: {source}")]
    Parse {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error(
        "{path} is {bytes} bytes, over the {MAX_RECORD_BYTES} byte limit for a \
         sample record. A record describes one image in a few dozen fields; \
         something this size is not one, and parsing it would cost the run more \
         memory than the images do"
    )]
    RecordTooLarge { path: String, bytes: u64 },
    #[error(
        "{path} holds more than {MAX_ENTRIES_PER_DIR} entries. Names are sorted \
         before any sample is emitted, so a directory this wide is held in \
         memory in full; split it into shards"
    )]
    DirectoryTooWide { path: String },
    #[error(
        "this corpus holds more than {MAX_DIRECTORIES} directories. A walk \
         queues the directories it has not reached yet, so a tree this broad is \
         held in memory in full; check the directory that was opened"
    )]
    TooManyDirectories,
    #[error(
        "{path} is neither a regular file nor a directory. A named pipe or a \
         device where a sample record belongs would block the read for as long \
         as nobody wrote to it, so it is refused rather than opened"
    )]
    NotARegularFile { path: String },
    #[error(
        "{path} is more than {MAX_DEPTH} directories below the corpus root. A \
         corpus is flat or one directory per arm, so this is a tree that was \
         pointed at by mistake"
    )]
    TooDeep { path: String },
    #[error(
        "this corpus offers more than {MAX_SAMPLES} samples, which is not a \
         corpus. Check the directory that was opened"
    )]
    TooManySamples,
    #[error(
        "{path} is a symbolic link. Links are refused rather than followed: a \
         loop would hang the walk, and a link out of the corpus would score \
         bytes the corpus does not name. Copy the target in, or point at where \
         it really lives"
    )]
    Symlink { path: String },
    #[error(
        "{path} is not valid UTF-8. An item id is written into a JSON result \
         document, so a name that cannot be spelled there cannot be scored"
    )]
    NameNotUtf8 { path: String },
    #[error(
        "{parent} holds a name containing a control character ({name:?}). The \
         name would go into an item id and from there into line-oriented \
         output, where it would split one record into two"
    )]
    NameHasControlCharacter { parent: String, name: String },
    #[error(
        "{id} has a record but no image beside it. A sample is an image and a \
         JSON record sharing a basename; one alone means a partial extraction, \
         a truncated download, or a directory that is not an unpacked corpus. \
         It is refused rather than skipped, because a skipped item scores fewer \
         images than the corpus holds and still reports success"
    )]
    RecordWithoutImage { id: String },
    #[error(
        "{id} has an image but no record beside it. Without the record there is \
         nothing to say whether it carries a payload, which cover it came from, \
         or which side of the split it is on, so it cannot be scored. See the \
         note on a record without an image: it is refused for the same reason"
    )]
    ImageWithoutRecord { id: String },
    #[error(
        "{id} has one record and {} images ({}). Which one the record describes \
         is a guess, and a guess here mislabels a measurement",
        .images.len(),
        .images.join(", ")
    )]
    AmbiguousImage { id: String, images: Vec<String> },
    #[error("{id}: {problem}")]
    BadRecord { id: String, problem: String },
}

/// A streaming walk of an unpacked corpus directory.
///
/// WHAT IS HELD AND WHAT IS NOT
/// ----------------------------
/// One directory's sorted file names, and the list of directories not yet
/// visited. Nothing per sample survives the item it belongs to, so the cost is
/// the widest directory plus the tree's breadth, not the corpus. Section 12 of
/// the engineering baseline is the reason: a Core tier is over a million
/// samples and an enumeration that accumulated one small struct each would be
/// the first thing to fall over at ten times that.
///
/// The per-directory sort is the honest exception, and it is not avoidable:
/// filesystem iteration order varies by machine, so an enumeration that did not
/// sort would yield a different order on the machine that resumed the run than
/// on the one that started it. [`MAX_ENTRIES_PER_DIR`] is what bounds it.
///
/// WHAT AN ERROR MEANS
/// -------------------
/// The walk stops. Every error here says the corpus is not what it claims to
/// be, and continuing would produce a number measured on an unknown subset.
#[derive(Debug)]
pub struct Samples {
    root: PathBuf,
    /// Directories still to visit, with their depth. Popped from the back, so
    /// they are pushed in reverse sorted order to come out sorted.
    pending: Vec<(PathBuf, usize)>,
    /// The directory currently being emitted, relative to the root, POSIX
    /// separators, with a trailing slash unless it is the root itself.
    current_prefix: String,
    current_dir: PathBuf,
    /// Its file names, sorted.
    names: Vec<String>,
    cursor: usize,
    emitted: u64,
    stopped: bool,
}

impl Samples {
    /// Opens a corpus directory, checking it is one before any walking starts.
    pub fn open(root: &Path) -> Result<Self, SampleError> {
        let meta = std::fs::metadata(root).map_err(|e| SampleError::Read {
            path: root.display().to_string(),
            source: e,
        })?;
        if !meta.is_dir() {
            return Err(SampleError::NotADirectory {
                path: root.display().to_string(),
            });
        }
        Ok(Samples {
            root: root.to_path_buf(),
            pending: vec![(root.to_path_buf(), 0)],
            current_prefix: String::new(),
            current_dir: root.to_path_buf(),
            names: Vec::new(),
            cursor: 0,
            emitted: 0,
            stopped: false,
        })
    }

    /// Reads one directory: validates its names, sorts them, and queues its
    /// subdirectories.
    fn scan(&mut self, dir: &Path, depth: usize) -> Result<(), SampleError> {
        let read = std::fs::read_dir(dir).map_err(|e| SampleError::Read {
            path: dir.display().to_string(),
            source: e,
        })?;

        let mut files = Vec::new();
        let mut subdirs = Vec::new();
        let mut seen = 0usize;
        for item in read {
            let item = item.map_err(|e| SampleError::Read {
                path: dir.display().to_string(),
                source: e,
            })?;
            seen += 1;
            if seen > MAX_ENTRIES_PER_DIR {
                return Err(SampleError::DirectoryTooWide {
                    path: dir.display().to_string(),
                });
            }

            let path = item.path();
            // `file_type` on a DirEntry does not follow the link, which is the
            // whole point: a loop must be caught before it is walked into.
            let kind = item.file_type().map_err(|e| SampleError::Read {
                path: path.display().to_string(),
                source: e,
            })?;
            if kind.is_symlink() {
                return Err(SampleError::Symlink {
                    path: path.display().to_string(),
                });
            }

            let name = item.file_name();
            let Some(name) = name.to_str() else {
                return Err(SampleError::NameNotUtf8 {
                    path: path.display().to_string(),
                });
            };
            // Parity with the shipped loader's `_noise`: an operating system
            // sidecar beside the data is not a corpus defect, and treating one
            // as a half sample would refuse a directory that is perfectly fine.
            if name.starts_with('.') {
                continue;
            }
            if name.chars().any(char::is_control) {
                return Err(SampleError::NameHasControlCharacter {
                    parent: dir.display().to_string(),
                    name: name.to_string(),
                });
            }

            if kind.is_dir() {
                if depth + 1 > MAX_DEPTH {
                    return Err(SampleError::TooDeep {
                        path: path.display().to_string(),
                    });
                }
                subdirs.push(name.to_string());
            } else if kind.is_file() {
                files.push(name.to_string());
            } else {
                // A FIFO named `000123.json` would block `File::open` until
                // somebody wrote to it, which is a hang with no deadline on it.
                return Err(SampleError::NotARegularFile {
                    path: path.display().to_string(),
                });
            }
        }

        // Sorted on the STEM first, not on the whole name. Runs of one stem
        // have to be contiguous for the grouping below to see a sample whole,
        // and plain name order does not guarantee that: `000123.json`,
        // `000123.m.n` and `000123.png` sort in exactly that order, and the
        // middle name, whose stem is `000123.m`, splits the pair into two half
        // samples and refuses a corpus that is fine.
        files.sort_by(|a, b| (stem_of(a), a.as_str()).cmp(&(stem_of(b), b.as_str())));
        subdirs.sort();
        if self.pending.len() + subdirs.len() > MAX_DIRECTORIES {
            return Err(SampleError::TooManyDirectories);
        }
        // Popped from the back, so reversing here makes the walk visit them in
        // sorted order rather than backwards.
        for name in subdirs.into_iter().rev() {
            self.pending.push((dir.join(&name), depth + 1));
        }

        self.current_prefix = match dir.strip_prefix(&self.root) {
            Ok(rel) if rel.as_os_str().is_empty() => String::new(),
            // Every component was checked for UTF-8 on the way in, so the
            // lossy conversion cannot lose anything here.
            Ok(rel) => {
                let mut s = String::new();
                for part in rel.components() {
                    s.push_str(&part.as_os_str().to_string_lossy());
                    s.push('/');
                }
                s
            }
            // Unreachable: every queued directory was built by joining onto the
            // root. Falling back to the empty prefix keeps ids relative rather
            // than leaking an absolute path into a resumable key.
            Err(_) => String::new(),
        };
        self.current_dir = dir.to_path_buf();
        self.names = files;
        self.cursor = 0;
        Ok(())
    }

    /// Turns the next run of same-stem names into a sample, or into the reason
    /// it is not one. `Ok(None)` means the run held no sample material at all.
    fn take_run(&mut self) -> Result<Option<Sample>, SampleError> {
        let Some(first) = self.names.get(self.cursor) else {
            return Ok(None);
        };
        let stem = stem_of(first).to_string();
        let start = self.cursor;
        while self.cursor < self.names.len() && stem_of(&self.names[self.cursor]) == stem {
            self.cursor += 1;
        }
        let run = &self.names[start..self.cursor];

        let mut record: Option<&str> = None;
        let mut images: Vec<&str> = Vec::new();
        for name in run {
            match extension_of(name) {
                Some(ext) if ext.eq_ignore_ascii_case("json") => record = Some(name),
                Some(ext) if IMAGE_EXTENSIONS.iter().any(|k| ext.eq_ignore_ascii_case(k)) => {
                    images.push(name)
                }
                _ => {}
            }
        }

        let id = format!("{}{stem}", self.current_prefix);
        // Matched as a slice so that every count is handled by name and the one
        // image case binds rather than indexes: there is no arm left over to
        // need an `unreachable!`.
        match (record, images.as_slice()) {
            (None, []) => Ok(None),
            (None, _) => Err(SampleError::ImageWithoutRecord { id }),
            (Some(_), []) => Err(SampleError::RecordWithoutImage { id }),
            (Some(rec), [image]) => {
                let value = read_record(&self.current_dir.join(rec))?;
                let (role, cover, split, declared_name, digest, arm) = describe(&id, &value)?;
                Ok(Some(Sample {
                    id,
                    image: self.current_dir.join(image),
                    role,
                    cover,
                    split,
                    declared_name,
                    digest,
                    arm,
                }))
            }
            (Some(_), _) => Err(SampleError::AmbiguousImage {
                id,
                images: images.into_iter().map(str::to_string).collect(),
            }),
        }
    }
}

impl Iterator for Samples {
    type Item = Result<Sample, SampleError>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.stopped {
                return None;
            }
            if self.cursor < self.names.len() {
                match self.take_run() {
                    Ok(Some(sample)) => {
                        self.emitted += 1;
                        if self.emitted > MAX_SAMPLES {
                            self.stopped = true;
                            return Some(Err(SampleError::TooManySamples));
                        }
                        return Some(Ok(sample));
                    }
                    Ok(None) => continue,
                    Err(e) => {
                        self.stopped = true;
                        return Some(Err(e));
                    }
                }
            }
            let Some((dir, depth)) = self.pending.pop() else {
                self.stopped = true;
                return None;
            };
            if let Err(e) = self.scan(&dir, depth) {
                self.stopped = true;
                return Some(Err(e));
            }
        }
    }
}

/// Everything before the final dot, or the whole name if there is no dot.
fn stem_of(name: &str) -> &str {
    match name.rfind('.') {
        // A leading dot is the whole name, not a separator, but dotfiles are
        // already skipped before anything gets here.
        Some(0) | None => name,
        Some(i) => &name[..i],
    }
}

/// Everything after the final dot, lower-cased comparison left to the caller
/// by way of the ASCII check in [`describe`]'s callers.
fn extension_of(name: &str) -> Option<&str> {
    match name.rfind('.') {
        Some(0) | None => None,
        Some(i) => Some(&name[i + 1..]),
    }
}

/// Reads one record, refusing an oversized one before it is parsed.
fn read_record(path: &Path) -> Result<serde_json::Value, SampleError> {
    let file = std::fs::File::open(path).map_err(|e| SampleError::Read {
        path: path.display().to_string(),
        source: e,
    })?;
    let meta = file.metadata().map_err(|e| SampleError::Read {
        path: path.display().to_string(),
        source: e,
    })?;
    if meta.len() > MAX_RECORD_BYTES {
        return Err(SampleError::RecordTooLarge {
            path: path.display().to_string(),
            bytes: meta.len(),
        });
    }
    // The length was checked a moment ago on a file anybody with write access
    // can still be extending, so the read is capped as well rather than
    // trusting the answer. One byte over the limit is enough to tell.
    let mut text = String::new();
    let read = file
        .take(MAX_RECORD_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(|e| SampleError::Read {
            path: path.display().to_string(),
            source: e,
        })?;
    if read as u64 > MAX_RECORD_BYTES {
        return Err(SampleError::RecordTooLarge {
            path: path.display().to_string(),
            bytes: read as u64,
        });
    }
    serde_json::from_str(&text).map_err(|e| SampleError::Parse {
        path: path.display().to_string(),
        source: e,
    })
}

/// What the record says: which side, which cover, which split, which name,
/// which digest, and which arm.
type Described = (
    Role,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    ArmInfo,
);

fn describe(id: &str, value: &serde_json::Value) -> Result<Described, SampleError> {
    let object = value.as_object().ok_or_else(|| SampleError::BadRecord {
        id: id.to_string(),
        problem: "the record is not a JSON object. A sample record describes \
                  one image; a list or a bare value describes nothing this can \
                  score"
            .into(),
    })?;

    // The ladder is ordered by how explicit the evidence is, because the two
    // packers say it differently. `pack_arms.py` writes `role: "clean"` on the
    // synthesised clean arms and leaves it off the stego rows; a cover-tier row
    // is a `manifest-v1` row with no arm and no tool at all.
    let role = match object.get("role") {
        Some(serde_json::Value::String(s)) if s == "clean" => Role::Clean,
        Some(serde_json::Value::String(s)) if s == "stego" => Role::Stego,
        Some(serde_json::Value::String(s)) => {
            return Err(SampleError::BadRecord {
                id: id.to_string(),
                problem: format!(
                    "role is {s:?}, which is neither \"clean\" nor \"stego\". A \
                     measurement has two sides and this record claims a third"
                ),
            })
        }
        Some(other) => {
            return Err(SampleError::BadRecord {
                id: id.to_string(),
                problem: format!("role is {other}, which is not a string"),
            })
        }
        None => match object.get("tool").and_then(serde_json::Value::as_str) {
            Some("clean") => Role::Clean,
            Some(_) => Role::Stego,
            // No role, no tool. Either an arm row that names only its arm, or a
            // cover-tier row, and a cover is clean by definition.
            None => {
                if object.contains_key("arm") || object.contains_key("stego") {
                    Role::Stego
                } else {
                    Role::Clean
                }
            }
        },
    };

    let cover = match object.get("source_png") {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::String(s)) => Some(check_cover_name(id, "source_png", s)?),
        Some(other) => {
            return Err(SampleError::BadRecord {
                id: id.to_string(),
                problem: format!("source_png is {other}, which is not a file name"),
            })
        }
    };

    let split = match object.get("split") {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::String(s)) => Some(s.clone()),
        Some(other) => {
            return Err(SampleError::BadRecord {
                id: id.to_string(),
                problem: format!("split is {other}, which is not a label"),
            })
        }
    };

    // Taken exactly as written and judged nowhere here. A record states the
    // digest of its own image, so it is a claim by whoever built the corpus
    // rather than a measurement, and this module's job is to report what the
    // corpus says. A caller computing a corpus-wide digest from these is
    // identifying the manifest rather than verifying the bytes, and the result
    // document has a separate field to say which of the two it did.
    let digest = match object.get("sha256") {
        Some(serde_json::Value::String(s)) if !s.trim().is_empty() => Some(s.clone()),
        _ => None,
    };

    // Reported, never mapped. See [`ArmInfo`]: turning `jpeg-dct` into this
    // project's own word for it is a judgement about two vocabularies, and it
    // belongs with whoever writes the result document.
    let arm = ArmInfo {
        name: text(object, "arm"),
        tool: text(object, "tool"),
        rate: object.get("rate").and_then(serde_json::Value::as_f64),
        rate_unit: text(object, "rate_unit"),
        domain: text(object, "domain"),
    };

    // Held to the same rule as `source_png`, because it is joined against it.
    let declared_name = match object.get("file") {
        Some(serde_json::Value::String(s)) if !s.trim().is_empty() => {
            // A manifest may carry a path here rather than a bare name, and the
            // join only ever needs the last component.
            let last = s.rsplit(['/', '\\']).next().unwrap_or(s);
            Some(check_cover_name(id, "file", last)?)
        }
        _ => None,
    };

    Ok((role, cover, split, declared_name, digest, arm))
}

/// A record field read as a string, or None where it is absent, empty, null or
/// some other type. A corpus this cannot describe is not a corpus this should
/// refuse to score.
fn text(object: &serde_json::Map<String, serde_json::Value>, key: &str) -> Option<String> {
    match object.get(key) {
        Some(serde_json::Value::String(s)) if !s.trim().is_empty() => Some(s.clone()),
        _ => None,
    }
}

/// A cover name is a bare basename and is checked as one.
///
/// Nothing here joins it onto a path, but a caller reconciling pairs across a
/// release very reasonably would, and `../../etc/passwd` arriving from a record
/// somebody downloaded is the boundary this crate is supposed to hold.
fn check_cover_name(id: &str, field: &str, name: &str) -> Result<String, SampleError> {
    let bad = name.is_empty()
        || name == "."
        || name == ".."
        || name.contains('/')
        || name.contains('\\')
        || name.chars().any(char::is_control);
    if bad {
        return Err(SampleError::BadRecord {
            id: id.to_string(),
            problem: format!(
                "{field} is {name:?}, which is not a plain file name. The \
                 field names an image and is joined against a manifest, so a \
                 path in it would reach outside the corpus"
            ),
        });
    }
    Ok(name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn record(extra: &str) -> String {
        format!("{{\"sha256\": \"{}\"{extra}}}", "a".repeat(64))
    }

    /// Writes an image and its record under `rel`, without the extension.
    fn pair(root: &Path, rel: &str, image_ext: &str, extra: &str) {
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path.with_extension(image_ext), b"not really an image").unwrap();
        fs::write(path.with_extension("json"), record(extra)).unwrap();
    }

    fn collect(root: &Path) -> Vec<Sample> {
        Samples::open(root)
            .unwrap()
            .map(|r| r.unwrap())
            .collect::<Vec<_>>()
    }

    fn ids(root: &Path) -> Vec<String> {
        collect(root).into_iter().map(|s| s.id).collect()
    }

    fn first_error(root: &Path) -> SampleError {
        Samples::open(root)
            .unwrap()
            .find_map(|r| r.err())
            .expect("expected the walk to refuse")
    }

    #[test]
    fn a_well_formed_directory_yields_every_sample_exactly_once() {
        let dir = TempDir::new().unwrap();
        for i in 0..5 {
            pair(dir.path(), &format!("{i:06}"), "png", "");
        }
        assert_eq!(
            ids(dir.path()),
            ["000000", "000001", "000002", "000003", "000004"]
        );
    }

    #[test]
    fn a_name_that_sorts_between_a_pair_does_not_split_it() {
        // `000000.json` < `000000.m.n` < `000000.png` in plain name order, and
        // the middle name's stem is `000000.m`. Grouping on name order alone
        // would see a record, then a gap, then an image, and refuse a corpus
        // that is perfectly fine. Sorting on the stem first is what stops it.
        let dir = TempDir::new().unwrap();
        pair(dir.path(), "000000", "png", "");
        fs::write(dir.path().join("000000.m.n"), b"a stray sidecar").unwrap();
        assert_eq!(ids(dir.path()), ["000000"]);
    }

    #[test]
    fn an_upper_case_extension_is_the_same_extension() {
        // A case-insensitive filesystem hands back whatever case the file was
        // created with. Reading `.PNG` as an unknown format would turn the
        // record beside it into a half sample and refuse the corpus.
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("000000.PNG"), b"x").unwrap();
        fs::write(dir.path().join("000000.JSON"), record("")).unwrap();
        assert_eq!(ids(dir.path()), ["000000"]);
    }

    #[cfg(unix)]
    #[test]
    fn a_named_pipe_where_a_record_belongs_is_refused_rather_than_opened() {
        // Opening it would block until somebody wrote to it, and there is no
        // deadline on that wait.
        use std::os::unix::fs::FileTypeExt;
        let dir = TempDir::new().unwrap();
        let fifo = dir.path().join("000000.json");
        let status = std::process::Command::new("mkfifo").arg(&fifo).status();
        // Not every environment has mkfifo; the check is worth having where it
        // does rather than worth faking where it does not.
        if !status.map(|s| s.success()).unwrap_or(false) {
            return;
        }
        assert!(fs::symlink_metadata(&fifo).unwrap().file_type().is_fifo());
        fs::write(dir.path().join("000000.png"), b"x").unwrap();
        let err = first_error(dir.path());
        assert!(matches!(err, SampleError::NotARegularFile { .. }));
        assert!(err.to_string().contains("000000.json"));
    }

    #[test]
    fn the_order_is_sorted_rather_than_whatever_the_filesystem_returns() {
        // Filesystem order varies between machines, so a run resumed elsewhere
        // would key on a different item than the one it stopped at. Names are
        // created in an order chosen to be wrong to check the walk imposes its
        // own.
        let dir = TempDir::new().unwrap();
        for name in ["zulu", "alpha", "mike", "bravo"] {
            pair(dir.path(), name, "png", "");
        }
        assert_eq!(ids(dir.path()), ["alpha", "bravo", "mike", "zulu"]);
    }

    #[test]
    fn two_runs_over_the_same_directory_agree() {
        let dir = TempDir::new().unwrap();
        for i in 0..20 {
            pair(dir.path(), &format!("arm-{}/{i:06}", i % 3), "png", "");
        }
        assert_eq!(collect(dir.path()), collect(dir.path()));
    }

    #[test]
    fn nested_directories_are_walked_and_the_id_carries_the_path() {
        // The basename alone is not unique: every arm restarts at 000000, so an
        // id that dropped the directory would collide thirty-nine ways.
        let dir = TempDir::new().unwrap();
        pair(dir.path(), "wow-0200/000000", "png", "");
        pair(dir.path(), "mipod-0400/000000", "png", "");
        assert_eq!(ids(dir.path()), ["mipod-0400/000000", "wow-0200/000000"]);
    }

    #[test]
    fn an_id_does_not_change_when_the_corpus_moves() {
        // A resumable run keys on the id. If it contained the absolute path, a
        // corpus moved between the interruption and the resume would rescore
        // every item and report the work as new.
        let a = TempDir::new().unwrap();
        let b = TempDir::new().unwrap();
        pair(a.path(), "arm/000001", "png", "");
        pair(b.path(), "arm/000001", "png", "");
        assert_eq!(ids(a.path()), ids(b.path()));
        assert_eq!(ids(a.path()), ["arm/000001"]);
    }

    #[test]
    fn an_image_with_no_record_is_refused_and_names_the_basename() {
        // Skipping it would score fewer images than the corpus holds and still
        // exit zero, which is the failure this whole module exists to prevent.
        let dir = TempDir::new().unwrap();
        pair(dir.path(), "000000", "png", "");
        fs::write(dir.path().join("000001.png"), b"orphan").unwrap();
        let err = first_error(dir.path());
        assert!(matches!(err, SampleError::ImageWithoutRecord { .. }));
        assert!(err.to_string().contains("000001"));
    }

    #[test]
    fn a_record_with_no_image_is_refused_and_names_the_basename() {
        let dir = TempDir::new().unwrap();
        pair(dir.path(), "000000", "png", "");
        fs::write(dir.path().join("000001.json"), record("")).unwrap();
        let err = first_error(dir.path());
        assert!(matches!(err, SampleError::RecordWithoutImage { .. }));
        assert!(err.to_string().contains("000001"));
    }

    #[test]
    fn one_record_beside_two_images_is_refused_rather_than_guessed() {
        let dir = TempDir::new().unwrap();
        pair(dir.path(), "000000", "png", "");
        fs::write(dir.path().join("000000.jpg"), b"a second one").unwrap();
        let err = first_error(dir.path());
        assert!(matches!(err, SampleError::AmbiguousImage { .. }));
        assert!(err.to_string().contains("000000.jpg"));
    }

    #[test]
    fn a_walk_stops_at_the_first_corpus_defect_rather_than_carrying_on() {
        // Continuing past a defect produces a number measured on an unknown
        // subset, which is worse than no number.
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("000000.json"), record("")).unwrap();
        pair(dir.path(), "000001", "png", "");
        let got: Vec<_> = Samples::open(dir.path()).unwrap().collect();
        assert_eq!(got.len(), 1);
        assert!(got[0].is_err());
    }

    #[test]
    fn a_stego_record_reports_the_cover_it_was_made_from() {
        let dir = TempDir::new().unwrap();
        pair(
            dir.path(),
            "wow-0200/000000",
            "png",
            ", \"tool\": \"wow\", \"arm\": \"wow-0200\", \"source_png\": \"09710.png\"",
        );
        let got = collect(dir.path());
        assert_eq!(got[0].role, Role::Stego);
        assert_eq!(got[0].cover.as_deref(), Some("09710.png"));
    }

    #[test]
    fn the_digest_a_record_states_is_carried_through_exactly_as_written() {
        // A caller names a corpus by what its records declare, so the field is
        // reported rather than judged. A record with nothing to declare says
        // None, which is what stops a partly digested corpus being given a
        // digest that would name less than it appears to.
        let dir = TempDir::new().unwrap();
        pair(dir.path(), "a/000000", "png", "");
        assert_eq!(
            collect(dir.path())[0].digest.as_deref(),
            Some(&*"a".repeat(64))
        );

        let bare = TempDir::new().unwrap();
        fs::write(bare.path().join("b.png"), b"x").unwrap();
        fs::write(bare.path().join("b.json"), "{}").unwrap();
        assert_eq!(collect(bare.path())[0].digest, None);
    }

    #[test]
    fn a_record_reports_the_name_it_gives_its_own_image() {
        // The name on disk and the name in the record are different things on
        // a packed release, and a caller joining stego rows to covers needs the
        // second one. A path in the field is reduced to its last component,
        // because that is all a join ever uses and it cannot then reach out of
        // the corpus.
        let dir = TempDir::new().unwrap();
        pair(
            dir.path(),
            "a/000000",
            "png",
            ", \"file\": \"covers/09710.png\"",
        );
        assert_eq!(
            collect(dir.path())[0].declared_name.as_deref(),
            Some("09710.png")
        );

        let bare = TempDir::new().unwrap();
        fs::write(bare.path().join("b.png"), b"x").unwrap();
        fs::write(bare.path().join("b.json"), "{}").unwrap();
        assert_eq!(collect(bare.path())[0].declared_name, None);
    }

    #[test]
    fn a_file_field_that_is_not_a_name_at_all_is_refused() {
        let dir = TempDir::new().unwrap();
        pair(dir.path(), "a/000000", "png", ", \"file\": \"../..\"");
        let err = Samples::open(dir.path())
            .unwrap()
            .find_map(std::result::Result::err)
            .expect("refused");
        assert!(err.to_string().contains("plain file name"), "{err}");
    }

    #[test]
    fn a_clean_arm_row_reports_itself_as_clean() {
        let dir = TempDir::new().unwrap();
        pair(
            dir.path(),
            "clean-jpeg/000000",
            "jpg",
            ", \"role\": \"clean\", \"tool\": \"clean\", \"source_png\": \"09710.png\"",
        );
        let got = collect(dir.path());
        assert_eq!(got[0].role, Role::Clean);
    }

    #[test]
    fn a_cover_tier_row_is_clean_and_carries_its_split() {
        // A cover row has no arm and no tool; it is its own cover, and the
        // split lives here because it is a property of the cover.
        let dir = TempDir::new().unwrap();
        pair(
            dir.path(),
            "000000",
            "png",
            ", \"file\": \"00000.png\", \"split\": \"test\", \"tier_order\": 0",
        );
        let got = collect(dir.path());
        assert_eq!(got[0].role, Role::Clean);
        assert_eq!(got[0].split.as_deref(), Some("test"));
        assert_eq!(got[0].cover, None);
    }

    #[test]
    fn an_arm_row_with_no_role_field_still_reads_as_stego() {
        // `pack_arms.py` sets `role` on the clean arms it synthesises and
        // leaves it off the stego rows, so an absent role is not an absent side.
        let dir = TempDir::new().unwrap();
        pair(dir.path(), "000000", "png", ", \"arm\": \"wow-0200\"");
        assert_eq!(collect(dir.path())[0].role, Role::Stego);
    }

    #[test]
    fn a_role_that_is_neither_side_is_refused() {
        let dir = TempDir::new().unwrap();
        pair(dir.path(), "000000", "png", ", \"role\": \"maybe\"");
        let err = first_error(dir.path());
        assert!(err.to_string().contains("neither"));
    }

    #[test]
    fn a_record_that_is_not_an_object_is_refused() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("000000.png"), b"x").unwrap();
        fs::write(dir.path().join("000000.json"), b"[1, 2, 3]").unwrap();
        assert!(first_error(dir.path())
            .to_string()
            .contains("not a JSON object"));
    }

    #[test]
    fn a_record_that_is_not_json_is_refused_naming_the_file() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("000000.png"), b"x").unwrap();
        fs::write(dir.path().join("000000.json"), b"{not json").unwrap();
        let err = first_error(dir.path());
        assert!(matches!(err, SampleError::Parse { .. }));
        assert!(err.to_string().contains("000000.json"));
    }

    #[test]
    fn an_oversized_record_is_refused_before_it_is_parsed() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("000000.png"), b"x").unwrap();
        let padding = "x".repeat(MAX_RECORD_BYTES as usize);
        fs::write(
            dir.path().join("000000.json"),
            format!("{{\"note\": \"{padding}\"}}"),
        )
        .unwrap();
        let err = first_error(dir.path());
        assert!(matches!(err, SampleError::RecordTooLarge { .. }));
        assert!(err.to_string().contains("over the"));
    }

    #[test]
    fn a_path_in_source_png_is_refused_rather_than_joined() {
        // Nothing here joins it, but a caller reconciling pairs across a
        // release would, and the record arrived with somebody's download.
        let dir = TempDir::new().unwrap();
        pair(
            dir.path(),
            "000000",
            "png",
            ", \"source_png\": \"../../etc/passwd\"",
        );
        let err = first_error(dir.path());
        let text = err.to_string();
        assert!(text.contains("not a plain file name"), "got: {text}");
        assert!(text.contains("source_png"), "got: {text}");
    }

    /// The same check guards `file`, and the message has to name the field the
    /// reader has to go and fix. Saying `source_png` to somebody whose record
    /// carries a bad `file` sends them to a key that is perfectly fine.
    #[test]
    fn a_bad_file_field_is_refused_in_its_own_name() {
        let dir = TempDir::new().unwrap();
        pair(dir.path(), "000000", "png", ", \"file\": \"..\"");
        let text = first_error(dir.path()).to_string();
        assert!(text.contains("not a plain file name"), "got: {text}");
        assert!(
            text.contains("file is") && !text.contains("source_png"),
            "got: {text}"
        );
    }

    #[test]
    fn a_name_carrying_a_newline_is_refused() {
        // It would go into an item id and from there into line-oriented output,
        // where one record would read as two.
        let dir = TempDir::new().unwrap();
        pair(dir.path(), "000000", "png", "");
        if fs::write(dir.path().join("00\n01.png"), b"x").is_err() {
            // Some filesystems refuse the name outright, which is the same
            // outcome by a different route.
            return;
        }
        assert!(matches!(
            first_error(dir.path()),
            SampleError::NameHasControlCharacter { .. }
        ));
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_directory_is_refused_rather_than_followed() {
        // A link back to an ancestor is a loop, and a walk that followed one
        // would never return. Refusing also stops a link reaching bytes the
        // corpus does not name.
        let dir = TempDir::new().unwrap();
        pair(dir.path(), "000000", "png", "");
        std::os::unix::fs::symlink(dir.path(), dir.path().join("loop")).unwrap();
        let err = first_error(dir.path());
        assert!(matches!(err, SampleError::Symlink { .. }));
        assert!(err.to_string().contains("loop"));
    }

    #[test]
    fn a_directory_deeper_than_the_cap_is_refused_naming_the_path() {
        let dir = TempDir::new().unwrap();
        let mut deep = dir.path().to_path_buf();
        for i in 0..=MAX_DEPTH {
            deep = deep.join(format!("d{i}"));
        }
        fs::create_dir_all(&deep).unwrap();
        let err = first_error(dir.path());
        assert!(matches!(err, SampleError::TooDeep { .. }));
        assert!(err.to_string().contains(&format!("d{MAX_DEPTH}")));
    }

    #[test]
    fn a_file_that_is_neither_an_image_nor_a_record_is_not_a_sample() {
        // A shard holds nothing but samples; a directory on somebody's disk
        // holds a checksums file and a licence, and refusing those would refuse
        // a corpus that is perfectly fine.
        let dir = TempDir::new().unwrap();
        pair(dir.path(), "000000", "png", "");
        fs::write(dir.path().join("SHA256SUMS"), b"...").unwrap();
        fs::write(dir.path().join("LICENCE.txt"), b"...").unwrap();
        assert_eq!(collect(dir.path()).len(), 1);
    }

    #[test]
    fn an_operating_system_sidecar_is_skipped_the_way_the_shipped_loader_skips_it() {
        let dir = TempDir::new().unwrap();
        pair(dir.path(), "000000", "png", "");
        fs::write(dir.path().join(".DS_Store"), b"...").unwrap();
        assert_eq!(collect(dir.path()).len(), 1);
    }

    #[test]
    fn a_path_that_is_not_a_directory_is_refused_at_open() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("pentimento-core-00000.tar");
        fs::write(&file, b"...").unwrap();
        assert!(matches!(
            Samples::open(&file),
            Err(SampleError::NotADirectory { .. })
        ));
    }

    #[test]
    fn a_missing_directory_is_refused_at_open_with_the_underlying_reason() {
        let dir = TempDir::new().unwrap();
        let err = Samples::open(&dir.path().join("nope")).unwrap_err();
        assert!(matches!(err, SampleError::Read { .. }));
        assert!(err.to_string().contains("nope"));
    }

    #[test]
    fn an_empty_directory_yields_nothing_and_does_not_error() {
        let dir = TempDir::new().unwrap();
        assert!(collect(dir.path()).is_empty());
    }

    #[test]
    fn a_stem_is_everything_before_the_final_dot() {
        assert_eq!(stem_of("000000.png"), "000000");
        assert_eq!(stem_of("000000.tar.bin"), "000000.tar");
        assert_eq!(stem_of("README"), "README");
        assert_eq!(extension_of("000000.png"), Some("png"));
        assert_eq!(extension_of("README"), None);
    }

    #[test]
    fn a_stopped_walk_stays_stopped() {
        // An iterator that resumed after an error would let a caller ignoring
        // the error still collect a partial corpus and score it.
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("000000.json"), record("")).unwrap();
        pair(dir.path(), "000001", "png", "");
        let mut walk = Samples::open(dir.path()).unwrap();
        assert!(walk.next().unwrap().is_err());
        assert!(walk.next().is_none());
    }
}

/// The starter corpus this repository ships, checked as the corpus it claims
/// to be.
///
/// It exists so a fresh install can score something, which means a first run
/// takes exactly the paths a real run takes: the walk, the cover join, the
/// split inheritance and the pairing comparison. If any of those were special
/// cased here, the first thing a new user saw working would be the one thing
/// that does not work on a real corpus.
#[cfg(test)]
mod starter_corpus_tests {
    use super::*;
    use std::collections::HashMap;

    fn root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpora/starter")
    }

    fn walk() -> Vec<Sample> {
        Samples::open(&root())
            .unwrap_or_else(|e| panic!("the shipped starter corpus does not open: {e}"))
            .map(|s| s.unwrap_or_else(|e| panic!("the shipped starter corpus is defective: {e}")))
            .collect()
    }

    #[test]
    fn it_walks_cleanly_and_has_both_sides_of_a_measurement() {
        let samples = walk();
        let stego = samples.iter().filter(|s| s.role == Role::Stego).count();
        let clean = samples.len() - stego;
        assert_eq!((clean, stego), (6, 12));
    }

    #[test]
    fn every_stego_image_names_a_cover_that_is_actually_here() {
        let samples = walk();
        let covers: HashMap<&str, &Sample> = samples
            .iter()
            .filter(|s| s.role == Role::Clean)
            .filter_map(|s| s.declared_name.as_deref().map(|n| (n, s)))
            .collect();
        for s in samples.iter().filter(|s| s.role == Role::Stego) {
            let name = s
                .cover
                .as_deref()
                .unwrap_or_else(|| panic!("{} names no cover", s.id));
            assert!(
                covers.contains_key(name),
                "{} descends from {name}, which is not in this corpus",
                s.id
            );
        }
    }

    /// The join a real release takes, rather than the degenerate one.
    ///
    /// A packed tier renames every member to its position, so a stego row
    /// reaches its cover by the name the cover's RECORD gives it and not by the
    /// name on disk. A starter corpus whose two names agreed would exercise a
    /// path no real corpus uses, and the first thing to break on real data
    /// would be the thing the demonstration proved worked.
    #[test]
    fn the_cover_join_goes_through_the_declared_name_not_the_name_on_disk() {
        for s in walk().iter().filter(|s| s.role == Role::Clean) {
            let declared = s.declared_name.as_deref().expect("a cover with no name");
            let on_disk = s.image.file_name().and_then(|n| n.to_str()).unwrap();
            assert_ne!(declared, on_disk, "{} takes the easy join", s.id);
        }
    }

    #[test]
    fn the_split_is_a_property_of_the_cover_and_has_both_sides() {
        let samples = walk();
        let mut sides: Vec<&str> = samples
            .iter()
            .filter(|s| s.role == Role::Clean)
            .filter_map(|s| s.split.as_deref())
            .collect();
        sides.sort_unstable();
        sides.dedup();
        assert_eq!(sides, vec!["test", "train"]);
        // No stego row states one, so none can contradict its cover. That is
        // the corpus getting split discipline right by construction rather
        // than by agreeing with itself.
        assert!(samples
            .iter()
            .filter(|s| s.role == Role::Stego)
            .all(|s| s.split.is_none()));
    }

    /// The reachable half of the pairing rule, run against the shipped files.
    #[test]
    fn a_stego_image_differs_from_its_cover_in_nothing_a_header_can_see() {
        let samples = walk();
        let shapes: HashMap<&str, crate::header::Shape> = samples
            .iter()
            .filter(|s| s.role == Role::Clean)
            .filter_map(|s| {
                let name = s.declared_name.as_deref()?;
                Some((name, crate::header::read(&s.image).ok()?))
            })
            .collect();
        let mut compared = 0;
        for s in samples.iter().filter(|s| s.role == Role::Stego) {
            let cover = shapes[s.cover.as_deref().unwrap()];
            let stego = crate::header::read(&s.image).expect("a stego image that cannot be read");
            assert_eq!(
                stego, cover,
                "{} differs from its cover in its header",
                s.id
            );
            compared += 1;
        }
        assert_eq!(compared, 12, "the pairing check looked at nothing");
    }

    /// The guarantee that stops a number from here being quoted.
    #[test]
    fn its_registry_entry_can_never_earn_a_named_run() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/registry");
        let reg = crate::registry::Registry::load(&dir).expect("the shipped registry loads");
        let entry = reg
            .corpora
            .get("stegobench-starter")
            .expect("the starter corpus is not registered");
        assert!(entry.demonstration);
        // `score` marks a run `named` only when this digest is declared AND
        // matches, so a demonstration entry forbidden from declaring one can
        // only ever produce `custom`.
        assert!(entry
            .integrity
            .as_ref()
            .and_then(|i| i.records_sha256.as_deref())
            .is_none());
        assert!(entry.download.is_empty());
        assert_eq!(entry.validate(), Ok(()));
    }
}
