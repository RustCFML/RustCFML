//! Character encodings for the CFML surface that names one: `<cffile
//! charset=>`, `fileRead`/`fileWrite`/`fileAppend`, and
//! `charsetEncode`/`charsetDecode`.
//!
//! Everything used to be UTF-8: the tag dropped `charset` at lowering, the file
//! BIFs ignored a charset argument, and `charsetEncode`/`charsetDecode` were
//! pass-through no-ops that returned the input's UTF-8 bytes whatever encoding
//! was asked for. A caller who asked for UTF-16 got UTF-8 and no warning.
//!
//! Behaviour probed against Lucee 7.0.4 (`aé€` = U+0061 U+00E9 U+20AC):
//!
//! | charset | bytes written |
//! |---|---|
//! | `utf-8` | `61 C3 A9 E2 82 AC` — no BOM |
//! | `utf-16` | `FE FF` BOM then UTF-16**BE** |
//! | `utf-16be` / `utf-16le` | no BOM |
//! | `iso-8859-1` | `61 E9 3F` — an unmappable character becomes `?` |
//! | `windows-1252` | `61 E9 80` — `€` is 0x80 in cp1252 |
//! | `us-ascii` | `61 3F 3F` |
//!
//! Reading with the matching charset round-trips exactly. A **BOM wins over the
//! requested charset**: Lucee reads the BOM'd UTF-16 file correctly even when
//! asked for `utf-8`, and with no charset at all. Undecodable bytes become
//! U+FFFD rather than raising. `fileAppend` with a charset appends that
//! encoding in full, BOM included — Lucee does not suppress a second BOM.

/// The encodings the engine can name. Deliberately the set Java/Lucee callers
/// actually use; anything else is reported as unknown rather than silently
/// treated as UTF-8.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Charset {
    /// UTF-8, no BOM emitted (a BOM present on read is still honoured).
    Utf8,
    /// `UTF-16` — writes a big-endian BOM, reads the endianness from the BOM.
    Utf16Bom,
    Utf16Be,
    Utf16Le,
    /// ISO-8859-1 (Latin-1), a straight 1:1 map of U+0000..U+00FF.
    Latin1,
    /// windows-1252 — Latin-1 plus the 0x80..0x9F punctuation block.
    Cp1252,
    UsAscii,
}

/// windows-1252's 0x80..0x9F range, the only place it differs from Latin-1.
/// `\u{FFFD}` marks the five unassigned positions (0x81, 0x8D, 0x8F, 0x90, 0x9D).
const CP1252_HIGH: [char; 32] = [
    '\u{20AC}', '\u{FFFD}', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}', '\u{2020}', '\u{2021}',
    '\u{02C6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{FFFD}', '\u{017D}', '\u{FFFD}',
    '\u{FFFD}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}', '\u{2013}', '\u{2014}',
    '\u{02DC}', '\u{2122}', '\u{0161}', '\u{203A}', '\u{0153}', '\u{FFFD}', '\u{017E}', '\u{0178}',
];

/// Resolve a CFML/Java charset name. Case- and separator-insensitive, so
/// `UTF-8`, `utf8` and `UTF_8` all land on the same encoding. `None` means the
/// name is unknown — callers surface that as an error instead of defaulting.
pub fn resolve(name: &str) -> Option<Charset> {
    let key: String = name
        .chars()
        .filter(|c| !matches!(c, '-' | '_' | ' '))
        .flat_map(|c| c.to_lowercase())
        .collect();
    Some(match key.as_str() {
        "utf8" | "utf8bom" => Charset::Utf8,
        "utf16" | "unicode" => Charset::Utf16Bom,
        "utf16be" | "unicodebigunmarked" | "unicodebig" => Charset::Utf16Be,
        "utf16le" | "unicodelittleunmarked" | "unicodelittle" => Charset::Utf16Le,
        "iso88591" | "latin1" | "l1" | "88591" | "cp819" | "isolatin1" => Charset::Latin1,
        "windows1252" | "cp1252" | "ansi" => Charset::Cp1252,
        "usascii" | "ascii" | "iso646us" | "ansix3.4-1968" | "ansix3.41968" => Charset::UsAscii,
        _ => return None,
    })
}

