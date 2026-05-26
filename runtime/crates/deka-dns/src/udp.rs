use std::{net::SocketAddr, sync::Arc};

use tokio::net::UdpSocket;
use tracing::{debug, warn};

use crate::{Resolver, Store};

pub async fn serve<S: Store>(
    addr: SocketAddr,
    resolver: Arc<Resolver<S>>,
) -> Result<(), std::io::Error> {
    let socket = UdpSocket::bind(addr).await?;
    tracing::info!("deka-dns UDP listening on {addr}");
    let mut buf = [0_u8; 1500];

    loop {
        let (len, peer) = socket.recv_from(&mut buf).await?;
        let query = buf[..len].to_vec();
        let resolver = Arc::clone(&resolver);

        match resolver.resolve_bytes(&query).await {
            Ok(response) => {
                if let Err(err) = socket.send_to(&response, peer).await {
                    warn!("failed to send DNS response to {peer}: {err}");
                } else {
                    debug!("answered DNS query from {peer}");
                }
            }
            Err(err) => warn!("failed to resolve DNS query from {peer}: {err}"),
        }
    }
}
