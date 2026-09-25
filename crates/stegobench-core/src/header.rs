// Author:  Daniel Iwugo
// Comment: Christ is King
// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Daniel Iwugo
//! Reading what an image says about itself, from its first few bytes.
//!
//! WHY A BENCHMARK HARNESS PARSES IMAGE HEADERS
//! --------------------------------------------
//! The pairing rule is the method: a clean image and its stego twin must differ
//! in nothing but the payload. Two measurement rounds have already been lost to
//! breaking it, and the harness has so far *declared* the rule held rather than
//! checked it, which is the same posture this project exists to correct in
//! other people's results.
//!
//! Two files on disk cannot prove the rule. Proving it would mean owning a
//! decoder for every format, decoding both images and diffing the pixel arrays,
//! and even that would not catch a cover that was re-encoded before the payload
//! went in. What the first few bytes CAN prove is the opposite: if a stego image
//! and the cover it names differ in width, height, format, bit depth or channel
//! count, then something other than the payload changed, and the arm is
//! confounded whatever its metadata claims.
//!
//! A cheap check that can only return "definitely broken" or "nothing visible
//! here" is worth having, as long as the second answer is never dressed up as
//! the first. That is why [`crate::result::Pairing`] carries a third state.
//!
//! WHY NOT AN IMAGE CRATE
//! ----------------------
//! `stegobench-core` parses formats other people are meant to adopt, and its
//! dependency set is small on purpose: a number somebody has to defend should
//! not rest on a graph they have to audit first. A full decoder would pull in
//! several crates to read five fields that sit at a fixed offset in one case
//! and behind a short marker walk in the other. The parsing here is under two
//! hundred lines, reads a bounded prefix, never allocates per pixel, and cannot
//! decompress anything, which also means it cannot be made to decompress
//! something enormous.

use std::io::Read;
use std::path::Path;

/// The largest prefix of a file this will read before giving up.
///
/// PNG answers in 33 bytes. JPEG answers after however many metadata segments
/// the camera or the editor wrote first, and a colour profile or a thumbnail
/// can be tens of kilobytes. 64 KiB clears every real file seen so far and
/// still refuses to walk a file that is secretly a gigabyte of marker segments.
pub const MAX_HEADER_BYTES: usize = 64 * 1024;

/// How many JPEG marker segments the walk will step over before giving up.
///
/// A photograph out of a camera carries perhaps a dozen: JFIF, Exif, a colour
/// profile, quantisation and Huffman tables. 256 is two orders of magnitude of
/// headroom, and it is a separate cap from [`MAX_HEADER_BYTES`] because a file
/// of a hundred thousand empty segments is small and would otherwise be walked
/// in full.
pub const MAX_SEGMENTS: usize = 256;

/// What kind of file this is, according to its own first bytes rather than its
/// name.
///
/// An extension is a claim by whoever wrote the file; the magic number is what
/// a decoder will actually act on. They disagree more often than they should,
/// and a corpus where the cover is a renamed JPEG is confounded in a way no
/// amount of reading file names would show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Png,
    Jpeg,
    Gif,
    Bmp,
    Tiff,
    Webp,
    /// Netpbm: PGM, PPM, PBM and the rest of the family.
    Netpbm,
}

impl Format {
    /// The name a person would use for it.
    pub fn name(self) -> &'static str {
        match self {
            Format::Png => "PNG",
            Format::Jpeg => "JPEG",
            Format::Gif => "GIF",
            Format::Bmp => "BMP",
            Format::Tiff => "TIFF",
            Format::Webp => "WebP",
            Format::Netpbm => "Netpbm",
        }
    }
}

/// The measurements that can be compared between two images.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Geometry {
    pub width: u32,
    pub height: u32,
    /// Bits per sample. 8 for an ordinary photograph, 16 for a deep PNG.
    pub depth: u8,
    /// Samples per pixel: 1 greyscale, 3 colour, 4 with an alpha channel.
    pub channels: u8,
}

/// What a file's header says about it.
///
/// [`Shape::geometry`] is `None` for a format this module recognises but does
/// not measure. That is a deliberate third answer rather than a zero: "this is
/// a GIF and I did not read its size" and "this is a GIF of no size" are
/// different statements, and only one of them is true.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shape {
    pub format: Format,
    pub geometry: Option<Geometry>,
}

