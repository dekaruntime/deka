use std::net::SocketAddr;

pub fn bind_reuseport(addr: SocketAddr) -> Result<tokio::net::TcpListener, String> {
    use socket2::{Domain, Socket, Type};

    let domain = Domain::for_address(addr);
    let socket = Socket::new(domain, Type::STREAM, None)
        .map_err(|err| format!("socket create failed: {}", err))?;
    socket
        .set_reuse_address(true)
        .map_err(|err| format!("set_reuse_address failed: {}", err))?;
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        let fd = socket.as_raw_fd();
        let value: libc::c_int = 1;
        let rc = unsafe {
            libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_REUSEPORT,
                &value as *const _ as *const libc::c_void,
                std::mem::size_of_val(&value) as libc::socklen_t,
            )
        };
        if rc != 0 {
            return Err(format!(
                "set_reuse_port failed: {}",
                std::io::Error::last_os_error()
            ));
        }
    }
    socket
        .bind(&addr.into())
        .map_err(|err| format!("bind failed: {}", err))?;
    socket
        .listen(1024)
        .map_err(|err| format!("listen failed: {}", err))?;
    let listener: std::net::TcpListener = socket.into();
    listener
        .set_nonblocking(true)
        .map_err(|err| format!("set_nonblocking failed: {}", err))?;
    tokio::net::TcpListener::from_std(listener)
        .map_err(|err| format!("tokio listener failed: {}", err))
}
