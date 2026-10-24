// UNICODE LICENSE V3
//
// COPYRIGHT AND PERMISSION NOTICE
//
// Copyright © 2020-2024 Unicode, Inc.
//
// NOTICE TO USER: Carefully read the following legal agreement. BY
// DOWNLOADING, INSTALLING, COPYING OR OTHERWISE USING DATA FILES, AND/OR
// SOFTWARE, YOU UNEQUIVOCALLY ACCEPT, AND AGREE TO BE BOUND BY, ALL OF THE
// TERMS AND CONDITIONS OF THIS AGREEMENT. IF YOU DO NOT AGREE, DO NOT
// DOWNLOAD, INSTALL, COPY, DISTRIBUTE OR USE THE DATA FILES OR SOFTWARE.
//
// Permission is hereby granted, free of charge, to any person obtaining a
// copy of data files and any associated documentation (the "Data Files") or
// software and any associated documentation (the "Software") to deal in the
// Data Files or Software without restriction, including without limitation
// the rights to use, copy, modify, merge, publish, distribute, and/or sell
// copies of the Data Files or Software, and to permit persons to whom the
// Data Files or Software are furnished to do so, provided that either (a)
// this copyright and permission notice appear with all copies of the Data
// Files or Software, or (b) this copyright and permission notice appear in
// associated Documentation.
//
// THE DATA FILES AND SOFTWARE ARE PROVIDED "AS IS", WITHOUT WARRANTY OF ANY
// KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
// MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT OF
// THIRD PARTY RIGHTS.
//
// IN NO EVENT SHALL THE COPYRIGHT HOLDER OR HOLDERS INCLUDED IN THIS NOTICE
// BE LIABLE FOR ANY CLAIM, OR ANY SPECIAL INDIRECT OR CONSEQUENTIAL DAMAGES,
// OR ANY DAMAGES WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS,
// WHETHER IN AN ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION,
// ARISING OUT OF OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THE DATA
// FILES OR SOFTWARE.
//
// Except as contained in this notice, the name of a copyright holder shall
// not be used in advertising or otherwise to promote the sale, use or other
// dealings in these Data Files or Software without prior written
// authorization of the copyright holder.
//
// SPDX-License-Identifier: Unicode-3.0

//! Canonical decomposition and combining classes for collation.
//!
//! The collation key (`fold_key`, `compare_text`) must give the same order to a letter typed
//! precomposed (`ế`, U+1EBF) and typed as base + marks (`e` U+0302 U+0301), and to marks of
//! different combining classes typed in either order (`a` U+0302 U+0323 and `a` U+0323 U+0302).
//! Only the Latin letters U+00C0..=U+024F and U+1E00..=U+1EFF and the marks U+0300..=U+036F are
//! covered. [`canonical`] turns text into the one form the key builder reads: fully decomposed
//! (canonical decomposition, flattened) and with each run of combining marks put in canonical
//! order (UAX #15 canonical ordering).
//!
//! The tables cover the Latin letters in U+00C0..=U+024F and U+1E00..=U+1EFF (497 characters
//! that decompose) and the 111 combining marks of U+0300..=U+036F that have a nonzero combining
//! class, four of which decompose to other marks (U+0340 and U+0341, the grave and acute tone
//! marks, are U+0300 and U+0301; U+0343 is U+0313; U+0344 is U+0308 U+0301). Anything outside them is left alone: it has no canonical decomposition here and a
//! combining class of 0, so it is a starter. This is not a general NFD; it is the part of NFD
//! that Vietnamese and the Latin scripts GridCraft sorts need.
//!
//! The tables are generated from the Unicode Character Database 17.0 (via ICU4X
//! `icu_normalizer` =2.3.0, data generated with CLDR 48.2.1 / ICU 78.1rc, the `icuexport` tag in
//! `icu_normalizer_data` 2.3.0) and verified against it by the test `tables_match_icu_normalizer`.
//! Canonical decompositions are frozen by Unicode's normalization stability policy, so later
//! Unicode versions do not change them.
//!
//! Attribution: Unicode Character Database 17.0 data as packaged in ICU4X `icu_normalizer_data`
//! 2.3.0, Unicode-3.0, Copyright © 2020-2024 Unicode, Inc. The licence notice at the top of this
//! file is that package's notice.
//!
//! Nothing here panics: lookups use `get` and binary search, and the work is linear in the input
//! except for the sort of a mark run, which is capped at 32 marks.

use std::cmp::Ordering;

/// Longest run of combining marks that is reordered. A longer run is returned in input order:
/// it is not text, and sorting it would let one hostile string cost quadratic time.
const MAX_RUN: usize = 32;

/// First scalar of the first range in [`DECOMPOSITION`].
const DEC_FIRST: u32 = 0xc0;

