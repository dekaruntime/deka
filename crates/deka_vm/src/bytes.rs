//! Immutable binary standard library (APS 15). Host declarations are the API.
use crate::{HostOp, HostReply, HostType, HostValue, Hosts, Result};

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
fn some(value: HostValue) -> HostValue {
    HostValue::Option(Some(Box::new(value)))
}
fn optional_bytes(value: Option<Vec<u8>>) -> HostValue {
    HostValue::Option(value.map(|value| Box::new(HostValue::Bytes(value))))
}
fn index(value: f64, len: usize) -> usize {
    if value.is_nan() {
        return 0;
    }
    let value = value.trunc();
    if value < 0. {
        (len as f64 + value).clamp(0., len as f64) as usize
    } else {
        value.min(len as f64) as usize
    }
}
pub fn to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 15) as usize] as char);
    }
    out
}
pub fn from_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    text.as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            Some(((pair[0] as char).to_digit(16)? * 16 + (pair[1] as char).to_digit(16)?) as u8)
        })
        .collect()
}
pub fn to_base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = ((chunk[0] as u32) << 16)
            | ((chunk.get(1).copied().unwrap_or(0) as u32) << 8)
            | chunk.get(2).copied().unwrap_or(0) as u32;
        out.push(BASE64[((n >> 18) & 63) as usize] as char);
        out.push(BASE64[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            BASE64[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            BASE64[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}
pub fn from_base64(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(4) {
        return None;
    }
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    for (i, chunk) in text.as_bytes().chunks_exact(4).enumerate() {
        let last = i + 1 == text.len() / 4;
        let pad = if chunk[2] == b'=' && chunk[3] == b'=' {
            2
        } else if chunk[3] == b'=' {
            1
        } else {
            0
        };
        if pad != 0 && !last {
            return None;
        }
        let mut n = 0u32;
        for (j, byte) in chunk.iter().enumerate() {
            let value = if j >= 4 - pad {
                0
            } else {
                BASE64.iter().position(|v| v == byte)? as u32
            };
            n = (n << 6) | value;
        }
        out.push((n >> 16) as u8);
        if pad < 2 {
            out.push((n >> 8) as u8);
        }
        if pad == 0 {
            out.push(n as u8);
        }
    }
    Some(out)
}
pub fn register(hosts: &mut Hosts) -> Result<()> {
    for (name, args, output, defaults, result) in [
        (
            "len",
            vec![HostType::Bytes],
            HostType::Number,
            vec![],
            false,
        ),
        (
            "get",
            vec![HostType::Bytes, HostType::Number],
            HostType::Option(Box::new(HostType::Number)),
            vec![],
            false,
        ),
        (
            "slice",
            vec![HostType::Bytes, HostType::Number, HostType::Number],
            HostType::Bytes,
            vec![HostValue::Number(9_007_199_254_740_991.)],
            false,
        ),
        (
            "concat",
            vec![HostType::Bytes, HostType::Bytes],
            HostType::Bytes,
            vec![],
            false,
        ),
        (
            "from_array",
            vec![HostType::List(Box::new(HostType::Number))],
            HostType::Option(Box::new(HostType::Bytes)),
            vec![],
            false,
        ),
        (
            "from_string",
            vec![HostType::String],
            HostType::Bytes,
            vec![],
            false,
        ),
        (
            "to_string",
            vec![HostType::Bytes],
            HostType::String,
            vec![],
            true,
        ),
        (
            "to_hex",
            vec![HostType::Bytes],
            HostType::String,
            vec![],
            false,
        ),
        (
            "from_hex",
            vec![HostType::String],
            HostType::Option(Box::new(HostType::Bytes)),
            vec![],
            false,
        ),
        (
            "to_base64",
            vec![HostType::Bytes],
            HostType::String,
            vec![],
            false,
        ),
        (
            "from_base64",
            vec![HostType::String],
            HostType::Option(Box::new(HostType::Bytes)),
            vec![],
            false,
        ),
    ] {
        let mut op = HostOp::new(&format!("bytes_{name}"), args, output, false, move |args| {
            let value = match (name, args.as_slice()) {
                ("len", [HostValue::Bytes(b)]) => HostValue::Number(b.len() as f64),
                ("get", [HostValue::Bytes(b), HostValue::Number(i)]) => {
                    if i.is_finite() && i.fract() == 0. && (0. ..b.len() as f64).contains(i) {
                        some(HostValue::Number(b[*i as usize] as f64))
                    } else {
                        HostValue::Option(None)
                    }
                }
                (
                    "slice",
                    [
                        HostValue::Bytes(b),
                        HostValue::Number(start),
                        HostValue::Number(end),
                    ],
                ) => {
                    let start = index(*start, b.len());
                    let end = index(*end, b.len()).max(start);
                    HostValue::Bytes(b[start..end].to_vec())
                }
                ("concat", [HostValue::Bytes(a), HostValue::Bytes(b)]) => {
                    HostValue::Bytes([a.as_slice(), b.as_slice()].concat())
                }
                ("from_array", [HostValue::List(items)]) => optional_bytes(
                    items
                        .iter()
                        .map(|value| {
                            let HostValue::Number(n) = value else {
                                unreachable!("checked number array")
                            };
                            (n.is_finite() && n.fract() == 0. && (0. ..=255.).contains(n))
                                .then_some(*n as u8)
                        })
                        .collect(),
                ),
                ("from_string", [HostValue::String(s)]) => HostValue::Bytes(s.as_bytes().to_vec()),
                ("to_string", [HostValue::Bytes(b)]) => {
                    return HostReply::Ready(
                        std::str::from_utf8(b)
                            .map(|s| HostValue::String(s.into()))
                            .map_err(|e| format!("invalid UTF-8: {e}")),
                    );
                }
                ("to_hex", [HostValue::Bytes(b)]) => HostValue::String(to_hex(b)),
                ("from_hex", [HostValue::String(s)]) => optional_bytes(from_hex(s)),
                ("to_base64", [HostValue::Bytes(b)]) => HostValue::String(to_base64(b)),
                ("from_base64", [HostValue::String(s)]) => optional_bytes(from_base64(s)),
                _ => unreachable!("checked bytes operation"),
            };
            HostReply::Ready(Ok(value))
        })
        .with_defaults(defaults);
        if result {
            op = op.with_result_channel();
        }
        hosts.register(op)?;
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_binary_roundtrips_and_invalid_input() {
        let bytes: Vec<u8> = (0..=255).collect();
        assert_eq!(from_hex(&to_hex(&bytes)), Some(bytes.clone()));
        assert_eq!(from_base64(&to_base64(&bytes)), Some(bytes));
        for (raw, encoded) in [
            (b"".as_slice(), ""),
            (b"f", "Zg=="),
            (b"fo", "Zm8="),
            (b"foo", "Zm9v"),
        ] {
            assert_eq!(to_base64(raw), encoded);
            assert_eq!(from_base64(encoded), Some(raw.to_vec()));
        }
        for bad in ["a", "6g", "61 6", "é", "0x00"] {
            assert!(from_hex(bad).is_none(), "{bad}");
        }
        for bad in [
            "Zg", "Zg=", "====", "Z===", "AA=A", "Zg==Zm8=", "!!!!", "Zm9v\n",
        ] {
            assert!(from_base64(bad).is_none(), "{bad}");
        }
        assert_eq!(from_hex("AbCD"), Some(vec![0xab, 0xcd]));
    }
    #[test]
    fn slice_offsets_truncate_clip_and_handle_nonfinite_values() {
        for (offset, expected) in [
            (f64::NAN, 0),
            (f64::INFINITY, 5),
            (f64::NEG_INFINITY, 0),
            (-1.9, 4),
            (1.9, 1),
            (-99., 0),
            (99., 5),
        ] {
            assert_eq!(index(offset, 5), expected);
        }
    }
}
