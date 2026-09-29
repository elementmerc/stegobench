// Author:  Daniel Iwugo
// Comment: Christ is King
// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Daniel Iwugo

//! Column alignment for the one-line listings `stegobench list` prints.
//!
//! WHY THIS EXISTS RATHER THAN A FORMAT WIDTH
//!
//! The listings used to pad with a literal, `{:<16}`, and a literal is a guess
//! about the longest name anybody will ever register. `stegobench-starter` is
//! eighteen characters, so the day it was added its row's columns stopped
//! lining up with every other row's and nothing failed. A width taken from the
//! rows actually being printed cannot go stale that way: a registry entry with
//! a longer name widens the column for everybody instead of overflowing it.
//!
//! The last cell in a row is never padded. It is the end of the line, so
//! padding it only writes trailing spaces, which some terminals show as a
//! selection and every diff shows as noise.

/// Pads each row's cells to the widest cell in its column and joins them with
/// a single space.
///
/// Rows may be ragged. A row with fewer cells than another simply ends early,
/// and its last cell is unpadded like any other last cell.
///
/// Width is counted in `char`s rather than bytes, so a name outside ASCII is
/// padded by what a reader sees rather than by how UTF-8 spells it. That is
/// still not column width for a wide or combining glyph, and no registry entry
/// has one; a name that did would need a width crate, which this deliberately
/// does not pull in for a listing.
pub fn align(rows: &[Vec<String>]) -> Vec<String> {
    let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
    let mut widths = vec![0usize; columns];
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(cell.chars().count());
        }
    }
    rows.iter()
        .map(|row| {
            let mut line = String::new();
            for (i, cell) in row.iter().enumerate() {
                if i > 0 {
                    line.push(' ');
                }
                line.push_str(cell);
                if i + 1 < row.len() {
                    let pad = widths[i].saturating_sub(cell.chars().count());
                    line.push_str(&" ".repeat(pad));
                }
            }
            line
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(rows: &[&[&str]]) -> Vec<Vec<String>> {
        rows.iter()
            .map(|r| r.iter().map(|c| (*c).to_string()).collect())
            .collect()
    }

    /// The bug this exists for: one long name used to overflow a fixed pad and
    /// leave its own row's later columns out of step with every other row's.
    #[test]
    fn a_long_cell_widens_the_column_for_every_row() {
        let out = align(&rows(&[
            &["short", "1", "x"],
            &["a-very-long-name-indeed", "2", "y"],
            &["mid", "3", "z"],
        ]));
        // Where each row's second column begins: past the name, past its
        // padding, at the first character that is not a space.
        let starts: Vec<usize> = out
            .iter()
            .map(|line| {
                line.find(|c: char| c.is_ascii_digit())
                    .expect("a second cell")
            })
            .collect();
        assert_eq!(starts, vec![starts[0]; 3], "{out:#?}");
        assert_eq!(starts[0], "a-very-long-name-indeed".len() + 1, "{out:#?}");
        // And the third column lines up too, which a fixed pad also got wrong.
        for line in &out {
            assert_eq!(line.len(), starts[0] + 3, "{line:?}");
        }
    }

    #[test]
    fn the_last_cell_is_never_padded() {
        let out = align(&rows(&[&["a", "long-tail"], &["b", "t"]]));
        assert_eq!(out[1], "b t");
        assert!(!out[0].ends_with(' '), "{:?}", out[0]);
    }

    #[test]
    fn a_ragged_row_ends_early_rather_than_padding_to_nothing() {
        let out = align(&rows(&[&["a", "bb", "cc"], &["ddd"]]));
        assert_eq!(out[1], "ddd");
        assert!(out[0].starts_with("a   bb"), "{:?}", out[0]);
    }

    #[test]
    fn nothing_to_align_is_nothing_rather_than_a_blank_line() {
        assert!(align(&[]).is_empty());
    }

    /// Padded by what a reader sees. A name spelled in multi-byte UTF-8 would
    /// otherwise be padded by its byte length and print short.
    #[test]
    fn width_is_counted_in_characters_rather_than_bytes() {
        let out = align(&rows(&[&["éé", "x"], &["ab", "y"]]));
        assert_eq!(out[0], "éé x");
        assert_eq!(out[1], "ab y");
    }
}
