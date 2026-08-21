use std::collections::HashMap;
use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::{RuntimeState, TcpOptions};
use engine::execute_request;
use engine::{RequestEnvelope, ResponseEnvelope};

pub async fn serve_tcp(state: Arc<RuntimeState>, options: TcpOptions) -> Result<(), String> {
    let listener = tokio::net::TcpListener::bind(&options.addr)
        .await
        .map_err(|err| format!("Failed to bind TCP listener {}: {}", options.addr, err))?;

    tracing::info!("🚀 Deka Runtime TCP listening on {}", options.addr);

    loop {
        let (stream, _) = listener
            .accept()
            .await
            .map_err(|err| format!("Failed to accept TCP connection: {}", err))?;
        let state = Arc::clone(&state);
        tokio::spawn(async move {
            let (read, mut write) = stream.into_split();
            let mut reader = BufReader::new(read);
            let mut line = String::new();
            loop {
                line.clear();
                let bytes = match reader.read_line(&mut line).await {
                    Ok(0) => break,
                    Ok(bytes) => bytes,
                    Err(err) => {
                        tracing::warn!("TCP read failed: {}", err);
                        break;
                    }
                };

                if bytes == 0 {
                    break;
                }

                let payload = line.trim();
                if payload.is_empty() {
                    continue;
                }

                let response = match serde_json::from_str::<RequestEnvelope>(payload) {
                    Ok(request) => match execute_request(Arc::clone(&state), request).await {
                        Ok(response) => response,
                        Err(err) => ResponseEnvelope {
                            status: 500,
                            headers: HashMap::new(),
                            body: err,
                            body_base64: None,
                            upgrade: None,
                        },
                    },
                    Err(err) => ResponseEnvelope {
                        status: 400,
                        headers: HashMap::new(),
                        body: format!("Invalid request envelope: {}", err),
                        body_base64: None,
                        upgrade: None,
                    },
                };

                let response_json = match serde_json::to_string(&response) {
                    Ok(json) => json,
                    Err(err) => {
                        tracing::warn!("TCP response serialize failed: {}", err);
                        continue;
                    }
                };

                if let Err(err) = write.write_all(response_json.as_bytes()).await {
                    tracing::warn!("TCP response write failed: {}", err);
                    break;
                }
                if let Err(err) = write.write_all(b"\n").await {
                    tracing::warn!("TCP response newline failed: {}", err);
                    break;
                }
            }
        });
    }
}
