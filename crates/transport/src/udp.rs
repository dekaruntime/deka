use std::sync::Arc;

use crate::{RuntimeState, UdpOptions};

pub async fn serve_udp(_state: Arc<RuntimeState>, options: UdpOptions) -> Result<(), String> {
    Err(format!(
        "UDP transport not implemented (addr {})",
        options.addr
    ))
}