/// Canonical (full, flattened) decomposition of each character in U+00C0..=U+024F, U+0300..=U+036F
/// and U+1E00..=U+1EFF that has one, sorted by code point. 501 entries: 497 letters and the four
/// marks U+0340, U+0341, U+0343 and U+0344, which decompose to other marks.
static DECOMPOSITION: &[(char, &str)] = &[
    ('\u{c0}', "A\u{300}"),
    ('\u{c1}', "A\u{301}"),
    ('\u{c2}', "A\u{302}"),
    ('\u{c3}', "A\u{303}"),
    ('\u{c4}', "A\u{308}"),
    ('\u{c5}', "A\u{30a}"),
    ('\u{c7}', "C\u{327}"),
    ('\u{c8}', "E\u{300}"),
    ('\u{c9}', "E\u{301}"),
    ('\u{ca}', "E\u{302}"),
    ('\u{cb}', "E\u{308}"),
    ('\u{cc}', "I\u{300}"),
    ('\u{cd}', "I\u{301}"),
    ('\u{ce}', "I\u{302}"),
    ('\u{cf}', "I\u{308}"),
    ('\u{d1}', "N\u{303}"),
    ('\u{d2}', "O\u{300}"),
    ('\u{d3}', "O\u{301}"),
    ('\u{d4}', "O\u{302}"),
    ('\u{d5}', "O\u{303}"),
    ('\u{d6}', "O\u{308}"),
    ('\u{d9}', "U\u{300}"),
    ('\u{da}', "U\u{301}"),
    ('\u{db}', "U\u{302}"),
    ('\u{dc}', "U\u{308}"),
    ('\u{dd}', "Y\u{301}"),
    ('\u{e0}', "a\u{300}"),
    ('\u{e1}', "a\u{301}"),
    ('\u{e2}', "a\u{302}"),
    ('\u{e3}', "a\u{303}"),
    ('\u{e4}', "a\u{308}"),
    ('\u{e5}', "a\u{30a}"),
    ('\u{e7}', "c\u{327}"),
    ('\u{e8}', "e\u{300}"),
    ('\u{e9}', "e\u{301}"),
    ('\u{ea}', "e\u{302}"),
    ('\u{eb}', "e\u{308}"),
    ('\u{ec}', "i\u{300}"),
    ('\u{ed}', "i\u{301}"),
    ('\u{ee}', "i\u{302}"),
    ('\u{ef}', "i\u{308}"),
    ('\u{f1}', "n\u{303}"),
    ('\u{f2}', "o\u{300}"),
    ('\u{f3}', "o\u{301}"),
    ('\u{f4}', "o\u{302}"),
    ('\u{f5}', "o\u{303}"),
    ('\u{f6}', "o\u{308}"),
    ('\u{f9}', "u\u{300}"),
    ('\u{fa}', "u\u{301}"),
    ('\u{fb}', "u\u{302}"),
    ('\u{fc}', "u\u{308}"),
    ('\u{fd}', "y\u{301}"),
    ('\u{ff}', "y\u{308}"),
    ('\u{100}', "A\u{304}"),
    ('\u{101}', "a\u{304}"),
    ('\u{102}', "A\u{306}"),
    ('\u{103}', "a\u{306}"),
    ('\u{104}', "A\u{328}"),
    ('\u{105}', "a\u{328}"),
    ('\u{106}', "C\u{301}"),
    ('\u{107}', "c\u{301}"),
    ('\u{108}', "C\u{302}"),
    ('\u{109}', "c\u{302}"),
    ('\u{10a}', "C\u{307}"),
    ('\u{10b}', "c\u{307}"),
    ('\u{10c}', "C\u{30c}"),
    ('\u{10d}', "c\u{30c}"),
    ('\u{10e}', "D\u{30c}"),
    ('\u{10f}', "d\u{30c}"),
    ('\u{112}', "E\u{304}"),
    ('\u{113}', "e\u{304}"),
    ('\u{114}', "E\u{306}"),
    ('\u{115}', "e\u{306}"),
    ('\u{116}', "E\u{307}"),
    ('\u{117}', "e\u{307}"),
    ('\u{118}', "E\u{328}"),
    ('\u{119}', "e\u{328}"),
    ('\u{11a}', "E\u{30c}"),
    ('\u{11b}', "e\u{30c}"),
    ('\u{11c}', "G\u{302}"),
    ('\u{11d}', "g\u{302}"),
    ('\u{11e}', "G\u{306}"),
    ('\u{11f}', "g\u{306}"),
    ('\u{120}', "G\u{307}"),
    ('\u{121}', "g\u{307}"),
    ('\u{122}', "G\u{327}"),
    ('\u{123}', "g\u{327}"),
    ('\u{124}', "H\u{302}"),
    ('\u{125}', "h\u{302}"),
    ('\u{128}', "I\u{303}"),
    ('\u{129}', "i\u{303}"),
    ('\u{12a}', "I\u{304}"),
    ('\u{12b}', "i\u{304}"),
    ('\u{12c}', "I\u{306}"),
    ('\u{12d}', "i\u{306}"),
    ('\u{12e}', "I\u{328}"),
    ('\u{12f}', "i\u{328}"),
    ('\u{130}', "I\u{307}"),
    ('\u{134}', "J\u{302}"),
    ('\u{135}', "j\u{302}"),
    ('\u{136}', "K\u{327}"),
    ('\u{137}', "k\u{327}"),
    ('\u{139}', "L\u{301}"),
    ('\u{13a}', "l\u{301}"),
    ('\u{13b}', "L\u{327}"),
    ('\u{13c}', "l\u{327}"),
    ('\u{13d}', "L\u{30c}"),
    ('\u{13e}', "l\u{30c}"),
    ('\u{143}', "N\u{301}"),
    ('\u{144}', "n\u{301}"),
    ('\u{145}', "N\u{327}"),
    ('\u{146}', "n\u{327}"),
    ('\u{147}', "N\u{30c}"),
    ('\u{148}', "n\u{30c}"),
    ('\u{14c}', "O\u{304}"),
    ('\u{14d}', "o\u{304}"),
    ('\u{14e}', "O\u{306}"),
    ('\u{14f}', "o\u{306}"),
    ('\u{150}', "O\u{30b}"),
    ('\u{151}', "o\u{30b}"),
    ('\u{154}', "R\u{301}"),
    ('\u{155}', "r\u{301}"),
    ('\u{156}', "R\u{327}"),
    ('\u{157}', "r\u{327}"),
    ('\u{158}', "R\u{30c}"),
    ('\u{159}', "r\u{30c}"),
    ('\u{15a}', "S\u{301}"),
    ('\u{15b}', "s\u{301}"),
    ('\u{15c}', "S\u{302}"),
    ('\u{15d}', "s\u{302}"),
    ('\u{15e}', "S\u{327}"),
    ('\u{15f}', "s\u{327}"),
    ('\u{160}', "S\u{30c}"),
    ('\u{161}', "s\u{30c}"),
    ('\u{162}', "T\u{327}"),
    ('\u{163}', "t\u{327}"),
    ('\u{164}', "T\u{30c}"),
    ('\u{165}', "t\u{30c}"),
    ('\u{168}', "U\u{303}"),
    ('\u{169}', "u\u{303}"),
    ('\u{16a}', "U\u{304}"),
    ('\u{16b}', "u\u{304}"),
    ('\u{16c}', "U\u{306}"),
    ('\u{16d}', "u\u{306}"),
    ('\u{16e}', "U\u{30a}"),
    ('\u{16f}', "u\u{30a}"),
    ('\u{170}', "U\u{30b}"),
    ('\u{171}', "u\u{30b}"),
    ('\u{172}', "U\u{328}"),
    ('\u{173}', "u\u{328}"),
    ('\u{174}', "W\u{302}"),
    ('\u{175}', "w\u{302}"),
    ('\u{176}', "Y\u{302}"),
    ('\u{177}', "y\u{302}"),
    ('\u{178}', "Y\u{308}"),
    ('\u{179}', "Z\u{301}"),
    ('\u{17a}', "z\u{301}"),
    ('\u{17b}', "Z\u{307}"),
    ('\u{17c}', "z\u{307}"),
    ('\u{17d}', "Z\u{30c}"),
    ('\u{17e}', "z\u{30c}"),
    ('\u{1a0}', "O\u{31b}"),
    ('\u{1a1}', "o\u{31b}"),
    ('\u{1af}', "U\u{31b}"),
    ('\u{1b0}', "u\u{31b}"),
    ('\u{1cd}', "A\u{30c}"),
    ('\u{1ce}', "a\u{30c}"),
    ('\u{1cf}', "I\u{30c}"),
    ('\u{1d0}', "i\u{30c}"),
    ('\u{1d1}', "O\u{30c}"),
    ('\u{1d2}', "o\u{30c}"),
    ('\u{1d3}', "U\u{30c}"),
    ('\u{1d4}', "u\u{30c}"),
    ('\u{1d5}', "U\u{308}\u{304}"),
    ('\u{1d6}', "u\u{308}\u{304}"),
    ('\u{1d7}', "U\u{308}\u{301}"),
    ('\u{1d8}', "u\u{308}\u{301}"),
    ('\u{1d9}', "U\u{308}\u{30c}"),
    ('\u{1da}', "u\u{308}\u{30c}"),
    ('\u{1db}', "U\u{308}\u{300}"),
    ('\u{1dc}', "u\u{308}\u{300}"),
    ('\u{1de}', "A\u{308}\u{304}"),
    ('\u{1df}', "a\u{308}\u{304}"),
    ('\u{1e0}', "A\u{307}\u{304}"),
    ('\u{1e1}', "a\u{307}\u{304}"),
    ('\u{1e2}', "\u{c6}\u{304}"),
    ('\u{1e3}', "\u{e6}\u{304}"),
    ('\u{1e6}', "G\u{30c}"),
    ('\u{1e7}', "g\u{30c}"),
    ('\u{1e8}', "K\u{30c}"),
    ('\u{1e9}', "k\u{30c}"),
    ('\u{1ea}', "O\u{328}"),
    ('\u{1eb}', "o\u{328}"),
    ('\u{1ec}', "O\u{328}\u{304}"),
    ('\u{1ed}', "o\u{328}\u{304}"),
    ('\u{1ee}', "\u{1b7}\u{30c}"),
    ('\u{1ef}', "\u{292}\u{30c}"),
    ('\u{1f0}', "j\u{30c}"),
    ('\u{1f4}', "G\u{301}"),
    ('\u{1f5}', "g\u{301}"),
    ('\u{1f8}', "N\u{300}"),
    ('\u{1f9}', "n\u{300}"),
    ('\u{1fa}', "A\u{30a}\u{301}"),
    ('\u{1fb}', "a\u{30a}\u{301}"),
    ('\u{1fc}', "\u{c6}\u{301}"),
    ('\u{1fd}', "\u{e6}\u{301}"),
    ('\u{1fe}', "\u{d8}\u{301}"),
    ('\u{1ff}', "\u{f8}\u{301}"),
    ('\u{200}', "A\u{30f}"),
    ('\u{201}', "a\u{30f}"),
    ('\u{202}', "A\u{311}"),
    ('\u{203}', "a\u{311}"),
    ('\u{204}', "E\u{30f}"),
    ('\u{205}', "e\u{30f}"),
    ('\u{206}', "E\u{311}"),
    ('\u{207}', "e\u{311}"),
    ('\u{208}', "I\u{30f}"),
    ('\u{209}', "i\u{30f}"),
    ('\u{20a}', "I\u{311}"),
    ('\u{20b}', "i\u{311}"),
    ('\u{20c}', "O\u{30f}"),
    ('\u{20d}', "o\u{30f}"),
    ('\u{20e}', "O\u{311}"),
    ('\u{20f}', "o\u{311}"),
    ('\u{210}', "R\u{30f}"),
    ('\u{211}', "r\u{30f}"),
    ('\u{212}', "R\u{311}"),
    ('\u{213}', "r\u{311}"),
    ('\u{214}', "U\u{30f}"),
    ('\u{215}', "u\u{30f}"),
    ('\u{216}', "U\u{311}"),
    ('\u{217}', "u\u{311}"),
    ('\u{218}', "S\u{326}"),
    ('\u{219}', "s\u{326}"),
    ('\u{21a}', "T\u{326}"),
    ('\u{21b}', "t\u{326}"),
    ('\u{21e}', "H\u{30c}"),
    ('\u{21f}', "h\u{30c}"),
    ('\u{226}', "A\u{307}"),
    ('\u{227}', "a\u{307}"),
    ('\u{228}', "E\u{327}"),
    ('\u{229}', "e\u{327}"),
    ('\u{22a}', "O\u{308}\u{304}"),
    ('\u{22b}', "o\u{308}\u{304}"),
    ('\u{22c}', "O\u{303}\u{304}"),
    ('\u{22d}', "o\u{303}\u{304}"),
    ('\u{22e}', "O\u{307}"),
    ('\u{22f}', "o\u{307}"),
    ('\u{230}', "O\u{307}\u{304}"),
    ('\u{231}', "o\u{307}\u{304}"),
    ('\u{232}', "Y\u{304}"),
    ('\u{233}', "y\u{304}"),
    ('\u{340}', "\u{300}"),
    ('\u{341}', "\u{301}"),
    ('\u{343}', "\u{313}"),
    ('\u{344}', "\u{308}\u{301}"),
    ('\u{1e00}', "A\u{325}"),
    ('\u{1e01}', "a\u{325}"),
    ('\u{1e02}', "B\u{307}"),
    ('\u{1e03}', "b\u{307}"),
    ('\u{1e04}', "B\u{323}"),
    ('\u{1e05}', "b\u{323}"),
    ('\u{1e06}', "B\u{331}"),
    ('\u{1e07}', "b\u{331}"),
    ('\u{1e08}', "C\u{327}\u{301}"),
    ('\u{1e09}', "c\u{327}\u{301}"),
    ('\u{1e0a}', "D\u{307}"),
    ('\u{1e0b}', "d\u{307}"),
    ('\u{1e0c}', "D\u{323}"),
    ('\u{1e0d}', "d\u{323}"),
    ('\u{1e0e}', "D\u{331}"),
    ('\u{1e0f}', "d\u{331}"),
    ('\u{1e10}', "D\u{327}"),
    ('\u{1e11}', "d\u{327}"),
    ('\u{1e12}', "D\u{32d}"),
    ('\u{1e13}', "d\u{32d}"),
    ('\u{1e14}', "E\u{304}\u{300}"),
    ('\u{1e15}', "e\u{304}\u{300}"),
    ('\u{1e16}', "E\u{304}\u{301}"),
    ('\u{1e17}', "e\u{304}\u{301}"),
    ('\u{1e18}', "E\u{32d}"),
    ('\u{1e19}', "e\u{32d}"),
    ('\u{1e1a}', "E\u{330}"),
    ('\u{1e1b}', "e\u{330}"),
    ('\u{1e1c}', "E\u{327}\u{306}"),
    ('\u{1e1d}', "e\u{327}\u{306}"),
    ('\u{1e1e}', "F\u{307}"),
    ('\u{1e1f}', "f\u{307}"),
    ('\u{1e20}', "G\u{304}"),
    ('\u{1e21}', "g\u{304}"),
    ('\u{1e22}', "H\u{307}"),
    ('\u{1e23}', "h\u{307}"),
    ('\u{1e24}', "H\u{323}"),
    ('\u{1e25}', "h\u{323}"),
    ('\u{1e26}', "H\u{308}"),
    ('\u{1e27}', "h\u{308}"),
    ('\u{1e28}', "H\u{327}"),
    ('\u{1e29}', "h\u{327}"),
    ('\u{1e2a}', "H\u{32e}"),
    ('\u{1e2b}', "h\u{32e}"),
    ('\u{1e2c}', "I\u{330}"),
    ('\u{1e2d}', "i\u{330}"),
    ('\u{1e2e}', "I\u{308}\u{301}"),
    ('\u{1e2f}', "i\u{308}\u{301}"),
    ('\u{1e30}', "K\u{301}"),
    ('\u{1e31}', "k\u{301}"),
    ('\u{1e32}', "K\u{323}"),
    ('\u{1e33}', "k\u{323}"),
    ('\u{1e34}', "K\u{331}"),
    ('\u{1e35}', "k\u{331}"),
    ('\u{1e36}', "L\u{323}"),
    ('\u{1e37}', "l\u{323}"),
    ('\u{1e38}', "L\u{323}\u{304}"),
    ('\u{1e39}', "l\u{323}\u{304}"),
    ('\u{1e3a}', "L\u{331}"),
    ('\u{1e3b}', "l\u{331}"),
    ('\u{1e3c}', "L\u{32d}"),
    ('\u{1e3d}', "l\u{32d}"),
    ('\u{1e3e}', "M\u{301}"),
    ('\u{1e3f}', "m\u{301}"),
    ('\u{1e40}', "M\u{307}"),
    ('\u{1e41}', "m\u{307}"),
    ('\u{1e42}', "M\u{323}"),
    ('\u{1e43}', "m\u{323}"),
    ('\u{1e44}', "N\u{307}"),
    ('\u{1e45}', "n\u{307}"),
    ('\u{1e46}', "N\u{323}"),
    ('\u{1e47}', "n\u{323}"),
    ('\u{1e48}', "N\u{331}"),
    ('\u{1e49}', "n\u{331}"),
    ('\u{1e4a}', "N\u{32d}"),
    ('\u{1e4b}', "n\u{32d}"),
    ('\u{1e4c}', "O\u{303}\u{301}"),
    ('\u{1e4d}', "o\u{303}\u{301}"),
    ('\u{1e4e}', "O\u{303}\u{308}"),
    ('\u{1e4f}', "o\u{303}\u{308}"),
    ('\u{1e50}', "O\u{304}\u{300}"),
    ('\u{1e51}', "o\u{304}\u{300}"),
    ('\u{1e52}', "O\u{304}\u{301}"),
    ('\u{1e53}', "o\u{304}\u{301}"),
    ('\u{1e54}', "P\u{301}"),
    ('\u{1e55}', "p\u{301}"),
    ('\u{1e56}', "P\u{307}"),
    ('\u{1e57}', "p\u{307}"),
    ('\u{1e58}', "R\u{307}"),
    ('\u{1e59}', "r\u{307}"),
    ('\u{1e5a}', "R\u{323}"),
    ('\u{1e5b}', "r\u{323}"),
    ('\u{1e5c}', "R\u{323}\u{304}"),
    ('\u{1e5d}', "r\u{323}\u{304}"),
    ('\u{1e5e}', "R\u{331}"),
    ('\u{1e5f}', "r\u{331}"),
    ('\u{1e60}', "S\u{307}"),
    ('\u{1e61}', "s\u{307}"),
    ('\u{1e62}', "S\u{323}"),
    ('\u{1e63}', "s\u{323}"),
    ('\u{1e64}', "S\u{301}\u{307}"),
    ('\u{1e65}', "s\u{301}\u{307}"),
    ('\u{1e66}', "S\u{30c}\u{307}"),
    ('\u{1e67}', "s\u{30c}\u{307}"),
    ('\u{1e68}', "S\u{323}\u{307}"),
    ('\u{1e69}', "s\u{323}\u{307}"),
    ('\u{1e6a}', "T\u{307}"),
    ('\u{1e6b}', "t\u{307}"),
    ('\u{1e6c}', "T\u{323}"),
    ('\u{1e6d}', "t\u{323}"),
    ('\u{1e6e}', "T\u{331}"),
    ('\u{1e6f}', "t\u{331}"),
    ('\u{1e70}', "T\u{32d}"),
    ('\u{1e71}', "t\u{32d}"),
    ('\u{1e72}', "U\u{324}"),
    ('\u{1e73}', "u\u{324}"),
    ('\u{1e74}', "U\u{330}"),
    ('\u{1e75}', "u\u{330}"),
    ('\u{1e76}', "U\u{32d}"),
    ('\u{1e77}', "u\u{32d}"),
    ('\u{1e78}', "U\u{303}\u{301}"),
    ('\u{1e79}', "u\u{303}\u{301}"),
    ('\u{1e7a}', "U\u{304}\u{308}"),
    ('\u{1e7b}', "u\u{304}\u{308}"),
    ('\u{1e7c}', "V\u{303}"),
    ('\u{1e7d}', "v\u{303}"),
    ('\u{1e7e}', "V\u{323}"),
    ('\u{1e7f}', "v\u{323}"),
    ('\u{1e80}', "W\u{300}"),
    ('\u{1e81}', "w\u{300}"),
    ('\u{1e82}', "W\u{301}"),
    ('\u{1e83}', "w\u{301}"),
    ('\u{1e84}', "W\u{308}"),
    ('\u{1e85}', "w\u{308}"),
    ('\u{1e86}', "W\u{307}"),
    ('\u{1e87}', "w\u{307}"),
    ('\u{1e88}', "W\u{323}"),
    ('\u{1e89}', "w\u{323}"),
    ('\u{1e8a}', "X\u{307}"),
    ('\u{1e8b}', "x\u{307}"),
    ('\u{1e8c}', "X\u{308}"),
    ('\u{1e8d}', "x\u{308}"),
    ('\u{1e8e}', "Y\u{307}"),
    ('\u{1e8f}', "y\u{307}"),
    ('\u{1e90}', "Z\u{302}"),
    ('\u{1e91}', "z\u{302}"),
    ('\u{1e92}', "Z\u{323}"),
    ('\u{1e93}', "z\u{323}"),
    ('\u{1e94}', "Z\u{331}"),
    ('\u{1e95}', "z\u{331}"),
    ('\u{1e96}', "h\u{331}"),
    ('\u{1e97}', "t\u{308}"),
    ('\u{1e98}', "w\u{30a}"),
    ('\u{1e99}', "y\u{30a}"),
    ('\u{1e9b}', "\u{17f}\u{307}"),
    ('\u{1ea0}', "A\u{323}"),
    ('\u{1ea1}', "a\u{323}"),
    ('\u{1ea2}', "A\u{309}"),
    ('\u{1ea3}', "a\u{309}"),
    ('\u{1ea4}', "A\u{302}\u{301}"),
    ('\u{1ea5}', "a\u{302}\u{301}"),
    ('\u{1ea6}', "A\u{302}\u{300}"),
    ('\u{1ea7}', "a\u{302}\u{300}"),
    ('\u{1ea8}', "A\u{302}\u{309}"),
    ('\u{1ea9}', "a\u{302}\u{309}"),
    ('\u{1eaa}', "A\u{302}\u{303}"),
    ('\u{1eab}', "a\u{302}\u{303}"),
    ('\u{1eac}', "A\u{323}\u{302}"),
    ('\u{1ead}', "a\u{323}\u{302}"),
    ('\u{1eae}', "A\u{306}\u{301}"),
    ('\u{1eaf}', "a\u{306}\u{301}"),
    ('\u{1eb0}', "A\u{306}\u{300}"),
    ('\u{1eb1}', "a\u{306}\u{300}"),
    ('\u{1eb2}', "A\u{306}\u{309}"),
    ('\u{1eb3}', "a\u{306}\u{309}"),
    ('\u{1eb4}', "A\u{306}\u{303}"),
    ('\u{1eb5}', "a\u{306}\u{303}"),
    ('\u{1eb6}', "A\u{323}\u{306}"),
    ('\u{1eb7}', "a\u{323}\u{306}"),
    ('\u{1eb8}', "E\u{323}"),
    ('\u{1eb9}', "e\u{323}"),
    ('\u{1eba}', "E\u{309}"),
    ('\u{1ebb}', "e\u{309}"),
    ('\u{1ebc}', "E\u{303}"),
    ('\u{1ebd}', "e\u{303}"),
    ('\u{1ebe}', "E\u{302}\u{301}"),
    ('\u{1ebf}', "e\u{302}\u{301}"),
    ('\u{1ec0}', "E\u{302}\u{300}"),
    ('\u{1ec1}', "e\u{302}\u{300}"),
    ('\u{1ec2}', "E\u{302}\u{309}"),
    ('\u{1ec3}', "e\u{302}\u{309}"),
    ('\u{1ec4}', "E\u{302}\u{303}"),
    ('\u{1ec5}', "e\u{302}\u{303}"),
    ('\u{1ec6}', "E\u{323}\u{302}"),
    ('\u{1ec7}', "e\u{323}\u{302}"),
    ('\u{1ec8}', "I\u{309}"),
    ('\u{1ec9}', "i\u{309}"),
    ('\u{1eca}', "I\u{323}"),
    ('\u{1ecb}', "i\u{323}"),
    ('\u{1ecc}', "O\u{323}"),
    ('\u{1ecd}', "o\u{323}"),
    ('\u{1ece}', "O\u{309}"),
    ('\u{1ecf}', "o\u{309}"),
    ('\u{1ed0}', "O\u{302}\u{301}"),
    ('\u{1ed1}', "o\u{302}\u{301}"),
    ('\u{1ed2}', "O\u{302}\u{300}"),
    ('\u{1ed3}', "o\u{302}\u{300}"),
    ('\u{1ed4}', "O\u{302}\u{309}"),
    ('\u{1ed5}', "o\u{302}\u{309}"),
    ('\u{1ed6}', "O\u{302}\u{303}"),
    ('\u{1ed7}', "o\u{302}\u{303}"),
    ('\u{1ed8}', "O\u{323}\u{302}"),
    ('\u{1ed9}', "o\u{323}\u{302}"),
    ('\u{1eda}', "O\u{31b}\u{301}"),
    ('\u{1edb}', "o\u{31b}\u{301}"),
    ('\u{1edc}', "O\u{31b}\u{300}"),
    ('\u{1edd}', "o\u{31b}\u{300}"),
    ('\u{1ede}', "O\u{31b}\u{309}"),
    ('\u{1edf}', "o\u{31b}\u{309}"),
    ('\u{1ee0}', "O\u{31b}\u{303}"),
    ('\u{1ee1}', "o\u{31b}\u{303}"),
    ('\u{1ee2}', "O\u{31b}\u{323}"),
    ('\u{1ee3}', "o\u{31b}\u{323}"),
    ('\u{1ee4}', "U\u{323}"),
    ('\u{1ee5}', "u\u{323}"),
    ('\u{1ee6}', "U\u{309}"),
    ('\u{1ee7}', "u\u{309}"),
    ('\u{1ee8}', "U\u{31b}\u{301}"),
    ('\u{1ee9}', "u\u{31b}\u{301}"),
    ('\u{1eea}', "U\u{31b}\u{300}"),
    ('\u{1eeb}', "u\u{31b}\u{300}"),
    ('\u{1eec}', "U\u{31b}\u{309}"),
    ('\u{1eed}', "u\u{31b}\u{309}"),
    ('\u{1eee}', "U\u{31b}\u{303}"),
    ('\u{1eef}', "u\u{31b}\u{303}"),
    ('\u{1ef0}', "U\u{31b}\u{323}"),
    ('\u{1ef1}', "u\u{31b}\u{323}"),
    ('\u{1ef2}', "Y\u{300}"),
    ('\u{1ef3}', "y\u{300}"),
    ('\u{1ef4}', "Y\u{323}"),
    ('\u{1ef5}', "y\u{323}"),
    ('\u{1ef6}', "Y\u{309}"),
    ('\u{1ef7}', "y\u{309}"),
    ('\u{1ef8}', "Y\u{303}"),
    ('\u{1ef9}', "y\u{303}"),
];