/// Encode text. A character the target cannot represent becomes `?`, which is
/// what Java's encoders (and therefore Lucee) substitute.
pub fn encode(text: &str, cs: Charset) -> Vec<u8> {
    match cs {
        Charset::Utf8 => text.as_bytes().to_vec(),
        Charset::Utf16Bom => {
            let mut out = vec![0xFE, 0xFF];
            out.extend(encode_utf16(text, true));
            out
        }
        Charset::Utf16Be => encode_utf16(text, true),
        Charset::Utf16Le => encode_utf16(text, false),
        Charset::Latin1 => text
            .chars()
            .map(|c| if (c as u32) <= 0xFF { c as u8 } else { b'?' })
            .collect(),
        Charset::Cp1252 => text
            .chars()
            .map(|c| {
                let code = c as u32;
                if code <= 0x7F || (0xA0..=0xFF).contains(&code) {
                    return code as u8;
                }
                // The 0x80..0x9F block, matched back through the decode table.
                match CP1252_HIGH.iter().position(|&h| h == c && h != '\u{FFFD}') {
                    Some(i) => 0x80 + i as u8,
                    None => b'?',
                }
            })
            .collect(),
        Charset::UsAscii => text
            .chars()
            .map(|c| if (c as u32) <= 0x7F { c as u8 } else { b'?' })
            .collect(),
    }
}

fn encode_utf16(text: &str, big_endian: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() * 2);
    for unit in text.encode_utf16() {
        let [hi, lo] = unit.to_be_bytes();
        if big_endian {
            out.push(hi);
            out.push(lo);
        } else {
            out.push(lo);
            out.push(hi);
        }
    }
    out
}

/// Decode bytes. Undecodable input becomes U+FFFD rather than an error, as
/// Java's lenient CharsetDecoder does.
///
/// A **byte-order mark wins over `cs`**: Lucee decodes a BOM'd UTF-16 file
/// correctly even when the caller asks for UTF-8 (probed), so a file written by
/// one engine reads back on the other regardless of the declared charset. Only
/// the single-byte encodings, which cannot carry a BOM, skip the sniff.
pub fn decode(bytes: &[u8], cs: Charset) -> String {
    match cs {
        Charset::Latin1 => bytes.iter().map(|&b| b as char).collect(),
        Charset::Cp1252 => bytes
            .iter()
            .map(|&b| {
                if (0x80..=0x9F).contains(&b) {
                    CP1252_HIGH[(b - 0x80) as usize]
                } else {
                    b as char
                }
            })
            .collect(),
        Charset::UsAscii => bytes
            .iter()
            .map(|&b| if b <= 0x7F { b as char } else { '\u{FFFD}' })
            .collect(),
        Charset::Utf8 | Charset::Utf16Bom | Charset::Utf16Be | Charset::Utf16Le => {
            match sniff_bom(bytes) {
                Some((Charset::Utf16Be, skip)) => decode_utf16(&bytes[skip..], true),
                Some((Charset::Utf16Le, skip)) => decode_utf16(&bytes[skip..], false),
                Some((_, skip)) => String::from_utf8_lossy(&bytes[skip..]).into_owned(),
                None => match cs {
                    Charset::Utf16Be | Charset::Utf16Bom => decode_utf16(bytes, true),
                    Charset::Utf16Le => decode_utf16(bytes, false),
                    _ => String::from_utf8_lossy(bytes).into_owned(),
                },
            }
        }
    }
}

/// Incremental form of [`decode`], for a reader that must yield text before it
/// has seen the whole file — `loop file=`, whose entire purpose is to not hold
/// the file (GH #367).
///
/// Byte blocks arrive in order and text comes out. Two things make this more
/// than a per-block `decode` call: a multi-byte character can straddle a block
/// boundary (so an incomplete tail is held back until the next block completes
/// it), and a byte-order mark is only meaningful at the very start of the
/// stream (so the sniff happens once, not per block).
///
/// Undecodable input becomes U+FFFD exactly as [`decode`] does — including a
/// tail still incomplete at end of stream, which is what a truncated file has.
pub struct StreamDecoder {
    cs: Charset,
    /// Bytes received but not yet decodable — an incomplete trailing character.
    /// Bounded by the longest encoded character (4 bytes), except before the
    /// BOM sniff, where it holds at most the BOM's 3.
    tail: Vec<u8>,
    /// Whether the leading BOM has been looked for yet.
    sniffed: bool,
}

impl StreamDecoder {
    pub fn new(cs: Charset) -> Self {
        StreamDecoder { cs, tail: Vec::new(), sniffed: false }
    }

