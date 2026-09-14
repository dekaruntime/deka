//! Desktop host: a Tauri-stack webview (wry + tao) serving a compile-time
//! snapshot of the web app. First cut is macOS-only (deka#921).

use compile::vfs::{RuntimeMode, VFS};

pub fn run_desktop(vfs: VFS) -> Result<(), String> {
    if vfs.mode != RuntimeMode::Desktop {
        return Err("embedded VFS is not a desktop snapshot".into());
    }
    #[cfg(target_os = "macos")]
    {
        macos::run(vfs)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = vfs;
        Err("desktop webview launch is macOS-only in this first cut (rfd#64 / deka#921)".into())
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use compile::vfs::DESKTOP_META_PATH;
    use std::borrow::Cow;
    use std::collections::HashMap;
    use std::io::Read;
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    use tao::dpi::LogicalSize;
    use tao::event::{Event, WindowEvent};
    use tao::event_loop::{ControlFlow, EventLoopBuilder};
    use tao::window::WindowBuilder;
    use wry::WebViewBuilder;
    use wry::http::{Request, Response, header::CONTENT_TYPE};

    const DUMP_PREFIX: &str = "__DEKA_DESKTOP_DUMP__";
    const DUMP_TIMEOUT: Duration = Duration::from_secs(30);

    pub fn run(vfs: VFS) -> Result<(), String> {
        let dump = std::env::var_os("DEKA_DESKTOP_DUMP").is_some();
        let files = Arc::new(decompress_files(&vfs)?);
        let meta = window_meta(files.get(DESKTOP_META_PATH).map(|b| b.as_slice()));

        let event_loop = EventLoopBuilder::<String>::with_user_event().build();
        let proxy = event_loop.create_proxy();
        let window = WindowBuilder::new()
            .with_title(&meta.title)
            .with_inner_size(LogicalSize::new(meta.width, meta.height))
            .build(&event_loop)
            .map_err(|e| format!("failed to create desktop window: {e}"))?;

        let protocol_files = Arc::clone(&files);
        let mut builder = WebViewBuilder::new()
            .with_custom_protocol("deka".into(), move |_id, request| {
                serve_vfs(&protocol_files, &request)
            })
            .with_url("deka://localhost/index.html");

        if dump {
            let ipc_proxy = proxy.clone();
            builder = builder
                .with_ipc_handler(move |request: Request<String>| {
                    let _ = ipc_proxy.send_event(request.body().clone());
                })
                .with_initialization_script(
                    r#"
(function () {
  function send(text) {
    try { window.ipc.postMessage(String(text)); } catch (e) {}
  }
  function dump() {
    send(document.body ? document.body.innerText : "");
  }
  if (document.readyState === "complete") setTimeout(dump, 50);
  else window.addEventListener("load", function () { setTimeout(dump, 50); });
})();
"#,
                );
        }

        let _webview = builder
            .build(&window)
            .map_err(|e| format!("failed to create desktop webview: {e}"))?;

        stdio::log("desktop", "Window opened");
        let deadline = Instant::now() + DUMP_TIMEOUT;
        event_loop.run(move |event, _, control_flow| {
            if dump {
                *control_flow = ControlFlow::WaitUntil(deadline);
                if Instant::now() > deadline {
                    eprintln!("desktop dump timed out waiting for the webview");
                    std::process::exit(1);
                }
            } else {
                *control_flow = ControlFlow::Wait;
            }
            match event {
                Event::UserEvent(text) => {
                    println!("{DUMP_PREFIX}");
                    println!("{text}");
                    *control_flow = ControlFlow::Exit;
                }
                Event::WindowEvent {
                    event: WindowEvent::CloseRequested,
                    ..
                } => {
                    stdio::log("desktop", "Window closed");
                    *control_flow = ControlFlow::Exit;
                }
                _ => {}
            }
        });
    }

    struct WindowMeta {
        title: String,
        width: f64,
        height: f64,
    }

    fn window_meta(bytes: Option<&[u8]>) -> WindowMeta {
        let parsed = bytes.and_then(|b| serde_json::from_slice::<serde_json::Value>(b).ok());
        WindowMeta {
            title: parsed
                .as_ref()
                .and_then(|v| v.get("title"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .unwrap_or("Deka")
                .to_string(),
            width: parsed
                .as_ref()
                .and_then(|v| v.get("width"))
                .and_then(|v| v.as_u64())
                .unwrap_or(1200) as f64,
            height: parsed
                .as_ref()
                .and_then(|v| v.get("height"))
                .and_then(|v| v.as_u64())
                .unwrap_or(800) as f64,
        }
    }

    fn serve_vfs(
        files: &HashMap<String, Vec<u8>>,
        request: &Request<Vec<u8>>,
    ) -> Response<Cow<'static, [u8]>> {
        let path = request.uri().path();
        let relative = if path.is_empty() || path == "/" {
            "index.html"
        } else {
            path.trim_start_matches('/')
        };
        if relative == DESKTOP_META_PATH {
            return not_found();
        }
        match files.get(relative) {
            Some(bytes) => Response::builder()
                .header(CONTENT_TYPE, mime_type(relative))
                .body(Cow::Owned(bytes.clone()))
                .unwrap_or_else(|_| not_found()),
            None => not_found(),
        }
    }

    fn not_found() -> Response<Cow<'static, [u8]>> {
        Response::builder()
            .status(404)
            .header(CONTENT_TYPE, "text/plain")
            .body(Cow::Borrowed(&b"not found"[..]))
            .unwrap_or_else(|_| {
                Response::builder()
                    .status(404)
                    .body(Cow::Borrowed(&b"not found"[..]))
                    .expect("static 404")
            })
    }

    fn mime_type(path: &str) -> &'static str {
        if path.ends_with(".html") {
            "text/html; charset=utf-8"
        } else if path.ends_with(".css") {
            "text/css"
        } else if path.ends_with(".js") || path.ends_with(".mjs") {
            "text/javascript"
        } else if path.ends_with(".json") {
            "application/json"
        } else if path.ends_with(".svg") {
            "image/svg+xml"
        } else if path.ends_with(".png") {
            "image/png"
        } else if path.ends_with(".jpg") || path.ends_with(".jpeg") {
            "image/jpeg"
        } else if path.ends_with(".woff") {
            "font/woff"
        } else if path.ends_with(".woff2") {
            "font/woff2"
        } else {
            "application/octet-stream"
        }
    }

    fn decompress_files(vfs: &VFS) -> Result<HashMap<String, Vec<u8>>, String> {
        let mut files = HashMap::new();
        for (path, entry) in &vfs.files {
            let content = if entry.metadata.compressed {
                let mut decoder = flate2::read::GzDecoder::new(&entry.content[..]);
                let mut decompressed = Vec::new();
                decoder
                    .read_to_end(&mut decompressed)
                    .map_err(|e| format!("failed to decompress {path}: {e}"))?;
                decompressed
            } else {
                entry.content.clone()
            };
            files.insert(path.clone(), content);
        }
        Ok(files)
    }
}