/// Why a header could not be read.
#[derive(Debug, thiserror::Error)]
pub enum HeaderError {
    #[error("cannot read {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "{path} does not begin like any image format this recognises. Either it \
         is not an image, or it is one nothing here can read"
    )]
    Unrecognised { path: String },
    #[error("{path} claims to be {format} and its header does not hold up: {reason}")]
    Malformed {
        path: String,
        format: &'static str,
        reason: String,
    },
}

/// How much of a file is read before anything else is tried.
///
/// PNG answers in 33 bytes and an ordinary JPEG within the first kilobyte or
/// two, so this settles almost every file in one read. It matters because the
/// caller runs this over every image in a corpus, and a Core tier is 344,357 of
/// them: reading [`MAX_HEADER_BYTES`] from each unconditionally would move
/// twenty gigabytes to answer a question that usually fits in a disk block.
const FIRST_CHUNK: usize = 4 * 1024;

/// Reads the header of the image at `path`.
///
/// Reads a bounded prefix and stops. Nothing here decompresses, so a file
/// cannot cost more than [`MAX_HEADER_BYTES`] of memory however large or
/// however hostile it is.
pub fn read(path: &Path) -> Result<Shape, HeaderError> {
    let file = std::fs::File::open(path).map_err(|e| HeaderError::Read {
        path: path.display().to_string(),
        source: e,
    })?;
    let mut buf = Vec::new();
    let mut taken = prefix(&file, &mut buf, FIRST_CHUNK, path)?;
    let shape = parse(&buf, path)?;

    // The one case worth a second read: a JPEG whose frame header sits behind
    // more metadata than the first chunk held. Anything else has already given
    // its answer, and re-reading would cost the corpus a great deal to learn
    // nothing.
    if shape.geometry.is_none() && shape.format == Format::Jpeg && taken == FIRST_CHUNK {
        taken = prefix(&file, &mut buf, MAX_HEADER_BYTES, path)?;
        debug_assert!(taken <= MAX_HEADER_BYTES);
        return parse(&buf, path);
    }
    Ok(shape)
}

/// Reads the first `want` bytes of `file` into `buf`, replacing what is there.
fn prefix(
    file: &std::fs::File,
    buf: &mut Vec<u8>,
    want: usize,
    path: &Path,
) -> Result<usize, HeaderError> {
    use std::io::Seek;
    buf.clear();
    let mut handle = file;
    handle.rewind().map_err(|e| HeaderError::Read {
        path: path.display().to_string(),
        source: e,
    })?;
    handle
        .take(want as u64)
        .read_to_end(buf)
        .map_err(|e| HeaderError::Read {
            path: path.display().to_string(),
            source: e,
        })?;
    Ok(buf.len())
}

/// The same thing over bytes already in hand, which is what the tests use.
pub fn parse(bytes: &[u8], path: &Path) -> Result<Shape, HeaderError> {
    let name = || path.display().to_string();

    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Ok(Shape {
            format: Format::Png,
            geometry: png(bytes, path)?,
        });
    }
    if bytes.starts_with(&[0xFF, 0xD8]) {
        return Ok(Shape {
            format: Format::Jpeg,
            geometry: jpeg(bytes, path)?,
        });
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Ok(Shape {
            format: Format::Gif,
            geometry: None,
        });
    }
    if bytes.starts_with(b"BM") {
        return Ok(Shape {
            format: Format::Bmp,
            geometry: None,
        });
    }
    if bytes.starts_with(b"II\x2a\x00") || bytes.starts_with(b"MM\x00\x2a") {
        return Ok(Shape {
            format: Format::Tiff,
            geometry: None,
        });
    }
    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return Ok(Shape {
            format: Format::Webp,
            geometry: None,
        });
    }
    if bytes.len() >= 2 && bytes[0] == b'P' && (b'1'..=b'7').contains(&bytes[1]) {
        return Ok(Shape {
            format: Format::Netpbm,
            geometry: None,
        });
    }
    Err(HeaderError::Unrecognised { path: name() })
}

