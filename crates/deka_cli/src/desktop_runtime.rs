//! Drive Tokio's reactor while the platform owns the main thread's event loop.
//! VM/component work remains on that main thread and uses bounded turns.
use crate::{Result, cli_runtime};
use deka_native_ui::{Application, Node, Reload};
use std::{sync::Arc, thread::JoinHandle};
struct Reactor {
    runtime: Arc<tokio::runtime::Runtime>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}
impl Reactor {
    fn new() -> Result<Self> {
        let runtime = Arc::new(cli_runtime()?);
        let (send, receive) = tokio::sync::oneshot::channel();
        let driver = runtime.clone();
        let thread = std::thread::Builder::new()
            .name("deka-io".into())
            .spawn(move || {
                driver.block_on(async {
                    let _ = receive.await;
                });
            })
            .map_err(|e| format!("start desktop I/O reactor: {e}"))?;
        Ok(Self {
            runtime,
            shutdown: Some(send),
            thread: Some(thread),
        })
    }
}
impl Drop for Reactor {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            eprintln!("deka: desktop I/O reactor stopped unexpectedly");
        }
    }
}
/// The renderer only sees Application's turn/ready/wake contract.
/// Drop the application (and its pending futures) before stopping the reactor.
pub(crate) struct Desktop<A> {
    app: A,
    reactor: Reactor,
}
impl<A: Application> Desktop<A> {
    pub fn new(create: impl FnOnce() -> Result<A>) -> Result<Self> {
        let reactor = Reactor::new()?;
        let app = {
            let _entered = reactor.runtime.enter();
            create()?
        };
        Ok(Self { app, reactor })
    }
}
impl<A: Application> Application for Desktop<A> {
    fn initial_state(&self) -> Vec<f64> {
        self.app.initial_state()
    }
    fn render(&self, state: &[f64]) -> Node {
        self.app.render(state)
    }
    fn event(&self, handler: usize, state: &mut [f64]) {
        let _entered = self.reactor.runtime.enter();
        self.app.event(handler, state);
    }
    fn set_waker(&mut self, waker: deka_native_ui::Waker) {
        self.app.set_waker(waker);
    }
    fn has_ready_work(&self) -> bool {
        self.app.has_ready_work()
    }
    fn run_turn(&mut self, budget: usize) -> bool {
        let _entered = self.reactor.runtime.enter();
        self.app.run_turn(budget)
    }
    fn poll_reload(&mut self) -> Reload {
        let _entered = self.reactor.runtime.enter();
        self.app.poll_reload()
    }
    fn live(&self) -> bool {
        self.app.live()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deka_vm::*;
    use std::{
        sync::mpsc,
        task::{Wake, Waker},
        time::Duration,
    };
    struct Signal(mpsc::SyncSender<()>);
    impl Wake for Signal {
        fn wake(self: Arc<Self>) {
            let _ = self.0.try_send(());
        }
    }
    fn text(node: &Node) -> String {
        let mut value = node.text.clone().unwrap_or_default();
        for child in &node.children {
            value.push_str(&text(child));
        }
        value
    }
    #[test]
    fn production_desktop_reactor_wakes_a_button_for_timers_and_network_io() {
        for network in [false, true] {
            let app=Desktop::new(|| {
                let mut hosts=crate::hosts()?;
                hosts.register(HostOp::new("probe", vec![], HostType::String, true, move |_| HostReply::Pending(Box::pin(async move {
                    if network {
                        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.map_err(|e|e.to_string())?;
                        let address=listener.local_addr().map_err(|e|e.to_string())?;
                        let (server,client)=tokio::join!(listener.accept(),tokio::net::TcpStream::connect(address));
                        server.map_err(|e|e.to_string())?;client.map_err(|e|e.to_string())?;
                    } else { tokio::time::sleep(Duration::ZERO).await; }
                    Ok(HostValue::String("finished".into()))
                }))))?;
                let program=compiler::compile_entry(r#"import {probe} from "vm:host";
export fn App() { let message="idle"; return <view><p>{message}</p><button onClick={async fn() { message="waiting";message=await probe(); }}>Go</button></view>; }"#, &hosts, "App")?;
                ui::VmApp::with_hosts(program,hosts)
            }).unwrap();
            let mut host = deka_native_ui::Host::new(app);
            let (send, receive) = mpsc::sync_channel(1);
            let waker = Waker::from(Arc::new(Signal(send)));
            host.set_waker(deka_native_ui::Waker::new(move || waker.wake_by_ref()));
            host.click(0);
            for _ in 0..100 {
                if text(&host.render()) == "finishedGo" {
                    break;
                }
                // Wait for the exact wake, never sleep hoping the reactor ran.
                receive
                    .recv_timeout(Duration::from_secs(2))
                    .expect("desktop reactor did not wake the window");
                host.run_turn(256);
            }
            assert_eq!(text(&host.render()), "finishedGo");
            drop(host);
        }
    }

    #[test]
    fn desktop_button_reads_a_file_snapshot_and_returns_to_idle() {
        let app=Desktop::new(||{
            let hosts=crate::hosts()?;
            let program=compiler::compile_entry(r#"export fn App(){let message="Ready";
                return (<view><p>{message}</p><button onClick={async fn(){
                    message="Reading";
                    const file=unwrap(File([TextEncoder().encode("notes")],"notes.txt")) or{message="Failed";return;};
                    const part=unwrap(file.slice(1,4)) or{message="Failed";return;};
                    const text=unwrap(await part.text()) or{message="Failed";return;};message=file.name+":"+text;
                }}>Read</button></view>);
            }"#,&hosts,"App")?;
            ui::VmApp::with_hosts(program,hosts)
        }).unwrap();
        let mut host = deka_native_ui::Host::new(app);
        let (send, receive) = mpsc::sync_channel(1);
        let waker = Waker::from(Arc::new(Signal(send)));
        host.set_waker(deka_native_ui::Waker::new(move || waker.wake_by_ref()));
        assert_eq!(text(&host.render()), "ReadyRead");
        host.click(0);
        for _ in 0..200 {
            if text(&host.render()) == "notes.txt:oteRead" {
                break;
            }
            if host.has_ready_work() {
                host.run_turn(32);
            } else {
                receive
                    .recv_timeout(Duration::from_secs(5))
                    .expect("snapshot read did not wake window");
            }
        }
        assert_eq!(text(&host.render()), "notes.txt:oteRead");
        assert!(!host.has_ready_work());
        for _ in 0..32 {
            assert!(!host.run_turn(32));
        }
    }

    mod http_fixture {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../deka_vm/tests/support/http_server.rs"
        ));
    }
    #[test]
    fn production_desktop_button_fetches_typed_json_and_updates_without_blocking_the_window() {
        let server = http_fixture::Server::new(|_, _| {
            Ok(http_fixture::reply(
                "200 OK",
                "Content-Type: application/json\r\n",
                br#""After""#,
            ))
        })
        .unwrap();
        let source = format!(
            r#"export fn App() {{
    let message="Ready";
    return (<view><p>{{message}}</p><button onClick={{async fn() {{
        message="Loading";
        const response=unwrap(await fetch("{}")) or{{message="Fetch failed";return;}};
        message=match await response.json<string>(){{Ok(text)=>text,Err(error)=>error}};
    }}}}>Load</button></view>);
}}"#,
            server.url("/message")
        );
        let app = Desktop::new(|| {
            let hosts = crate::hosts()?;
            let program = compiler::compile_entry(&source, &hosts, "App")?;
            ui::VmApp::with_hosts(program, hosts)
        })
        .unwrap();
        let mut host = deka_native_ui::Host::new(app);
        let (send, receive) = mpsc::sync_channel(1);
        let waker = Waker::from(Arc::new(Signal(send)));
        host.set_waker(deka_native_ui::Waker::new(move || waker.wake_by_ref()));
        assert_eq!(text(&host.render()), "ReadyLoad");
        host.click(0);
        for _ in 0..200 {
            if text(&host.render()) == "AfterLoad" {
                break;
            }
            if host.has_ready_work() {
                host.run_turn(32);
            } else {
                receive
                    .recv_timeout(Duration::from_secs(5))
                    .expect("HTTP completion did not wake the window");
            }
        }
        assert_eq!(text(&host.render()), "AfterLoad");
        assert!(!host.has_ready_work());
        for _ in 0..32 {
            assert!(!host.run_turn(32));
        }
        drop(host);
        assert_eq!(server.finish().unwrap(), ["/message"]);
    }
    #[cfg(feature = "desktop")]
    #[test]
    fn a_reloaded_vm_keeps_the_windows_wake_and_finishes_new_async_work() {
        let directory = tempfile::tempdir().unwrap();
        let entry = directory.path().join("App.dsx");
        std::fs::write(&entry, "export fn App() { return <p>before</p>; }").unwrap();
        let source = crate::source(&entry, None).unwrap();
        let payload = crate::compile(&source).unwrap();
        let watched = compiler::source_files(&source.path)
            .unwrap()
            .into_iter()
            .map(|path| {
                let bytes = std::fs::read(&path).ok();
                (path, bytes)
            })
            .collect();
        let mut app = Desktop::new(|| {
            Ok(crate::DevApp {
                source,
                app: ui::VmApp::with_hosts(payload.program, crate::hosts()?)?,
                last: std::time::Instant::now(),
                watched,
            })
        })
        .unwrap();
        let (send, receive) = mpsc::sync_channel(1);
        let waker = Waker::from(Arc::new(Signal(send)));
        app.set_waker(deka_native_ui::Waker::new(move || waker.wake_by_ref()));
        std::fs::write(&entry, r#"import {sleep} from "time";
export fn App() { let message="loading"; const load=async fn() { await sleep(0); message="reloaded"; }; load(); return <p>{message}</p>; }"#).unwrap();
        app.app.last -= Duration::from_secs(1);
        assert!(matches!(app.poll_reload(), Reload::Reset));
        for _ in 0..100 {
            if text(&app.render(&[])) == "reloaded" {
                break;
            }
            receive
                .recv_timeout(Duration::from_secs(2))
                .expect("reloaded VM lost the window wake");
            app.run_turn(32);
        }
        assert_eq!(text(&app.render(&[])), "reloaded");
    }
}
