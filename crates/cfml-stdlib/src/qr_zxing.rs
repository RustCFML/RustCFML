//! QR codes the way ZXing 3.2.1 draws them (GH #518).
//!
//! `qrCodeGenerate()` and the `net.glxn.qrgen` shim stand in for QRGen 2.2.0
//! over ZXing 3.2.1 — the jars Preside ships — and callers compare the result
//! byte for byte (Preside's `QrCodeGeneratorTest` checks an MD5 of a GIF).
//! The `qrcode` crate is a correct encoder, but not the same one: it segments
//! the text optimally, scores masks differently, and its callers chose their
//! own error-correction default. Every place those choices differ produces a
//! different, equally valid, symbol. So the *decisions* are ported from ZXing
//! here, and only the spec-fixed mechanics (bit packing, Reed–Solomon, module
//! placement, format/version info) are borrowed from the crate:
//!
//! * `Encoder.encode` — one mode for the whole text (`chooseMode`), ECI only
//!   for a non-default charset, the two-pass version choice, and the mask with
//!   the lowest `MaskUtil` penalty (first one wins a tie).
//! * `QRCodeWriter.renderResult` — whole-pixel modules, centred, the leftover
//!   space as white padding; the image is exactly the requested width × height
//!   (or the bare symbol plus quiet zone, if that is bigger).
//! * `GIFImageWriter` + `LZWCompressor` (OpenJDK ImageIO) — what
//!   `MatrixToImageWriter` hands a `TYPE_BYTE_BINARY` image to. Ported so the
//!   GIF is byte-identical, not merely pixel-identical.

use qrcode::bits::Bits;
use qrcode::canvas::{Canvas, MaskPattern};
use qrcode::types::{Color, EcLevel, Version};

/// ZXing `QRCodeWriter.QUIET_ZONE_SIZE`.
pub const DEFAULT_QUIET_ZONE: u32 = 4;

/// The byte-mode encoding when no charset is given (`Encoder.DEFAULT_BYTE_MODE_ENCODING`).
pub const DEFAULT_CHARSET: &str = "ISO-8859-1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Numeric,
    Alphanumeric,
    Byte,
}

impl Mode {
    /// `Mode.getCharacterCountBits`.
    fn count_bits(self, version: i16) -> usize {
        let range = if version <= 9 { 0 } else if version <= 26 { 1 } else { 2 };
        match self {
            Mode::Numeric => [10, 12, 14][range],
            Mode::Alphanumeric => [9, 11, 13][range],
            Mode::Byte => [8, 16, 16][range],
        }
    }
}

/// A finished symbol: `width × width` modules, row-major, `true` = dark.
#[derive(Debug, Clone)]
pub struct Symbol {
    pub width: usize,
    pub modules: Vec<bool>,
    pub mask: u8,
    pub version: i16,
}

/// The byte encodings a charset name selects, and the ECI value ZXing writes
/// for it. Java's charset lookup is case-insensitive, but ZXing's
/// `CharacterSetECI.getCharacterSetECIByName` is not — so `"utf-8"` encodes
/// UTF-8 bytes *without* an ECI header, exactly as it does on the JVM.
enum Charset {
    Latin1,
    Ascii,
    Utf8,
}

fn charset_for(name: &str) -> Result<(Charset, Option<u32>), String> {
    let lower = name.to_ascii_lowercase().replace('_', "-");
    let cs = match lower.as_str() {
        "iso-8859-1" | "iso8859-1" | "latin1" | "iso-latin-1" | "8859-1" | "l1" => Charset::Latin1,
        "us-ascii" | "ascii" => Charset::Ascii,
        "utf-8" | "utf8" => Charset::Utf8,
        _ => {
            return Err(format!(
                "charset [{}] is not supported for QR byte mode; use ISO-8859-1 (the default) or UTF-8",
                name
            ))
        }
    };
    // The default is checked by exact name before any ECI lookup.
    if name == DEFAULT_CHARSET {
        return Ok((cs, None));
    }
    let eci = match name {
        "ISO8859_1" | "ISO-8859-1" => Some(1),
        "ASCII" | "US-ASCII" => Some(27),
        "UTF8" | "UTF-8" => Some(26),
        _ => None,
    };
    Ok((cs, eci))
}

/// `String.getBytes(charset)`: an unmappable code point becomes `?`.
fn encode_text(text: &str, cs: &Charset) -> Vec<u8> {
    match cs {
        Charset::Utf8 => text.as_bytes().to_vec(),
        Charset::Latin1 => text.chars().map(|c| if (c as u32) < 0x100 { c as u8 } else { b'?' }).collect(),
        Charset::Ascii => text.chars().map(|c| if (c as u32) < 0x80 { c as u8 } else { b'?' }).collect(),
    }
}

