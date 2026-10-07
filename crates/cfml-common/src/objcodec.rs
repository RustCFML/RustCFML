//! Compact binary encoding behind `objectSave()` / `objectLoad()` (format 2).
//!
//! Lucee's `objectSave` is Java serialization: binary, strings stored as raw
//! bytes. RustCFML has no JVM, so format 1 was JSON behind a magic header —
//! which turns every newline, tab and quote in a cached HTML page into a
//! two-byte escape that `objectLoad` must then parse and undo (34k of them in a
//! typical Preside full-page-cache entry). This format stores strings and
//! binaries as length-prefixed raw bytes, so loading is mostly copying.
//!
//! Every value is a one-byte tag and a payload; lengths and counts are LEB128
//! varints. Tags are explicit, so a user struct that happens to carry a
//! `_cftype` key round-trips as that struct (format 1 could misread it).
//!
//! Type fidelity is at least format 1's: Doubles and TimeSpans keep their type
//! (Lucee preserves them; JSON turned `1.0` into an Int). Functions, closures,
//! components and native objects are not serializable and load as null, as
//! before; a flyweight instance stores its public data as a struct, as before.
//!
//! The decoder bounds-checks every read and limits nesting, so a truncated or
//! hostile blob is an error, never a panic or a stack overflow.

use crate::dynamic::{CfmlQuery, CfmlValue, ValueMap};
use std::sync::Arc;

const T_NULL: u8 = 0;
const T_FALSE: u8 = 1;
const T_TRUE: u8 = 2;
const T_INT: u8 = 3;
const T_DOUBLE: u8 = 4;
const T_TIMESPAN: u8 = 5;
const T_STRING: u8 = 6;
const T_BINARY: u8 = 7;
const T_ARRAY: u8 = 8;
const T_STRUCT: u8 = 9;
const T_QUERY: u8 = 10;
const T_DATETIME: u8 = 11;

/// Stops a crafted blob recursing the stack away. The same limit serde_json
/// applies, so format 1 never loaded anything deeper either.
const MAX_DEPTH: usize = 128;

/// Encode `v`, appending to `out`. Errors on nesting deeper than the decoder
/// accepts — which is also how a value that contains itself is caught, rather
/// than recursing until the stack overflows.
pub fn encode(v: &CfmlValue, out: &mut Vec<u8>) -> Result<(), String> {
    enc(v, out, 0)
}

fn enc(v: &CfmlValue, out: &mut Vec<u8>, depth: usize) -> Result<(), String> {
    if depth > MAX_DEPTH {
        return Err("value is nested too deeply (or contains a reference to itself)".into());
    }
    match v {
        CfmlValue::Null => out.push(T_NULL),
        CfmlValue::Bool(false) => out.push(T_FALSE),
        CfmlValue::Bool(true) => out.push(T_TRUE),
        CfmlValue::Int(i) => {
            out.push(T_INT);
            out.extend_from_slice(&i.to_le_bytes());
        }
        CfmlValue::Double(d) => {
            out.push(T_DOUBLE);
            out.extend_from_slice(&d.to_le_bytes());
        }
        CfmlValue::TimeSpan(d) => {
            out.push(T_TIMESPAN);
            out.extend_from_slice(&d.to_le_bytes());
        }
        CfmlValue::String(s) => {
            out.push(T_STRING);
            put_bytes(out, s.as_bytes());
        }
        CfmlValue::Binary(b) => {
            out.push(T_BINARY);
            put_bytes(out, b);
        }
        CfmlValue::Array(a) => {
            let snap = a.snapshot();
            out.push(T_ARRAY);
            put_len(out, snap.len());
            for e in snap.iter() {
                enc(e, out, depth + 1)?;
            }
        }
        CfmlValue::QueryColumn(cells, _) => {
            out.push(T_ARRAY);
            put_len(out, cells.len());
            for e in cells.iter() {
                enc(e, out, depth + 1)?;
            }
        }
        CfmlValue::Struct(m) => encode_map(&m.snapshot(), out, depth)?,
        CfmlValue::Query(q) => {
            let backing = q.backing();
            let d = backing.read();
            out.push(T_QUERY);
            put_len(out, d.columns.len());
            for c in d.columns.iter() {
                put_bytes(out, c.as_bytes());
            }
            let rows = d.row_count();
            put_len(out, rows);
            for r in 0..rows {
                let row = d.row_at(r).unwrap_or_default();
                for c in d.columns.iter() {
                    match row.get(c.as_str()) {
                        Some(cell) => enc(&cell, out, depth + 1)?,
                        None => out.push(T_NULL),
                    }
                }
            }
        }
        CfmlValue::DateTime(d) => {
            out.push(T_DATETIME);
            out.extend_from_slice(&d.epoch_secs().to_le_bytes());
            out.extend_from_slice(&d.subsec_nanos().to_le_bytes());
            out.push(match d.kind() {
                crate::datetime::DateKind::DateTime => 0,
                crate::datetime::DateKind::Date => 1,
                crate::datetime::DateKind::Time => 2,
            });
        }
        #[cfg(feature = "component-instance")]
        CfmlValue::Instance(inst) => encode_map(&inst.read().public_entries(), out, depth)?,
        CfmlValue::Closure(_)
        | CfmlValue::Function(_)
        | CfmlValue::Component(_)
        | CfmlValue::NativeObject(_) => out.push(T_NULL),
    }
    Ok(())
}

