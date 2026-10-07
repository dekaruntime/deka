#![cfg(all(feature = "compiler", feature = "host"))]
use deka_vm::*;
use std::{collections::BTreeMap, future::Future, time::Duration};
fn hosts() -> Hosts {
    let mut h = Hosts::default();
    http_headers::register(&mut h).unwrap();
    http_request::register(&mut h).unwrap();
    http_response::register(&mut h).unwrap();
    fetch::register(&mut h).unwrap();
    http_server::register(&mut h).unwrap();
    url::register(&mut h).unwrap();
    bytes::register(&mut h).unwrap();
    h
}
async fn bounded<T>(f: impl Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(10), f)
        .await
        .unwrap()
}
#[tokio::test]
async fn source_native_fetch_and_shutdown_use_the_same_web_resources() {
    let h = hosts();
    let p=compiler::compile(r#"
import {serve} from "http";
async fn main() Promise<string>{
 const server=unwrap(serve({hostname:"127.0.0.1",port:0},async fn(req:Request) Promise<Result<Response,string>>{
     const body=unwrap(await req.text()) or{return Err("body");};
     const response=unwrap(Response(req.method+":"+body+":"+req.url)) or{return Err("response");};
     response.headers.set("x-deka","native");return Ok(response);
 }))or{return "bind";};
 const response=unwrap(await fetch("http://127.0.0.1:"+string(server.addr.port)+"/hello?q=1"))or{return "fetch";};
 const body=unwrap(await response.text())or{return "text";};
 const header=unwrap(response.headers.get("x-deka"))or{return "header";};
 const value=unwrap(header)or{return "absent";};
 await server.shutdown();await server.finished();return value+":"+body;
}"#,&h).unwrap();
    let p = serde_json::from_slice(&serde_json::to_vec(&p).unwrap()).unwrap();
    let mut vm = Vm::new(p, h).unwrap();
    let result = bounded(vm.run()).await.unwrap();
    let HostValue::String(s) = result else {
        panic!("wrong result")
    };
    assert!(s.starts_with("native:GET::http://127.0.0.1:"), "{s}");
    assert!(s.ends_with("/hello?q=1"));
    assert_eq!(vm.stats().live, 0);
}
#[tokio::test]
async fn default_fetch_object_context_is_typed_and_served_through_bytecode() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("server.ds");
    std::fs::write(&path,r#"export default { async fetch(req) { const body=unwrap(await req.text())or{return Err("body");};return Response(req.method+":"+body); } };"#).unwrap();
    let (send, mut recv) = tokio::sync::mpsc::unbounded_channel();
    let mut h = hosts();
    h.register(HostOp::new(
        "__cli_http_options",
        vec![],
        http_server::options_type(),
        false,
        |_| {
            HostReply::Ready(Ok(HostValue::Record(BTreeMap::from([
                ("hostname".into(), HostValue::String("127.0.0.1".into())),
                ("port".into(), HostValue::Number(0.)),
            ]))))
        },
    ))
    .unwrap();
    h.register(HostOp::new(
        "__cli_http_listening",
        vec![HostType::Record(BTreeMap::from([
            ("hostname".into(), HostType::String),
            ("port".into(), HostType::Number),
            ("transport".into(), HostType::String),
        ]))],
        HostType::Unit,
        false,
        move |args| {
            let HostValue::Record(f) = &args[0] else {
                unreachable!()
            };
            let HostValue::Number(port) = f["port"] else {
                unreachable!()
            };
            send.send(port as u16).unwrap();
            HostReply::Ready(Ok(HostValue::Unit))
        },
    ))
    .unwrap();
    let (p, server) = compiler::compile_script_file(&path, &h, None).unwrap();
    assert!(server);
    let p = serde_json::from_slice(&serde_json::to_vec(&p).unwrap()).unwrap();
    let mut vm = Vm::new(p, h).unwrap();
    let port=bounded(async{tokio::select!{result=vm.run()=>panic!("unexpected {result:?}"),port=recv.recv()=>port.unwrap()}}).await;
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let requests = async {
        let response = client
            .post(format!("http://127.0.0.1:{port}/upload"))
            .body("binary text")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(response.text().await.unwrap(), "POST:binary text");
    };
    bounded(async {
        tokio::select! {result=vm.run()=>panic!("unexpected {result:?}"),()=requests=>{}}
    })
    .await;
    vm.cancel().unwrap();
    assert_eq!(vm.stats().live, 0);
    std::net::TcpListener::bind(("127.0.0.1", port)).unwrap();
}

#[tokio::test]
async fn optional_initializer_fields_keep_presence_and_value_types() {
    let h = hosts();
    let source = r#"
fn label<T>(input: {value?: T}) Option<T> { return input.value; }
fn main() string {
    let opts: {port?: number; hostname?: string} = {};
    assert(match opts.port {None=>true,Some(_)=>false});
    opts.port=41;
    assert(match opts.port {Some(n)=>n==41,None=>false});
    return match label({value:"typed"}) {Some(s)=>s,None=>"absent"};
}"#;
    let mut h = h;
    h.register(
        HostOp::new(
            "assert",
            vec![HostType::Bool],
            HostType::Unit,
            false,
            |args| {
                if args[0] == HostValue::Bool(true) {
                    HostReply::Ready(Ok(HostValue::Unit))
                } else {
                    HostReply::Ready(Err("assertion failed".into()))
                }
            },
        )
        .with_global_binding(),
    )
    .unwrap();
    let p = compiler::compile(source, &h).unwrap();
    let mut vm = Vm::new(p, h).unwrap();
    assert_eq!(
        bounded(vm.run()).await.unwrap(),
        HostValue::String("typed".into())
    );
    assert_eq!(vm.stats().live, 0);
    for bad in [
        "fn main(){const opts:{port?:number}={port:\"wrong\"};}",
        "fn main(){const opts:{port?:number}={port:Some(1)};}",
        "fn main(){const opts:{port?:number}={};const n:number=opts.port;}",
        "fn use(x:{port:number}){} fn main(){const opts:{port?:number}={};use(opts);}",
        "fn main(){let opts:{port?:number}={};opts.port=\"wrong\";}",
    ] {
        assert!(compiler::compile(bad, &hosts()).is_err(), "accepted {bad}");
    }
}

fn ready_host(h: &mut Hosts) -> tokio::sync::mpsc::UnboundedReceiver<u16> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    h.register(HostOp::new(
        "ready",
        vec![HostType::Number],
        HostType::Unit,
        false,
        move |args| {
            let HostValue::Number(n) = args[0] else {
                unreachable!()
            };
            tx.send(n as u16).unwrap();
            HostReply::Ready(Ok(HostValue::Unit))
        },
    ))
    .unwrap();
    rx
}
async fn started(source: &str, mut h: Hosts) -> (Vm, u16) {
    let mut rx = ready_host(&mut h);
    let source = format!("import{{ready}}from\"vm:host\";\n{source}");
    let p = compiler::compile(&source, &h).unwrap();
    let p = serde_json::from_slice(&serde_json::to_vec(&p).unwrap()).unwrap();
    let mut vm = Vm::new(p, h).unwrap();
    let port=bounded(async {tokio::select!{result=vm.run()=>panic!("early server exit {result:?}"),port=rx.recv()=>port.unwrap()}}).await;
    (vm, port)
}
async fn drive<T>(vm: &mut Vm, future: impl Future<Output = T>) -> T {
    bounded(async {
        tokio::select! {result=vm.run()=>panic!("early server exit {result:?}"),value=future=>value}
    })
    .await
}
fn stopped(mut vm: Vm, port: u16) {
    vm.cancel().unwrap();
    assert_eq!(vm.stats().live, 0);
    std::net::TcpListener::bind(("127.0.0.1", port)).unwrap();
}
fn client() -> reqwest::Client {
    reqwest::Client::builder().no_proxy().build().unwrap()
}
#[tokio::test]
async fn binary_and_typed_json_bodies_headers_head_and_handler_errors() {
    let source = r#"
import {serve} from "http";
import {to_hex} from "bytes";
struct Input { message: string }
async fn main(){
 const s=unwrap(serve({hostname:"127.0.0.1",port:0},async fn(req:Request) Promise<Result<Response,string>>{
   const address=unwrap(URL(req.url))or{return Err("url");};if(address.pathname=="/error"){return Err("private failure");}
   if(address.pathname=="/json"){
       const body=unwrap(await req.json<Input>()) or {return Err("shape");};
       const copy=req;
       const second=await copy.text();
       const used=match second{Err(_)=>req.bodyUsed,Ok(_)=>false};
       return Response(body.message+":"+string(used));
   }
   if(address.pathname=="/consumed") {const response=unwrap(Response("used"))or{return Err("construct");};await response.text();return Ok(response);}
   const body=unwrap(await req.bytes())or{return Err("bytes");};
   const response=unwrap(Response(req.method+":"+to_hex(body)))or{return Err("construct");};
   response.headers.append("set-cookie","a=1");response.headers.append("set-cookie","b=2");
   response.headers.set("content-length","9999");return Ok(response);
 }))or{return;};ready(s.addr.port);
}"#;
    let (mut vm, port) = started(source, hosts()).await;
    drive(&mut vm, async {
        let c = client();
        let root = format!("http://127.0.0.1:{port}");
        let r = c
            .post(format!("{root}/bytes"))
            .body(vec![0, 255, 128, 1])
            .send()
            .await
            .unwrap();
        assert_eq!(r.headers().get_all("set-cookie").iter().count(), 2);
        assert_eq!(r.content_length(), Some(13));
        assert_eq!(r.text().await.unwrap(), "POST:00ff8001");
        let r = c
            .post(format!("{root}/json"))
            .body(r#"{"Input":{"message":"typed"}}"#)
            .send()
            .await
            .unwrap();
        assert_eq!(r.text().await.unwrap(), "typed:true");
        for path in ["error", "consumed"] {
            let r = c.get(format!("{root}/{path}")).send().await.unwrap();
            assert_eq!(r.status(), 500);
            assert!(!r.text().await.unwrap().contains("private"));
        }
        let r = c.head(format!("{root}/head")).send().await.unwrap();
        assert_eq!(r.status(), 200);
        assert_eq!(r.headers()["content-length"], "5");
        assert!(r.bytes().await.unwrap().is_empty());
        assert_eq!(
            c.get(format!("{root}/good"))
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap(),
            "GET:"
        );
    })
    .await;
    stopped(vm, port);
}
#[tokio::test]
async fn concurrent_request_progress_and_graceful_shutdown_drain() {
    let (entered_tx, mut entered_rx) = {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        (tx, rx)
    };
    let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
    let release = std::rc::Rc::new(std::cell::RefCell::new(Some(release_rx)));
    let mut h = hosts();
    h.register(
        HostOp::new("block", vec![], HostType::Unit, true, move |_| {
            entered_tx.send(()).unwrap();
            let rx = release.borrow_mut().take().unwrap();
            HostReply::Pending(Box::pin(async move {
                rx.await.map_err(|e| e.to_string())?;
                Ok(HostValue::Unit)
            }))
        })
        .with_global_binding(),
    )
    .unwrap();
    let source = r#"import {serve} from "http";async fn main(){
 const s=unwrap(serve({hostname:"127.0.0.1",port:0},async fn(req:Request) Promise<Result<Response,string>>{
   const address=unwrap(URL(req.url))or{return Err("url");};if(address.pathname=="/slow"){await block();}return Response(req.method);
 }))or{return;};ready(s.addr.port);
}"#;
    let (mut vm, port) = started(source, h).await;
    drive(&mut vm,async{
        let c=client();let slow=c.get(format!("http://127.0.0.1:{port}/slow")).send();tokio::pin!(slow);
        tokio::select!{result=&mut slow=>panic!("slow completed {result:?}"),entered=entered_rx.recv()=>entered.unwrap()};
        assert_eq!(c.get(format!("http://127.0.0.1:{port}/fast")).send().await.unwrap().text().await.unwrap(),"GET");
        release_tx.send(()).unwrap();assert_eq!(slow.await.unwrap().text().await.unwrap(),"GET");
    }).await;
    stopped(vm, port);
}
#[tokio::test]
async fn body_cap_covers_content_length_and_chunked_input() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let(mut vm,port)=started(r#"import{serve}from"http";fn main(){const s=unwrap(serve({hostname:"127.0.0.1",port:0},fn(req:Request)Result<Response,string>{return Response("ok");}))or{return;};ready(s.addr.port);}"#,hosts()).await;
    drive(&mut vm,async{
        let root=format!("http://127.0.0.1:{port}");
        assert_eq!(client().post(&root).body(vec![b'x';16*1024*1024+1]).send().await.unwrap().status(),413);
        let mut stream=tokio::net::TcpStream::connect(("127.0.0.1",port)).await.unwrap();
        stream.write_all(b"POST / HTTP/1.1\r\nHost: localhost\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n1000001\r\n").await.unwrap();
        let bytes=vec![b'x';16*1024*1024+1];
        // The server may close immediately once the cap is exceeded.
        let _=stream.write_all(&bytes).await;let _=stream.write_all(b"\r\n0\r\n\r\n").await;
        let mut response=vec![];stream.read_to_end(&mut response).await.unwrap();assert!(response.starts_with(b"HTTP/1.1 413"),"{}",String::from_utf8_lossy(&response));
        assert_eq!(client().get(&root).send().await.unwrap().status(),200);
    }).await;
    stopped(vm, port);
}
mod support {
    pub mod tls;
}
#[tokio::test]
async fn shared_rustls_serves_generated_identity_and_cancellation_drops_idle_handshake() {
    let i = support::tls::identity();
    let source=support::tls::program(r#"import{from_hex}from"bytes";import{serve}from"http";async fn roundTrip(cert:bytes,key:bytes,ca:bytes)Promise<string>{
 const s=unwrap(serve({hostname:"127.0.0.1",port:0,cert:cert,key:key},fn(req:Request)Result<Response,string>{return Response(req.url);}))or{return "bind";};ready(s.addr.port);await s.finished();return "done";}
"#,&i).replace("async fn main(){","async fn main()Promise<string>{").replace("or{return;}","or{return \"decode\";}").replace("console.log(await roundTrip(cert,key,ca));","return await roundTrip(cert,key,ca);");
    let (mut vm, port) = started(&source, hosts()).await;
    let c = reqwest::Client::builder()
        .no_proxy()
        .add_root_certificate(reqwest::Certificate::from_pem(&i.ca).unwrap())
        .build()
        .unwrap();
    let idle = drive(&mut vm, async {
        let idle = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        let url = format!("https://127.0.0.1:{port}/tls");
        let r = c.get(&url).send().await.unwrap();
        assert_eq!(r.text().await.unwrap(), url);
        idle
    })
    .await;
    stopped(vm, port);
    drop(idle);
}
#[test]
fn server_contract_rejects_wrong_resources_untyped_results_and_unknown_methods() {
    for source in [
        "import{serve}from\"http\";fn main(){serve({port:\"wrong\"},fn(r:Request)Result<Response,string>{return Response(\"ok\");});}",
        "import{serve}from\"http\";fn main(){serve({},fn(r:string)Result<Response,string>{return Response(r);});}",
        "import{serve}from\"http\";fn main(){serve({},fn(r:Request)string{return \"wrong\";});}",
        "import{serve}from\"http\";fn main(){const s=unwrap(serve({},fn(r:Request)Result<Response,string>{return Response(\"ok\");}))or{return;};s.unknown();}",
    ] {
        assert!(
            compiler::compile(source, &hosts()).is_err(),
            "accepted {source}"
        );
    }
}
#[tokio::test]
async fn shutdown_closes_listener_while_an_accepted_handler_finishes() {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let (entered, mut entries) = tokio::sync::mpsc::unbounded_channel();
    let (release, wait) = tokio::sync::oneshot::channel::<()>();
    let wait = std::rc::Rc::new(std::cell::RefCell::new(Some(wait)));
    let mut h = hosts();
    h.register(
        HostOp::new(
            "resource",
            vec![HostType::Handle("HttpServer".into())],
            HostType::Unit,
            false,
            move |args| {
                let HostValue::Handle(v) = &args[0] else {
                    unreachable!()
                };
                tx.send(v.downcast_ref::<http_server::Server>().unwrap().clone())
                    .unwrap();
                HostReply::Ready(Ok(HostValue::Unit))
            },
        )
        .with_global_binding(),
    )
    .unwrap();
    h.register(
        HostOp::new("block", vec![], HostType::Unit, true, move |_| {
            entered.send(()).unwrap();
            let wait = wait.borrow_mut().take().unwrap();
            HostReply::Pending(Box::pin(async move {
                wait.await.map_err(|e| e.to_string())?;
                Ok(HostValue::Unit)
            }))
        })
        .with_global_binding(),
    )
    .unwrap();
    let(mut vm,port)=started(r#"import{serve}from"http";fn main(){const s=unwrap(serve({hostname:"127.0.0.1",port:0},async fn(req:Request)Promise<Result<Response,string>>{await block();return Response("finished");}))or{return;};resource(s);ready(s.addr.port);}"#,h).await;
    let server = rx.recv().await.unwrap();
    let tasks = async {
        let c = client();
        let request = c.get(format!("http://127.0.0.1:{port}/slow")).send();
        tokio::pin!(request);
        tokio::select! {result=&mut request=>panic!("early request {result:?}"),signal=entries.recv()=>signal.unwrap()};
        server.stop();
        assert!(
            tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .is_err()
        );
        release.send(()).unwrap();
        assert_eq!(request.await.unwrap().text().await.unwrap(), "finished");
    };
    let (result, ()) = bounded(async { tokio::join!(vm.run(), tasks) }).await;
    result.unwrap();
    bounded(server.finished()).await.unwrap();
    assert_eq!(vm.stats().live, 0);
    std::net::TcpListener::bind(("127.0.0.1", port)).unwrap();
}
#[tokio::test]
async fn exact_buffered_response_bytes_and_output_limit() {
    let mut h = hosts();
    h.register(
        HostOp::new(
            "binary",
            vec![HostType::Bool],
            HostType::Handle("Response".into()),
            false,
            |args| {
                let bytes = if args[0] == HostValue::Bool(true) {
                    vec![0; 16 * 1024 * 1024 + 1]
                } else {
                    vec![0, 255, 128]
                };
                HostReply::Ready(
                    http_response::ResponseObject::from_parts(
                        200,
                        String::new(),
                        String::new(),
                        http_headers::HeaderList::default(),
                        Some(bytes),
                    )
                    .map(http_response::ResponseObject::into_host_value),
                )
            },
        )
        .with_global_binding()
        .with_result_channel(),
    )
    .unwrap();
    let(mut vm,port)=started(r#"import{serve}from"http";fn main(){const s=unwrap(serve({hostname:"127.0.0.1",port:0},fn(req:Request)Result<Response,string>{const address=unwrap(URL(req.url))or{return Err("url");};return binary(address.pathname=="/big");}))or{return;};ready(s.addr.port);}"#,h).await;
    drive(&mut vm, async {
        let c = client();
        let root = format!("http://127.0.0.1:{port}");
        assert_eq!(
            &c.get(&root).send().await.unwrap().bytes().await.unwrap()[..],
            &[0, 255, 128]
        );
        assert_eq!(
            c.get(format!("{root}/big")).send().await.unwrap().status(),
            500
        );
    })
    .await;
    stopped(vm, port);
}
#[tokio::test]
async fn invalid_listener_configuration_is_a_result_without_a_started_job() {
    let h = hosts();
    let source = r#"import{serve}from"http";fn main()boolean{
 const invalid=serve({port:0.5},fn(req:Request)Result<Response,string>{return Response("ok");});
 const cert=unwrap(from_hex("00"))or{return false;};
 const half=serve({port:0,cert:cert},fn(req:Request)Result<Response,string>{return Response("ok");});
 return (match invalid{Err(_)=>true,Ok(_)=>false})&&(match half{Err(_)=>true,Ok(_)=>false});
}"#;
    let source = format!("import{{from_hex}}from\"bytes\";{source}");
    let p = compiler::compile(&source, &h).unwrap();
    let mut vm = Vm::new(p, h).unwrap();
    assert_eq!(bounded(vm.run()).await.unwrap(), HostValue::Bool(true));
    assert_eq!(vm.stats().live, 0);
}
#[tokio::test]
async fn compiler_snapshot_freezes_source_and_resolved_package_entry() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("deka.json"),
        r#"{"dependencies":{"example":"1.0.0"}}"#,
    )
    .unwrap();
    let package = root.path().join("ds_modules/example");
    std::fs::create_dir_all(&package).unwrap();
    let manifest = package.join("deka.json");
    std::fs::write(&manifest, r#"{"version":"1.0.0","entry":"first.ds"}"#).unwrap();
    std::fs::write(package.join("first.ds"), "export const greeting=\"first\";").unwrap();
    std::fs::write(
        package.join("second.ds"),
        "export const greeting=\"second\";",
    )
    .unwrap();
    let source = root.path().join("main.ds");
    std::fs::write(
        &source,
        "import{greeting}from\"example\";fn main()string{return greeting;}",
    )
    .unwrap();
    let input = compiler::ScriptInputs::load(&source).unwrap();
    std::fs::write(&manifest, r#"{"version":"1.0.0","entry":"second.ds"}"#).unwrap();
    std::fs::write(
        package.join("first.ds"),
        "export const greeting=\"changed\";",
    )
    .unwrap();
    let h = hosts();
    let (p, server) = input.compile(&h, Some("main")).unwrap();
    assert!(!server);
    let mut vm = Vm::new(p, h).unwrap();
    assert_eq!(
        bounded(vm.run()).await.unwrap(),
        HostValue::String("first".into())
    );
    let next = compiler::ScriptInputs::load(&source).unwrap();
    assert_ne!(
        input
            .resolutions()
            .iter()
            .find(|e| e.specifier == "example")
            .unwrap()
            .target,
        next.resolutions()
            .iter()
            .find(|e| e.specifier == "example")
            .unwrap()
            .target
    );
    let h = hosts();
    let (p, _) = next.compile(&h, Some("main")).unwrap();
    let mut vm = Vm::new(p, h).unwrap();
    assert_eq!(
        bounded(vm.run()).await.unwrap(),
        HostValue::String("second".into())
    );
}