/// `Encoder.getAlphanumericCode` != -1.
fn is_alphanumeric(c: char) -> bool {
    matches!(c, 'A'..='Z' | ' ' | '$' | '%' | '*' | '+' | '-' | '.' | '/' | ':')
}

/// `Encoder.chooseMode` (Shift_JIS/Kanji aside: no charset here selects it).
fn choose_mode(text: &str) -> Mode {
    let mut has_numeric = false;
    let mut has_alpha = false;
    for c in text.chars() {
        if c.is_ascii_digit() {
            has_numeric = true;
        } else if is_alphanumeric(c) {
            has_alpha = true;
        } else {
            return Mode::Byte;
        }
    }
    if has_alpha {
        Mode::Alphanumeric
    } else if has_numeric {
        Mode::Numeric
    } else {
        Mode::Byte
    }
}

fn data_bytes(version: i16, ec: EcLevel) -> usize {
    Bits::new(Version::Normal(version)).max_len(ec).unwrap_or(0) / 8
}

/// `Encoder.chooseVersion`.
fn choose_version(bits: usize, ec: EcLevel) -> Option<i16> {
    (1..=40).find(|&v| data_bytes(v, ec) >= bits.div_ceil(8))
}

/// `Encoder.encode`: the module grid ZXing would produce.
pub fn encode(text: &str, ec: EcLevel, charset: Option<&str>) -> Result<Symbol, String> {
    if text.is_empty() {
        return Err("Found empty contents".to_string());
    }
    let (cs, eci) = charset_for(charset.unwrap_or(DEFAULT_CHARSET))?;
    let mode = choose_mode(text);
    let eci = if mode == Mode::Byte { eci } else { None };
    let bytes = match mode {
        Mode::Byte => encode_text(text, &cs),
        _ => text.as_bytes().to_vec(),
    };

    let header_bits = 4 + if eci.is_some() { 12 } else { 0 };
    let n = bytes.len();
    let data_bits = match mode {
        Mode::Numeric => 10 * (n / 3) + [0, 4, 7][n % 3],
        Mode::Alphanumeric => 11 * (n / 2) + 6 * (n % 2),
        Mode::Byte => 8 * n,
    };
    let too_big = || "Data too big".to_string();
    let provisional =
        choose_version(header_bits + mode.count_bits(1) + data_bits, ec).ok_or_else(too_big)?;
    let version =
        choose_version(header_bits + mode.count_bits(provisional) + data_bits, ec).ok_or_else(too_big)?;

    let mut bits = Bits::new(Version::Normal(version));
    let fail = |e: qrcode::types::QrError| format!("data bits cannot fit in the QR Code ({:?})", e);
    if let Some(eci) = eci {
        bits.push_eci_designator(eci).map_err(fail)?;
    }
    match mode {
        Mode::Numeric => bits.push_numeric_data(&bytes),
        Mode::Alphanumeric => bits.push_alphanumeric_data(&bytes),
        Mode::Byte => bits.push_byte_data(&bytes),
    }
    .map_err(fail)?;
    bits.push_terminator(ec).map_err(fail)?;

    let data = bits.into_bytes();
    let (encoded, ec_bytes) =
        qrcode::ec::construct_codewords(&data, Version::Normal(version), ec).map_err(fail)?;
    let mut canvas = Canvas::new(Version::Normal(version), ec);
    canvas.draw_all_functional_patterns();
    canvas.draw_data(&encoded, &ec_bytes);

    const MASKS: [MaskPattern; 8] = [
        MaskPattern::Checkerboard,
        MaskPattern::HorizontalLines,
        MaskPattern::VerticalLines,
        MaskPattern::DiagonalLines,
        MaskPattern::LargeCheckerboard,
        MaskPattern::Fields,
        MaskPattern::Diamonds,
        MaskPattern::Meadow,
    ];
    let width = Version::Normal(version).width() as usize;
    let mut best: Option<(u32, u8, Vec<bool>)> = None;
    for (i, mask) in MASKS.iter().enumerate() {
        let mut c = canvas.clone();
        c.apply_mask(*mask);
        let modules: Vec<bool> = c.into_colors().into_iter().map(|m| m == Color::Dark).collect();
        let penalty = mask_penalty(&modules, width);
        // Strictly less: ZXing keeps the first mask on a tie.
        if best.as_ref().is_none_or(|(p, _, _)| penalty < *p) {
            best = Some((penalty, i as u8, modules));
        }
    }
    let (_, mask, modules) = best.expect("eight masks were scored");
    Ok(Symbol { width, modules, mask, version })
}

