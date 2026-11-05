//! Minimal protobuf wire-format reader and writer.
//!
//! Dewey only ever handles one protobuf payload (a Komikku/Tachiyomi backup), so
//! this module implements exactly the wire format and nothing else: tags,
//! varints, length-delimited fields, and the ability to skip anything it does
//! not recognise. No codegen, no `build.rs`, no schema.

use anyhow::{anyhow, bail, Result};

const WIRE_VARINT: u8 = 0;
const WIRE_FIXED64: u8 = 1;
const WIRE_LEN: u8 = 2;
const WIRE_START_GROUP: u8 = 3;
const WIRE_END_GROUP: u8 = 4;
const WIRE_FIXED32: u8 = 5;

/// A cursor over an encoded protobuf message body.
pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    pub fn is_empty(&self) -> bool {
        self.pos >= self.buf.len()
    }

    fn advance(&mut self, n: usize) -> Result<()> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|end| *end <= self.buf.len())
            .ok_or_else(|| anyhow!("truncated field: wanted {n} bytes at offset {}", self.pos))?;
        self.pos = end;
        Ok(())
    }

    pub fn take_varint(&mut self) -> Result<u64> {
        let mut result: u64 = 0;
        let mut shift: u32 = 0;
        loop {
            let byte = *self
                .buf
                .get(self.pos)
                .ok_or_else(|| anyhow!("truncated varint at offset {}", self.pos))?;
            self.pos += 1;
            if shift >= 64 {
                bail!("varint overflow at offset {}", self.pos);
            }
            result |= u64::from(byte & 0x7f) << shift;
            shift += 7;
            if byte & 0x80 == 0 {
                return Ok(result);
            }
        }
    }

    /// Returns `(field_number, wire_type)` for the next field.
    pub fn take_tag(&mut self) -> Result<(u32, u8)> {
        let key = self.take_varint()?;
        let wire = (key & 0x7) as u8;
        let field = (key >> 3) as u32;
        if field == 0 {
            bail!("invalid field number 0 at offset {}", self.pos);
        }
        Ok((field, wire))
    }

    pub fn take_bytes(&mut self) -> Result<&'a [u8]> {
        let len = self.take_varint()? as usize;
        let end = self
            .pos
            .checked_add(len)
            .filter(|end| *end <= self.buf.len())
            .ok_or_else(|| anyhow!("truncated length-delimited field at offset {}", self.pos))?;
        let slice = &self.buf[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    pub fn take_string(&mut self) -> Result<String> {
        Ok(String::from_utf8_lossy(self.take_bytes()?).into_owned())
    }

    pub fn take_message(&mut self) -> Result<Reader<'a>> {
        Ok(Reader::new(self.take_bytes()?))
    }

    pub fn take_bool(&mut self) -> Result<bool> {
        Ok(self.take_varint()? != 0)
    }

    pub fn take_f32(&mut self) -> Result<f32> {
        let start = self.pos;
        self.advance(4)?;
        let bytes: [u8; 4] = self.buf[start..start + 4].try_into().unwrap();
        Ok(f32::from_le_bytes(bytes))
    }

    /// Decodes a `repeated int64` that arrived packed (wire type 2).
    pub fn take_packed_i64(&mut self) -> Result<Vec<i64>> {
        let mut inner = self.take_message()?;
        let mut out = Vec::new();
        while !inner.is_empty() {
            out.push(inner.take_varint()? as i64);
        }
        Ok(out)
    }

    /// Consumes one field of any wire type, including nested groups.
    pub fn skip(&mut self, wire: u8, field: u32) -> Result<()> {
        match wire {
            WIRE_VARINT => {
                self.take_varint()?;
            }
            WIRE_FIXED64 => self.advance(8)?,
            WIRE_LEN => {
                self.take_bytes()?;
            }
            WIRE_FIXED32 => self.advance(4)?,
            WIRE_START_GROUP => loop {
                let (inner_field, inner_wire) = self.take_tag()?;
                if inner_wire == WIRE_END_GROUP {
                    if inner_field != field {
                        bail!("mismatched end-group: {inner_field} closes {field}");
                    }
                    break;
                }
                self.skip(inner_wire, inner_field)?;
            },
            WIRE_END_GROUP => bail!("unexpected end-group for field {field}"),
            other => bail!("unknown wire type {other} for field {field}"),
        }
        Ok(())
    }
}

