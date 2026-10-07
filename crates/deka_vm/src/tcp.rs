//! Native TCP projection of the old net bridge: opaque ownership and raw bytes.
//! Tokio readiness replaces blocking sockets and timeout polling. No JSON bridge.
use crate::{HostHandle, HostOp, HostReply, HostType, HostValue, Hosts, Result};
use std::{cell::RefCell, collections::BTreeMap, net::SocketAddr, rc::Rc, sync::Arc};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::{Mutex, watch},
};

pub(crate) const MAX_READ: usize = 16 * 1024 * 1024;
#[derive(Clone)]
pub struct Connection(Rc<ConnectionState>);
struct ConnectionState {
    stream: RefCell<Option<Arc<TcpStream>>>,
    closed: watch::Sender<bool>,
    read: Mutex<()>,
    write: Mutex<()>,
    local: SocketAddr,
    remote: SocketAddr,
}
impl Connection {
    pub fn new(stream: TcpStream) -> Result<Self> {
        let local = stream.local_addr().map_err(|e| e.to_string())?;
        let remote = stream.peer_addr().map_err(|e| e.to_string())?;
        Ok(Self(Rc::new(ConnectionState {
            stream: RefCell::new(Some(Arc::new(stream))),
            closed: watch::channel(false).0,
            read: Mutex::new(()),
            write: Mutex::new(()),
            local,
            remote,
        })))
    }
    fn stream(&self) -> Result<Arc<TcpStream>> {
        self.0
            .stream
            .borrow()
            .clone()
            .ok_or_else(|| "TCP connection is closed".into())
    }
    pub fn close(&self) {
        self.0.closed.send_replace(true);
        self.0.stream.borrow_mut().take();
    }
    /// A TLS upgrade transfers ownership only when no raw I/O is pending.
    pub(crate) fn take_stream(&self) -> Result<TcpStream> {
        let _read = self
            .0
            .read
            .try_lock()
            .map_err(|_| "TCP connection has pending reads")?;
        let _write = self
            .0
            .write
            .try_lock()
            .map_err(|_| "TCP connection has pending writes")?;
        let mut slot = self.0.stream.borrow_mut();
        if slot.as_ref().is_some_and(|s| Arc::strong_count(s) != 1) {
            return Err("TCP connection has pending I/O".into());
        }
        let stream = slot.take().ok_or("TCP connection is closed")?;
        self.0.closed.send_replace(true);
        Arc::try_unwrap(stream).map_err(|_| "TCP connection has pending I/O".into())
    }
    pub async fn close_write(&self) -> Result<()> {
        let _guard = self.0.write.lock().await;
        socket2::SockRef::from(self.stream()?.as_ref())
            .shutdown(std::net::Shutdown::Write)
            .map_err(|e| e.to_string())
    }
    pub async fn read(&self, max: usize) -> Result<Option<Vec<u8>>> {
        let mut closed = self.0.closed.subscribe();
        if *closed.borrow() {
            return Err("TCP connection is closed".into());
        }
        tokio::select! {
            biased;
            _ = closed.changed() => Err("TCP connection is closed".into()),
            result = async {
                let _guard = self.0.read.lock().await;
                let stream = self.stream()?;
                let mut bytes = vec![0; max];
                loop {
                    stream.readable().await.map_err(|e| e.to_string())?;
                    match stream.try_read(&mut bytes) {
                        Ok(0) => return Ok(None),
                        Ok(count) => { bytes.truncate(count); return Ok(Some(bytes)); }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
                        Err(e) => return Err(e.to_string()),
                    }
                }
            } => result,
        }
    }
    pub async fn write(&self, bytes: &[u8]) -> Result<usize> {
        let mut closed = self.0.closed.subscribe();
        if *closed.borrow() {
            return Err("TCP connection is closed".into());
        }
        tokio::select! {
            biased;
            _ = closed.changed() => Err("TCP connection is closed".into()),
            result = async {
                let _guard = self.0.write.lock().await;
                let stream = self.stream()?;
                if bytes.is_empty() { return Ok(0); }
                loop {
                    stream.writable().await.map_err(|e| e.to_string())?;
                    match stream.try_write(bytes) {
                        Ok(count) => return Ok(count),
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
                        Err(e) => return Err(e.to_string()),
                    }
                }
            } => result,
        }
    }
    pub fn local_addr(&self) -> SocketAddr {
        self.0.local
    }
    pub fn remote_addr(&self) -> SocketAddr {
        self.0.remote
    }
}
#[derive(Clone)]
pub struct Listener(Rc<ListenerState>);
struct ListenerState {
    listener: RefCell<Option<Rc<TcpListener>>>,
    closed: watch::Sender<bool>,
    addr: SocketAddr,
}
impl Listener {
    pub fn bind(hostname: &str, port: u16) -> Result<Self> {
        // Deno's synchronous listen accepts a literal IP; DNS belongs to connect.
        let ip = hostname
            .parse::<std::net::IpAddr>()
            .map_err(|_| "listen hostname must be an IP address")?;
        let listener = std::net::TcpListener::bind((ip, port)).map_err(|e| e.to_string())?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let listener = TcpListener::from_std(listener).map_err(|e| e.to_string())?;
        let addr = listener.local_addr().map_err(|e| e.to_string())?;
        Ok(Self(Rc::new(ListenerState {
            listener: RefCell::new(Some(Rc::new(listener))),
            closed: watch::channel(false).0,
            addr,
        })))
    }
    pub fn addr(&self) -> SocketAddr {
        self.0.addr
    }
    pub fn close(&self) {
        self.0.closed.send_replace(true);
        self.0.listener.borrow_mut().take();
    }
    pub(crate) fn closed(&self) -> watch::Receiver<bool> {
        self.0.closed.subscribe()
    }
    pub async fn accept(&self) -> Result<Connection> {
        let mut closed = self.0.closed.subscribe();
        let listener = self
            .0
            .listener
            .borrow()
            .clone()
            .ok_or("TCP listener is closed")?;
        tokio::select! {
            biased;
            _ = closed.changed() => Err("TCP listener is closed".into()),
            result = listener.accept() => Connection::new(result.map_err(|e| e.to_string())?.0),
        }
    }
}
pub(crate) fn address_type() -> HostType {
    HostType::Record(BTreeMap::from([
        ("hostname".into(), HostType::String),
        ("port".into(), HostType::Number),
        ("transport".into(), HostType::String),
    ]))
}
pub(crate) fn address(addr: SocketAddr) -> HostValue {
    HostValue::Record(BTreeMap::from([
        ("hostname".into(), HostValue::String(addr.ip().to_string())),
        ("port".into(), HostValue::Number(f64::from(addr.port()))),
        ("transport".into(), HostValue::String("tcp".into())),
    ]))
}
pub(crate) fn port(number: f64, allow_zero: bool) -> Result<u16> {
    if !number.is_finite()
        || number.fract() != 0.
        || !(if allow_zero { 0. } else { 1. }..=65535.).contains(&number)
    {
        return Err("port must be an integer in the allowed TCP range".into());
    }
    Ok(number as u16)
}
pub(crate) fn options_type() -> HostType {
    HostType::Record(BTreeMap::from([
        (
            "hostname".into(),
            HostType::Option(Box::new(HostType::String)),
        ),
        ("port".into(), HostType::Number),
    ]))
}
pub(crate) fn options(value: &HostValue, default: &str, allow_zero: bool) -> Result<(String, u16)> {
    let HostValue::Record(fields) = value else {
        return Err("invalid TCP options".into());
    };
    let HostValue::Number(number) = fields["port"] else {
        return Err("invalid TCP port".into());
    };
    let hostname = match fields.get("hostname") {
        Some(HostValue::Option(Some(name))) => match name.as_ref() {
            HostValue::String(name) => name.clone(),
            _ => return Err("invalid TCP hostname".into()),
        },
        _ => default.into(),
    };
    Ok((hostname, port(number, allow_zero)?))
}
fn listener(value: &HostValue) -> Result<Listener> {
    let HostValue::Handle(handle) = value else {
        return Err("invalid TCP listener receiver".into());
    };
    handle
        .downcast_ref::<Listener>()
        .cloned()
        .ok_or("invalid TCP listener resource".into())
}
pub fn handle(conn: Connection) -> HostValue {
    HostValue::Handle(HostHandle::new("TcpConn", conn))
}
pub fn register(hosts: &mut Hosts) -> Result<()> {
    let conn = HostType::Handle("TcpConn".into());
    let listen = HostType::Handle("TcpListener".into());
    hosts.register(
        HostOp::new(
            "tcp_connect",
            vec![options_type()],
            conn.clone(),
            true,
            |args| {
                let target = options(&args[0], "127.0.0.1", false);
                HostReply::Pending(Box::pin(async move {
                    let (host, port) = target?;
                    Connection::new(
                        TcpStream::connect((host.as_str(), port))
                            .await
                            .map_err(|e| e.to_string())?,
                    )
                    .map(handle)
                }))
            },
        )
        .with_result_channel(),
    )?;
    hosts.register(
        HostOp::new(
            "tcp_listen",
            vec![options_type()],
            listen.clone(),
            false,
            |args| {
                HostReply::Ready(
                    options(&args[0], "0.0.0.0", true)
                        .and_then(|(host, port)| Listener::bind(&host, port))
                        .map(|l| HostValue::Handle(HostHandle::new("TcpListener", l))),
                )
            },
        )
        .with_result_channel(),
    )?;
    hosts.register(
        HostOp::new(
            "__tcp_accept",
            vec![listen.clone()],
            conn.clone(),
            true,
            |args| {
                let l = listener(&args[0]);
                HostReply::Pending(Box::pin(async move { l?.accept().await.map(handle) }))
            },
        )
        .with_receiver_method("TcpListener", "accept")
        .with_result_channel(),
    )?;
    register_connection::<Connection>(hosts, "tcp")?;
    for (brand, ty) in [("TcpListener", listen)] {
        hosts.register(
            HostOp::new(
                &format!("__{brand}_close"),
                vec![ty.clone()],
                HostType::Unit,
                false,
                move |args| {
                    HostReply::Ready(
                        listener(&args[0])
                            .map(|l| l.close())
                            .map(|()| HostValue::Unit),
                    )
                },
            )
            .with_receiver_method(brand, "close")
            .with_result_channel(),
        )?;
        let fields = &["addr"][..];
        for &field in fields {
            hosts.register(
                HostOp::new(
                    &format!("__{brand}_{field}"),
                    vec![ty.clone()],
                    address_type(),
                    false,
                    move |args| HostReply::Ready(listener(&args[0]).map(|l| address(l.addr()))),
                )
                .with_receiver_property(brand, field),
            )?;
        }
    }
    Ok(())
}

