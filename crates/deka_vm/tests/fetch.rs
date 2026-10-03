#![cfg(all(feature = "compiler", feature = "host"))]
use deka_vm::*;
#[path = "support/http_server.rs"]
mod server;
use server::{Server, reply};
fn hosts() -> Hosts {
    let mut hosts = Hosts::default();
    http_headers::register(&mut hosts).unwrap();
    text_codec::register(&mut hosts).unwrap();
    http_response::register(&mut hosts).unwrap();
    fetch::register(&mut hosts).unwrap();
    hosts
}
async fn run(source: &str) -> HostValue {
    let hosts = hosts();
    let program = compiler::compile(source, &hosts).unwrap();
    let program = serde_json::from_slice(&serde_json::to_vec(&program).unwrap()).unwrap();
    let mut vm = Vm::new(program, hosts).unwrap();
    let result = tokio::time::timeout(std::time::Duration::from_secs(10), vm.run())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(vm.stats().live, 0);
    result
}
#[test]
fn catalog_and_checker_do_not_need_a_running_reactor() {
    let hosts = hosts();
    compiler::compile("async fn main() {fetch(\"http://example.test/\");}", &hosts).unwrap();
    for source in [
        "fn main(){fetch(7);}",
        "fn main(){fetch();}",
        "fn main(){fetch(\"http://example.test/\",7);}",
    ] {
        assert!(compiler::compile(source, &hosts).is_err(), "{source}");
    }
}
#[tokio::test]
async fn real_response_metadata_headers_and_typed_json_share_the_native_resources() {
    let server=Server::new(|path, _|Ok(match path {
        "/json"=>reply("200 OK","Content-Type: application/json\r\nX-Value: one\r\nX-Value: two\r\nSet-Cookie: a=1\r\nSet-Cookie: b=2\r\n",br#"{"Project":{"title":"Deka"}}"#),
        _=>reply("404 Not Found","",b"missing"),
    })).unwrap();
    let source = format!(
        r#"
struct Project {{ title: string; }}
fn (p Project) label() string {{return "Project: "+p.title;}}
async fn main() Promise<string> {{
    const response=unwrap(await fetch("{}#local")) or{{return "fetch";}};
    const header=unwrap(response.headers.get("x-value")) or{{return "header";}};
    const cookies=JSON.stringify(response.headers.getSetCookie());
    const project=unwrap(await response.json<Project>()) or{{return "json";}};
    const used=match await response.text(){{Ok(t)=>"bad",Err(e)=>"used"}};
    const missing=unwrap(await fetch("{}")) or{{return "missing fetch";}};
    const text=unwrap(await missing.text()) or{{return "missing body";}};
    return string(response.status)+";"+string(response.ok)+";"+string(header)+";"+cookies+";"+project.label()+";"+used+";"+string(missing.status)+";"+string(missing.ok)+";"+text+";"+response.url;
}}"#,
        server.url("/json"),
        server.url("/missing")
    );
    assert_eq!(
        run(&source).await,
        HostValue::String(format!(
            "200;true;Some(\"one, two\");[\"a=1\",\"b=2\"];Project: Deka;used;404;false;missing;{}",
            server.url("/json")
        ))
    );
    assert_eq!(server.finish().unwrap(), ["/json", "/missing"]);
}
#[tokio::test]
async fn redirects_have_final_url_and_empty_statuses_have_no_body() {
    let server = Server::new(|path, _| {
        Ok(match path {
            "/start" => reply("302 Found", "Location: /done\r\n", b""),
            "/done" => reply("204 No Content", "", b""),
            _ => reply("404 Not Found", "", b"missing"),
        })
    })
    .unwrap();
    let source = format!(
        r#"async fn main() Promise<string> {{
    const r=unwrap(await fetch("{}")) or{{return "fetch";}};
    const a=unwrap(await r.text()) or{{return "a";}};
    const b=unwrap(await r.text()) or{{return "b";}};
    return string(r.status)+";"+string(r.ok)+";"+string(r.bodyUsed)+";"+a+b+r.url;
}}"#,
        server.url("/start")
    );
    assert_eq!(
        run(&source).await,
        HostValue::String(format!("204;true;false;{}", server.url("/done")))
    );
    assert_eq!(server.finish().unwrap(), ["/start", "/done"]);
}
#[tokio::test]
async fn redirect_loops_url_failures_and_oversized_declared_bodies_are_results() {
    let server = Server::new(|path, _| {
        Ok(match path {
            "/loop" => reply("302 Found", "Location: /loop\r\n", b""),
            "/large" => {
                b"HTTP/1.1 200 OK\r\nContent-Length: 16777217\r\nConnection: close\r\n\r\n".to_vec()
            }
            _ => reply("404 Not Found", "", b"missing"),
        })
    })
    .unwrap();
    let source = format!(
        r#"async fn main() Promise<string> {{
    const a=match await fetch("{}"){{Ok(r)=>"bad",Err(e)=>"loop"}};
    const b=match await fetch("{}"){{Ok(r)=>"bad",Err(e)=>e}};
    const c=match await fetch("data:text/plain,x"){{Ok(r)=>"bad",Err(e)=>"scheme"}};
    const d=match await fetch("https://u:p@example.test/"){{Ok(r)=>"bad",Err(e)=>"credentials"}};
    return a+";"+b+";"+c+";"+d;
}}"#,
        server.url("/loop"),
        server.url("/large")
    );
    assert_eq!(
        run(&source).await,
        HostValue::String(
            "loop;response body exceeds 16 MiB buffered limit;scheme;credentials".into()
        )
    );
    let requests = server.finish().unwrap();
    assert_eq!(requests.iter().filter(|p| *p == "/loop").count(), 10);
    assert_eq!(requests.last().unwrap(), "/large");
}