/// `MaskUtil` rules 1–4, exactly as ZXing 3.2.1 scores them — including its
/// rule 3, which looks for the finder-like run with *four* light modules on
/// either side, clipped at the symbol edge.
fn mask_penalty(m: &[bool], w: usize) -> u32 {
    let at = |x: usize, y: usize| m[y * w + x];

    // Rule 1: runs of five or more in a row/column.
    let mut rule1 = 0u32;
    for horizontal in [true, false] {
        for i in 0..w {
            let mut run = 0u32;
            let mut prev: Option<bool> = None;
            for j in 0..w {
                let bit = if horizontal { at(j, i) } else { at(i, j) };
                if Some(bit) == prev {
                    run += 1;
                } else {
                    if run >= 5 {
                        rule1 += 3 + (run - 5);
                    }
                    run = 1;
                    prev = Some(bit);
                }
            }
            if run >= 5 {
                rule1 += 3 + (run - 5);
            }
        }
    }

    // Rule 2: 2×2 blocks of one colour.
    let mut rule2 = 0u32;
    for y in 0..w - 1 {
        for x in 0..w - 1 {
            let v = at(x, y);
            if v == at(x + 1, y) && v == at(x, y + 1) && v == at(x + 1, y + 1) {
                rule2 += 1;
            }
        }
    }

    // Rule 3: 1:1:3:1:1 with four light modules before or after.
    const PATTERN: [bool; 7] = [true, false, true, true, true, false, true];
    let white_h = |y: usize, from: isize, to: isize| {
        (from..to).all(|i| i < 0 || i >= w as isize || !at(i as usize, y))
    };
    let white_v = |x: usize, from: isize, to: isize| {
        (from..to).all(|i| i < 0 || i >= w as isize || !at(x, i as usize))
    };
    let mut rule3 = 0u32;
    for y in 0..w {
        for x in 0..w {
            let (xi, yi) = (x as isize, y as isize);
            if x + 6 < w
                && (0..7).all(|k| at(x + k, y) == PATTERN[k])
                && (white_h(y, xi - 4, xi) || white_h(y, xi + 7, xi + 11))
            {
                rule3 += 1;
            }
            if y + 6 < w
                && (0..7).all(|k| at(x, y + k) == PATTERN[k])
                && (white_v(x, yi - 4, yi) || white_v(x, yi + 7, yi + 11))
            {
                rule3 += 1;
            }
        }
    }

    // Rule 4: dark proportion, in whole 5% steps away from half.
    let dark = m.iter().filter(|b| **b).count() as i64;
    let total = (w * w) as i64;
    let rule4 = ((dark * 2 - total).abs() * 10 / total) as u32;

    rule1 + rule2 * 3 + rule3 * 40 + rule4 * 10
}

/// `QRCodeWriter.renderResult`: a `width × height` bitmap (`true` = dark),
/// grown to the symbol-plus-quiet-zone size if the request is smaller.
pub fn render(sym: &Symbol, width: u32, height: u32, quiet_zone: u32) -> (u32, u32, Vec<bool>) {
    let input = sym.width as u32;
    let qr = input + quiet_zone * 2;
    let out_w = width.max(qr);
    let out_h = height.max(qr);
    let multiple = (out_w / qr).min(out_h / qr);
    let left = (out_w - input * multiple) / 2;
    let top = (out_h - input * multiple) / 2;
    let mut px = vec![false; (out_w * out_h) as usize];
    for my in 0..input {
        for mx in 0..input {
            if !sym.modules[(my * input + mx) as usize] {
                continue;
            }
            let (x0, y0) = (left + mx * multiple, top + my * multiple);
            for y in y0..y0 + multiple {
                let row = (y * out_w) as usize;
                px[row + x0 as usize..row + (x0 + multiple) as usize].fill(true);
            }
        }
    }
    (out_w, out_h, px)
}