fn encode_map(m: &ValueMap, out: &mut Vec<u8>, depth: usize) -> Result<(), String> {
    out.push(T_STRUCT);
    put_len(out, m.len());
    for (k, v) in m.iter() {
        put_bytes(out, k.as_bytes());
        enc(v, out, depth + 1)?;
    }
    Ok(())
}

fn put_len(out: &mut Vec<u8>, mut n: usize) {
    loop {
        let b = (n & 0x7f) as u8;
        n >>= 7;
        if n == 0 {
            out.push(b);
            return;
        }
        out.push(b | 0x80);
    }
}

fn put_bytes(out: &mut Vec<u8>, b: &[u8]) {
    put_len(out, b.len());
    out.extend_from_slice(b);
}

/// Decode one complete value; trailing bytes are an error.
pub fn decode(bytes: &[u8]) -> Result<CfmlValue, String> {
    let mut r = Reader { b: bytes, i: 0 };
    let v = r.value(0)?;
    if r.i != bytes.len() {
        return Err(format!("{} unexpected trailing bytes", bytes.len() - r.i));
    }
    Ok(v)
}

struct Reader<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.i.checked_add(n).filter(|&e| e <= self.b.len()).ok_or("truncated data")?;
        let s = &self.b[self.i..end];
        self.i = end;
        Ok(s)
    }

    fn byte(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }

    fn len(&mut self) -> Result<usize, String> {
        let mut n: usize = 0;
        for shift in (0..64).step_by(7) {
            let b = self.byte()?;
            n |= ((b & 0x7f) as usize).checked_shl(shift).ok_or("length overflow")?;
            if b & 0x80 == 0 {
                // A count can never exceed the bytes left (every item takes >= 1).
                if n > self.b.len() - self.i {
                    return Err("length exceeds data".into());
                }
                return Ok(n);
            }
        }
        Err("length overflow".into())
    }

    fn text(&mut self) -> Result<String, String> {
        let n = self.len()?;
        let raw = self.take(n)?;
        std::str::from_utf8(raw).map(str::to_owned).map_err(|_| "invalid UTF-8 in string".to_string())
    }

    fn fixed8(&mut self) -> Result<[u8; 8], String> {
        Ok(self.take(8)?.try_into().expect("8 bytes"))
    }

    fn value(&mut self, depth: usize) -> Result<CfmlValue, String> {
        if depth > MAX_DEPTH {
            return Err("nesting too deep".into());
        }
        Ok(match self.byte()? {
            T_NULL => CfmlValue::Null,
            T_FALSE => CfmlValue::Bool(false),
            T_TRUE => CfmlValue::Bool(true),
            T_INT => CfmlValue::Int(i64::from_le_bytes(self.fixed8()?)),
            T_DOUBLE => CfmlValue::Double(f64::from_le_bytes(self.fixed8()?)),
            T_TIMESPAN => CfmlValue::TimeSpan(f64::from_le_bytes(self.fixed8()?)),
            T_STRING => CfmlValue::String(Arc::new(self.text()?)),
            T_BINARY => {
                let n = self.len()?;
                CfmlValue::binary(self.take(n)?.to_vec())
            }
            T_ARRAY => {
                let n = self.len()?;
                let mut out = Vec::with_capacity(n);
                for _ in 0..n {
                    out.push(self.value(depth + 1)?);
                }
                CfmlValue::array(out)
            }
            T_STRUCT => {
                let n = self.len()?;
                let mut m = ValueMap::with_capacity_and_hasher(n, Default::default());
                for _ in 0..n {
                    let k = self.text()?;
                    let v = self.value(depth + 1)?;
                    m.insert(k, v);
                }
                CfmlValue::strukt(m)
            }
            T_QUERY => {
                let ncols = self.len()?;
                let mut cols = Vec::with_capacity(ncols);
                for _ in 0..ncols {
                    cols.push(self.text()?);
                }
                let nrows = self.len()?;
                let mut rows = Vec::with_capacity(nrows);
                for _ in 0..nrows {
                    let mut row = ValueMap::with_capacity_and_hasher(ncols, Default::default());
                    for c in &cols {
                        row.insert(c.clone(), self.value(depth + 1)?);
                    }
                    rows.push(row);
                }
                CfmlValue::Query(CfmlQuery::from_parts(cols, rows))
            }
            T_DATETIME => {
                let secs = i64::from_le_bytes(self.fixed8()?);
                let nanos = u32::from_le_bytes(self.take(4)?.try_into().expect("4 bytes"));
                let kind = match self.byte()? {
                    1 => crate::datetime::DateKind::Date,
                    2 => crate::datetime::DateKind::Time,
                    _ => crate::datetime::DateKind::DateTime,
                };
                CfmlValue::DateTime(
                    crate::datetime::CfmlDate::from_epoch(secs, nanos as i64).with_kind(kind),
                )
            }
            t => return Err(format!("unknown value tag {t}")),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(v: &CfmlValue) -> CfmlValue {
        let mut out = Vec::new();
        encode(v, &mut out).unwrap();
        decode(&out).expect("decodes")
    }

    #[test]
    fn scalars_keep_their_type() {
        for v in [
            CfmlValue::Null,
            CfmlValue::Bool(true),
            CfmlValue::Bool(false),
            CfmlValue::Int(i64::MIN),
            CfmlValue::Int(i64::MAX),
            CfmlValue::Double(1.0),
            CfmlValue::Double(-0.5),
            CfmlValue::TimeSpan(1.25),
        ] {
            let back = round_trip(&v);
            assert_eq!(format!("{back:?}"), format!("{v:?}"));
        }
    }

    #[test]
    fn strings_binaries_and_containers() {
        let html = "<p class=\"a\">line\n\ttab \\ é € 😀</p>\u{0}";
        let mut m = ValueMap::default();
        m.insert("Body".to_string(), CfmlValue::string(html));
        m.insert("_cftype".to_string(), CfmlValue::string("binary"));
        m.insert("bin".to_string(), CfmlValue::binary(vec![0, 255, 7]));
        m.insert(
            "list".to_string(),
            CfmlValue::array(vec![CfmlValue::Int(1), CfmlValue::string(""), CfmlValue::Null]),
        );
        let back = round_trip(&CfmlValue::strukt(m));
        let CfmlValue::Struct(s) = back else { panic!("struct") };
        assert_eq!(s.get("Body").unwrap().as_string(), html);
        // A user `_cftype` key stays an ordinary string key.
        assert_eq!(s.get("_cftype").unwrap().as_string(), "binary");
        match s.get("bin").unwrap() {
            CfmlValue::Binary(b) => assert_eq!(*b, vec![0, 255, 7]),
            other => panic!("{other:?}"),
        }
        let keys: Vec<String> = s.snapshot().keys().map(|k| k.to_string()).collect();
        assert_eq!(keys, ["Body", "_cftype", "bin", "list"], "key order and case kept");
    }

    #[test]
    fn query_and_date() {
        let mut r1 = ValueMap::default();
        r1.insert("id".to_string(), CfmlValue::Int(1));
        r1.insert("name".to_string(), CfmlValue::string("one \"1\""));
        let mut r2 = ValueMap::default();
        r2.insert("id".to_string(), CfmlValue::Int(2));
        r2.insert("name".to_string(), CfmlValue::Null);
        let q = CfmlValue::Query(CfmlQuery::from_parts(vec!["id".into(), "name".into()], vec![r1, r2]));
        let back = round_trip(&q);
        let CfmlValue::Query(bq) = back else { panic!("query") };
        let backing = bq.backing();
        let d = backing.read();
        assert_eq!(d.columns, vec!["id".to_string(), "name".to_string()]);
        assert_eq!(d.row_count(), 2);
        assert_eq!(d.row_at(0).unwrap().get("name").unwrap().as_string(), "one \"1\"");

        let dt = CfmlValue::DateTime(crate::datetime::CfmlDate::from_epoch(1_700_000_000, 123_456_789));
        assert_eq!(format!("{:?}", round_trip(&dt)), format!("{dt:?}"));
    }

    #[test]
    fn corrupt_input_is_an_error_not_a_panic() {
        let mut good = Vec::new();
        let mut m = ValueMap::default();
        m.insert("k".to_string(), CfmlValue::array(vec![CfmlValue::string("abc"); 3]));
        encode(&CfmlValue::strukt(m), &mut good).unwrap();
        for cut in 0..good.len() {
            assert!(decode(&good[..cut]).is_err(), "truncated at {cut} must fail");
        }
        assert!(decode(&[99]).is_err());
        assert!(decode(&[T_STRING, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f]).is_err());
        assert!(decode(&[T_STRING, 2, 0xff, 0xfe]).is_err(), "invalid UTF-8");
        let mut deep = vec![T_ARRAY, 1].repeat(MAX_DEPTH + 10);
        deep.push(T_NULL);
        assert!(decode(&deep).is_err(), "nesting limit");
        // A struct that contains itself is refused, not a stack overflow.
        let cyc = crate::dynamic::CfmlStruct::new(ValueMap::default());
        cyc.insert("me".to_string(), CfmlValue::Struct(cyc.clone()));
        assert!(encode(&CfmlValue::Struct(cyc), &mut Vec::new()).is_err(), "cycle");
        let mut extra = good.clone();
        extra.push(0);
        assert!(decode(&extra).is_err(), "trailing bytes");
    }
}
