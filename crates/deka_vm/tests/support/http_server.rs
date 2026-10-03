use std::{
    io::{self, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::Duration,
};

/// Blocking, bounded fixture sockets. Shutdown wakes accept; worker I/O failures
/// travel through joins, including during panic cleanup (no second panic).
pub struct Server {
    address: SocketAddr,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<io::Result<Vec<String>>>>,
}
impl Server {
    pub fn new(
        handler: impl Fn(&str, &mut TcpStream) -> io::Result<Vec<u8>> + Send + Sync + 'static,
    ) -> io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(false)?;
        let address = listener.local_addr()?;
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let handler = Arc::new(handler);
        let requests = Arc::new(Mutex::new(Vec::new()));
        let thread = std::thread::spawn(move || -> io::Result<Vec<String>> {
            let mut workers = vec![];
            loop {
                let (mut stream, _) = listener.accept()?;
                if stopping.load(Ordering::Acquire) {
                    break;
                }
                stream.set_nonblocking(false)?;
                stream.set_read_timeout(Some(Duration::from_secs(5)))?;
                stream.set_write_timeout(Some(Duration::from_secs(5)))?;
                let handler = handler.clone();
                let requests = requests.clone();
                workers.push(std::thread::spawn(move || -> io::Result<()> {
                    let mut bytes = Vec::new();
                    while !bytes.ends_with(b"\r\n\r\n") {
                        if bytes.len() >= 32768 {
                            return Err(io::Error::other("request header too large"));
                        }
                        let mut byte = [0];
                        stream.read_exact(&mut byte)?;
                        bytes.push(byte[0]);
                    }
                    let request = String::from_utf8(bytes).map_err(io::Error::other)?;
                    let path = request
                        .split_whitespace()
                        .nth(1)
                        .ok_or_else(|| io::Error::other("missing request path"))?;
                    requests
                        .lock()
                        .map_err(|e| io::Error::other(e.to_string()))?
                        .push(path.to_owned());
                    let response = handler(path, &mut stream)?;
                    stream.write_all(&response)
                }));
            }
            for worker in workers {
                worker
                    .join()
                    .map_err(|_| io::Error::other("HTTP fixture worker panicked"))??;
            }
            let seen = requests
                .lock()
                .map_err(|e| io::Error::other(e.to_string()))?
                .clone();
            Ok(seen)
        });
        Ok(Self {
            address,
            stop,
            thread: Some(thread),
        })
    }
    pub fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.address)
    }
    fn shutdown(&mut self) -> io::Result<Vec<String>> {
        self.stop.store(true, Ordering::Release);
        let wake = TcpStream::connect_timeout(&self.address, Duration::from_secs(2));
        let joined = self
            .thread
            .take()
            .ok_or_else(|| io::Error::other("fixture already joined"))?
            .join()
            .map_err(|_| io::Error::other("HTTP fixture server panicked"))?;
        // A server that already returned an I/O error may have closed its
        // listener. Prefer that diagnostic to the failed wake connection.
        let requests = joined?;
        wake?;
        Ok(requests)
    }
    pub fn finish(mut self) -> io::Result<Vec<String>> {
        self.shutdown()
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        if self.thread.is_some()
            && let Err(error) = self.shutdown()
        {
            eprintln!("HTTP fixture cleanup: {error}");
        }
    }
}
pub fn reply(status: &str, headers: &str, body: &[u8]) -> Vec<u8> {
    let mut bytes = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n",
        body.len()
    )
    .into_bytes();
    bytes.extend_from_slice(body);
    bytes
}
