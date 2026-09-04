use std::sync::Arc;

use crate::{RedisOptions, RuntimeState};

pub async fn serve_redis(_state: Arc<RuntimeState>, options: RedisOptions) -> Result<(), String> {
    Err(format!(
        "Redis transport not implemented (addr {})",
        options.addr
    ))
}