#[tokio::test]
async fn two_fetches_reach_the_server_before_either_response_is_released() {
    use std::sync::{Arc, Condvar, Mutex};
    let (seen, mut accepted) = tokio::sync::mpsc::unbounded_channel();
    let released = Arc::new((Mutex::new(false), Condvar::new()));
    let waiting = released.clone();
    let server = Server::new(move |path, _| {
        seen.send(path.to_owned()).map_err(std::io::Error::other)?;
        let (lock, wake) = &*waiting;
        let state = lock
            .lock()
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        let (state, _) = wake
            .wait_timeout_while(state, std::time::Duration::from_secs(5), |ready| !*ready)
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        if !*state {
            return Err(std::io::Error::other("both requests were not released"));
        }
        Ok(reply("200 OK", "", path.as_bytes()))
    })
    .unwrap();
    let source = format!(
        r#"
async fn text(result:Result<Response,string>) Promise<string> {{
    return match result{{Ok(r)=>match await r.text(){{Ok(text)=>text,Err(e)=>e}},Err(e)=>e}};
}}
async fn main() Promise<string> {{
    const a=fetch("{}");
    const b=fetch("{}");
    const results=await Promise.all([a,b]);
    const first=results.has(0)?await text(results[0]):"missing";
    const second=results.has(1)?await text(results[1]):"missing";
    return first+";"+second;
}}"#,
        server.url("/a"),
        server.url("/b")
    );
    let control = async {
        let mut paths = vec![
            accepted.recv().await.unwrap(),
            accepted.recv().await.unwrap(),
        ];
        paths.sort();
        assert_eq!(paths, ["/a", "/b"]);
        let (lock, wake) = &*released;
        *lock.lock().unwrap() = true;
        wake.notify_all();
    };
    let (output, ()) = tokio::join!(run(&source), control);
    assert_eq!(output, HostValue::String("/a;/b".into()));
    let mut requests = server.finish().unwrap();
    requests.sort();
    assert_eq!(requests, ["/a", "/b"]);
}

#[tokio::test]
async fn broken_transport_and_invalid_json_are_results_without_entering_catch() {
    let server = Server::new(|path, _| {
        Ok(match path {
            "/broken" => {
                b"HTTP/1.1 200 OK\r\nContent-Length: 20\r\nConnection: close\r\n\r\nshort".to_vec()
            }
            "/json" => reply("200 OK", "Content-Type: application/json\r\n", b"not json"),
            _ => reply("404 Not Found", "", b"missing"),
        })
    })
    .unwrap();
    let source = format!(
        r#"async fn main() Promise<string> {{
    try {{
        const failed=match await fetch("{}"){{Ok(r)=>"bad",Err(e)=>"transport"}};
        const response=unwrap(await fetch("{}")) or{{return "fetch";}};
        const parsed=match await response.json<number>(){{Ok(n)=>"bad",Err(e)=>"parse"}};
        return failed+";"+parsed;
    }} catch(e){{return "wrong: catch";}}
}}"#,
        server.url("/broken"),
        server.url("/json")
    );
    assert_eq!(
        run(&source).await,
        HostValue::String("transport;parse".into())
    );
    assert_eq!(server.finish().unwrap(), ["/broken", "/json"]);
}