    /// Decode what `bytes` completes, holding back an incomplete tail.
    pub fn push(&mut self, bytes: &[u8]) -> String {
        let mut out = String::new();
        self.push_into(bytes, &mut out);
        out
    }

    /// [`push`](Self::push) appending into a caller's buffer.
    ///
    /// The reason this exists rather than just `push`: a UTF-8 (or ASCII)
    /// block that is wholly valid — every block of a plain text file — appends
    /// with no allocation and no copy beyond the `push_str`, which is what a
    /// million-line `loop file=` does on every block. Going through an
    /// intermediate `String` cost ~10% of the line loop's wall clock.
    pub fn push_into(&mut self, bytes: &[u8], out: &mut String) {
        if self.sniffed && self.tail.is_empty() && matches!(self.cs, Charset::Utf8) {
            match std::str::from_utf8(bytes) {
                Ok(text) => {
                    out.push_str(text);
                    return;
                }
                // Only the FINAL character is incomplete: emit the rest and
                // hold those bytes for the next block.
                Err(e) if e.error_len().is_none() => {
                    let valid = e.valid_up_to();
                    if let Ok(text) = std::str::from_utf8(&bytes[..valid]) {
                        out.push_str(text);
                        self.tail.extend_from_slice(&bytes[valid..]);
                        return;
                    }
                }
                // A genuinely invalid byte in the middle — fall through to the
                // lossy path below, which substitutes U+FFFD.
                Err(_) => {}
            }
        }
        self.push_slow(bytes, out);
    }

    fn push_slow(&mut self, bytes: &[u8], out: &mut String) {
        self.tail.extend_from_slice(bytes);
        // A single-byte encoding cannot carry a BOM, and `decode` does not
        // sniff for one there — so neither does this, or a Latin-1 file whose
        // first two bytes happen to be `FE FF` would lose them as a "mark".
        if !self.sniffed && matches!(self.cs, Charset::Latin1 | Charset::Cp1252 | Charset::UsAscii)
        {
            self.sniffed = true;
        }
        if !self.sniffed {
            // The BOM wins over the declared charset (see `decode`), but only
            // once the first bytes could not be a *prefix* of one — otherwise a
            // block boundary landing inside `FE FF` would be read as content.
            if let Some((bom_cs, skip)) = sniff_bom(&self.tail) {
                self.cs = bom_cs;
                self.tail.drain(..skip);
                self.sniffed = true;
            } else if self.tail.len() >= 3 || !could_start_bom(&self.tail) {
                self.sniffed = true;
            } else {
                // Still ambiguous — wait for more bytes rather than guess.
                return;
            }
        }
        let split = self.decodable_prefix_len();
        let ready: Vec<u8> = self.tail.drain(..split).collect();
        out.push_str(&self.decode_whole(&ready));
    }

    /// Decode whatever is left, U+FFFD-ing an incomplete final character.
    pub fn finish(&mut self) -> String {
        let rest: Vec<u8> = std::mem::take(&mut self.tail);
        if rest.is_empty() {
            return String::new();
        }
        self.decode_whole(&rest)
    }

    /// `decode` minus the BOM sniff, which this type does once for the stream.
    fn decode_whole(&self, bytes: &[u8]) -> String {
        match self.cs {
            Charset::Latin1 => bytes.iter().map(|&b| b as char).collect(),
            Charset::Cp1252 => bytes
                .iter()
                .map(|&b| {
                    if (0x80..=0x9F).contains(&b) {
                        CP1252_HIGH[(b - 0x80) as usize]
                    } else {
                        b as char
                    }
                })
                .collect(),
            Charset::UsAscii => bytes
                .iter()
                .map(|&b| if b <= 0x7F { b as char } else { '\u{FFFD}' })
                .collect(),
            Charset::Utf16Be | Charset::Utf16Bom => decode_utf16(bytes, true),
            Charset::Utf16Le => decode_utf16(bytes, false),
            Charset::Utf8 => String::from_utf8_lossy(bytes).into_owned(),
        }
    }

