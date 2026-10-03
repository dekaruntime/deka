//! Rust-owned text codecs. Checked declarations and dispatch share HostOp schemas.
use crate::{HostHandle, HostOp, HostReply, HostType, HostValue, Hosts, Result};
use encoding_rs::{CoderResult, Decoder, DecoderResult, Encoding, REPLACEMENT};
use std::{cell::RefCell, collections::BTreeMap};

struct Encoder;
struct TextDecoder {
    encoding: &'static Encoding,
    fatal: bool,
    ignore_bom: bool,
    decoder: RefCell<Decoder>,
}
impl TextDecoder {
    fn new(label: &str, fatal: bool, ignore_bom: bool) -> Result<Self> {
        let encoding = Encoding::for_label(label.as_bytes())
            .filter(|encoding| *encoding != REPLACEMENT)
            .ok_or_else(|| format!("unsupported text encoding: {label}"))?;
        Ok(Self {
            encoding,
            fatal,
            ignore_bom,
            decoder: RefCell::new(Self::decoder(encoding, ignore_bom)),
        })
    }
    fn decoder(encoding: &'static Encoding, ignore_bom: bool) -> Decoder {
        if ignore_bom {
            encoding.new_decoder_without_bom_handling()
        } else {
            encoding.new_decoder_with_bom_removal()
        }
    }
    fn decode(&self, input: &[u8], stream: bool) -> Result<String> {
        let mut decoder = self.decoder.borrow_mut();
        let mut output = String::new();
        let mut input = input;
        let result = loop {
            // encoding_rs never reallocates the supplied String. Keep enough
            // space for a complete character on every step, including flush.
            output
                .try_reserve(4096)
                .map_err(|_| "decoded text allocation failed")?;
            if self.fatal {
                let (status, read) =
                    decoder.decode_to_string_without_replacement(input, &mut output, !stream);
                input = &input[read..];
                match status {
                    DecoderResult::InputEmpty => break Ok(output),
                    DecoderResult::Malformed(_, _) => break Err("malformed encoded text".into()),
                    DecoderResult::OutputFull => {}
                }
            } else {
                let (status, read, _) = decoder.decode_to_string(input, &mut output, !stream);
                input = &input[read..];
                if status == CoderResult::InputEmpty {
                    break Ok(output);
                }
            }
        };
        if !stream {
            *decoder = Self::decoder(self.encoding, self.ignore_bom);
        }
        result
    }
}
/// Complete UTF-8 body decoding uses the same replacement/BOM machinery as
/// the public TextDecoder, without introducing a second decoder.
pub(crate) fn decode_utf8(input: &[u8]) -> Result<String> {
    TextDecoder::new("utf-8", false, false)?.decode(input, false)
}
fn fields<const N: usize>(fields: [(&str, HostType); N]) -> HostType {
    HostType::Record(
        fields
            .into_iter()
            .map(|(name, ty)| (name.into(), ty))
            .collect(),
    )
}
fn options<const N: usize>(fields: [(&str, bool); N]) -> HostValue {
    HostValue::Record(
        fields
            .into_iter()
            .map(|(name, value)| (name.into(), HostValue::Bool(value)))
            .collect(),
    )
}
fn flag(fields: &BTreeMap<String, HostValue>, name: &str) -> bool {
    matches!(fields.get(name), Some(HostValue::Bool(true)))
}
pub fn register(hosts: &mut Hosts) -> Result<()> {
    hosts.register(
        HostOp::new(
            "TextEncoder",
            vec![],
            HostType::Handle("TextEncoder".into()),
            false,
            |_| {
                HostReply::Ready(Ok(HostValue::Handle(HostHandle::new(
                    "TextEncoder",
                    Encoder,
                ))))
            },
        )
        .with_global_binding(),
    )?;
    hosts.register(
        HostOp::new(
            "__text_encode",
            vec![HostType::Handle("TextEncoder".into()), HostType::String],
            HostType::Bytes,
            false,
            |args| {
                let [HostValue::Handle(handle), HostValue::String(text)] = args.as_slice() else {
                    unreachable!("checked text encoder arguments")
                };
                if handle.downcast_ref::<Encoder>().is_none() {
                    return HostReply::Ready(Err("invalid TextEncoder resource".into()));
                }
                HostReply::Ready(Ok(HostValue::Bytes(text.as_bytes().to_vec())))
            },
        )
        .with_defaults(vec![HostValue::String(String::new())])
        .with_receiver_method("TextEncoder", "encode"),
    )?;
    hosts.register(
        HostOp::new(
            "TextDecoder",
            vec![
                HostType::String,
                fields([("fatal", HostType::Bool), ("ignoreBOM", HostType::Bool)]),
            ],
            HostType::Handle("TextDecoder".into()),
            false,
            |args| {
                let [HostValue::String(label), HostValue::Record(options)] = args.as_slice() else {
                    unreachable!("checked text decoder arguments")
                };
                HostReply::Ready(
                    TextDecoder::new(label, flag(options, "fatal"), flag(options, "ignoreBOM"))
                        .map(|decoder| HostValue::Handle(HostHandle::new("TextDecoder", decoder))),
                )
            },
        )
        .with_defaults(vec![
            HostValue::String("utf-8".into()),
            options([("fatal", false), ("ignoreBOM", false)]),
        ])
        .with_global_binding()
        .with_result_channel(),
    )?;
    hosts.register(
        HostOp::new(
            "__text_decode",
            vec![
                HostType::Handle("TextDecoder".into()),
                HostType::Bytes,
                fields([("stream", HostType::Bool)]),
            ],
            HostType::String,
            false,
            |args| {
                let [
                    HostValue::Handle(handle),
                    HostValue::Bytes(bytes),
                    HostValue::Record(options),
                ] = args.as_slice()
                else {
                    unreachable!("checked decode arguments")
                };
                let Some(decoder) = handle.downcast_ref::<TextDecoder>() else {
                    return HostReply::Ready(Err("invalid TextDecoder resource".into()));
                };
                HostReply::Ready(
                    decoder
                        .decode(bytes, flag(options, "stream"))
                        .map(HostValue::String),
                )
            },
        )
        .with_defaults(vec![options([("stream", false)])])
        .with_receiver_method("TextDecoder", "decode")
        .with_result_channel(),
    )?;
    hosts.register(
        HostOp::new(
            "__text_encoder_encoding",
            vec![HostType::Handle("TextEncoder".into())],
            HostType::String,
            false,
            |args| {
                let HostValue::Handle(handle) = &args[0] else {
                    unreachable!("checked encoder")
                };
                if handle.downcast_ref::<Encoder>().is_none() {
                    return HostReply::Ready(Err("invalid TextEncoder resource".into()));
                }
                HostReply::Ready(Ok(HostValue::String("utf-8".into())))
            },
        )
        .with_receiver_property("TextEncoder", "encoding"),
    )?;
    for (property, ty) in [
        ("encoding", HostType::String),
        ("fatal", HostType::Bool),
        ("ignoreBOM", HostType::Bool),
    ] {
        hosts.register(
            HostOp::new(
                &format!("__text_decoder_{property}"),
                vec![HostType::Handle("TextDecoder".into())],
                ty,
                false,
                move |args| {
                    let HostValue::Handle(handle) = &args[0] else {
                        unreachable!("checked decoder")
                    };
                    let Some(decoder) = handle.downcast_ref::<TextDecoder>() else {
                        return HostReply::Ready(Err("invalid TextDecoder resource".into()));
                    };
                    HostReply::Ready(Ok(match property {
                        "encoding" => {
                            HostValue::String(decoder.encoding.name().to_ascii_lowercase())
                        }
                        "fatal" => HostValue::Bool(decoder.fatal),
                        _ => HostValue::Bool(decoder.ignore_bom),
                    }))
                },
            )
            .with_receiver_property("TextDecoder", property),
        )?;
    }
    Ok(())
}