/// Canonical combining class of the combining marks in U+0300..=U+036F whose class is not 0,
/// sorted by code point. 111 entries.
static COMBINING_CLASS: &[(char, u8)] = &[
    ('\u{300}', 230),
    ('\u{301}', 230),
    ('\u{302}', 230),
    ('\u{303}', 230),
    ('\u{304}', 230),
    ('\u{305}', 230),
    ('\u{306}', 230),
    ('\u{307}', 230),
    ('\u{308}', 230),
    ('\u{309}', 230),
    ('\u{30a}', 230),
    ('\u{30b}', 230),
    ('\u{30c}', 230),
    ('\u{30d}', 230),
    ('\u{30e}', 230),
    ('\u{30f}', 230),
    ('\u{310}', 230),
    ('\u{311}', 230),
    ('\u{312}', 230),
    ('\u{313}', 230),
    ('\u{314}', 230),
    ('\u{315}', 232),
    ('\u{316}', 220),
    ('\u{317}', 220),
    ('\u{318}', 220),
    ('\u{319}', 220),
    ('\u{31a}', 232),
    ('\u{31b}', 216),
    ('\u{31c}', 220),
    ('\u{31d}', 220),
    ('\u{31e}', 220),
    ('\u{31f}', 220),
    ('\u{320}', 220),
    ('\u{321}', 202),
    ('\u{322}', 202),
    ('\u{323}', 220),
    ('\u{324}', 220),
    ('\u{325}', 220),
    ('\u{326}', 220),
    ('\u{327}', 202),
    ('\u{328}', 202),
    ('\u{329}', 220),
    ('\u{32a}', 220),
    ('\u{32b}', 220),
    ('\u{32c}', 220),
    ('\u{32d}', 220),
    ('\u{32e}', 220),
    ('\u{32f}', 220),
    ('\u{330}', 220),
    ('\u{331}', 220),
    ('\u{332}', 220),
    ('\u{333}', 220),
    ('\u{334}', 1),
    ('\u{335}', 1),
    ('\u{336}', 1),
    ('\u{337}', 1),
    ('\u{338}', 1),
    ('\u{339}', 220),
    ('\u{33a}', 220),
    ('\u{33b}', 220),
    ('\u{33c}', 220),
    ('\u{33d}', 230),
    ('\u{33e}', 230),
    ('\u{33f}', 230),
    ('\u{340}', 230),
    ('\u{341}', 230),
    ('\u{342}', 230),
    ('\u{343}', 230),
    ('\u{344}', 230),
    ('\u{345}', 240),
    ('\u{346}', 230),
    ('\u{347}', 220),
    ('\u{348}', 220),
    ('\u{349}', 220),
    ('\u{34a}', 230),
    ('\u{34b}', 230),
    ('\u{34c}', 230),
    ('\u{34d}', 220),
    ('\u{34e}', 220),
    ('\u{350}', 230),
    ('\u{351}', 230),
    ('\u{352}', 230),
    ('\u{353}', 220),
    ('\u{354}', 220),
    ('\u{355}', 220),
    ('\u{356}', 220),
    ('\u{357}', 230),
    ('\u{358}', 232),
    ('\u{359}', 220),
    ('\u{35a}', 220),
    ('\u{35b}', 230),
    ('\u{35c}', 233),
    ('\u{35d}', 234),
    ('\u{35e}', 234),
    ('\u{35f}', 233),
    ('\u{360}', 234),
    ('\u{361}', 234),
    ('\u{362}', 233),
    ('\u{363}', 230),
    ('\u{364}', 230),
    ('\u{365}', 230),
    ('\u{366}', 230),
    ('\u{367}', 230),
    ('\u{368}', 230),
    ('\u{369}', 230),
    ('\u{36a}', 230),
    ('\u{36b}', 230),
    ('\u{36c}', 230),
    ('\u{36d}', 230),
    ('\u{36e}', 230),
    ('\u{36f}', 230),
];