/// A wire-format encoder, the mirror of [`Reader`].
///
/// It follows proto3 semantics: omitting a field means "default value", so it is
/// the caller's job to skip empties. Repeated numeric fields are written
/// unpacked, which is what Komikku's own backups contain.
#[derive(Default)]
pub struct Writer {
    buf: Vec<u8>,
}

fn put_varint_into(buf: &mut Vec<u8>, mut value: u64) {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            buf.push(byte);
            return;
        }
        buf.push(byte | 0x80);
    }
}

impl Writer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.buf
    }

    pub fn into_vec(self) -> Vec<u8> {
        self.buf
    }

    fn tag(&mut self, field: u32, wire: u8) {
        debug_assert!(
            field > 0 && field < (1 << 29),
            "invalid field number {field}"
        );
        self.put_varint((u64::from(field) << 3) | u64::from(wire));
    }

    fn put_varint(&mut self, value: u64) {
        put_varint_into(&mut self.buf, value);
    }

    /// `repeated int64`, in the packed form Komikku writes for categories.
    pub fn packed_int(&mut self, field: u32, values: &[i64]) {
        let mut body = Vec::new();
        for value in values {
            put_varint_into(&mut body, *value as u64);
        }
        self.bytes(field, &body);
    }

    pub fn varint(&mut self, field: u32, value: u64) {
        self.tag(field, WIRE_VARINT);
        self.put_varint(value);
    }

    /// `int64`/enum: negatives go out as their two's complement, per protobuf.
    pub fn int(&mut self, field: u32, value: i64) {
        self.varint(field, value as u64);
    }

    pub fn boolean(&mut self, field: u32, value: bool) {
        self.varint(field, u64::from(value));
    }

    /// `float` (fixed32) — the type Komikku uses for chapter numbers.
    pub fn float(&mut self, field: u32, value: f32) {
        self.tag(field, WIRE_FIXED32);
        self.buf.extend_from_slice(&value.to_le_bytes());
    }

    pub fn string(&mut self, field: u32, value: &str) {
        self.bytes(field, value.as_bytes());
    }

    pub fn bytes(&mut self, field: u32, value: &[u8]) {
        self.tag(field, WIRE_LEN);
        self.put_varint(value.len() as u64);
        self.buf.extend_from_slice(value);
    }

    /// Nests an already-encoded message body.
    pub fn message(&mut self, field: u32, body: &Writer) {
        self.bytes(field, body.as_slice());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varints_round_trip() {
        for value in [0u64, 1, 127, 128, 300, u64::MAX] {
            let mut encoded = Vec::new();
            let mut v = value;
            loop {
                let mut byte = (v & 0x7f) as u8;
                v >>= 7;
                if v != 0 {
                    byte |= 0x80;
                }
                encoded.push(byte);
                if v == 0 {
                    break;
                }
            }
            let mut reader = Reader::new(&encoded);
            assert_eq!(reader.take_varint().unwrap(), value);
            assert!(reader.is_empty());
        }
    }

    #[test]
    fn reads_tags_and_length_delimited_fields() {
        // field 1 (wire 2) = "hi", field 3 (wire 0) = 7
        let buf = [0x0a, 0x02, b'h', b'i', 0x18, 0x07];
        let mut r = Reader::new(&buf);
        assert_eq!(r.take_tag().unwrap(), (1, WIRE_LEN));
        assert_eq!(r.take_string().unwrap(), "hi");
        assert_eq!(r.take_tag().unwrap(), (3, WIRE_VARINT));
        assert_eq!(r.take_varint().unwrap(), 7);
        assert!(r.is_empty());
    }

    #[test]
    fn skips_every_wire_type() {
        let mut buf = Vec::new();
        // varint
        buf.extend_from_slice(&[0x08, 0x96, 0x01]);
        // fixed64
        buf.extend_from_slice(&[0x11, 1, 2, 3, 4, 5, 6, 7, 8]);
        // length-delimited
        buf.extend_from_slice(&[0x1a, 0x03, b'a', b'b', b'c']);
        // fixed32
        buf.extend_from_slice(&[0x25, 1, 2, 3, 4]);
        // nested group: start(4) > varint(5) > end(4)
        buf.extend_from_slice(&[0x23, 0x28, 0x01, 0x24]);

        let mut r = Reader::new(&buf);
        while !r.is_empty() {
            let (field, wire) = r.take_tag().unwrap();
            r.skip(wire, field).unwrap();
        }
    }

    #[test]
    fn group_skip_requires_matching_end() {
        // start group 4, end group 5 -> mismatch
        let buf = [0x23, 0x2c];
        let mut r = Reader::new(&buf);
        let (field, wire) = r.take_tag().unwrap();
        assert!(r.skip(wire, field).is_err());
    }

    #[test]
    fn truncated_input_errors() {
        let buf = [0x0a, 0x05, b'a'];
        let mut r = Reader::new(&buf);
        let _ = r.take_tag().unwrap();
        assert!(r.take_string().is_err());
    }

    #[test]
    fn packed_varints_decode() {
        // field 1 packed: 1, 2, 300
        let buf = [0x0a, 0x04, 0x01, 0x02, 0xac, 0x02];
        let mut r = Reader::new(&buf);
        assert_eq!(r.take_tag().unwrap(), (1, WIRE_LEN));
        assert_eq!(r.take_packed_i64().unwrap(), vec![1, 2, 300]);
    }

    #[test]
    fn floats_decode_from_fixed32() {
        // field 9 (wire 5) = 194.0f
        let mut buf = vec![0x4d];
        buf.extend_from_slice(&194.0f32.to_le_bytes());
        let mut r = Reader::new(&buf);
        assert_eq!(r.take_tag().unwrap(), (9, WIRE_FIXED32));
        assert_eq!(r.take_f32().unwrap(), 194.0);
    }

    #[test]
    fn writer_round_trips_through_reader() {
        let mut inner = Writer::new();
        inner.string(1, "Weeb Central");
        inner.int(2, 2454235896419992439);

        let mut w = Writer::new();
        w.string(1, "Chainsaw Man");
        w.boolean(5, true);
        w.int(6, -3);
        w.float(9, 12.5);
        w.message(101, &inner);
        w.varint(100, 7);
        w.varint(100, 8);

        let bytes = w.into_vec();
        let mut r = Reader::new(&bytes);
        let mut categories = Vec::new();
        let mut source = None;
        while !r.is_empty() {
            let (field, wire) = r.take_tag().unwrap();
            match (field, wire) {
                (1, WIRE_LEN) => assert_eq!(r.take_string().unwrap(), "Chainsaw Man"),
                (5, WIRE_VARINT) => assert!(r.take_bool().unwrap()),
                (6, WIRE_VARINT) => assert_eq!(r.take_varint().unwrap() as i64, -3),
                (9, WIRE_FIXED32) => assert_eq!(r.take_f32().unwrap(), 12.5),
                (101, WIRE_LEN) => {
                    let mut m = r.take_message().unwrap();
                    let mut name = String::new();
                    let mut id = 0i64;
                    while !m.is_empty() {
                        let (f, _) = m.take_tag().unwrap();
                        match f {
                            1 => name = m.take_string().unwrap(),
                            2 => id = m.take_varint().unwrap() as i64,
                            other => panic!("unexpected field {other}"),
                        }
                    }
                    source = Some((name, id));
                }
                (100, WIRE_VARINT) => categories.push(r.take_varint().unwrap()),
                other => panic!("unexpected field {other:?}"),
            }
        }

        assert_eq!(categories, vec![7, 8]);
        assert_eq!(
            source,
            Some(("Weeb Central".to_string(), 2454235896419992439))
        );
    }

    #[test]
    fn writer_emits_nothing_for_omitted_fields() {
        let w = Writer::new();
        assert!(w.as_slice().is_empty());

        let mut w = Writer::new();
        w.string(1, "");
        // An empty string still occupies a field; callers must skip empties.
        let mut r = Reader::new(w.as_slice());
        assert_eq!(r.take_tag().unwrap(), (1, WIRE_LEN));
        assert_eq!(r.take_string().unwrap(), "");
    }
}