    /// How much of `tail` ends on a character boundary and can be decoded now.
    fn decodable_prefix_len(&self) -> usize {
        let tail = &self.tail;
        match self.cs {
            // Every byte is one character.
            Charset::Latin1 | Charset::Cp1252 | Charset::UsAscii => tail.len(),
            Charset::Utf8 => match std::str::from_utf8(tail) {
                Ok(_) => tail.len(),
                Err(e) => match e.error_len() {
                    // A genuinely invalid byte: decode through it so the
                    // U+FFFD comes out now rather than blocking the stream.
                    Some(_) => tail.len(),
                    // Incomplete final character — hold it for the next block.
                    None => e.valid_up_to(),
                },
            },
            Charset::Utf16Be | Charset::Utf16Bom | Charset::Utf16Le => {
                let whole = tail.len() - tail.len() % 2;
                // Do not split a surrogate pair: a trailing HIGH surrogate
                // needs its LOW half, which is in the next block.
                if whole >= 2 {
                    let (a, b) = (tail[whole - 2], tail[whole - 1]);
                    let unit = if matches!(self.cs, Charset::Utf16Le) {
                        u16::from_le_bytes([a, b])
                    } else {
                        u16::from_be_bytes([a, b])
                    };
                    if (0xD800..0xDC00).contains(&unit) {
                        return whole - 2;
                    }
                }
                whole
            }
        }
    }
}

/// Whether `bytes` could still be the start of a byte-order mark. Only called
/// with fewer than 3 bytes in hand.
fn could_start_bom(bytes: &[u8]) -> bool {
    matches!(bytes, [] | [0xEF] | [0xEF, 0xBB] | [0xFE] | [0xFF])
}

/// The encoding a leading byte-order mark declares, plus its length in bytes.
fn sniff_bom(bytes: &[u8]) -> Option<(Charset, usize)> {
    match bytes {
        [0xEF, 0xBB, 0xBF, ..] => Some((Charset::Utf8, 3)),
        [0xFE, 0xFF, ..] => Some((Charset::Utf16Be, 2)),
        [0xFF, 0xFE, ..] => Some((Charset::Utf16Le, 2)),
        _ => None,
    }
}