/// The canonical combining class of `c` (0 for a starter and for every character outside the
/// table).
pub(crate) fn ccc(c: char) -> u8 {
    if !('\u{300}'..='\u{36f}').contains(&c) {
        return 0;
    }
    match COMBINING_CLASS.binary_search_by_key(&c, |&(k, _)| k) {
        Ok(i) => COMBINING_CLASS.get(i).map_or(0, |&(_, class)| class),
        Err(_) => 0,
    }
}

/// The full canonical decomposition of `c`, or `None` when it has none in the table.
fn decompose(c: char) -> Option<&'static str> {
    if (c as u32) < DEC_FIRST {
        return None;
    }
    let i = DECOMPOSITION.binary_search_by_key(&c, |&(k, _)| k).ok()?;
    DECOMPOSITION.get(i).map(|&(_, d)| d)
}

/// `s` with every character in the table fully decomposed and each run of combining marks put in
/// canonical order (what `EXACT` compares: NFC and NFD text give the same result). Case is kept
/// and characters outside the table are kept as they are. A run of more than 32 marks keeps its
/// input order. ASCII text comes back unchanged.
pub fn canonical(s: &str) -> String {
    if s.is_ascii() {
        return s.to_string();
    }
    let mut chars: Vec<char> = Vec::with_capacity(s.len());
    for c in s.chars() {
        match decompose(c) {
            Some(d) => chars.extend(d.chars()),
            None => chars.push(c),
        }
    }
    reorder_marks(&mut chars);
    chars.into_iter().collect()
}