/// What OpenJDK's `GIFImageWriter` writes for a `TYPE_BYTE_BINARY` image with
/// the default black/white palette and default write params, as
/// `ImageIO.write( image, "gif", stream )` does: GIF89a, a two-entry global
/// colour table (index 0 black, 1 white), no extensions, an **interlaced**
/// image, minimum code size 2, and `LZWCompressor`'s code-width schedule.
pub fn encode_gif_bilevel(width: u32, height: u32, dark: &[bool]) -> Vec<u8> {
    let mut out = Vec::with_capacity(64 + dark.len() / 8);
    out.extend_from_slice(b"GIF89a");
    out.extend_from_slice(&(width as u16).to_le_bytes());
    out.extend_from_slice(&(height as u16).to_le_bytes());
    // Global colour table, colour resolution 8 bits, 2 entries.
    out.push(0x80 | (7 << 4));
    out.push(0); // background colour index
    out.push(0); // pixel aspect ratio
    out.extend_from_slice(&[0, 0, 0, 0xFF, 0xFF, 0xFF]);

    out.push(0x2C);
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&(width as u16).to_le_bytes());
    out.extend_from_slice(&(height as u16).to_le_bytes());
    out.push(0x40); // interlaced, no local colour table

    const INIT_CODE_SIZE: u32 = 2; // 1 bit per pixel, raised to GIF's minimum
    out.push(INIT_CODE_SIZE as u8);
    let mut lzw = LzwCompressor::new(INIT_CODE_SIZE, &mut out);
    // Interlace passes: rows 0+8n, 4+8n, 2+4n, 1+2n.
    let w = width as usize;
    for (start, step) in [(0usize, 8usize), (4, 8), (2, 4), (1, 2)] {
        let mut y = start;
        while y < height as usize {
            for &d in &dark[y * w..(y + 1) * w] {
                // Palette index: 0 = black (dark), 1 = white.
                lzw.compress(if d { 0 } else { 1 }, &mut out);
            }
            y += step;
        }
    }
    lzw.flush(&mut out);
    out.push(0x00); // block terminator
    out.push(0x3B); // trailer
    out
}

/// `com.sun.imageio.plugins.common.LZWCompressor` + `BitFile` (GIF mode).
/// Its table is a `(prefix, byte) → code` dictionary; the hash probing in
/// `LZWStringTable` only finds entries, it never changes which code is found.
struct LzwCompressor {
    code_size: u32,
    clear_code: u32,
    end_of_info: u32,
    num_bits: u32,
    limit: u32,
    prefix: Option<u32>,
    table: std::collections::HashMap<(u32, u8), u32>,
    next_code: u32,
    bits: BitFile,
}

impl LzwCompressor {
    const MAX_CODES: u32 = 1 << 12;

    /// Writes the leading clear code, as the Java constructor does.
    fn new(code_size: u32, out: &mut Vec<u8>) -> Self {
        let mut c = LzwCompressor {
            code_size,
            clear_code: 1 << code_size,
            end_of_info: (1 << code_size) + 1,
            num_bits: code_size + 1,
            limit: (1 << (code_size + 1)) - 1,
            prefix: None,
            table: std::collections::HashMap::new(),
            next_code: 0,
            bits: BitFile::new(),
        };
        c.clear_table();
        c.bits.write(c.clear_code, c.num_bits, out);
        c
    }

    fn clear_table(&mut self) {
        self.table.clear();
        // The roots, plus the clear and end-of-information codes.
        self.next_code = (1 << self.code_size) + 2;
    }

    /// `LZWStringTable.addCharString`: the code just assigned, or 0xFFFF once
    /// all 4096 are used.
    fn add(&mut self, prefix: u32, b: u8) -> u32 {
        if self.next_code >= Self::MAX_CODES {
            return 0xFFFF;
        }
        let code = self.next_code;
        self.table.insert((prefix, b), code);
        self.next_code += 1;
        code
    }

    fn compress(&mut self, c: u8, out: &mut Vec<u8>) {
        let Some(prefix) = self.prefix else {
            self.prefix = Some(c as u32);
            return;
        };
        if let Some(&code) = self.table.get(&(prefix, c)) {
            self.prefix = Some(code);
            return;
        }
        self.bits.write(prefix, self.num_bits, out);
        if self.add(prefix, c) > self.limit {
            if self.num_bits == 12 {
                self.bits.write(self.clear_code, self.num_bits, out);
                self.clear_table();
                self.num_bits = self.code_size + 1;
            } else {
                self.num_bits += 1;
            }
            self.limit = (1 << self.num_bits) - 1;
        }
        self.prefix = Some(c as u32);
    }

    fn flush(&mut self, out: &mut Vec<u8>) {
        if let Some(prefix) = self.prefix {
            self.bits.write(prefix, self.num_bits, out);
        }
        self.bits.write(self.end_of_info, self.num_bits, out);
        self.bits.flush(out);
    }
}

