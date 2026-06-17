use anyhow::{Context, Result, bail};
use std::collections::HashSet;
use std::fs;
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::path::Path;
use tokio::net::UnixStream;

use crate::protocol::ServiceReply;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PeerCredentials {
    pub(crate) pid: u32,
    pub(crate) uid: u32,
    pub(crate) gid: u32,
}

pub(crate) fn peer_credentials(stream: &UnixStream) -> Result<PeerCredentials> {
    let fd = stream.as_raw_fd();
    let mut cred = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let rc = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut cred as *mut _ as *mut libc::c_void,
            &mut len,
        )
    };
    if rc != 0 {
        return Err(io::Error::last_os_error()).context("getsockopt SO_PEERCRED");
    }
    peer_credentials_from_ucred(cred)
}

pub(crate) fn peer_credentials_from_ucred(cred: libc::ucred) -> Result<PeerCredentials> {
    Ok(PeerCredentials {
        pid: u32::try_from(cred.pid).context("peer pid is negative")?,
        uid: cred.uid,
        gid: cred.gid,
    })
}

pub(crate) fn authorize_peer(peer: PeerCredentials, orchestrator_gid: u32) -> Result<bool> {
    if peer.uid == 0 || peer.gid == orchestrator_gid {
        return Ok(true);
    }
    Ok(user_group_ids(peer.uid)?.contains(&orchestrator_gid))
}

pub(crate) fn forbidden_peer_reply() -> ServiceReply {
    ServiceReply::json_value(
        403,
        serde_json::json!({
            "error": "forbidden: gild-agent only accepts root or gild-orchestrator peers; gild-agents members are not authorized"
        }),
    )
}

pub(crate) fn group_gid(group: &str) -> Result<Option<u32>> {
    let raw = match fs::read_to_string("/etc/group") {
        Ok(raw) => raw,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).context("read /etc/group"),
    };
    Ok(raw.lines().find_map(|line| {
        let mut fields = line.split(':');
        let name = fields.next()?;
        let _password = fields.next()?;
        let gid = fields.next()?;
        (name == group).then(|| gid.parse::<u32>().ok()).flatten()
    }))
}

pub(crate) fn warn_agent_members_in_orchestrator(
    passwd_path: &str,
    group_path: &str,
    group_name: &str,
    group_gid: u32,
) {
    match agent_users_in_group(
        Path::new(passwd_path),
        Path::new(group_path),
        group_name,
        group_gid,
    ) {
        Ok(users) if !users.is_empty() => {
            eprintln!(
                "WARNING: agent users still belong to {group_name}: {}; run gild-agent-migrate-groups.sh",
                users.join(", ")
            );
        }
        Ok(_) => {}
        Err(err) => eprintln!("WARNING: could not audit {group_name} agent membership: {err:#}"),
    }
}

pub(crate) fn agent_users_in_group(
    passwd_path: &Path,
    group_path: &Path,
    group_name: &str,
    group_gid: u32,
) -> Result<Vec<String>> {
    let passwd = fs::read_to_string(passwd_path)
        .with_context(|| format!("read {}", passwd_path.display()))?;
    let agent_users = passwd
        .lines()
        .filter_map(passwd_agent_user)
        .collect::<Vec<_>>();
    let agent_set = agent_users
        .iter()
        .map(|(name, _)| name.clone())
        .collect::<HashSet<_>>();
    let mut unsafe_users = HashSet::new();

    for (name, primary_gid) in agent_users {
        if primary_gid == group_gid {
            unsafe_users.insert(name);
        }
    }

    let groups =
        fs::read_to_string(group_path).with_context(|| format!("read {}", group_path.display()))?;
    for line in groups.lines() {
        let mut fields = line.split(':');
        let Some(name) = fields.next() else { continue };
        let _password = fields.next();
        let _gid = fields.next();
        let members = fields.next().unwrap_or_default();
        if name == group_name {
            for member in members.split(',').filter(|member| !member.is_empty()) {
                if agent_set.contains(member) {
                    unsafe_users.insert(member.to_string());
                }
            }
        }
    }

    let mut users = unsafe_users.into_iter().collect::<Vec<_>>();
    users.sort();
    Ok(users)
}

fn passwd_agent_user(line: &str) -> Option<(String, u32)> {
    let mut fields = line.split(':');
    let name = fields.next()?;
    let _password = fields.next()?;
    let _uid = fields.next()?;
    let gid = fields.next()?.parse::<u32>().ok()?;
    name.starts_with("agent-").then(|| (name.to_string(), gid))
}

fn user_group_ids(uid: u32) -> Result<Vec<u32>> {
    let username = username_for_uid(uid)?;
    let Some(username) = username else {
        return Ok(Vec::new());
    };
    let raw = fs::read_to_string("/etc/group").context("read /etc/group")?;
    Ok(raw
        .lines()
        .filter_map(|line| group_line_contains_user(line, &username))
        .collect())
}

fn username_for_uid(uid: u32) -> Result<Option<String>> {
    let raw = match fs::read_to_string("/etc/passwd") {
        Ok(raw) => raw,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).context("read /etc/passwd"),
    };
    Ok(raw.lines().find_map(|line| {
        let mut fields = line.split(':');
        let name = fields.next()?;
        let _password = fields.next()?;
        let parsed_uid = fields.next()?.parse::<u32>().ok()?;
        (parsed_uid == uid).then(|| name.to_string())
    }))
}

pub(crate) fn group_line_contains_user(line: &str, username: &str) -> Option<u32> {
    let mut fields = line.split(':');
    let _name = fields.next()?;
    let _password = fields.next()?;
    let gid = fields.next()?.parse::<u32>().ok()?;
    let members = fields.next().unwrap_or_default();
    members
        .split(',')
        .any(|member| member == username)
        .then_some(gid)
}

pub(crate) fn prepare_socket_path(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_socket() => {
            fs::remove_file(path)
                .with_context(|| format!("remove stale socket {}", path.display()))?;
        }
        Ok(_) => bail!("{} exists and is not a socket", path.display()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => return Err(err).with_context(|| format!("stat {}", path.display())),
    }
    Ok(())
}

pub(crate) fn secure_socket(path: &Path, gid: u32) -> Result<()> {
    fs::set_permissions(path, fs::Permissions::from_mode(0o660))
        .with_context(|| format!("chmod 0660 {}", path.display()))?;
    let c_path = std::ffi::CString::new(path.as_os_str().as_encoded_bytes())
        .context("socket path contains NUL")?;
    let rc = unsafe { libc::chown(c_path.as_ptr(), 0, gid) };
    if rc != 0 {
        return Err(io::Error::last_os_error())
            .with_context(|| format!("chown root:{gid} {}", path.display()));
    }
    Ok(())
}
