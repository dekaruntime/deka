use std::{
    io::{IoSlice, IoSliceMut},
    net::{IpAddr, Ipv4Addr, SocketAddr},
    os::fd::{AsFd, AsRawFd, RawFd},
    sync::Arc,
};

use nix::{
    cmsg_space, libc,
    sys::socket::{
        ControlMessage, ControlMessageOwned, MsgFlags, SockaddrStorage, recvmsg, sendmsg,
        setsockopt, sockopt,
    },
};
use tokio::io::unix::AsyncFd;
use tracing::{debug, warn};

use crate::{Resolver, Store};

#[derive(Debug)]
struct Datagram {
    len: usize,
    peer: SocketAddr,
    local_addr: Option<IpAddr>,
}

pub async fn serve<S: Store>(
    addr: SocketAddr,
    resolver: Arc<Resolver<S>>,
) -> Result<(), std::io::Error> {
    let socket = std::net::UdpSocket::bind(addr)?;
    enable_packet_info(&socket, addr)?;
    socket.set_nonblocking(true)?;
    let socket = AsyncFd::new(socket)?;
    tracing::info!("deka-dns UDP listening on {addr}");
    let mut buf = [0_u8; 1500];

    loop {
        let datagram = recv_datagram(&socket, &mut buf).await?;
        let peer = datagram.peer;
        let local_addr = datagram.local_addr;
        let query = buf[..datagram.len].to_vec();
        let resolver = Arc::clone(&resolver);

        match resolver.resolve_bytes(&query).await {
            Ok(response) => {
                if let Err(err) = send_response(&socket, &response, peer, local_addr).await {
                    warn!("failed to send DNS response to {peer}: {err}");
                } else {
                    debug!("answered DNS query from {peer}");
                }
            }
            Err(err) => warn!("failed to resolve DNS query from {peer}: {err}"),
        }
    }
}

fn enable_packet_info(
    socket: &std::net::UdpSocket,
    addr: SocketAddr,
) -> Result<(), std::io::Error> {
    let fd = socket.as_fd();
    match addr {
        SocketAddr::V4(_) => setsockopt(&fd, sockopt::Ipv4PacketInfo, &true)?,
        SocketAddr::V6(_) => setsockopt(&fd, sockopt::Ipv6RecvPacketInfo, &true)?,
    }
    Ok(())
}

async fn recv_datagram(
    socket: &AsyncFd<std::net::UdpSocket>,
    buf: &mut [u8],
) -> Result<Datagram, std::io::Error> {
    loop {
        let mut ready = socket.readable().await?;
        match ready.try_io(|inner| recv_datagram_now(inner.get_ref().as_raw_fd(), buf)) {
            Ok(result) => return result,
            Err(_would_block) => continue,
        }
    }
}

fn recv_datagram_now(fd: RawFd, buf: &mut [u8]) -> Result<Datagram, std::io::Error> {
    let mut iov = [IoSliceMut::new(buf)];
    let mut cmsg = cmsg_space!(libc::in_pktinfo, libc::in6_pktinfo);
    let msg = recvmsg::<SockaddrStorage>(fd, &mut iov, Some(&mut cmsg), MsgFlags::MSG_DONTWAIT)?;

    let peer = msg
        .address
        .as_ref()
        .and_then(socket_addr_from_storage)
        .ok_or_else(|| std::io::Error::other("UDP datagram missing peer address"))?;
    let local_addr = msg.cmsgs()?.find_map(local_addr_from_cmsg);

    Ok(Datagram {
        len: msg.bytes,
        peer,
        local_addr,
    })
}

fn socket_addr_from_storage(addr: &SockaddrStorage) -> Option<SocketAddr> {
    if let Some(addr) = addr.as_sockaddr_in() {
        return Some(SocketAddr::from(*addr));
    }
    if let Some(addr) = addr.as_sockaddr_in6() {
        return Some(SocketAddr::from(*addr));
    }
    None
}

