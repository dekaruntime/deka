//! Filesystem operations shared by the blocking and asynchronous native exports.
//! Bytes are never decoded and directory entry names must be valid UTF-8.
use crate::{HostEnum, HostOp, HostReply, HostStruct, HostType, HostValue, Hosts, Result};
use std::collections::BTreeMap;

#[derive(Clone, Copy)]
enum Operation {
    ReadFile,
    WriteFile,
    ReadDir,
    Mkdirs,
}
impl Operation {
    const ALL: [Self; 4] = [Self::ReadFile, Self::WriteFile, Self::ReadDir, Self::Mkdirs];
    fn name(self) -> &'static str {
        match self {
            Self::ReadFile => "read_file",
            Self::WriteFile => "write_file",
            Self::ReadDir => "read_dir",
            Self::Mkdirs => "mkdirs",
        }
    }
    fn execute(self, path: &str, data: &[u8]) -> Result<Output> {
        let path = path.trim_matches('\0');
        if path.is_empty() {
            return Err(format!("{}: missing path", self.name()));
        }
        let error = |error| format!("{}: {error}", self.name());
        match self {
            Self::ReadFile => std::fs::read(path).map(Output::Bytes).map_err(error),
            Self::WriteFile => {
                std::fs::write(path, data).map_err(error)?;
                Ok(Output::Written(data.len()))
            }
            Self::Mkdirs => {
                std::fs::create_dir_all(path).map_err(error)?;
                Ok(Output::Created)
            }
            Self::ReadDir => {
                let mut entries = Vec::new();
                for entry in std::fs::read_dir(path).map_err(error)? {
                    let entry = entry.map_err(error)?;
                    let kind = entry.file_type().map_err(error)?;
                    let name = entry
                        .file_name()
                        .into_string()
                        .map_err(|_| "read_dir: entry name is not valid UTF-8".to_string())?;
                    entries.push(Entry {
                        name,
                        is_dir: kind.is_dir(),
                        is_file: kind.is_file(),
                    });
                }
                Ok(Output::Entries(entries))
            }
        }
    }
}

struct Entry {
    name: String,
    is_dir: bool,
    is_file: bool,
}
enum Output {
    Bytes(Vec<u8>),
    Written(usize),
    Created,
    Entries(Vec<Entry>),
}
impl Output {
    fn into_host(self) -> HostValue {
        match self {
            Self::Bytes(bytes) => HostValue::Bytes(bytes),
            Self::Written(count) => HostValue::Number(count as f64),
            Self::Created => HostValue::Bool(true),
            Self::Entries(entries) => HostValue::List(
                entries
                    .into_iter()
                    .map(|entry| {
                        HostValue::Record(BTreeMap::from([
                            ("name".into(), HostValue::String(entry.name)),
                            ("is_dir".into(), HostValue::Bool(entry.is_dir)),
                            ("is_file".into(), HostValue::Bool(entry.is_file)),
                        ]))
                    })
                    .collect(),
            ),
        }
    }
}
fn error_schema() -> HostEnum {
    HostEnum {
        name: "FsError".into(),
        cases: vec![
            (
                "PermissionDenied".into(),
                Some(HostType::Struct(Box::new(HostStruct {
                    name: "FsPermission".into(),
                    fields: BTreeMap::from([
                        ("capability".into(), HostType::String),
                        ("target".into(), HostType::String),
                    ]),
                }))),
            ),
            ("UnsupportedHost".into(), None),
            ("InvalidPayload".into(), None),
            ("Failed".into(), Some(HostType::String)),
        ],
    }
}
pub fn register(hosts: &mut Hosts) -> Result<()> {
    for operation in Operation::ALL {
        let args = if matches!(operation, Operation::WriteFile) {
            vec![HostType::String, HostType::Bytes]
        } else {
            vec![HostType::String]
        };
        let result = match operation {
            Operation::ReadFile => HostType::Bytes,
            Operation::WriteFile => HostType::Number,
            Operation::Mkdirs => HostType::Bool,
            Operation::ReadDir => HostType::List(Box::new(HostType::Record(BTreeMap::from([
                ("name".into(), HostType::String),
                ("is_dir".into(), HostType::Bool),
                ("is_file".into(), HostType::Bool),
            ])))),
        };
        for asynchronous in [false, true] {
            let name = format!(
                "fs_{}{}",
                operation.name(),
                if asynchronous { "" } else { "_sync" }
            );
            hosts.register(
                HostOp::new(
                    &name,
                    args.clone(),
                    result.clone(),
                    asynchronous,
                    move |args| {
                        let mut args = args.into_iter();
                        let Some(HostValue::String(path)) = args.next() else {
                            unreachable!("checked fs path")
                        };
                        let data = match args.next() {
                            Some(HostValue::Bytes(data)) => data,
                            None => vec![],
                            _ => unreachable!("checked fs bytes"),
                        };
                        if asynchronous {
                            HostReply::Pending(Box::pin(async move {
                                tokio::task::spawn_blocking(move || operation.execute(&path, &data))
                                    .await
                                    .map_err(|error| format!("filesystem worker failed: {error}"))?
                                    .map(Output::into_host)
                            }))
                        } else {
                            HostReply::Ready(operation.execute(&path, &data).map(Output::into_host))
                        }
                    },
                )
                .with_enum_result_channel(error_schema(), "Failed"),
            )?;
        }
    }
    Ok(())
}
