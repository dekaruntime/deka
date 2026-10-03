//! Immutable Rust-owned byte snapshots; Blob and File share storage and readers.
use crate::{HostHandle, HostOp, HostReply, HostType, HostValue, Hosts, Result};
use std::{ops::Range, sync::Arc};
const MAX_BLOB_BYTES: usize = 16 * 1024 * 1024;
const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.;
#[derive(Clone, Debug)]
struct FileMetadata {
    name: String,
    last_modified: f64,
}
#[derive(Clone, Debug)]
pub struct BlobObject {
    data: Arc<[u8]>,
    range: Range<usize>,
    media_type: String,
    file: Option<FileMetadata>,
}
fn media_type(value: &str) -> String {
    if value.bytes().all(|byte| (0x20..=0x7e).contains(&byte)) {
        value.to_ascii_lowercase()
    } else {
        String::new()
    }
}
fn integer(value: f64) -> Result<()> {
    if !value.is_finite() || value.fract() != 0. || value.abs() > MAX_SAFE_INTEGER {
        Err("expected a whole safe integer".into())
    } else {
        Ok(())
    }
}
impl BlobObject {
    pub fn new(parts: &[Vec<u8>], content_type: &str) -> Result<Self> {
        let size = parts.iter().try_fold(0usize, |total, part| {
            total
                .checked_add(part.len())
                .filter(|size| *size <= MAX_BLOB_BYTES)
                .ok_or("Blob exceeds 16 MiB")
        })?;
        let mut data = Vec::new();
        data.try_reserve_exact(size)
            .map_err(|_| "Blob allocation failed")?;
        for part in parts {
            data.extend_from_slice(part);
        }
        Ok(Self {
            data: data.into(),
            range: 0..size,
            media_type: media_type(content_type),
            file: None,
        })
    }
    pub fn as_bytes(&self) -> &[u8] {
        &self.data[self.range.clone()]
    }
    pub fn slice(&self, start: f64, end: f64, content_type: &str) -> Result<Self> {
        integer(start)?;
        integer(end)?;
        let size = self.range.len() as f64;
        let position = |n: f64| {
            if n < 0. {
                (size + n).clamp(0., size) as usize
            } else {
                n.min(size) as usize
            }
        };
        let start = position(start);
        let end = position(end).max(start);
        Ok(Self {
            data: self.data.clone(),
            range: self.range.start + start..self.range.start + end,
            media_type: media_type(content_type),
            file: None,
        })
    }
    pub fn into_file(mut self, name: String, last_modified: f64) -> Result<Self> {
        integer(last_modified)?;
        self.file = Some(FileMetadata {
            name,
            last_modified,
        });
        Ok(self)
    }
    pub fn into_host_value(self) -> HostValue {
        let kind = if self.file.is_some() { "File" } else { "Blob" };
        HostValue::Handle(HostHandle::new(kind, self))
    }
}
fn resource<'a>(value: &'a HostValue, kind: &str) -> Result<&'a BlobObject> {
    let HostValue::Handle(handle) = value else {
        unreachable!("checked byte snapshot receiver")
    };
    let blob = handle
        .downcast_ref::<BlobObject>()
        .ok_or_else(|| format!("invalid {kind} resource"))?;
    if blob.file.is_some() != (kind == "File") {
        return Err(format!("invalid {kind} resource"));
    }
    Ok(blob)
}
fn parts(value: HostValue) -> Vec<Vec<u8>> {
    let HostValue::List(items) = value else {
        unreachable!("checked byte parts")
    };
    items
        .into_iter()
        .map(|part| {
            let HostValue::Bytes(bytes) = part else {
                unreachable!("checked byte part")
            };
            bytes
        })
        .collect()
}
fn current_milliseconds() -> Result<f64> {
    let time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| format!("File timestamp unavailable: {error}"))?;
    let value = time.as_millis() as f64;
    integer(value)?;
    Ok(value)
}
pub fn register(hosts: &mut Hosts) -> Result<()> {
    let byte_parts = HostType::List(Box::new(HostType::Bytes));
    hosts.register(
        HostOp::new(
            "Blob",
            vec![byte_parts.clone()],
            HostType::Handle("Blob".into()),
            false,
            |args| {
                let input = parts(args.into_iter().next().expect("checked Blob argument"));
                HostReply::Ready(BlobObject::new(&input, "").map(BlobObject::into_host_value))
            },
        )
        .with_defaults(vec![HostValue::List(vec![])])
        .with_global_binding()
        .with_result_channel(),
    )?;
    hosts.register(
        HostOp::new(
            "File",
            vec![byte_parts, HostType::String],
            HostType::Handle("File".into()),
            false,
            |args| {
                let mut args = args.into_iter();
                let input = parts(args.next().expect("checked File parts"));
                let Some(HostValue::String(name)) = args.next() else {
                    unreachable!("checked File name")
                };
                HostReply::Ready(
                    BlobObject::new(&input, "")
                        .and_then(|blob| blob.into_file(name, current_milliseconds()?))
                        .map(BlobObject::into_host_value),
                )
            },
        )
        .with_global_binding()
        .with_result_channel(),
    )?;
    for kind in ["Blob", "File"] {
        let receiver = HostType::Handle(kind.into());
        let mut fields = vec![("size", HostType::Number), ("type", HostType::String)];
        if kind == "File" {
            fields.extend([
                ("name", HostType::String),
                ("lastModified", HostType::Number),
            ]);
        }
        for (field, ty) in fields {
            hosts.register(
                HostOp::new(
                    &format!("__{kind}_{field}"),
                    vec![receiver.clone()],
                    ty,
                    false,
                    move |args| {
                        HostReply::Ready(resource(&args[0], kind).map(|blob| match field {
                            "size" => HostValue::Number(blob.range.len() as f64),
                            "type" => HostValue::String(blob.media_type.clone()),
                            "name" => HostValue::String(
                                blob.file.as_ref().expect("checked File").name.clone(),
                            ),
                            "lastModified" => HostValue::Number(
                                blob.file.as_ref().expect("checked File").last_modified,
                            ),
                            _ => unreachable!("registered snapshot field"),
                        }))
                    },
                )
                .with_receiver_property(kind, field),
            )?;
        }
        hosts.register(
            HostOp::new(
                &format!("__{kind}_slice"),
                vec![
                    receiver.clone(),
                    HostType::Number,
                    HostType::Number,
                    HostType::String,
                ],
                HostType::Handle("Blob".into()),
                false,
                move |args| {
                    let (
                        HostValue::Number(start),
                        HostValue::Number(end),
                        HostValue::String(content_type),
                    ) = (&args[1], &args[2], &args[3])
                    else {
                        unreachable!("checked slice arguments")
                    };
                    HostReply::Ready(
                        resource(&args[0], kind)
                            .and_then(|blob| blob.slice(*start, *end, content_type))
                            .map(BlobObject::into_host_value),
                    )
                },
            )
            .with_receiver_method(kind, "slice")
            .with_defaults(vec![
                HostValue::Number(0.),
                HostValue::Number(MAX_SAFE_INTEGER),
                HostValue::String(String::new()),
            ])
            .with_result_channel(),
        )?;
        for method in ["bytes", "arrayBuffer", "text"] {
            hosts.register(
                HostOp::new(
                    &format!("__{kind}_{method}"),
                    vec![receiver.clone()],
                    if method == "text" {
                        HostType::String
                    } else {
                        HostType::Bytes
                    },
                    true,
                    move |args| {
                        let blob = match resource(&args[0], kind) {
                            Ok(blob) => blob.clone(),
                            Err(error) => return HostReply::Ready(Err(error)),
                        };
                        HostReply::Pending(Box::pin(async move {
                            if method == "text" {
                                tokio::task::spawn_blocking(move || {
                                    crate::text_codec::decode_utf8(blob.as_bytes())
                                })
                                .await
                                .map_err(|error| format!("Blob reader failed: {error}"))?
                                .map(HostValue::String)
                            } else {
                                tokio::task::spawn_blocking(move || blob.as_bytes().to_vec())
                                    .await
                                    .map_err(|error| format!("Blob reader failed: {error}"))
                                    .map(HostValue::Bytes)
                            }
                        }))
                    },
                )
                .with_receiver_method(kind, method)
                .with_result_channel(),
            )?;
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn slice_owns_the_shared_bytes_and_normalizes_ranges_and_type() {
        let blob = BlobObject::new(&[b"abc".to_vec(), b"def".to_vec()], "TEXT/PLAIN").unwrap();
        assert_eq!(blob.media_type, "text/plain");
        let slice = blob.slice(-4., -1., "IMAGE/PNG").unwrap();
        assert!(Arc::ptr_eq(&blob.data, &slice.data));
        drop(blob);
        assert_eq!(slice.as_bytes(), b"cde");
        assert_eq!(slice.media_type, "image/png");
        assert_eq!(slice.slice(3., 1., "bad\n").unwrap().as_bytes(), b"");
        assert_eq!(slice.slice(-100., 100., "é").unwrap().media_type, "");
        assert_eq!(
            slice.slice(0., MAX_SAFE_INTEGER, "").unwrap().as_bytes(),
            b"cde"
        );
        for value in [f64::NAN, f64::INFINITY, 0.5, MAX_SAFE_INTEGER + 1.] {
            assert!(slice.slice(value, 3., "").is_err());
        }
    }
    #[test]
    fn construction_is_bounded_and_file_slices_are_plain_blobs() {
        assert!(BlobObject::new(&[vec![0; MAX_BLOB_BYTES], vec![1]], "").is_err());
        let file = BlobObject::new(&[b"data".to_vec()], "")
            .unwrap()
            .into_file("name.ds".into(), 42.)
            .unwrap();
        assert_eq!(file.file.as_ref().unwrap().last_modified, 42.);
        let slice = file.slice(0., 2., "").unwrap();
        assert!(slice.file.is_none());
        assert_eq!(slice.as_bytes(), b"da");
        assert!(file.into_file("bad".into(), 1.5).is_err());
    }
}