/// `BitFile` with `blocks = true`: LSB-first codes, emitted in sub-blocks of
/// up to 255 bytes, each prefixed with its length.
struct BitFile {
    buffer: [u8; 256],
    index: usize,
    bits_left: u32,
}

impl BitFile {
    fn new() -> Self {
        BitFile { buffer: [0; 256], index: 0, bits_left: 8 }
    }

    fn write(&mut self, code: u32, num_bits: u32, out: &mut Vec<u8>) {
        let mut bits = code;
        let mut numbits = num_bits;
        loop {
            if (self.index == 254 && self.bits_left == 0) || self.index > 254 {
                out.push(255);
                out.extend_from_slice(&self.buffer[..255]);
                self.buffer[0] = 0;
                self.index = 0;
                self.bits_left = 8;
            }
            if numbits <= self.bits_left {
                self.buffer[self.index] |= ((bits & ((1 << numbits) - 1)) << (8 - self.bits_left)) as u8;
                self.bits_left -= numbits;
                numbits = 0;
            } else {
                self.buffer[self.index] |=
                    ((bits & ((1 << self.bits_left) - 1)) << (8 - self.bits_left)) as u8;
                bits >>= self.bits_left;
                numbits -= self.bits_left;
                self.index += 1;
                self.buffer[self.index] = 0;
                self.bits_left = 8;
            }
            if numbits == 0 {
                break;
            }
        }
    }

    fn flush(&mut self, out: &mut Vec<u8>) {
        let n = self.index + if self.bits_left == 8 { 0 } else { 1 };
        if n > 0 {
            out.push(n as u8);
            out.extend_from_slice(&self.buffer[..n]);
            self.buffer[0] = 0;
            self.index = 0;
            self.bits_left = 8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Encoder.encode( "hello world", L )` on ZXing 3.2.1, printed by the
    /// real jar (mask 7, version 1, byte mode).
    const HELLO_WORLD_L: [&str; 21] = [
        "111111100101101111111",
        "100000101101001000001",
        "101110101100101011101",
        "101110100101001011101",
        "101110101000101011101",
        "100000101001101000001",
        "111111101010101111111",
        "000000001111100000000",
        "110100110110001110110",
        "000101011110001110011",
        "101001100110100101101",
        "011011011111001011011",
        "111010110010110110000",
        "000000001011000000101",
        "111111101000001111110",
        "100000100111100100001",
        "101110100001010000010",
        "101110101110000111111",
        "101110100100101010101",
        "100000101001010000000",
        "111111101100101101010",
    ];

    #[test]
    fn hello_world_matches_zxing_grid() {
        let sym = encode("hello world", EcLevel::L, None).unwrap();
        assert_eq!((sym.version, sym.mask, sym.width), (1, 7, 21));
        let rows: Vec<String> = sym
            .modules
            .chunks(21)
            .map(|r| r.iter().map(|d| if *d { '1' } else { '0' }).collect())
            .collect();
        assert_eq!(rows, HELLO_WORLD_L);
    }

    #[test]
    fn render_pads_rather_than_stretches() {
        let sym = encode("hello world", EcLevel::L, None).unwrap();
        // 29 modules incl. quiet zone into 125px: 4px modules, 20px offset.
        let (w, h, px) = render(&sym, 125, 125, DEFAULT_QUIET_ZONE);
        assert_eq!((w, h), (125, 125));
        let xs: Vec<u32> = (0..h)
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .filter(|(x, y)| px[(y * w + x) as usize])
            .map(|(x, _)| x)
            .collect();
        assert_eq!(xs.iter().min(), Some(&20));
        assert_eq!(xs.iter().max(), Some(&103));
        // Smaller than the symbol: grows to symbol + quiet zone.
        assert_eq!(render(&sym, 10, 10, 4).0, 29);
    }

    #[test]
    fn gif_is_java_imageio_shape() {
        let sym = encode("hello world", EcLevel::L, None).unwrap();
        let (w, h, px) = render(&sym, 125, 125, DEFAULT_QUIET_ZONE);
        let gif = encode_gif_bilevel(w, h, &px);
        // Preside's helloWorldDefaults.gif is 797 bytes.
        assert_eq!(gif.len(), 797);
        assert_eq!(&gif[..13], b"GIF89a\x7d\x00\x7d\x00\xf0\x00\x00");
        assert_eq!(&gif[19..30], b"\x2c\x00\x00\x00\x00\x7d\x00\x7d\x00\x40\x02");
        assert_eq!(&gif[gif.len() - 2..], b"\x00\x3b");
    }
}