#[tokio::test]
async fn cancelling_the_vm_drops_the_real_pending_http_request_and_releases_its_heap() {
    use std::io::Read;
    let (seen, mut accepted) = tokio::sync::mpsc::unbounded_channel();
    let (closed, mut disconnected) = tokio::sync::mpsc::unbounded_channel();
    let server = Server::new(move |path, stream| {
        seen.send(path.to_owned()).map_err(std::io::Error::other)?;
        let mut byte = [0];
        let size = match stream.read(&mut byte) {
            Ok(size) => size,
            Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => 0,
            Err(error) => return Err(error),
        };
        if size != 0 {
            return Err(std::io::Error::other("unexpected request body"));
        }
        closed.send(()).map_err(std::io::Error::other)?;
        Ok(vec![])
    })
    .unwrap();
    let hosts = hosts();
    let source = format!(
        "async fn main() {{await fetch(\"{}\");}}",
        server.url("/pending")
    );
    let program = compiler::compile(&source, &hosts).unwrap();
    let mut vm = Vm::new(program, hosts).unwrap();
    tokio::select! {
        outcome=vm.run()=>panic!("request completed before cancellation: {outcome:?}"),
        request=accepted.recv()=>assert_eq!(request.unwrap(),"/pending"),
    }
    vm.cancel().unwrap();
    assert_eq!(vm.stats().live, 0);
    tokio::time::timeout(std::time::Duration::from_secs(10), disconnected.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(server.finish().unwrap(), ["/pending"]);
}

#[tokio::test]
async fn unknown_length_chunked_body_is_limited_while_receiving() {
    use std::io::Write;
    let server = Server::new(|_, stream| {
        stream.write_all(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
        )?;
        let chunk = vec![b'x'; 1024 * 1024];
        let result = (|| -> std::io::Result<()> {
            for _ in 0..17 {
                stream.write_all(b"100000\r\n")?;
                stream.write_all(&chunk)?;
                stream.write_all(b"\r\n")?;
            }
            stream.write_all(b"0\r\n\r\n")
        })();
        // This route intentionally exceeds the consumer's cap. The consumer
        // must close early, so its reset/broken pipe is the expected effect.
        if let Err(error) = result
            && !matches!(
                error.kind(),
                std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::ConnectionReset
            )
        {
            return Err(error);
        }
        Ok(vec![])
    })
    .unwrap();
    let source = format!(
        r#"async fn main() Promise<string> {{return match await fetch("{}"){{Ok(r)=>"accepted",Err(e)=>e}};}}"#,
        server.url("/chunked")
    );
    assert_eq!(
        run(&source).await,
        HostValue::String("response body exceeds 16 MiB buffered limit".into())
    );
    assert_eq!(server.finish().unwrap(), ["/chunked"]);
}

#[tokio::test]
async fn network_body_bytes_and_latin1_headers_survive_without_lossy_utf8_conversion() {
    let server=Server::new(|_,_|Ok(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nX-Latin: caf\xe9\r\nConnection: close\r\n\r\n\xff\0A".to_vec())).unwrap();
    let source = format!(
        r#"async fn main() Promise<string> {{
        const response=unwrap(await fetch("{}")) or{{return "fetch";}};
        const bytes=unwrap(await response.bytes()) or{{return "bytes";}};
        const decoder=unwrap(TextDecoder("windows-1252")) or{{return "decoder";}};
        const text=unwrap(decoder.decode(bytes)) or{{return "decode";}};
        const header=unwrap(response.headers.get("x-latin")) or{{return "header";}};
        return text+";"+match header{{Some(value)=>value,None=>"missing"}};
    }}"#,
        server.url("/bytes")
    );
    assert_eq!(run(&source).await, HostValue::String("ÿ\0A;café".into()));
    assert_eq!(server.finish().unwrap(), ["/bytes"]);
}
