//! Minimal GGUF header metadata reader.
//!
//! A GGUF file starts with a metadata block: the `GGUF` magic, a version, tensor
//! and key-value counts, then the typed key-value pairs. This reads only that
//! block (never the multi-gigabyte tensor data) to recover the fields the Models
//! hub wants for an imported or downloaded model: the architecture, name,
//! context length (`<arch>.context_length`, the model's real context window) and
//! the parameter count when the file records it (`general.parameter_count`).
//!
//! Fully bounds-checked and fail-closed: a truncated or malformed block returns
//! an error rather than panicking, and array parsing is depth-limited and never
//! pre-allocates from an untrusted count. See the GGUF spec (ggml) for the wire
//! format.

use std::path::Path;

/// The maximum number of leading bytes read for the metadata block. GGUF metadata
/// (including the tokenizer vocab array) is well under this for shipped models;
/// the tensor data that follows is never read.
const MAX_METADATA_BYTES: u64 = 32 * 1024 * 1024;

/// Read GGUF header metadata from the file at `path`, reading at most
/// [`MAX_METADATA_BYTES`] leading bytes. Errs if the file cannot be read or its
/// metadata block is malformed or truncated within that bound.
pub fn read_gguf_metadata(path: &Path) -> Result<GgufMetadata, String> {
    use std::io::Read;
    let file =
        std::fs::File::open(path).map_err(|e| format!("cannot open {}: {e}", path.display()))?;
    let mut buf = Vec::new();
    file.take(MAX_METADATA_BYTES)
        .read_to_end(&mut buf)
        .map_err(|e| format!("read failed: {e}"))?;
    parse_gguf_metadata(&buf)
}

/// The header metadata recovered from a GGUF file. Every field is optional: a
/// model that does not record a key simply leaves it `None`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GgufMetadata {
    /// `general.architecture` (e.g. `"llama"`, `"qwen2"`).
    pub architecture: Option<String>,
    /// `general.name` (the model's self-reported name).
    pub name: Option<String>,
    /// `<arch>.context_length`, the trained context window in tokens.
    pub context_length: Option<u64>,
    /// `general.parameter_count`, when the file records it.
    pub parameter_count: Option<u64>,
}

/// Maximum nesting depth for GGUF arrays. Real files use flat arrays; the limit
/// stops a hostile file from exhausting the stack.
const MAX_ARRAY_DEPTH: u32 = 8;

/// One parsed GGUF value. Ints collapse to `Uint`/`Int`, floats to `Float`; only
/// the shapes the hub reads are inspected, the rest are parsed to advance past.
enum GgufValue {
    Uint(u64),
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    Array(Vec<GgufValue>),
}

impl GgufValue {
    fn as_u64(&self) -> Option<u64> {
        match self {
            GgufValue::Uint(v) => Some(*v),
            GgufValue::Int(v) if *v >= 0 => Some(*v as u64),
            _ => None,
        }
    }

    fn into_string(self) -> Option<String> {
        match self {
            GgufValue::Str(s) => Some(s),
            _ => None,
        }
    }
}

/// A little-endian byte cursor with bounds checks: every read fails closed if the
/// buffer is shorter than the value being read.
struct Cursor<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Cursor { buf, pos: 0 }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.pos.checked_add(n).ok_or("length overflow")?;
        let slice = self
            .buf
            .get(self.pos..end)
            .ok_or("unexpected end of GGUF metadata")?;
        self.pos = end;
        Ok(slice)
    }

    fn u32(&mut self) -> Result<u32, String> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn u64(&mut self) -> Result<u64, String> {
        let b = self.take(8)?;
        Ok(u64::from_le_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }

    fn string(&mut self) -> Result<String, String> {
        let len = self.u64()? as usize;
        let bytes = self.take(len)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| "GGUF string is not UTF-8".to_string())
    }
}

fn parse_value(vtype: u32, c: &mut Cursor, depth: u32) -> Result<GgufValue, String> {
    Ok(match vtype {
        0 => GgufValue::Uint(c.take(1)?[0] as u64),
        1 => GgufValue::Int(c.take(1)?[0] as i8 as i64),
        2 => {
            let b = c.take(2)?;
            GgufValue::Uint(u16::from_le_bytes([b[0], b[1]]) as u64)
        }
        3 => {
            let b = c.take(2)?;
            GgufValue::Int(i16::from_le_bytes([b[0], b[1]]) as i64)
        }
        4 => GgufValue::Uint(c.u32()? as u64),
        5 => GgufValue::Int(c.u32()? as i32 as i64),
        6 => {
            let b = c.take(4)?;
            GgufValue::Float(f32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64)
        }
        7 => GgufValue::Bool(c.take(1)?[0] != 0),
        8 => GgufValue::Str(c.string()?),
        9 => {
            if depth >= MAX_ARRAY_DEPTH {
                return Err("GGUF array nested too deep".to_string());
            }
            let elem_type = c.u32()?;
            let count = c.u64()?;
            // Never pre-allocate from the untrusted count; the buffer bound caps
            // the real element count (each element is at least one byte).
            let mut items = Vec::new();
            for _ in 0..count {
                items.push(parse_value(elem_type, c, depth + 1)?);
            }
            GgufValue::Array(items)
        }
        10 => GgufValue::Uint(c.u64()?),
        11 => {
            let b = c.take(8)?;
            GgufValue::Int(i64::from_le_bytes([
                b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
            ]))
        }
        12 => {
            let b = c.take(8)?;
            GgufValue::Float(f64::from_le_bytes([
                b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
            ]))
        }
        other => return Err(format!("unknown GGUF value type {other}")),
    })
}

