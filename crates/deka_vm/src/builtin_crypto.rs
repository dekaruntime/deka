//! Imported crypto functions share the same engines as the crypto global.
use crate::{HostOp, HostReply, HostType, HostValue, Hosts, Result};
pub fn register(hosts: &mut Hosts) -> Result<()> {
    hosts.register(
        HostOp::new(
            "crypto_sha256",
            vec![HostType::Bytes],
            HostType::Bytes,
            true,
            |args| {
                let Some(HostValue::Bytes(bytes)) = args.into_iter().next() else {
                    unreachable!("checked sha256 input");
                };
                HostReply::Pending(Box::pin(async move {
                    tokio::task::spawn_blocking(move || crate::crypto::digest("SHA-256", &bytes))
                        .await
                        .map_err(|error| format!("digest worker failed: {error}"))?
                        .map(HostValue::Bytes)
                }))
            },
        )
        .with_result_channel(),
    )
}