/// TCP and TLS expose the same typed byte contract from one host catalog.
pub(crate) trait ByteConnection: Clone + 'static {
    const BRAND: &'static str;
    async fn read(&self, max: usize) -> Result<Option<Vec<u8>>>;
    async fn write(&self, bytes: &[u8]) -> Result<usize>;
    async fn close_write(&self) -> Result<()>;
    fn close(&self);
    fn local_addr(&self) -> SocketAddr;
    fn remote_addr(&self) -> SocketAddr;
}
impl ByteConnection for Connection {
    const BRAND: &'static str = "TcpConn";
    async fn read(&self, max: usize) -> Result<Option<Vec<u8>>> {
        self.read(max).await
    }
    async fn write(&self, bytes: &[u8]) -> Result<usize> {
        self.write(bytes).await
    }
    async fn close_write(&self) -> Result<()> {
        self.close_write().await
    }
    fn close(&self) {
        self.close()
    }
    fn local_addr(&self) -> SocketAddr {
        self.local_addr()
    }
    fn remote_addr(&self) -> SocketAddr {
        self.remote_addr()
    }
}
fn decode<C: ByteConnection>(value: &HostValue) -> Result<C> {
    let HostValue::Handle(handle) = value else {
        return Err("invalid connection receiver".into());
    };
    handle
        .downcast_ref::<C>()
        .cloned()
        .ok_or("invalid connection resource".into())
}
pub(crate) fn register_connection<C: ByteConnection>(
    hosts: &mut Hosts,
    prefix: &str,
) -> Result<()> {
    let conn = HostType::Handle(C::BRAND.into());
    hosts.register(
        HostOp::new(
            &format!("__{prefix}_read"),
            vec![conn.clone(), HostType::Number],
            HostType::Option(Box::new(HostType::Bytes)),
            true,
            |args| {
                let c = decode::<C>(&args[0]);
                let HostValue::Number(max) = args[1] else {
                    unreachable!("checked read max");
                };
                HostReply::Pending(Box::pin(async move {
                    if !max.is_finite()
                        || max.fract() != 0.
                        || !(1. ..=MAX_READ as f64).contains(&max)
                    {
                        return Err("read max must be an integer from 1 to 16777216".into());
                    }
                    c?.read(max as usize)
                        .await
                        .map(|v| HostValue::Option(v.map(|b| Box::new(HostValue::Bytes(b)))))
                }))
            },
        )
        .with_receiver_method(C::BRAND, "read")
        .with_result_channel(),
    )?;
    hosts.register(
        HostOp::new(
            &format!("__{prefix}_write"),
            vec![conn.clone(), HostType::Bytes],
            HostType::Number,
            true,
            |args| {
                let c = decode::<C>(&args[0]);
                let HostValue::Bytes(bytes) = args[1].clone() else {
                    unreachable!("checked write bytes");
                };
                HostReply::Pending(Box::pin(async move {
                    c?.write(&bytes).await.map(|n| HostValue::Number(n as f64))
                }))
            },
        )
        .with_receiver_method(C::BRAND, "write")
        .with_result_channel(),
    )?;

    hosts.register(
        HostOp::new(
            &format!("__{}_closeWrite", C::BRAND),
            vec![conn.clone()],
            HostType::Unit,
            true,
            |args| {
                let c = decode::<C>(&args[0]);
                HostReply::Pending(Box::pin(async move {
                    c?.close_write().await.map(|()| HostValue::Unit)
                }))
            },
        )
        .with_receiver_method(C::BRAND, "closeWrite")
        .with_result_channel(),
    )?;
    hosts.register(
        HostOp::new(
            &format!("__{}_close", C::BRAND),
            vec![conn.clone()],
            HostType::Unit,
            false,
            |args| {
                HostReply::Ready(decode::<C>(&args[0]).map(|c| {
                    c.close();
                    HostValue::Unit
                }))
            },
        )
        .with_receiver_method(C::BRAND, "close")
        .with_result_channel(),
    )?;
    for field in ["localAddr", "remoteAddr"] {
        hosts.register(
            HostOp::new(
                &format!("__{}_{field}", C::BRAND),
                vec![conn.clone()],
                address_type(),
                false,
                move |args| {
                    HostReply::Ready(decode::<C>(&args[0]).map(|c| {
                        address(if field == "localAddr" {
                            c.local_addr()
                        } else {
                            c.remote_addr()
                        })
                    }))
                },
            )
            .with_receiver_property(C::BRAND, field),
        )?;
    }
    Ok(())
}