/// PNG puts everything wanted here in its first chunk, at a fixed offset.
///
/// The specification requires IHDR to be first, so there is no chunk walk: a
/// file that puts something else there is malformed and is reported as such
/// rather than searched for a chunk that should not be where it is.
fn png(bytes: &[u8], path: &Path) -> Result<Option<Geometry>, HeaderError> {
    let bad = |reason: String| HeaderError::Malformed {
        path: path.display().to_string(),
        format: "PNG",
        reason,
    };
    // 8 magic, 4 length, 4 type, then 13 bytes of IHDR.
    if bytes.len() < 33 {
        return Err(bad(format!(
            "the file is {} bytes and a PNG header is 33",
            bytes.len()
        )));
    }
    if &bytes[12..16] != b"IHDR" {
        return Err(bad(
            "the first chunk is not IHDR, which the format requires".into(),
        ));
    }
    let width = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    let depth = bytes[24];
    let colour = bytes[25];
    if width == 0 || height == 0 {
        return Err(bad(format!("it declares a size of {width} by {height}")));
    }
    let channels = match colour {
        0 => 1, // greyscale
        2 => 3, // truecolour
        3 => 1, // palette index
        4 => 2, // greyscale with alpha
        6 => 4, // truecolour with alpha
        other => {
            return Err(bad(format!(
                "colour type {other} is not one of 0, 2, 3, 4 or 6"
            )))
        }
    };
    Ok(Some(Geometry {
        width,
        height,
        depth,
        channels,
    }))
}

