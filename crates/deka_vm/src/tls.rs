//! Rustls TLS on the native TCP ownership model; no JavaScript or second TLS stack.
use crate::{HostHandle, HostOp, HostReply, HostType, HostValue, Hosts, Result, tcp};
use rustls::pki_types::{CertificateDer, ServerName};
use std::{collections::BTreeMap, net::SocketAddr, rc::Rc, sync::Arc};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, ReadHalf, WriteHalf},
    net::TcpStream,
    sync::{Mutex, watch},
};
use tokio_rustls::{TlsAcceptor, TlsConnector, TlsStream};

type Read = ReadHalf<TlsStream<TcpStream>>;
type Write = WriteHalf<TlsStream<TcpStream>>;
#[derive(Clone)]
pub struct Connection(Rc<State>);
struct State {
    read: Mutex<Option<Read>>,
    write: Mutex<Option<Write>>,
    closed: watch::Sender<bool>,
    local: SocketAddr,
    remote: SocketAddr,
}
impl Connection {
    pub fn new(stream: TlsStream<TcpStream>) -> Result<Self> {
        let local = stream.get_ref().0.local_addr().map_err(|e| e.to_string())?;
        let remote = stream.get_ref().0.peer_addr().map_err(|e| e.to_string())?;
        let (read, write) = tokio::io::split(stream);
        Ok(Self(Rc::new(State {
            read: Mutex::new(Some(read)),
            write: Mutex::new(Some(write)),
            closed: watch::channel(false).0,
            local,
            remote,
        })))
    }
    fn release_closed(&self) {
        if *self.0.closed.borrow() {
            if let Ok(mut read) = self.0.read.try_lock() {
                read.take();
            }
            if let Ok(mut write) = self.0.write.try_lock() {
                write.take();
            }
        }
    }
    pub fn close(&self) {
        self.0.closed.send_replace(true);
        self.release_closed();
    }
    pub async fn read(&self, max: usize) -> Result<Option<Vec<u8>>> {
        let mut closed = self.0.closed.subscribe();
        if *closed.borrow() {
            return Err("TLS connection is closed".into());
        }
        let result = tokio::select! {biased;
            _=closed.changed()=>Err("TLS connection is closed".into()),
            result=async {
                let mut guard=self.0.read.lock().await;
                let read=guard.as_mut().ok_or("TLS connection is closed")?;
                let mut bytes=vec![0;max];
                let n=read.read(&mut bytes).await.map_err(|e|format!("TLS read: {e}"))?;
                bytes.truncate(n);Ok(if n==0{None}else{Some(bytes)})
            }=>result,
        };
        self.release_closed();
        result
    }
    pub async fn write(&self, bytes: &[u8]) -> Result<usize> {
        let mut closed = self.0.closed.subscribe();
        if *closed.borrow() {
            return Err("TLS connection is closed".into());
        }
        let result = tokio::select! {biased;
            _=closed.changed()=>Err("TLS connection is closed".into()),
            result=async {
                let mut guard=self.0.write.lock().await;
                let write=guard.as_mut().ok_or("TLS connection is closed")?;
                let n=write.write(bytes).await.map_err(|e|format!("TLS write: {e}"))?;
                // Tokio-rustls writes buffer plaintext. Flush before acknowledging delivery.
                write.flush().await.map_err(|e|format!("TLS flush: {e}"))?;Ok(n)
            }=>result,
        };
        self.release_closed();
        result
    }
    pub async fn close_write(&self) -> Result<()> {
        let mut closed = self.0.closed.subscribe();
        if *closed.borrow() {
            return Err("TLS connection is closed".into());
        }
        let result = tokio::select! {biased;
            _=closed.changed()=>Err("TLS connection is closed".into()),
            result=async {
                let mut guard=self.0.write.lock().await;
                guard.as_mut().ok_or("TLS connection is closed")?.shutdown().await.map_err(|e|format!("TLS closeWrite: {e}"))
            }=>result,
        };
        self.release_closed();
        result
    }
    pub fn local_addr(&self) -> SocketAddr {
        self.0.local
    }
    pub fn remote_addr(&self) -> SocketAddr {
        self.0.remote
    }
}
impl tcp::ByteConnection for Connection {
    const BRAND: &'static str = "TlsConn";
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
#[derive(Clone)]
pub struct Listener {
    tcp: tcp::Listener,
    config: Arc<rustls::ServerConfig>,
}
impl Listener {
    pub fn bind(hostname: &str, port: u16, config: Arc<rustls::ServerConfig>) -> Result<Self> {
        Ok(Self {
            tcp: tcp::Listener::bind(hostname, port)?,
            config,
        })
    }
    pub fn addr(&self) -> SocketAddr {
        self.tcp.addr()
    }
    pub fn close(&self) {
        self.tcp.close()
    }
    pub async fn accept(&self) -> Result<Connection> {
        let mut closed = self.tcp.closed();
        if *closed.borrow() {
            return Err("TLS listener is closed".into());
        }
        tokio::select! {biased;
            _=closed.changed()=>Err("TLS listener is closed".into()),
            result=async {
                let stream=self.tcp.accept().await?.take_stream()?;
                let stream=TlsAcceptor::from(self.config.clone()).accept(stream).await.map_err(handshake_error)?;
                Connection::new(stream.into())
            }=>result,
        }
    }
}
fn certs(pem: &[u8]) -> Result<Vec<CertificateDer<'static>>> {
    let certs = rustls_pemfile::certs(&mut &pem[..])
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|_| "TLS configuration: invalid certificate PEM")?;
    if certs.is_empty() {
        return Err("TLS configuration: no certificates found".into());
    }
    Ok(certs)
}
fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::aws_lc_rs::default_provider())
}
pub fn client_config(roots: &[Vec<u8>]) -> Result<Arc<rustls::ClientConfig>> {
    let provider = provider();
    let roots = roots
        .iter()
        .map(|pem| certs(pem))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .flatten();
    // Match native fetch's platform trust plus explicit extra roots. This is a
    // verifying implementation, never a permissive/custom no-verification shim.
    let verifier =
        rustls_platform_verifier::Verifier::new_with_extra_roots(roots, provider.clone())
            .map_err(|e| format!("TLS configuration: {e}"))?;
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| format!("TLS configuration: {e}"))?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth();
    Ok(Arc::new(config))
}
pub fn server_config(cert: &[u8], key: &[u8]) -> Result<Arc<rustls::ServerConfig>> {
    let certs = certs(cert)?;
    let key = rustls_pemfile::private_key(&mut &key[..])
        .map_err(|_| "TLS configuration: invalid private key PEM")?
        .ok_or("TLS configuration: no private key found")?;
    let config = rustls::ServerConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .map_err(|e| format!("TLS configuration: {e}"))?
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| format!("TLS configuration: {e}"))?;
    Ok(Arc::new(config))
}
fn handshake_error(error: std::io::Error) -> String {
    if matches!(
        error
            .get_ref()
            .and_then(|e| e.downcast_ref::<rustls::Error>()),
        Some(rustls::Error::InvalidCertificate(_))
    ) {
        format!("TLS certificate verification: {error}")
    } else {
        format!("TLS handshake: {error}")
    }
}
pub async fn start_tls(
    conn: tcp::Connection,
    hostname: String,
    config: Arc<rustls::ClientConfig>,
) -> Result<Connection> {
    let name = ServerName::try_from(hostname).map_err(|_| "TLS configuration: invalid hostname")?;
    let stream = conn.take_stream()?;
    let stream = TlsConnector::from(config)
        .connect(name, stream)
        .await
        .map_err(handshake_error)?;
    Connection::new(stream.into())
}
pub async fn connect(
    hostname: String,
    port: u16,
    config: Arc<rustls::ClientConfig>,
) -> Result<Connection> {
    // Validate SNI before connecting or transferring a raw socket.
    ServerName::try_from(hostname.clone()).map_err(|_| "TLS configuration: invalid hostname")?;
    let stream = TcpStream::connect((hostname.as_str(), port))
        .await
        .map_err(|e| format!("TLS transport: {e}"))?;
    start_tls(tcp::Connection::new(stream)?, hostname, config).await
}
fn handle(conn: Connection) -> HostValue {
    HostValue::Handle(HostHandle::new("TlsConn", conn))
}
fn fields(value: &HostValue) -> Result<&BTreeMap<String, HostValue>> {
    match value {
        HostValue::Record(f) => Ok(f),
        _ => Err("invalid TLS options".into()),
    }
}
fn hostname(f: &BTreeMap<String, HostValue>, default: &str) -> String {
    match f.get("hostname") {
        Some(HostValue::Option(Some(v))) => match v.as_ref() {
            HostValue::String(s) => s.clone(),
            _ => unreachable!("checked hostname"),
        },
        _ => default.into(),
    }
}
fn client_options(value: &HostValue) -> Result<(String, Arc<rustls::ClientConfig>)> {
    let f = fields(value)?;
    let roots = match f.get("caCerts") {
        Some(HostValue::Option(Some(v))) => match v.as_ref() {
            HostValue::List(items) => items
                .iter()
                .map(|v| match v {
                    HostValue::Bytes(b) => b.clone(),
                    _ => unreachable!("checked CA bytes"),
                })
                .collect(),
            _ => unreachable!("checked CA list"),
        },
        _ => vec![],
    };
    Ok((hostname(f, "127.0.0.1"), client_config(&roots)?))
}
fn client_type() -> BTreeMap<String, HostType> {
    BTreeMap::from([
        (
            "hostname".into(),
            HostType::Option(Box::new(HostType::String)),
        ),
        (
            "caCerts".into(),
            HostType::Option(Box::new(HostType::List(Box::new(HostType::Bytes)))),
        ),
    ])
}
fn listener(value: &HostValue) -> Result<Listener> {
    let HostValue::Handle(h) = value else {
        return Err("invalid TLS listener".into());
    };
    h.downcast_ref::<Listener>()
        .cloned()
        .ok_or("invalid TLS listener resource".into())
}
pub fn register(hosts: &mut Hosts) -> Result<()> {
    let conn = HostType::Handle("TlsConn".into());
    let listen = HostType::Handle("TlsListener".into());
    let mut connect_options = client_type();
    connect_options.insert("port".into(), HostType::Number);
    hosts.register(
        HostOp::new(
            "tls_connectTls",
            vec![HostType::Record(connect_options)],
            conn.clone(),
            true,
            |args| {
                let options = client_options(&args[0]).and_then(|(h, c)| {
                    let HostValue::Number(p) = fields(&args[0])?["port"] else {
                        unreachable!("checked port");
                    };
                    Ok((h, tcp::port(p, false)?, c))
                });
                HostReply::Pending(Box::pin(async move {
                    let (h, p, c) = options?;
                    connect(h, p, c).await.map(handle)
                }))
            },
        )
        .with_result_channel(),
    )?;
    hosts.register(
        HostOp::new(
            "tls_startTls",
            vec![
                HostType::Handle("TcpConn".into()),
                HostType::Record(client_type()),
            ],
            conn.clone(),
            true,
            |args| {
                let options = client_options(&args[1]);
                let raw = match &args[0] {
                    HostValue::Handle(h) => h
                        .downcast_ref::<tcp::Connection>()
                        .cloned()
                        .ok_or("invalid TCP resource"),
                    _ => Err("invalid TCP receiver"),
                };
                HostReply::Pending(Box::pin(async move {
                    let (h, c) = options?;
                    start_tls(raw?, h, c).await.map(handle)
                }))
            },
        )
        .with_result_channel(),
    )?;
    let mut server_options = tcp::options_type();
    if let HostType::Record(f) = &mut server_options {
        f.insert("cert".into(), HostType::Bytes);
        f.insert("key".into(), HostType::Bytes);
    }
    hosts.register(
        HostOp::new(
            "tls_listenTls",
            vec![server_options],
            listen.clone(),
            false,
            |args| {
                let result = (|| {
                    let (h, p) = tcp::options(&args[0], "0.0.0.0", true)?;
                    let f = fields(&args[0])?;
                    let (HostValue::Bytes(cert), HostValue::Bytes(key)) = (&f["cert"], &f["key"])
                    else {
                        unreachable!("checked PEM bytes");
                    };
                    Listener::bind(&h, p, server_config(cert, key)?)
                        .map(|l| HostValue::Handle(HostHandle::new("TlsListener", l)))
                })();
                HostReply::Ready(result)
            },
        )
        .with_result_channel(),
    )?;
    hosts.register(
        HostOp::new("__tls_accept", vec![listen.clone()], conn, true, |args| {
            let l = listener(&args[0]);
            HostReply::Pending(Box::pin(async move { l?.accept().await.map(handle) }))
        })
        .with_receiver_method("TlsListener", "accept")
        .with_result_channel(),
    )?;
    hosts.register(
        HostOp::new(
            "__TlsListener_close",
            vec![listen.clone()],
            HostType::Unit,
            false,
            |args| {
                HostReply::Ready(listener(&args[0]).map(|l| {
                    l.close();
                    HostValue::Unit
                }))
            },
        )
        .with_receiver_method("TlsListener", "close")
        .with_result_channel(),
    )?;
    hosts.register(
        HostOp::new(
            "__TlsListener_addr",
            vec![listen],
            tcp::address_type(),
            false,
            |args| HostReply::Ready(listener(&args[0]).map(|l| tcp::address(l.addr()))),
        )
        .with_receiver_property("TlsListener", "addr"),
    )?;
    tcp::register_connection::<Connection>(hosts, "tls")
}