fn local_addr_from_cmsg(cmsg: ControlMessageOwned) -> Option<IpAddr> {
    match cmsg {
        ControlMessageOwned::Ipv4PacketInfo(pktinfo) => {
            Some(IpAddr::V4(ipv4_from_in_addr(pktinfo.ipi_addr)))
        }
        ControlMessageOwned::Ipv6PacketInfo(pktinfo) => {
            Some(IpAddr::V6(pktinfo.ipi6_addr.s6_addr.into()))
        }
        _ => None,
    }
}

async fn send_response(
    socket: &AsyncFd<std::net::UdpSocket>,
    response: &[u8],
    peer: SocketAddr,
    local_addr: Option<IpAddr>,
) -> Result<(), std::io::Error> {
    loop {
        let mut ready = socket.writable().await?;
        match ready.try_io(|inner| {
            send_response_now(inner.get_ref().as_raw_fd(), response, peer, local_addr)
        }) {
            Ok(result) => return result,
            Err(_would_block) => continue,
        }
    }
}

fn send_response_now(
    fd: RawFd,
    response: &[u8],
    peer: SocketAddr,
    local_addr: Option<IpAddr>,
) -> Result<(), std::io::Error> {
    let iov = [IoSlice::new(response)];
    let dst = SockaddrStorage::from(peer);

    match (peer, local_addr) {
        (SocketAddr::V4(_), Some(IpAddr::V4(src))) => {
            let pktinfo = ipv4_pktinfo_for_source(src);
            let cmsgs = [ControlMessage::Ipv4PacketInfo(&pktinfo)];
            sendmsg(fd, &iov, &cmsgs, MsgFlags::MSG_DONTWAIT, Some(&dst))?;
        }
        (SocketAddr::V6(_), Some(IpAddr::V6(src))) => {
            let pktinfo = libc::in6_pktinfo {
                ipi6_addr: libc::in6_addr {
                    s6_addr: src.octets(),
                },
                ipi6_ifindex: 0,
            };
            let cmsgs = [ControlMessage::Ipv6PacketInfo(&pktinfo)];
            sendmsg(fd, &iov, &cmsgs, MsgFlags::MSG_DONTWAIT, Some(&dst))?;
        }
        _ => {
            sendmsg(fd, &iov, &[], MsgFlags::MSG_DONTWAIT, Some(&dst))?;
        }
    }

    Ok(())
}

fn ipv4_pktinfo_for_source(src: Ipv4Addr) -> libc::in_pktinfo {
    libc::in_pktinfo {
        ipi_ifindex: 0,
        ipi_spec_dst: in_addr_from_ipv4(src),
        ipi_addr: in_addr_from_ipv4(Ipv4Addr::UNSPECIFIED),
    }
}

fn ipv4_from_in_addr(addr: libc::in_addr) -> Ipv4Addr {
    Ipv4Addr::from(addr.s_addr.to_ne_bytes())
}

fn in_addr_from_ipv4(addr: Ipv4Addr) -> libc::in_addr {
    libc::in_addr {
        s_addr: u32::from_ne_bytes(addr.octets()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipv4_pktinfo_source_round_trips() {
        let source = Ipv4Addr::new(10, 20, 30, 40);
        let pktinfo = ipv4_pktinfo_for_source(source);

        assert_eq!(source, ipv4_from_in_addr(pktinfo.ipi_spec_dst));
        assert_eq!(Ipv4Addr::UNSPECIFIED, ipv4_from_in_addr(pktinfo.ipi_addr));
    }

    #[test]
    fn local_addr_extracts_ipv4_pktinfo_destination() {
        let dst = Ipv4Addr::new(203, 0, 113, 10);
        let pktinfo = libc::in_pktinfo {
            ipi_ifindex: 0,
            ipi_spec_dst: in_addr_from_ipv4(Ipv4Addr::UNSPECIFIED),
            ipi_addr: in_addr_from_ipv4(dst),
        };
        let cmsg = ControlMessageOwned::Ipv4PacketInfo(pktinfo);

        assert_eq!(Some(IpAddr::V4(dst)), local_addr_from_cmsg(cmsg));
    }
}