/// JPEG states its size in a frame header that sits behind however much
/// metadata the writer put first, so this walks segments until it finds one.
fn jpeg(bytes: &[u8], path: &Path) -> Result<Option<Geometry>, HeaderError> {
    let bad = |reason: String| HeaderError::Malformed {
        path: path.display().to_string(),
        format: "JPEG",
        reason,
    };
    let mut i = 2;
    for _ in 0..MAX_SEGMENTS {
        // Any number of 0xFF bytes may pad the gap before a marker.
        while bytes.get(i) == Some(&0xFF) {
            i += 1;
        }
        let Some(&marker) = bytes.get(i) else {
            // Ran off the end of the prefix rather than off the end of a
            // malformed file. Not being able to see the frame header is not
            // evidence that there isn't one.
            return Ok(None);
        };
        i += 1;

        match marker {
            // Standalone markers: no length, nothing to step over.
            0x01 | 0xD0..=0xD7 => continue,
            // Start of scan. Everything after this is entropy-coded data, and
            // a file that reaches here without a frame header is malformed.
            0xDA => {
                return Err(bad(
                    "the compressed data starts before any frame header, so the \
                     file never says how big the image is"
                        .into(),
                ))
            }
            0xD9 => {
                return Err(bad(
                    "the file ends before it says how big the image is".into()
                ))
            }
            _ => {}
        }

        let Some(length) = bytes
            .get(i..i + 2)
            .map(|l| u16::from_be_bytes([l[0], l[1]]) as usize)
        else {
            return Ok(None);
        };
        if length < 2 {
            return Err(bad(format!(
                "a segment declares a length of {length}, which is shorter than \
                 the length field itself"
            )));
        }

        // The frame headers. 0xC4, 0xC8 and 0xCC share the range and are not
        // frames: they carry Huffman tables, a reserved extension and
        // arithmetic coding conditioning.
        let is_frame = matches!(marker, 0xC0..=0xCF) && !matches!(marker, 0xC4 | 0xC8 | 0xCC);
        if is_frame {
            let Some(body) = bytes.get(i + 2..i + 8) else {
                return Ok(None);
            };
            let depth = body[0];
            let height = u16::from_be_bytes([body[1], body[2]]) as u32;
            let width = u16::from_be_bytes([body[3], body[4]]) as u32;
            let channels = body[5];
            // A height of zero is legal in one narrow case, where the real
            // height arrives later in a DNL segment. It is vanishingly rare,
            // this does not read DNL, and reporting zero would make two such
            // files compare equal whatever their real sizes.
            if width == 0 || height == 0 {
                return Ok(None);
            }
            return Ok(Some(Geometry {
                width,
                height,
                depth,
                channels,
            }));
        }
        i += length;
    }
    Err(bad(format!(
        "more than {MAX_SEGMENTS} metadata segments came before any frame \
         header, which no real image does"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(name: &str) -> std::path::PathBuf {
        std::path::PathBuf::from(name)
    }

    /// A PNG header with the fields this cares about, and nothing else.
    fn png_bytes(width: u32, height: u32, depth: u8, colour: u8) -> Vec<u8> {
        let mut v = b"\x89PNG\r\n\x1a\n".to_vec();
        v.extend_from_slice(&13u32.to_be_bytes());
        v.extend_from_slice(b"IHDR");
        v.extend_from_slice(&width.to_be_bytes());
        v.extend_from_slice(&height.to_be_bytes());
        v.push(depth);
        v.push(colour);
        v.extend_from_slice(&[0, 0, 0]); // compression, filter, interlace
        v.extend_from_slice(&[0, 0, 0, 0]); // CRC
        v
    }

    /// A JPEG with `padding` bytes of metadata segments before the frame.
    fn jpeg_bytes(width: u16, height: u16, channels: u8, segments: usize) -> Vec<u8> {
        let mut v = vec![0xFF, 0xD8];
        for _ in 0..segments {
            v.extend_from_slice(&[0xFF, 0xE0]);
            v.extend_from_slice(&66u16.to_be_bytes());
            v.extend_from_slice(&[b'x'; 64]);
        }
        v.extend_from_slice(&[0xFF, 0xC0]);
        v.extend_from_slice(&(8 + 3 * channels as u16).to_be_bytes());
        v.push(8);
        v.extend_from_slice(&height.to_be_bytes());
        v.extend_from_slice(&width.to_be_bytes());
        v.push(channels);
        for c in 0..channels {
            v.extend_from_slice(&[c + 1, 0x11, 0]);
        }
        v
    }

    #[test]
    fn a_png_header_reads_as_what_it_says() {
        let shape = parse(&png_bytes(512, 512, 8, 2), &at("a.png")).expect("reads");
        assert_eq!(shape.format, Format::Png);
        assert_eq!(
            shape.geometry,
            Some(Geometry {
                width: 512,
                height: 512,
                depth: 8,
                channels: 3
            })
        );
    }

    #[test]
    fn every_png_colour_type_maps_to_its_channel_count() {
        for (colour, channels) in [(0, 1), (2, 3), (3, 1), (4, 2), (6, 4)] {
            let shape = parse(&png_bytes(8, 8, 8, colour), &at("a.png")).expect("reads");
            assert_eq!(
                shape.geometry.unwrap().channels,
                channels,
                "colour {colour}"
            );
        }
        let err = parse(&png_bytes(8, 8, 8, 5), &at("a.png")).expect_err("refused");
        assert!(err.to_string().contains("colour type 5"), "{err}");
    }

    #[test]
    fn a_jpeg_frame_header_is_found_behind_its_metadata() {
        // The whole reason this walks segments rather than reading an offset:
        // a photograph carries Exif, a colour profile and quantisation tables
        // before it ever says how big it is.
        let shape = parse(&jpeg_bytes(640, 480, 3, 12), &at("a.jpg")).expect("reads");
        assert_eq!(shape.format, Format::Jpeg);
        let g = shape.geometry.expect("measured");
        assert_eq!((g.width, g.height, g.channels), (640, 480, 3));
    }

    #[test]
    fn the_height_and_width_are_not_swapped() {
        // JPEG writes height first and PNG writes width first, which is the
        // kind of thing that reads correctly and is wrong on every non-square
        // image in the corpus.
        let g = parse(&jpeg_bytes(100, 200, 3, 0), &at("a.jpg"))
            .expect("reads")
            .geometry
            .expect("measured");
        assert_eq!((g.width, g.height), (100, 200));
        let g = parse(&png_bytes(100, 200, 8, 2), &at("a.png"))
            .expect("reads")
            .geometry
            .expect("measured");
        assert_eq!((g.width, g.height), (100, 200));
    }

    #[test]
    fn a_progressive_jpeg_is_read_like_any_other() {
        // 0xC2 rather than 0xC0. Progressive files are ordinary on the web and
        // reading only the baseline marker would silently measure nothing.
        let mut bytes = jpeg_bytes(64, 64, 3, 1);
        let i = bytes.windows(2).position(|w| w == [0xFF, 0xC0]).unwrap();
        bytes[i + 1] = 0xC2;
        let g = parse(&bytes, &at("a.jpg"))
            .expect("reads")
            .geometry
            .expect("measured");
        assert_eq!((g.width, g.height), (64, 64));
    }

    #[test]
    fn a_huffman_table_is_not_mistaken_for_a_frame_header() {
        // 0xC4 sits in the middle of the frame marker range and carries
        // Huffman tables. Reading it as a frame gives an image whose size is
        // whatever the table happened to contain.
        let mut bytes = vec![0xFF, 0xD8, 0xFF, 0xC4];
        bytes.extend_from_slice(&20u16.to_be_bytes());
        bytes.extend_from_slice(&[0x11; 18]);
        bytes.extend_from_slice(&jpeg_bytes(320, 240, 1, 0)[2..]);
        let g = parse(&bytes, &at("a.jpg"))
            .expect("reads")
            .geometry
            .expect("measured");
        assert_eq!((g.width, g.height, g.channels), (320, 240, 1));
    }

    #[test]
    fn a_jpeg_that_starts_its_scan_without_a_frame_is_refused() {
        let bytes = vec![0xFF, 0xD8, 0xFF, 0xDA, 0x00, 0x02];
        let err = parse(&bytes, &at("a.jpg")).expect_err("refused");
        assert!(err.to_string().contains("never says how big"), "{err}");
    }

    #[test]
    fn a_file_of_endless_empty_segments_is_bounded_rather_than_walked() {
        // The adversarial input. Every segment is legal and there are more of
        // them than any real file has.
        let mut bytes = vec![0xFF, 0xD8];
        for _ in 0..(MAX_SEGMENTS + 10) {
            bytes.extend_from_slice(&[0xFF, 0xE0, 0x00, 0x02]);
        }
        let err = parse(&bytes, &at("a.jpg")).expect_err("refused");
        assert!(err.to_string().contains("metadata segments"), "{err}");
    }

    #[test]
    fn a_segment_shorter_than_its_own_length_field_cannot_walk_backwards() {
        // A length of zero would step the cursor back two bytes every time and
        // loop forever on a four byte file.
        let bytes = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x00, 0x00, 0x00];
        let err = parse(&bytes, &at("a.jpg")).expect_err("refused");
        assert!(err.to_string().contains("shorter than"), "{err}");
    }

    #[test]
    fn a_truncated_prefix_says_nothing_rather_than_guessing() {
        // Not knowing is a third answer. A JPEG whose frame header sits past
        // the byte cap must not read as a JPEG of no particular size.
        let bytes = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
        let shape = parse(&bytes, &at("a.jpg")).expect("recognised");
        assert_eq!(shape.format, Format::Jpeg);
        assert_eq!(shape.geometry, None);
    }

    #[test]
    fn a_png_too_short_to_hold_a_header_is_refused_rather_than_measured() {
        let err = parse(b"\x89PNG\r\n\x1a\n", &at("a.png")).expect_err("refused");
        assert!(err.to_string().contains("33"), "{err}");
    }

    #[test]
    fn the_other_formats_are_named_without_being_measured() {
        for (bytes, format) in [
            (b"GIF89a".to_vec(), Format::Gif),
            (b"BM\x00\x00".to_vec(), Format::Bmp),
            (b"II\x2a\x00".to_vec(), Format::Tiff),
            (b"RIFF\x00\x00\x00\x00WEBP".to_vec(), Format::Webp),
            (b"P6\n8 8\n255\n".to_vec(), Format::Netpbm),
        ] {
            let shape = parse(&bytes, &at("a")).expect("recognised");
            assert_eq!(shape.format, format);
            assert_eq!(shape.geometry, None, "{format:?}");
        }
    }

    #[test]
    fn something_that_is_not_an_image_is_refused() {
        let err = parse(b"<!DOCTYPE html>", &at("a.png")).expect_err("refused");
        assert!(err.to_string().contains("not an image"), "{err}");
    }

    #[test]
    fn an_empty_file_is_refused_rather_than_panicking() {
        parse(b"", &at("a.png")).expect_err("refused");
    }

    #[test]
    fn reading_from_disk_agrees_with_reading_from_memory() {
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("a.png");
        let bytes = png_bytes(32, 48, 16, 6);
        std::fs::write(&path, &bytes).unwrap();
        assert_eq!(read(&path).unwrap(), parse(&bytes, &path).unwrap());
    }

    #[test]
    fn a_jpeg_whose_frame_sits_past_the_first_read_is_still_measured() {
        // The staged read exists so a corpus is not dragged through 64 KiB per
        // image. It would be a bad trade if it also stopped measuring the
        // photographs that carry a colour profile or a thumbnail first.
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("big.jpg");
        let bytes = jpeg_bytes(800, 600, 3, 100);
        assert!(bytes.len() > FIRST_CHUNK, "{} bytes", bytes.len());
        std::fs::write(&path, &bytes).unwrap();
        let g = read(&path).expect("reads").geometry.expect("measured");
        assert_eq!((g.width, g.height), (800, 600));
    }

    #[test]
    fn a_missing_file_reports_the_path() {
        let err = read(&at("/nonexistent/a.png")).expect_err("refused");
        assert!(err.to_string().contains("/nonexistent/a.png"), "{err}");
    }
}
