use std::sync::Arc;

use crate::{DnsOptions, RuntimeState};

pub async fn serve_dns(_state: Arc<RuntimeState>, options: DnsOptions) -> Result<(), String> {
    Err(format!(
        "DNS transport not implemented (addr {})",
        options.addr
    ))
}