/// Parse GGUF header metadata from a prefix of a GGUF file (the caller reads a
/// bounded number of leading bytes; the metadata block lives at the start). Fails
/// closed on a bad magic, an unknown value type, or a truncated block.
pub fn parse_gguf_metadata(bytes: &[u8]) -> Result<GgufMetadata, String> {
    let mut c = Cursor::new(bytes);
    if c.take(4)? != super::installed::GGUF_MAGIC {
        return Err("not a GGUF file (bad magic)".to_string());
    }
    let _version = c.u32()?;
    let _tensor_count = c.u64()?;
    let kv_count = c.u64()?;

    let mut meta = GgufMetadata::default();
    for _ in 0..kv_count {
        let key = c.string()?;
        let vtype = c.u32()?;
        let value = parse_value(vtype, &mut c, 0)?;
        match key.as_str() {
            "general.architecture" => meta.architecture = value.into_string(),
            "general.name" => meta.name = value.into_string(),
            "general.parameter_count" => meta.parameter_count = value.as_u64(),
            k if k.ends_with(".context_length") => meta.context_length = value.as_u64(),
            _ => {}
        }
    }
    Ok(meta)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal GGUF metadata blob with the given kv pairs. Each kv is
    /// `(key, writer)` where writer appends the type tag + value bytes.
    fn gguf(kvs: &[(&str, Box<dyn Fn(&mut Vec<u8>)>)]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"GGUF");
        out.extend_from_slice(&3u32.to_le_bytes()); // version
        out.extend_from_slice(&0u64.to_le_bytes()); // tensor_count
        out.extend_from_slice(&(kvs.len() as u64).to_le_bytes());
        for (key, write_val) in kvs {
            out.extend_from_slice(&(key.len() as u64).to_le_bytes());
            out.extend_from_slice(key.as_bytes());
            write_val(&mut out);
        }
        out
    }

    fn str_val(s: &'static str) -> Box<dyn Fn(&mut Vec<u8>)> {
        Box::new(move |out: &mut Vec<u8>| {
            out.extend_from_slice(&8u32.to_le_bytes()); // STRING
            out.extend_from_slice(&(s.len() as u64).to_le_bytes());
            out.extend_from_slice(s.as_bytes());
        })
    }

    fn u32_val(v: u32) -> Box<dyn Fn(&mut Vec<u8>)> {
        Box::new(move |out: &mut Vec<u8>| {
            out.extend_from_slice(&4u32.to_le_bytes()); // UINT32
            out.extend_from_slice(&v.to_le_bytes());
        })
    }

    fn u64_val(v: u64) -> Box<dyn Fn(&mut Vec<u8>)> {
        Box::new(move |out: &mut Vec<u8>| {
            out.extend_from_slice(&10u32.to_le_bytes()); // UINT64
            out.extend_from_slice(&v.to_le_bytes());
        })
    }

    #[test]
    fn reads_architecture_name_context_and_params() {
        let blob = gguf(&[
            ("general.architecture", str_val("llama")),
            ("general.name", str_val("Llama 3.2 1B")),
            ("llama.context_length", u32_val(131072)),
            ("general.parameter_count", u64_val(1_240_000_000)),
        ]);
        let meta = parse_gguf_metadata(&blob).unwrap();
        assert_eq!(meta.architecture.as_deref(), Some("llama"));
        assert_eq!(meta.name.as_deref(), Some("Llama 3.2 1B"));
        assert_eq!(meta.context_length, Some(131072));
        assert_eq!(meta.parameter_count, Some(1_240_000_000));
    }

    #[test]
    fn skips_unrelated_keys_and_arrays() {
        let blob = gguf(&[
            (
                "tokenizer.ggml.tokens",
                Box::new(|out: &mut Vec<u8>| {
                    out.extend_from_slice(&9u32.to_le_bytes()); // ARRAY
                    out.extend_from_slice(&8u32.to_le_bytes()); // of STRING
                    out.extend_from_slice(&2u64.to_le_bytes()); // count 2
                    for s in ["<s>", "</s>"] {
                        out.extend_from_slice(&(s.len() as u64).to_le_bytes());
                        out.extend_from_slice(s.as_bytes());
                    }
                }),
            ),
            ("qwen2.context_length", u32_val(32768)),
        ]);
        let meta = parse_gguf_metadata(&blob).unwrap();
        assert_eq!(meta.context_length, Some(32768));
        assert!(meta.architecture.is_none());
    }

    #[test]
    fn bad_magic_and_truncation_fail_closed() {
        assert!(parse_gguf_metadata(b"NOPE\x00\x00\x00\x00").is_err());
        // A valid header claiming one kv but no kv bytes.
        let mut blob = Vec::new();
        blob.extend_from_slice(b"GGUF");
        blob.extend_from_slice(&3u32.to_le_bytes());
        blob.extend_from_slice(&0u64.to_le_bytes());
        blob.extend_from_slice(&1u64.to_le_bytes());
        assert!(parse_gguf_metadata(&blob).is_err());
    }
}