fn decode_utf16(bytes: &[u8], big_endian: bool) -> String {
    let units: Vec<u16> = bytes
        .chunks(2)
        .map(|pair| match pair {
            [a, b] if big_endian => u16::from_be_bytes([*a, *b]),
            [a, b] => u16::from_le_bytes([*a, *b]),
            // A trailing odd byte cannot form a unit — U+FFFD, as Java does.
            _ => 0xFFFD,
        })
        .collect();
    String::from_utf16_lossy(&units)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `StreamDecoder` fed one byte at a time must produce exactly what
    /// `decode` produces from the whole slice — that equivalence is the only
    /// reason `loop file=` can stream and still agree with `fileRead`.
    #[test]
    fn stream_decoder_matches_whole_slice_decode() {
        let samples: Vec<(Charset, Vec<u8>)> = vec![
            // ASCII, multi-byte UTF-8, a 4-byte astral char, and a truncated
            // final character.
            (Charset::Utf8, b"hello\nworld".to_vec()),
            (Charset::Utf8, "a\u{E9}\u{20AC}\u{1F600}z".as_bytes().to_vec()),
            (Charset::Utf8, vec![0x61, 0xE2, 0x82]),
            // Invalid UTF-8 mid-stream becomes U+FFFD, as Lucee's lenient
            // decoder does (a Latin-1 file read as UTF-8 is the common case).
            (Charset::Utf8, vec![0x63, 0x61, 0x66, 0xE9, 0x20, 0x6E, 0x61, 0xEF, 0x76, 0x65]),
            (Charset::Latin1, vec![0x63, 0x61, 0x66, 0xE9, 0x0A, 0xFE, 0xFF, 0x41]),
            (Charset::Cp1252, vec![0x61, 0x80, 0x9D, 0xE9]),
            (Charset::UsAscii, vec![0x61, 0xFF, 0x62]),
            // UTF-16 with a BOM, without one, and with a surrogate pair that a
            // one-byte-at-a-time feed must not split.
            (Charset::Utf16Bom, encode("a\u{E9}\u{1F600}", Charset::Utf16Bom)),
            (Charset::Utf16Be, encode("a\u{E9}\u{1F600}", Charset::Utf16Be)),
            (Charset::Utf16Le, encode("a\u{E9}\u{1F600}", Charset::Utf16Le)),
            // A UTF-8 BOM is dropped; and a BOM'd UTF-16 file wins over a
            // declared UTF-8, which is what Lucee does.
            (Charset::Utf8, encode("a\u{E9}", Charset::Utf8)),
            (Charset::Utf8, encode("a\u{E9}", Charset::Utf16Bom)),
        ];
        for (cs, bytes) in samples {
            let want = decode(&bytes, cs);
            for block in [1usize, 2, 3, 5, bytes.len().max(1)] {
                let mut dec = StreamDecoder::new(cs);
                let mut got = String::new();
                for part in bytes.chunks(block) {
                    got.push_str(&dec.push(part));
                }
                got.push_str(&dec.finish());
                assert_eq!(
                    got, want,
                    "charset {:?}, block size {}, bytes {:02X?}",
                    cs, block, bytes
                );
            }
        }
    }

    /// The byte sequences in the module docs, taken from Lucee 7.0.4.
    #[test]
    fn encodes_like_lucee() {
        let text = "a\u{E9}\u{20AC}";
        assert_eq!(encode(text, Charset::Utf8), vec![0x61, 0xC3, 0xA9, 0xE2, 0x82, 0xAC]);
        assert_eq!(
            encode(text, Charset::Utf16Bom),
            vec![0xFE, 0xFF, 0x00, 0x61, 0x00, 0xE9, 0x20, 0xAC]
        );
        assert_eq!(encode(text, Charset::Utf16Be), vec![0x00, 0x61, 0x00, 0xE9, 0x20, 0xAC]);
        assert_eq!(encode(text, Charset::Utf16Le), vec![0x61, 0x00, 0xE9, 0x00, 0xAC, 0x20]);
        assert_eq!(encode(text, Charset::Latin1), vec![0x61, 0xE9, b'?']);
        assert_eq!(encode(text, Charset::Cp1252), vec![0x61, 0xE9, 0x80]);
        assert_eq!(encode(text, Charset::UsAscii), vec![0x61, b'?', b'?']);
    }

    #[test]
    fn round_trips_each_encoding() {
        let text = "a\u{E9}\u{20AC}";
        for cs in [Charset::Utf8, Charset::Utf16Bom, Charset::Utf16Be, Charset::Utf16Le] {
            assert_eq!(decode(&encode(text, cs), cs), text, "{cs:?} did not round-trip");
        }
        // cp1252 covers all three characters; latin1/ascii lose the euro to `?`.
        assert_eq!(decode(&encode(text, Charset::Cp1252), Charset::Cp1252), text);
        assert_eq!(decode(&encode(text, Charset::Latin1), Charset::Latin1), "a\u{E9}?");
    }

    /// A BOM wins over the requested charset — that is what makes a file
    /// written as `utf-16` readable by a caller who asks for `utf-8`.
    #[test]
    fn bom_overrides_the_requested_charset() {
        let bom_utf16 = encode("a\u{E9}\u{20AC}", Charset::Utf16Bom);
        assert_eq!(decode(&bom_utf16, Charset::Utf8), "a\u{E9}\u{20AC}");
        let bom_utf8 = {
            let mut v = vec![0xEF, 0xBB, 0xBF];
            v.extend(encode("hi", Charset::Utf8));
            v
        };
        assert_eq!(decode(&bom_utf8, Charset::Utf8), "hi");
    }

    #[test]
    fn undecodable_bytes_become_replacement_chars_not_errors() {
        assert_eq!(decode(&[0x00, 0x61, 0x00], Charset::Utf8), "\u{0}a\u{0}");
        assert_eq!(decode(&[0xFF, 0x00], Charset::UsAscii), "\u{FFFD}\u{0}");
        // A lone trailing byte cannot complete a UTF-16 unit.
        assert_eq!(decode(&[0x00, 0x61, 0x00], Charset::Utf16Be), "a\u{FFFD}");
    }

    #[test]
    fn resolves_names_and_aliases_and_rejects_unknowns() {
        assert_eq!(resolve("UTF-8"), Some(Charset::Utf8));
        assert_eq!(resolve("utf8"), Some(Charset::Utf8));
        assert_eq!(resolve("UTF_16"), Some(Charset::Utf16Bom));
        assert_eq!(resolve("UTF-16LE"), Some(Charset::Utf16Le));
        assert_eq!(resolve("ISO-8859-1"), Some(Charset::Latin1));
        assert_eq!(resolve("Latin1"), Some(Charset::Latin1));
        assert_eq!(resolve("windows-1252"), Some(Charset::Cp1252));
        assert_eq!(resolve("US-ASCII"), Some(Charset::UsAscii));
        assert_eq!(resolve("not-a-charset"), None);
        assert_eq!(resolve(""), None);
    }
}
