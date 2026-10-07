//! HTTP/1.1 serving in the owning native VM; buffered Web objects, shared TLS.
use crate::{
    HostCallback, HostContext, HostHandle, HostOp, HostReply, HostType, HostValue, Hosts, Result,
    http_headers::HeaderList, http_request::RequestObject, http_response::ResponseObject, tcp,
};
use futures_util::{StreamExt, stream::FuturesUnordered};
use http_body_util::{BodyExt, Full, Limited};
use hyper::{
    body::{Body as _, Incoming},
    server::conn::http1,
    service::service_fn,
};
use hyper_util::rt::TokioIo;
use std::{
    collections::BTreeMap, convert::Infallible, future::Future, net::SocketAddr, pin::Pin, rc::Rc,
    sync::Arc,
};
use tokio::sync::watch;
use tokio_rustls::TlsAcceptor;
const LIMIT: usize = crate::http_transport::FETCH_BODY_LIMIT;
const MAX_CONNECTIONS: usize = 1024;
type WireResponse = hyper::Response<Full<::bytes::Bytes>>;
type ConnectionFuture = Pin<Box<dyn Future<Output = ()>>>;
#[derive(Clone)]
pub struct Server(Rc<State>);
struct State {
    listener: tcp::Listener,
    stopping: watch::Sender<bool>,
    finished: watch::Sender<Option<Result<()>>>,
}
impl Server {
    pub fn start(
        context: &HostContext,
        hostname: &str,
        port: u16,
        tls: Option<Arc<rustls::ServerConfig>>,
        handler: HostCallback,
    ) -> Result<Self> {
        let server = Self(Rc::new(State {
            listener: tcp::Listener::bind(hostname, port)?,
            stopping: watch::channel(false).0,
            finished: watch::channel(None).0,
        }));
        let work = server.clone();
        let job = context.job()?;
        context.spawn(
            job,
            Box::pin(async move {
                let result = work.run(tls, handler).await;
                work.0.listener.close();
                work.0.finished.send_replace(Some(result.clone()));
                result.map(|()| HostValue::Unit)
            }),
        )?;
        Ok(server)
    }
    pub fn addr(&self) -> SocketAddr {
        self.0.listener.addr()
    }
    pub fn stop(&self) {
        self.0.stopping.send_replace(true);
        self.0.listener.close();
    }
    pub async fn finished(&self) -> Result<()> {
        let mut finished = self.0.finished.subscribe();
        loop {
            if let Some(result) = finished.borrow().clone() {
                return result;
            }
            finished
                .changed()
                .await
                .map_err(|_| "HTTP server completion was dropped")?;
        }
    }
    async fn run(
        &self,
        tls: Option<Arc<rustls::ServerConfig>>,
        handler: HostCallback,
    ) -> Result<()> {
        let mut stopping = self.0.stopping.subscribe();
        let mut connections = FuturesUnordered::<ConnectionFuture>::new();
        loop {
            if *stopping.borrow() {
                break;
            }
            tokio::select! {biased;
                _=stopping.changed()=>break,
                _=connections.next(),if !connections.is_empty()=>{},
                result=self.0.listener.accept(),if connections.len()<MAX_CONNECTIONS=>{
                    let conn=match result{Ok(c)=>c,Err(_)if *stopping.borrow()=>break,Err(e)=>return Err(e)};
                    let stream=conn.take_stream()?;let handler=handler.clone();let tls=tls.clone();let stop=stopping.clone();
                    connections.push(Box::pin(async move {
                        if let Some(config)=tls {
                            if *stop.borrow() {return;}
                            let mut close=stop.clone();
                            let handshake=TlsAcceptor::from(config).accept(stream);
                            tokio::select!{biased;
                                _=close.changed()=>{},
                                result=handshake=>if let Ok(stream)=result{serve_connection(stream,handler,stop,"https").await;},
                            }
                        }else{serve_connection(stream,handler,stop,"http").await;}
                    }));
                },
            }
        }
        // Each accepted HTTP connection receives graceful_shutdown. Its current
        // request is allowed to finish; idle connections stop immediately.
        while connections.next().await.is_some() {}
        Ok(())
    }
}
async fn serve_connection<I>(
    io: I,
    handler: HostCallback,
    mut stopping: watch::Receiver<bool>,
    scheme: &'static str,
) where
    I: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + 'static,
{
    let service = service_fn(move |req| {
        let handler = handler.clone();
        async move { Ok::<_, Infallible>(dispatch(req, handler, scheme).await) }
    });
    let connection = http1::Builder::new().serve_connection(TokioIo::new(io), service);
    tokio::pin!(connection);
    if *stopping.borrow() {
        connection.as_mut().graceful_shutdown();
        let _ = connection.await;
        return;
    }
    tokio::select! {biased;
        _=stopping.changed()=>{connection.as_mut().graceful_shutdown();let _=connection.await;},
        _=&mut connection=>{},
    }
}
fn error(status: u16, text: &'static str) -> WireResponse {
    hyper::Response::builder()
        .status(status)
        .header("content-type", "text/plain;charset=UTF-8")
        .body(Full::new(::bytes::Bytes::from_static(text.as_bytes())))
        .expect("fixed error response")
}
async fn dispatch(
    req: hyper::Request<Incoming>,
    handler: HostCallback,
    scheme: &str,
) -> WireResponse {
    let method = req.method().clone();
    let authority = match req
        .headers()
        .get(hyper::header::HOST)
        .and_then(|h| h.to_str().ok())
    {
        Some(h) => h.to_owned(),
        None => return error(400, "Invalid request authority"),
    };
    let path = req.uri().path_and_query().map_or("/", |p| p.as_str());
    let url = format!("{scheme}://{authority}{path}");
    let mut headers = HeaderList::default();
    for (name, value) in req.headers() {
        let text = value
            .as_bytes()
            .iter()
            .map(|&b| char::from(b))
            .collect::<String>();
        if headers.append(name.as_str(), &text).is_err() {
            return error(400, "Invalid request headers");
        }
    }
    // Apply the cap while collecting, including chunked input without a length.
    let empty = req.body().size_hint().upper() == Some(0);
    let body = match Limited::new(req.into_body(), LIMIT).collect().await {
        Ok(b) => {
            if empty {
                None
            } else {
                Some(b.to_bytes().to_vec())
            }
        }
        Err(e) if e.is::<http_body_util::LengthLimitError>() => {
            return error(413, "Request body exceeds buffered limit");
        }
        Err(_) => return error(400, "Invalid request body"),
    };
    let request = match RequestObject::from_parts(&url, method.as_str().into(), headers, body) {
        Ok(r) => r,
        Err(_) => return error(400, "Invalid request URL"),
    };
    let result = match handler.call_async(vec![request.into_host_value()]) {
        Ok(f) => f.await,
        Err(e) => Err(e),
    };
    let response = match result {
        Ok(HostValue::Handle(h)) => h,
        _ => return error(500, "Internal Server Error"),
    };
    let Some(response) = response.downcast_ref::<ResponseObject>() else {
        return error(500, "Internal Server Error");
    };
    let (status, headers, body) = match response.wire_parts() {
        Ok(p) => p,
        Err(_) => return error(500, "Internal Server Error"),
    };
    if body.len() > LIMIT {
        return error(500, "Response body exceeds buffered limit");
    }
    let mut builder = hyper::Response::builder().status(status);
    for (name, value) in headers.wire_entries() {
        if matches!(name.as_str(), "content-length" | "transfer-encoding") {
            continue;
        }
        builder = builder.header(name, value);
    }
    // Hyper strips HEAD bodies; preserve the computed representation length.
    if method == hyper::Method::HEAD {
        builder = builder.header(hyper::header::CONTENT_LENGTH, body.len());
    }
    let body = if method == hyper::Method::HEAD {
        vec![]
    } else {
        body
    };
    builder
        .body(Full::new(::bytes::Bytes::from(body)))
        .unwrap_or_else(|_| error(500, "Invalid response headers"))
}
pub fn handler_type() -> HostType {
    HostType::TypedCallback {
        args: vec![HostType::Handle("Request".into())],
        result: Box::new(HostType::Handle("Response".into())),
        result_channel: true,
    }
}
pub fn options_type() -> HostType {
    HostType::OptionalRecord(BTreeMap::from([
        ("hostname".into(), HostType::String),
        ("port".into(), HostType::Number),
        ("cert".into(), HostType::Bytes),
        ("key".into(), HostType::Bytes),
    ]))
}
fn decode(value: &HostValue) -> Result<Server> {
    let HostValue::Handle(h) = value else {
        return Err("invalid HTTP server receiver".into());
    };
    h.downcast_ref::<Server>()
        .cloned()
        .ok_or("invalid HTTP server resource".into())
}
pub fn register(hosts: &mut Hosts) -> Result<()> {
    let server = HostType::Handle("HttpServer".into());
    hosts.register(
        HostOp::with_context(
            "http_serve",
            vec![options_type(), handler_type()],
            server.clone(),
            false,
            |context, args| {
                let result = (|| {
                    let HostValue::Record(f) = &args[0] else {
                        unreachable!("checked serve options")
                    };
                    let host = match f.get("hostname") {
                        Some(HostValue::String(s)) => s.as_str(),
                        _ => "0.0.0.0",
                    };
                    let port = match f.get("port") {
                        Some(HostValue::Number(n)) => tcp::port(*n, true)?,
                        _ => 8000,
                    };
                    let tls = match (f.get("cert"), f.get("key")) {
                        (None, None) => None,
                        (Some(HostValue::Bytes(cert)), Some(HostValue::Bytes(key))) => {
                            Some(crate::tls::server_config(cert, key)?)
                        }
                        _ => return Err("serve TLS requires both cert and key PEM bytes".into()),
                    };
                    let HostValue::Callback(handler) = &args[1] else {
                        unreachable!("checked serve handler")
                    };
                    Server::start(context, host, port, tls, handler.clone())
                        .map(|s| HostValue::Handle(HostHandle::new("HttpServer", s)))
                })();
                HostReply::Ready(result)
            },
        )
        .with_result_channel(),
    )?;
    hosts.register(
        HostOp::new(
            "__http_server_addr",
            vec![server.clone()],
            tcp::address_type(),
            false,
            |args| HostReply::Ready(decode(&args[0]).map(|s| tcp::address(s.addr()))),
        )
        .with_receiver_property("HttpServer", "addr"),
    )?;
    for method in ["shutdown", "finished"] {
        hosts.register(
            HostOp::new(
                &format!("__http_server_{method}"),
                vec![server.clone()],
                HostType::Unit,
                true,
                move |args| {
                    let server = decode(&args[0]);
                    if method == "shutdown"
                        && let Ok(s) = &server
                    {
                        s.stop();
                    }
                    HostReply::Pending(Box::pin(async move {
                        server?.finished().await.map(|()| HostValue::Unit)
                    }))
                },
            )
            .with_receiver_method("HttpServer", method)
            .with_result_channel(),
        )?;
    }
    Ok(())
}