/// Put each maximal run of marks with a nonzero combining class into ascending class order,
/// keeping the order of marks of equal class (a stable sort). Runs longer than [`MAX_RUN`] stay.
fn reorder_marks(chars: &mut [char]) {
    let mut i = 0;
    while i < chars.len() {
        if chars.get(i).is_none_or(|&c| ccc(c) == 0) {
            i += 1;
            continue;
        }
        let start = i;
        while chars.get(i).is_some_and(|&c| ccc(c) != 0) {
            i += 1;
        }
        if i.saturating_sub(start) <= MAX_RUN
            && let Some(run) = chars.get_mut(start..i)
        {
            run.sort_by_key(|&c| ccc(c));
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The Root text order.
// ---------------------------------------------------------------------------------------------

/// Most primary elements a key holds: a megabyte paste does not make a megabyte key. The rest of
/// a longer text only takes part through the final tie-break on [`fold_key`], which always holds
/// the whole text.
const MAX_PRIMARY: usize = 16_384;

/// Secondary code of the stroke of `đ` (primary `d`). Mark codes start at 32, so it never equals
/// one.
const STROKE: u16 = 11;

/// Lowercase one char to exactly one char (no full case folding), like the criteria helpers do.
fn lower_char(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

/// Whether `c` is a combining mark the key attaches to the element before it.
fn is_mark(c: char) -> bool {
    ('\u{300}'..='\u{36f}').contains(&c)
}

/// The secondary code of combining mark `c`: `32 + (cp - 0x300)`.
fn mark_code(c: char) -> u16 {
    u16::try_from((c as u32).saturating_sub(0x300).saturating_add(32)).unwrap_or(u16::MAX)
}

/// Append the lowercase, fully decomposed form of `c` to `out` (marks are not yet reordered).
///
/// A character the table decomposes is decomposed first and the pieces lowercased, so `İ`
/// (U+0130, whose one-to-one lowercase would lose its dot) equals `I` + U+0307. Any other
/// character is lowercased first, so one whose lowercase is a precomposed letter (the Angstrom
/// sign U+212B, lowercase `å`) is decomposed too. Either way NFC and NFD text give the same chars.
fn push_folded(c: char, out: &mut Vec<char>) {
    fold_char(c, |x| out.push(x));
}

/// [`push_folded`] without a buffer: hands each char of the folded form of `c` to `emit`.
fn fold_char(c: char, mut emit: impl FnMut(char)) {
    if let Some(d) = decompose(c) {
        d.chars().map(lower_char).for_each(emit);
        return;
    }
    let lowered = lower_char(c);
    match decompose(lowered) {
        Some(d) => d.chars().for_each(emit),
        None => emit(lowered),
    }
}

#[cfg(test)]
thread_local! {
    /// How many times [`fold_key`] ran on this thread (tests only: proves a caller folds once).
    static FOLDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Byte comparisons [`contains_prefolded`] made on this thread (tests only: proves the search
    /// is linear, which a wall-clock assertion could not do on an unknown machine).
    static SEARCH_STEPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The form two strings must share to be equal as text: lowercased one char to one char, fully
/// decomposed, each run of combining marks in canonical order. Two strings are equal by
/// [`compare_text`] and [`text_eq`] exactly when their `fold_key`s are equal, so it is the key to
/// group, deduplicate and look up by (`HashMap`, `BTreeMap`). It never changes the user's text.
pub fn fold_key(s: &str) -> String {
    #[cfg(test)]
    FOLDS.with(|n| n.set(n.get().saturating_add(1)));
    if s.is_ascii() {
        return s.to_ascii_lowercase();
    }
    let mut chars: Vec<char> = Vec::with_capacity(s.len());
    for c in s.chars() {
        push_folded(c, &mut chars);
    }
    reorder_marks(&mut chars);
    chars.into_iter().collect()
}

/// Whether `a` and `b` are the same text, ignoring case and encoding (`é` precomposed equals `e`
/// + U+0301; `Ánh` equals `ÁNH`). The same relation as `compare_text(a, b) == Equal`.
pub fn text_eq(a: &str, b: &str) -> bool {
    if a.is_ascii() && b.is_ascii() {
        return a.eq_ignore_ascii_case(b);
    }
    fold_key(a) == fold_key(b)
}

/// `text_eq(s, other)` with `other` already folded (`folded == fold_key(other)`), so a needle
/// compared with many cells is folded once. It allocates nothing unless `s` holds a combining mark
/// U+0300..=U+036F of its own: precomposed letters are folded char by char as they are read, and
/// the comparison stops at the first char that differs. (Each letter of the tables decomposes to
/// a base letter and marks already in canonical order, so only marks typed separately can need
/// the reordering of [`fold_key`]; text holding one is folded whole.)
pub fn text_eq_prefolded(s: &str, folded: &str) -> bool {
    if s.is_ascii() {
        return s.eq_ignore_ascii_case(folded);
    }
    // U+0300..=U+036F are the UTF-8 sequences CC 80..=CD AF; a CC or CD lead byte (U+0300..=U+037F)
    // takes the full path.
    if s.bytes().any(|b| b == 0xcc || b == 0xcd) {
        return fold_key(s) == folded;
    }
    let mut want = folded.chars();
    for c in s.chars() {
        let mut same = true;
        fold_char(c, |x| same &= want.next() == Some(x));
        if !same {
            return false;
        }
    }
    want.next().is_none()
}

/// A set of texts under [`text_eq`]: each text is folded once when it goes in, and a probe once
/// when it is looked up, so testing `n` cells against `k` chosen values is `n + k` folds, not
/// `n * k` comparisons (AutoFilter value lists, slicer and PivotTable selections).
#[derive(Debug, Clone, Default)]
pub struct TextSet(std::collections::HashSet<String>);

impl TextSet {
    /// The set of `items`, compared by [`text_eq`].
    pub fn new<I: IntoIterator<Item = S>, S: AsRef<str>>(items: I) -> TextSet {
        TextSet(items.into_iter().map(|t| fold_key(t.as_ref())).collect())
    }

    /// Adds `s`; true when no text [`text_eq`] to it was in the set yet.
    pub fn insert(&mut self, s: &str) -> bool {
        self.0.insert(fold_key(s))
    }

    /// Whether some text in the set is [`text_eq`] to `s`.
    pub fn contains(&self, s: &str) -> bool {
        self.0.contains(&fold_key(s))
    }

    /// How many distinct texts (under [`text_eq`]) the set holds.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the set is empty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// The weights `compare_text` compares, in order.
///
/// - `primary`: one entry per element of the folded text: the code point of a base character
///   (`đ` counts as `d`). Combining marks U+0300..=U+036F are not elements.
/// - `secondary`: for each element, its stroke code (`đ`) and the codes of the marks after it
///   (`32 + (cp - 0x300)`, in canonical order), then a `0` ending the element. A list per
///   element, not a sum, so different mark sets never collide (`ấ` is `[34, 33]`, `ä` is `[40]`).
///   A mark with no element before it is dropped here and survives in `text`.
/// - `text`: the whole [`fold_key`], so only equal text compares equal.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Root {
    primary: Vec<u32>,
    secondary: Vec<u16>,
    text: String,
}

impl Root {
    fn new(s: &str) -> Root {
        let text = fold_key(s);
        let mut primary: Vec<u32> = Vec::new();
        let mut secondary: Vec<u16> = Vec::new();
        for c in text.chars() {
            if is_mark(c) {
                if !primary.is_empty() {
                    secondary.push(mark_code(c));
                }
                continue;
            }
            if primary.len() >= MAX_PRIMARY {
                break;
            }
            if !primary.is_empty() {
                secondary.push(0);
            }
            if c == 'đ' {
                primary.push('d' as u32);
                secondary.push(STROKE);
            } else {
                primary.push(c as u32);
            }
        }
        if !primary.is_empty() {
            secondary.push(0);
        }
        Root { primary, secondary, text }
    }
}

impl Ord for Root {
    fn cmp(&self, other: &Root) -> Ordering {
        self.primary.cmp(&other.primary).then_with(|| self.secondary.cmp(&other.secondary)).then_with(|| self.text.cmp(&other.text))
    }
}

impl PartialOrd for Root {
    fn partial_cmp(&self, other: &Root) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Case-insensitive, encoding-insensitive text order (the Root order: ours, language-neutral, not
/// CLDR root).
///
/// Letters compare by their base letter first, so `Ánh` sorts next to `Anh` and not after `z`;
/// accents break ties, ordered by the code point of the mark (`Anh` < `Ánh` < `Ảnh`: acute U+0301
/// before hook above U+0309), and `đ` counts as `d` first and follows it as the tie-break. Text that is
/// the same after [`fold_key`] (case, NFC versus NFD, marks of different classes in either order)
/// compares equal. Characters outside the Latin tables keep code point order, including the Latin
/// letters that have no decomposition (such as `ø`, `ł`, `æ`, `ß`, `ð`, `ħ`, `ı`), which therefore sort
/// after `z`. Two ASCII strings compare by their lowercase bytes, as before.
pub fn compare_text(a: &str, b: &str) -> Ordering {
    if a.is_ascii() && b.is_ascii() {
        return a.bytes().map(|x| x.to_ascii_lowercase()).cmp(b.bytes().map(|x| x.to_ascii_lowercase()));
    }
    Root::new(a).cmp(&Root::new(b))
}

/// A precomputed sort key for text: `text_key(a).cmp(&text_key(b)) == compare_text(a, b)` for
/// every pair, and two keys are equal exactly when [`text_eq`] holds. Build it once per string
/// and compare keys (`sort_by_cached_key`, a field of a sorted item) instead of calling
/// [`compare_text`], which builds both keys on every comparison. Opaque: what it holds may
/// change (a later collation choice), only its order is the contract.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TextKey(Root);

/// The [`TextKey`] of `s` (the Root order of [`compare_text`]).
pub fn text_key(s: &str) -> TextKey {
    TextKey(Root::new(s))
}

// ---------------------------------------------------------------------------------------------
// Searching folded text: "contains", "begins with", "ends with".
// ---------------------------------------------------------------------------------------------

/// Whether the char at byte offset `at` of `s` is a combining mark.
fn mark_at(s: &str, at: usize) -> bool {
    s.get(at..).and_then(|t| t.chars().next()).is_some_and(is_mark)
}

/// Whether `haystack` contains `needle` as text, ignoring case and encoding (both sides go
/// through [`fold_key`]), matching whole letters: an occurrence followed by a combining mark
/// U+0300..=U+036F is not a match, so `e` is not found in `Nguyễn` while `ễ` and `nguyễn` are.
/// Every occurrence is tried, overlapping ones too, so a rejected one never hides a later match.
/// A needle that starts with a mark is taken as given. An empty needle is always found.
///
/// [`ends_with_folded`] differs for a needle that starts with a mark: there the match must begin
/// on a letter boundary, so `◌́` ends only text that starts with it.
pub fn contains_folded(haystack: &str, needle: &str) -> bool {
    contains_prefolded(haystack, &fold_key(needle))
}

/// The Knuth–Morris–Pratt failure function of `needle`: `f[i]` is the length of the longest
/// proper prefix of `needle[..=i]` that is also a suffix of it. Built once per needle, which is
/// what keeps [`contains_prefolded`] linear.
fn kmp_failure(needle: &[u8]) -> Vec<usize> {
    let mut f = vec![0usize; needle.len()];
    let mut len = 0usize;
    for i in 1..needle.len() {
        while len > 0 && needle.get(i) != needle.get(len) {
            len = len.checked_sub(1).and_then(|j| f.get(j).copied()).unwrap_or(0);
        }
        if needle.get(i) == needle.get(len) {
            len = len.saturating_add(1);
        }
        if let Some(slot) = f.get_mut(i) {
            *slot = len;
        }
    }
    f
}

/// [`contains_folded`] with the needle already folded (`folded_needle == fold_key(needle)`), so
/// a criterion tested against many cells is folded once.
///
/// One left-to-right Knuth–Morris–Pratt pass, so the work is `O(haystack + needle)` whatever
/// the text. Restarting [`str::find`] after each rejected occurrence would be quadratic on text a
/// workbook can hold: for a needle of combining marks inside a longer run of the same mark, every
/// occurrence is followed by a mark and so rejected, and each restart pays for the needle again
/// (~537 million byte comparisons for one cell at Excel's 32,767-character limit — a filter over a
/// column of them stops responding). Matching bytes is matching characters here, because a UTF-8
/// sequence occurs in another only at a character boundary.
pub fn contains_prefolded(haystack: &str, folded_needle: &str) -> bool {
    let needle = folded_needle.as_bytes();
    if needle.is_empty() {
        return true;
    }
    let hay = fold_key(haystack);
    let f = kmp_failure(needle);
    let mut len = 0usize;
    for (i, b) in hay.as_bytes().iter().enumerate() {
        #[cfg(test)]
        SEARCH_STEPS.with(|n| n.set(n.get().saturating_add(1)));
        while len > 0 && needle.get(len) != Some(b) {
            #[cfg(test)]
            SEARCH_STEPS.with(|n| n.set(n.get().saturating_add(1)));
            len = len.checked_sub(1).and_then(|j| f.get(j).copied()).unwrap_or(0);
        }
        if needle.get(len) == Some(b) {
            len = len.saturating_add(1);
        }
        if len == needle.len() {
            // An occurrence ending just past `i`: a match unless a combining mark follows it.
            if !mark_at(&hay, i.saturating_add(1)) {
                return true;
            }
            // Keep going from the longest proper prefix, so overlapping occurrences are tried too
            // and a rejected one never hides a later match.
            len = len.checked_sub(1).and_then(|j| f.get(j).copied()).unwrap_or(0);
        }
    }
    false
}

/// Whether `haystack` begins with `needle`, ignoring case and encoding like [`contains_folded`].
/// The prefix must end on a whole letter: `a` does not begin `Ánh`, `á` does. An empty needle
/// begins everything.
pub fn starts_with_folded(haystack: &str, needle: &str) -> bool {
    starts_with_prefolded(haystack, &fold_key(needle))
}

/// [`starts_with_folded`] with the needle already folded (`folded_needle == fold_key(needle)`).
pub fn starts_with_prefolded(haystack: &str, folded_needle: &str) -> bool {
    if folded_needle.is_empty() {
        return true;
    }
    let hay = fold_key(haystack);
    hay.starts_with(folded_needle) && !mark_at(&hay, folded_needle.len())
}

/// Whether `haystack` ends with `needle`, ignoring case and encoding like [`contains_folded`].
/// The suffix must start at a letter boundary: at the start of the text or on a character that
/// is not a combining mark. So `nh` ends `Ánh`, but a needle that starts with a mark (`◌̃n`) does
/// not end `ễn`, because it would begin inside the marks of `ễ`. An empty needle ends everything.
pub fn ends_with_folded(haystack: &str, needle: &str) -> bool {
    ends_with_prefolded(haystack, &fold_key(needle))
}

/// [`ends_with_folded`] with the needle already folded (`folded_needle == fold_key(needle)`).
pub fn ends_with_prefolded(haystack: &str, folded_needle: &str) -> bool {
    let needle = folded_needle;
    if needle.is_empty() {
        return true;
    }
    let hay = fold_key(haystack);
    if !hay.ends_with(needle) {
        return false;
    }
    let at = hay.len().saturating_sub(needle.len());
    at == 0 || !mark_at(&hay, at)
}

#[cfg(test)]
mod tables {
    use super::*;
    use icu_normalizer::DecomposingNormalizerBorrowed;
    use icu_normalizer::properties::CanonicalCombiningClassMapBorrowed;

    /// The scalar ranges the table covers: Latin-1 Supplement letters to Latin Extended-B, the
    /// combining marks, and Latin Extended Additional.
    fn covered() -> impl Iterator<Item = char> {
        ('\u{c0}'..='\u{24f}').chain('\u{300}'..='\u{36f}').chain('\u{1e00}'..='\u{1eff}')
    }

    /// The marks of U+0300..=U+036F that have a canonical decomposition of their own.
    const MARKS_THAT_DECOMPOSE: [char; 4] = ['\u{340}', '\u{341}', '\u{343}', '\u{344}'];

    #[test]
    fn tables_match_icu_normalizer() {
        let nfd = DecomposingNormalizerBorrowed::new_nfd();
        for c in covered() {
            let s = c.to_string();
            assert_eq!(canonical(&s), nfd.normalize(&s), "U+{:04X}", c as u32);
        }
        let map = CanonicalCombiningClassMapBorrowed::new();
        for c in '\u{300}'..='\u{36f}' {
            assert_eq!(ccc(c), map.get_u8(c), "ccc U+{:04X}", c as u32);
        }
        assert_eq!(ccc('a'), 0);
        assert_eq!(ccc('\u{10ffff}'), 0);
        // The marks that decompose, after a letter and next to marks of other classes, in either
        // order: decomposed, then ordered with their neighbours.
        for m in MARKS_THAT_DECOMPOSE {
            for base in ["", "a", "U", "\u{1b0}"] {
                for other in ["", "\u{323}", "\u{31b}", "\u{302}", "\u{345}"] {
                    for s in [format!("{base}{m}{other}"), format!("{base}{other}{m}")] {
                        assert_eq!(canonical(&s), nfd.normalize(&s), "{s:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn marks_that_decompose_equal_their_decomposition() {
        // U+0340 and U+0341 (the grave and acute tone marks) are U+0300 and U+0301; U+0343 is
        // U+0313; U+0344 is U+0308 U+0301.
        assert_eq!(canonical("\u{340}\u{341}\u{343}"), "\u{300}\u{301}\u{313}");
        assert_eq!(canonical("\u{344}"), "\u{308}\u{301}");
        assert_eq!(canonical("a\u{344}\u{323}"), "a\u{323}\u{308}\u{301}");
        assert_eq!(fold_key("\u{341}"), "\u{301}");
        assert!(text_eq("a\u{341}", "á"));
        assert!(text_eq("a\u{340}", "à"));
        assert!(text_eq("U\u{344}", "ǘ"));
        assert!(text_eq("Ma\u{341}", "MÁ"));
        assert_eq!(compare_text("ma\u{341}", "má"), Ordering::Equal);
        assert_eq!(compare_text("ma\u{340}", "mà"), Ordering::Equal);
        assert!(text_eq_prefolded("ma\u{341}", &fold_key("má")));
        assert!(text_eq_prefolded("má", &fold_key("ma\u{341}")));
        assert_eq!(TextSet::new(["má", "ma\u{341}", "MA\u{301}", "mà", "ma\u{340}"]).len(), 2);
    }

    #[test]
    fn fold_key_gives_nfc_and_nfd_the_same_key_for_every_covered_letter() {
        let nfd = DecomposingNormalizerBorrowed::new_nfd();
        for c in covered().chain(['\u{212a}', '\u{212b}']) {
            let s = c.to_string();
            let d = nfd.normalize(&s);
            assert_eq!(fold_key(&s), fold_key(&d), "U+{:04X}", c as u32);
            assert_eq!(compare_text(&s, &d), Ordering::Equal, "U+{:04X}", c as u32);
            let upper: String = s.chars().flat_map(char::to_uppercase).collect();
            if upper.chars().count() == 1 && upper.chars().all(|u| lower_char(u) == c) {
                assert_eq!(fold_key(&upper), fold_key(&s), "case U+{:04X}", c as u32);
            }
        }
    }

    #[test]
    fn reorders_marks_by_combining_class_stably() {
        assert_eq!(canonical("a\u{302}\u{323}"), "a\u{323}\u{302}"); // ậ typed in the wrong order
        assert_eq!(canonical("ậ"), "a\u{323}\u{302}");
        assert_eq!(canonical("o\u{323}\u{31b}"), "o\u{31b}\u{323}"); // ợ: horn (216) before dot below (220)
        assert_eq!(canonical("ợ"), "o\u{31b}\u{323}");
        assert_eq!(canonical("e\u{303}\u{302}"), "e\u{303}\u{302}"); // same class 230: order kept
        assert_ne!(canonical("e\u{303}\u{302}"), canonical("ễ"));
        assert_eq!(canonical("Nguyễn"), canonical("Nguye\u{302}\u{303}n"));
        assert_eq!(canonical("ABC xyz"), "ABC xyz");
        assert_eq!(canonical("Ấ"), "A\u{302}\u{301}"); // case kept
    }

    #[test]
    fn hostile_marks_never_panic_and_long_clusters_stay_put() {
        // 10 000 alternating marks of different classes: reordered or not, never a panic.
        let long: String = "a".to_string() + &"\u{323}\u{302}".repeat(5_000);
        let _ = canonical(&long);
        // A leading mark, the last scalar, and the dotted/capital specials.
        assert_eq!(canonical("\u{323}\u{302}"), "\u{323}\u{302}");
        assert_eq!(canonical("\u{302}\u{323}"), "\u{323}\u{302}");
        assert_eq!(canonical("\u{10FFFF}"), "\u{10FFFF}");
        assert_eq!(canonical("İ"), "I\u{307}");
        assert_eq!(canonical("ẞ"), "ẞ");
        // A megabyte of text.
        let big = "Nguyễn Văn ".repeat(100_000);
        assert_eq!(canonical(&big), "Nguye\u{302}\u{303}n Va\u{306}n ".repeat(100_000));
        // A cluster of 33 marks is longer than the cap: returned in input order.
        let marks: String = "\u{302}\u{323}".repeat(16) + "\u{302}";
        assert_eq!(marks.chars().count(), 33);
        let input = format!("a{marks}");
        assert_eq!(canonical(&input), input);
        // 32 marks are still reordered.
        let marks32: String = "\u{302}\u{323}".repeat(16);
        let out = canonical(&format!("a{marks32}"));
        assert_eq!(out, format!("a{}{}", "\u{323}".repeat(16), "\u{302}".repeat(16)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Small deterministic generator (xorshift64*): the repo has no proptest.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n.max(1) as u64) as usize
        }
    }

    /// The comparison before this module existed, kept as the oracle for ASCII text.
    fn old_compare_text(a: &str, b: &str) -> Ordering {
        let mut ai = a.chars().flat_map(char::to_lowercase);
        let mut bi = b.chars().flat_map(char::to_lowercase);
        loop {
            match (ai.next(), bi.next()) {
                (None, None) => return Ordering::Equal,
                (None, Some(_)) => return Ordering::Less,
                (Some(_), None) => return Ordering::Greater,
                (Some(x), Some(y)) if x != y => return x.cmp(&y),
                _ => {}
            }
        }
    }

    /// Pairs of equal text: a precomposed letter versus its base letter + marks.
    const CANONICAL_EQUAL: [(&str, &str); 8] = [
        ("ậ", "a\u{302}\u{323}"),
        ("ặ", "a\u{306}\u{323}"),
        ("ệ", "e\u{302}\u{323}"),
        ("ộ", "o\u{302}\u{323}"),
        ("ợ", "o\u{323}\u{31b}"),
        ("ự", "u\u{323}\u{31b}"),
        ("Nguyễn", "Nguye\u{302}\u{303}n"),
        ("Ánh", "A\u{301}nh"),
    ];

    fn sorted(words: &[&str]) -> Vec<String> {
        let mut v: Vec<String> = words.iter().map(|w| (*w).to_string()).collect();
        v.sort_by(|a, b| compare_text(a, b));
        v
    }

    #[test]
    fn nfc_and_nfd_compare_equal() {
        for (a, b) in CANONICAL_EQUAL {
            assert_eq!(compare_text(a, b), Ordering::Equal, "{a:?} {b:?}");
            assert_eq!(compare_text(b, a), Ordering::Equal, "{b:?} {a:?}");
            assert!(text_eq(a, b), "{a:?} {b:?}");
            assert_eq!(fold_key(a), fold_key(b), "{a:?} {b:?}");
            assert_eq!(canonical(a), canonical(b), "{a:?} {b:?}");
        }
        // The same text typed with the marks in the other order.
        assert!(text_eq("a\u{302}\u{323}", "a\u{323}\u{302}"));
        // Case and encoding together.
        assert!(text_eq("NGUYỄN", "nguye\u{302}\u{303}n"));
        assert!(text_eq("Đà Nẵng", "đa\u{300} NA\u{306}\u{303}NG"));
        // canonical_not_equal: tilde and circumflex share a combining class, so their order counts.
        assert_ne!(compare_text("ễ", "e\u{303}\u{302}"), Ordering::Equal);
        assert!(!text_eq("ễ", "e\u{303}\u{302}"));
        assert_ne!(fold_key("ễ"), fold_key("e\u{303}\u{302}"));
        // Characters whose lowercase is, or decomposes to, a letter with marks.
        assert!(text_eq("\u{212b}", "a\u{30a}")); // Angstrom sign
        assert!(text_eq("\u{212b}", "\u{c5}"));
        assert!(text_eq("\u{130}", "I\u{307}")); // I with dot above keeps its dot in both forms
        assert!(!text_eq("\u{130}", "i"));
    }

    #[test]
    fn accented_letters_sort_next_to_their_base() {
        assert_eq!(sorted(&["Zebra", "ấm", "bàn", "Ánh", "Anh", "Ấn"]), ["ấm", "Ấn", "Anh", "Ánh", "bàn", "Zebra"]);
        assert_eq!(compare_text("ấ", "ä"), Ordering::Less); // different marks stay different
        assert_eq!(compare_text("ä", "ấ"), Ordering::Greater);
        // Accents break ties between the same base letters; the unaccented letter comes first.
        // (The Root order ranks marks by code point: acute U+0301 before hook above U+0309.)
        assert_eq!(sorted(&["Ảnh", "Ánh", "Anh"]), ["Anh", "Ánh", "Ảnh"]);
        assert_eq!(sorted(&["Huỳnh", "Hoàng", "Hồ"]), ["Hồ", "Hoàng", "Huỳnh"]);
        // đ sorts with d, after it when the rest is equal: its primary is the d.
        assert_eq!(compare_text("Đặng", "Em"), Ordering::Less);
        assert_eq!(compare_text("Đặng", "Dung"), Ordering::Less); // a < u at the primary level
        assert_eq!(compare_text("Dũng", "Đào"), Ordering::Greater); // u > a
        assert_eq!(compare_text("đ", "d"), Ordering::Greater);
        assert_eq!(compare_text("đ", "e"), Ordering::Less);
        assert_eq!(compare_text("Đ", "đ"), Ordering::Equal);
        // These 23 strings differ pairwise, and the
        // weights alone (not the text tie-break) tell them apart.
        let words = ["a", "à", "ả", "ã", "á", "ạ", "ă", "ằ", "ẳ", "ẵ", "ắ", "ặ", "â", "ầ", "ẩ", "ẫ", "ấ", "ậ", "ä", "å", "ā", "ą", "ǎ"];
        assert_eq!(words.len(), 23);
        let weights: std::collections::HashSet<(Vec<u32>, Vec<u16>)> = words
            .iter()
            .map(|w| {
                let k = Root::new(w);
                (k.primary, k.secondary)
            })
            .collect();
        assert_eq!(weights.len(), words.len());
        for (i, a) in words.iter().enumerate() {
            for b in &words[i + 1..] {
                assert_ne!(compare_text(a, b), Ordering::Equal, "{a} {b}");
            }
            // Each still sorts with `a`: between `a` and `b`.
            assert_ne!(compare_text(a, "a"), Ordering::Less, "{a}");
            assert_eq!(compare_text(a, "b"), Ordering::Less, "{a}");
        }
    }

    #[test]
    fn ascii_order_is_unchanged() {
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        let alphabet: Vec<char> = " -_.,'0123456789abcXYZ~[]ABCdefgzZ".chars().collect();
        let mut words: Vec<String> = Vec::new();
        for _ in 0..200 {
            let len = rng.below(8);
            words.push((0..len).map(|_| alphabet[rng.below(alphabet.len())]).collect());
        }
        let mut new = words.clone();
        let mut old = words.clone();
        new.sort_by(|a, b| compare_text(a, b));
        old.sort_by(|a, b| old_compare_text(a, b));
        assert_eq!(new, old);
        for a in &words {
            for b in words.iter().take(40) {
                assert_eq!(compare_text(a, b), old_compare_text(a, b), "{a:?} {b:?}");
                assert_eq!(text_eq(a, b), old_compare_text(a, b) == Ordering::Equal, "{a:?} {b:?}");
            }
        }
    }

    /// A string from the pieces the Root key has to get right.
    fn random_text(rng: &mut Rng) -> String {
        const PIECES: [&str; 40] = [
            "a",
            "A",
            "b",
            "n",
            "g",
            "d",
            "Đ",
            "đ",
            "e",
            "o",
            "u",
            "y",
            "Z",
            " ",
            "-",
            "1",
            "ấ",
            "Ấ",
            "ậ",
            "ặ",
            "ễ",
            "ợ",
            "ự",
            "Ánh",
            "Nguyễn",
            "ä",
            "å",
            "ø",
            "İ",
            "ß",
            "ẞ",
            "\u{212b}",
            "\u{10ffff}",
            "й",
            "Ω",
            "z",
            "\u{302}",
            "\u{323}",
            "\u{301}",
            "\u{31b}",
        ];
        let mut s = String::new();
        for _ in 0..rng.below(6) {
            match rng.below(8) {
                0 => {
                    // a loose mark anywhere in U+0300..=U+036F
                    s.push(char::from_u32(0x300 + rng.below(0x70) as u32).unwrap_or('a'));
                }
                1 => {
                    // any scalar
                    s.push(char::from_u32(rng.below(0x3000) as u32).unwrap_or('a'));
                }
                _ => s.push_str(PIECES[rng.below(PIECES.len())]),
            }
        }
        s
    }

    /// `s` re-encoded: decomposed, with some letters upper-cased where that keeps the same
    /// lowercase (so `ß` stays `ß`: its upper case `SS` is another text).
    fn reencode(rng: &mut Rng, s: &str) -> String {
        let d = canonical(s);
        d.chars()
            .map(|c| {
                let mut up = c.to_uppercase();
                match (up.next(), up.next()) {
                    (Some(u), None) if rng.below(2) == 0 && lower_char(u) == lower_char(c) => u,
                    _ => c,
                }
            })
            .collect()
    }

    #[test]
    fn total_order_and_equality_invariants() {
        let mut rng = Rng(0x0123_4567_89ab_cdef);
        for _ in 0..2_000 {
            let a = random_text(&mut rng);
            let b = if rng.below(3) == 0 { reencode(&mut rng, &a) } else { random_text(&mut rng) };
            let c = if rng.below(3) == 0 { reencode(&mut rng, &b) } else { random_text(&mut rng) };
            let (ab, ba) = (compare_text(&a, &b), compare_text(&b, &a));
            assert_eq!(ab, ba.reverse(), "antisymmetry {a:?} {b:?}");
            assert_eq!(compare_text(&a, &a), Ordering::Equal, "reflexive {a:?}");
            let (bc, ac) = (compare_text(&b, &c), compare_text(&a, &c));
            if ab != Ordering::Greater && bc != Ordering::Greater {
                assert_ne!(ac, Ordering::Greater, "transitivity {a:?} {b:?} {c:?}");
            }
            if ab == Ordering::Less && bc == Ordering::Less {
                assert_eq!(ac, Ordering::Less, "transitivity {a:?} {b:?} {c:?}");
            }
            if ab == Ordering::Equal && bc == Ordering::Equal {
                assert_eq!(ac, Ordering::Equal, "equality transitivity {a:?} {b:?} {c:?}");
            }
            for (x, y, o) in [(&a, &b, ab), (&b, &c, bc), (&a, &c, ac)] {
                let eq = o == Ordering::Equal;
                assert_eq!(eq, text_eq(x, y), "text_eq {x:?} {y:?}");
                assert_eq!(eq, fold_key(x) == fold_key(y), "fold_key {x:?} {y:?}");
            }
        }
    }

    #[test]
    fn re_encoding_never_changes_the_order() {
        let mut rng = Rng(0xdead_beef_cafe_f00d);
        for _ in 0..1_000 {
            let a = random_text(&mut rng);
            let b = random_text(&mut rng);
            let (ra, rb) = (reencode(&mut rng, &a), reencode(&mut rng, &b));
            assert_eq!(compare_text(&a, &b), compare_text(&ra, &rb), "{a:?} {b:?}");
            assert_eq!(compare_text(&a, &ra), Ordering::Equal, "{a:?} {ra:?}");
            assert_eq!(fold_key(&fold_key(&a)), fold_key(&a), "idempotent {a:?}");
        }
    }

    #[test]
    fn hostile_text_never_panics() {
        let marks = "a".to_string() + &"\u{323}\u{302}".repeat(5_000);
        let leading = "\u{301}\u{323}\u{302}a".to_string();
        let big = "Nguyễn Văn ".repeat(100_000);
        assert!(big.len() >= 1_000_000);
        let cases: [&str; 11] =
            ["", "\u{301}", "a\u{301}\u{301}\u{301}", "\u{10FFFF}", "İ", "ẞ", "\u{212b}", &marks, &leading, &big, "đ\u{323}\u{302}"];
        for a in cases {
            let _ = fold_key(a);
            let _ = canonical(a);
            for b in cases {
                let o = compare_text(a, b);
                assert_eq!(o, compare_text(b, a).reverse());
                assert_eq!(o == Ordering::Equal, text_eq(a, b));
            }
        }
        // The key holds at most 16 384 elements; the rest takes part through the text.
        let k = Root::new(&big);
        assert_eq!(k.primary.len(), MAX_PRIMARY);
        assert!(k.text.len() >= 1_000_000, "the text keeps the whole string");
        let almost = format!("{big}x");
        assert_ne!(compare_text(&big, &almost), Ordering::Equal);
        // A mark with nothing before it is dropped from the weights and kept in the text.
        let k = Root::new("\u{301}a");
        assert_eq!((k.primary, k.secondary), (Root::new("a").primary, Root::new("a").secondary));
        assert_ne!(compare_text("\u{301}a", "a"), Ordering::Equal);
        // A run of marks longer than the cap keeps its input order, so it is not canonical-equal
        // to the other order; it must still compare consistently.
        let m33 = format!("a{}\u{302}", "\u{302}\u{323}".repeat(16));
        let m33b = format!("a\u{323}{}", "\u{302}\u{323}".repeat(16));
        assert_eq!(compare_text(&m33, &m33b), compare_text(&m33b, &m33).reverse());
    }

    #[test]
    fn contains_folded_matches_whole_letters() {
        // A base letter or a letter with fewer marks never matches inside a marked letter.
        assert!(!contains_folded("Nguyễn", "e"));
        assert!(contains_folded("Nguyễn", "ễ"));
        assert!(contains_folded("Nguye\u{302}\u{303}n", "nguyễn"));
        assert!(contains_folded("NGUYE\u{302}\u{303}N", "Nguyễn"));
        assert!(contains_folded("Nguyễn", "NGUYE\u{302}\u{303}N"));
        assert!(!contains_folded("Ánh", "a"));
        assert!(contains_folded("Ánh", "á"));
        assert!(contains_folded("A\u{301}nh", "Á"));
        assert!(!contains_folded("ế", "é"));
        assert!(!contains_folded("ễ", "ê"));
        assert!(contains_folded("Hà Nội và Hà Giang", "giang"));
        assert!(contains_folded("ậ", "a\u{302}\u{323}")); // marks typed in the other order
        assert!(contains_folded("abc", ""));
        assert!(contains_folded("", ""));
        assert!(!contains_folded("", "a"));
        // A rejected occurrence does not hide a later whole-letter one.
        assert!(contains_folded("ễ e", "e"));
        assert!(contains_folded("Ánh Anh", "anh"));
        // A needle that starts with a mark is taken as given.
        assert!(contains_folded("ễ", "\u{303}"));
        assert!(!contains_folded("abc", "abcd"));
        // An overlapping later occurrence is found after a rejected one.
        assert!(contains_folded("e\u{302}e\u{302}e", "e\u{302}e"));
        // ASCII is plain case-insensitive substring search.
        assert!(contains_folded("Hello World", "O W"));
        assert!(!contains_folded("Hello", "xyz"));
    }

    #[test]
    fn starts_with_folded_ends_on_a_whole_letter() {
        assert!(starts_with_folded("Nguyễn Văn", "nguyễn"));
        assert!(starts_with_folded("NGUYE\u{302}\u{303}N", "Nguyễn"));
        assert!(starts_with_folded("Ánh", "A\u{301}"));
        // The prefix must not stop before the marks of its last letter.
        assert!(!starts_with_folded("Ánh", "a"));
        assert!(!starts_with_folded("ễn", "ê"));
        assert!(starts_with_folded("Anh", "a"));
        assert!(starts_with_folded("Đà Nẵng", "đà"));
        assert!(!starts_with_folded("Đà Nẵng", "da"));
        assert!(starts_with_folded("abc", ""));
        assert!(starts_with_folded("\u{301}a", ""));
        assert!(starts_with_folded("", ""));
        assert!(!starts_with_folded("ab", "abc"));
        assert!(starts_with_folded("Hello", "HE"));
    }

    #[test]
    fn ends_with_folded_starts_on_a_base_letter() {
        assert!(ends_with_folded("Nguyễn", "ễn"));
        assert!(ends_with_folded("Nguye\u{302}\u{303}n", "YỄN"));
        assert!(ends_with_folded("Ngọc Ánh", "ánh"));
        assert!(ends_with_folded("Ánh", "nh"));
        assert!(!ends_with_folded("Ánh", "anh"));
        assert!(!ends_with_folded("Anh", "ánh"));
        // The suffix has to start at a base character, not inside a letter's marks: a needle
        // that starts with a mark matches only text that itself starts with that mark.
        assert!(!ends_with_folded("ễn", "\u{303}n"));
        assert!(!ends_with_folded("é", "\u{301}"));
        assert!(ends_with_folded("\u{301}", "\u{301}"));
        assert!(ends_with_folded("\u{301}x", "\u{301}X"));
        assert!(ends_with_folded("abc", ""));
        assert!(ends_with_folded("", ""));
        assert!(!ends_with_folded("bc", "abc"));
        assert!(ends_with_folded("Hello", "LO"));
    }

    #[test]
    fn folded_search_is_encoding_blind_and_never_panics() {
        let marks = "a".to_string() + &"\u{323}\u{302}".repeat(5_000);
        let big = "Nguyễn Văn ".repeat(100_000);
        let cases: [&str; 10] = ["", "\u{301}", "a\u{301}\u{301}", "\u{10FFFF}", "İ", "ẞ", "ss", &marks, &big, "e\u{302}"];
        for h in cases {
            for n in cases {
                let _ = (contains_folded(h, n), starts_with_folded(h, n), ends_with_folded(h, n));
            }
        }
        assert!(contains_folded(&big, "VĂN NGUYỄN"));
        assert!(!contains_folded(&big, "van"));
        assert!(!contains_folded(&marks, "\u{323}\u{302}a"));
        // NFC and NFD haystacks and needles give the same answers.
        for (h, n) in [("Nguyễn", "ễ"), ("Nguyễn", "e"), ("Ánh", "á"), ("Đà Nẵng", "nẵng"), ("Đà Nẵng", "nang")] {
            let (hd, nd) = (canonical(h), canonical(n));
            for (a, b) in [(h, n), (hd.as_str(), n), (h, nd.as_str()), (hd.as_str(), nd.as_str())] {
                assert_eq!(contains_folded(a, b), contains_folded(h, n), "{a:?} {b:?}");
                assert_eq!(starts_with_folded(a, b), starts_with_folded(h, n), "{a:?} {b:?}");
                assert_eq!(ends_with_folded(a, b), ends_with_folded(h, n), "{a:?} {b:?}");
            }
        }
    }

    #[test]
    fn text_key_orders_exactly_like_compare_text() {
        let mut rng = Rng(0x2545_f491_4f6c_dd1d);
        let alphabet: Vec<char> = " -_.,'0123456789abcXYZ~[]ABCdefgzZ".chars().collect();
        let ascii = |rng: &mut Rng| -> String { (0..rng.below(8)).map(|_| alphabet[rng.below(alphabet.len())]).collect() };
        for i in 0..20_000 {
            // ASCII against ASCII (compare_text's fast path), mixed, and fully random text.
            let (a, b) = match i % 3 {
                0 => (ascii(&mut rng), ascii(&mut rng)),
                1 => (ascii(&mut rng), random_text(&mut rng)),
                _ => {
                    let a = random_text(&mut rng);
                    let b = if rng.below(4) == 0 { reencode(&mut rng, &a) } else { random_text(&mut rng) };
                    (a, b)
                }
            };
            let (ka, kb) = (text_key(&a), text_key(&b));
            assert_eq!(ka.cmp(&kb), compare_text(&a, &b), "{a:?} {b:?}");
            assert_eq!(ka == kb, text_eq(&a, &b), "{a:?} {b:?}");
        }
        // Case and encoding give equal keys; the hostile strings still order consistently.
        assert_eq!(text_key("NGUYỄN"), text_key("nguye\u{302}\u{303}n"));
        let big = "Nguyễn Văn ".repeat(100_000);
        let cases = ["", "\u{301}", "\u{10FFFF}", "İ", "ẞ", "Zebra", "ấm", &big];
        for a in cases {
            for b in cases {
                assert_eq!(text_key(a).cmp(&text_key(b)), compare_text(a, b), "{a:.20?} {b:.20?}");
            }
        }
        let mut words = vec!["Zebra", "ấm", "bàn", "Ánh", "Anh", "Ấn"];
        words.sort_by_cached_key(|w| text_key(w));
        assert_eq!(words, ["ấm", "Ấn", "Anh", "Ánh", "bàn", "Zebra"]);
    }

    #[test]
    fn prefolded_search_equals_folded_search() {
        let cases = ["", "\u{301}", "e", "ễ", "Nguyễn", "NGUYE\u{302}\u{303}N", "ánh", "\u{303}n", "Đà Nẵng", "đà"];
        for h in cases {
            for n in cases {
                let k = fold_key(n);
                assert_eq!(contains_prefolded(h, &k), contains_folded(h, n), "{h:?} {n:?}");
                assert_eq!(starts_with_prefolded(h, &k), starts_with_folded(h, n), "{h:?} {n:?}");
                assert_eq!(ends_with_prefolded(h, &k), ends_with_folded(h, n), "{h:?} {n:?}");
            }
        }
    }

    fn folds() -> usize {
        FOLDS.with(std::cell::Cell::get)
    }

    fn search_steps() -> usize {
        SEARCH_STEPS.with(std::cell::Cell::get)
    }

    /// The search must stay linear on text a workbook can hold. Restarting [`str::find`] after
    /// each rejected occurrence was quadratic for one shape: a needle of combining marks inside a
    /// longer run of the same mark, where every occurrence is followed by a mark and so rejected,
    /// and each restart pays for the needle again. Measured on the shape below, the old loop grew
    /// ×4 per doubling (0.3 M byte comparisons at 1,000 marks, 1.0 M at 2,000, 4.0 M at 4,000,
    /// 16.0 M at 8,000); at Excel's 32,767-character limit that is ~537 M for a single cell, so a
    /// filter over a column of them stops responding.
    ///
    /// The budget is in **steps, not seconds**: a time limit would go red on a slow machine and
    /// green on a fast one for the same algorithm. Linear means the whole search costs a small
    /// multiple of the text it read; the old loop misses this budget by four orders of magnitude.
    #[test]
    fn contains_is_linear_on_a_run_of_combining_marks() {
        const N: usize = 20_000;
        let hay = "\u{301}".repeat(N);
        let needle = "\u{301}".repeat(N / 2);
        let folded = fold_key(&needle);
        let before = search_steps();
        // The last occurrence ends at the end of the text, where no mark follows it.
        assert!(contains_prefolded(&hay, &folded), "the needle is in the haystack");
        let used = search_steps().saturating_sub(before);
        let budget = 4 * (hay.len() + folded.len());
        assert!(used <= budget, "search took {used} steps, budget {budget}");
    }

    #[test]
    fn text_eq_prefolded_equals_text_eq() {
        let mut rng = Rng(0x5eed_0f0f_d1ab_1e00);
        // Every letter the tables decompose, both cases, against its NFD spelling and its
        // neighbours: the streaming fold must agree with `fold_key` letter by letter.
        let letters: Vec<char> = DECOMPOSITION.iter().map(|&(c, _)| c).collect();
        for &c in &letters {
            for x in [c, c.to_uppercase().next().unwrap_or(c), lower_char(c)] {
                let x = x.to_string();
                for y in [x.clone(), canonical(&x), x.to_uppercase(), "a".to_string(), letters[rng.below(letters.len())].to_string()] {
                    assert_eq!(text_eq_prefolded(&x, &fold_key(&y)), text_eq(&x, &y), "{x:?} {y:?}");
                    assert_eq!(text_eq_prefolded(&y, &fold_key(&x)), text_eq(&y, &x), "{y:?} {x:?}");
                }
            }
        }
        // Random text (loose marks, any scalar, Vietnamese pieces) against re-encoded and
        // re-cased copies of itself, prefixes, and other random text.
        for i in 0..30_000 {
            let a = random_text(&mut rng);
            let b = match i % 4 {
                0 => reencode(&mut rng, &a),
                1 => a.to_uppercase(),
                2 => a.chars().take(rng.below(4)).collect(),
                _ => random_text(&mut rng),
            };
            assert_eq!(text_eq_prefolded(&a, &fold_key(&b)), text_eq(&a, &b), "{a:?} {b:?}");
            assert_eq!(text_eq_prefolded(&b, &fold_key(&a)), text_eq(&a, &b), "{b:?} {a:?}");
        }
        let marks = "a".to_string() + &"\u{323}\u{302}".repeat(5_000);
        let big = "Nguyễn Văn ".repeat(100_000);
        let cases: [&str; 10] = ["", "\u{301}", "a\u{301}\u{301}", "\u{10FFFF}", "İ", "i\u{307}", "\u{212b}", "å", &marks, &big];
        for a in cases {
            for b in cases {
                assert_eq!(text_eq_prefolded(a, &fold_key(b)), text_eq(a, b), "{a:.20?} {b:.20?}");
            }
        }
    }

    #[test]
    fn text_set_insert_reports_new_texts_only() {
        let mut set = TextSet::new(["Đà Nẵng"]);
        assert!(!set.insert(&canonical("ĐÀ NẴNG")), "another spelling of a member is not new");
        assert!(set.insert("Hà Nội"));
        assert!(!set.insert("hà nội"));
        assert_eq!(set.len(), 2);
        assert!(set.contains(&canonical("Hà Nội")));
    }

    #[test]
    fn text_set_folds_each_text_once() {
        let names: Vec<String> = (0..2_000).map(|i| format!("Nguyễn Văn Ánh {i}")).collect();
        let before = folds();
        let set = TextSet::new(names.iter().skip(1));
        assert_eq!(folds() - before, names.len() - 1, "one fold per value put in");
        assert_eq!(set.len(), names.len() - 1);
        let before = folds();
        let hits = names.iter().filter(|n| set.contains(n)).count();
        assert_eq!(hits, names.len() - 1);
        assert_eq!(folds() - before, names.len(), "one fold per probe");
        // The same relation as `text_eq`: case and NFD spellings are members, accents are kept.
        assert!(set.contains(&canonical(&names[1].to_uppercase())));
        assert!(!set.contains("Nguyen Van Anh 1"));
        assert!(!set.contains(&names[0]));
        assert!(TextSet::new(["Hà", "HA\u{300}"]).len() == 1 && TextSet::default().is_empty());
        // Comparing with a folded needle folds nothing when the text has no loose marks.
        let k = fold_key("Nguyễn Văn Ánh");
        let before = folds();
        assert!(text_eq_prefolded("NGUYỄN VĂN ÁNH", &k));
        assert!(!text_eq_prefolded("Nguyễn Văn Anh", &k));
        assert_eq!(folds(), before);
    }
}
