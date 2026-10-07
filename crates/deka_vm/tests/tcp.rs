#![cfg(all(feature = "compiler", feature = "host"))]
use deka_vm::*;
use std::{future::Future, task::Poll, time::Duration};
fn hosts() -> Hosts {
    let mut h = Hosts::default();
    tcp::register(&mut h).unwrap();
    bytes::register(&mut h).unwrap();
    h
}
async fn run(source: &str) -> HostValue {
    let h = hosts();
    let p = compiler::compile(source, &h).unwrap();
    let p = serde_json::from_slice(&serde_json::to_vec(&p).unwrap()).unwrap();
    let mut vm = Vm::new(p, h).unwrap();
    let value = tokio::time::timeout(Duration::from_secs(10), vm.run())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(vm.stats().live, 0);
    value
}
#[tokio::test]
async fn language_loopback_transfers_binary_bytes_and_shared_alias_close() {
    assert_eq!(run(r#"
import {connect,listen} from "tcp";
import {from_hex,to_hex,concat,len,slice} from "bytes";
async fn receive(listener:TcpListener) Promise<string> {
    const c=unwrap(await listener.accept()) or {return "accept";};
    let data=unwrap(from_hex("")) or {return "hex";};
    for (;len(data)<5;){
        const chunk=unwrap(await c.read(2)) or {return "read";};
        const b=unwrap(chunk) or {return "eof";};
        data=concat(data,b);
    }
    const same=c;
    same.close();
    const closed=match await c.write(data){Ok(n)=>"bad",Err(e)=>"closed"};
    return to_hex(data)+":"+closed;
}
async fn main() Promise<string> {
    const l=unwrap(listen({hostname:Some("127.0.0.1"),port:0})) or {return "listen";};
    const received=receive(l);
    const c=unwrap(await connect({port:l.addr.port})) or {return "connect";};
    const bytes=unwrap(from_hex("00ff80c3a9")) or {return "hex";};
    let offset=0;
    for (;offset<len(bytes);){
        const n=unwrap(await c.write(slice(bytes,offset))) or {return "write";};
        offset+=n;
    }
    const result=await received;
    const end=unwrap(await c.read(1)) or {return "end";};
    const addresses=string(c.localAddr.port>0)+":"+string(c.remoteAddr.port==l.addr.port)+":"+c.localAddr.transport;
    c.close();l.close();
    return result+":"+string(end)+":"+addresses;
}"#).await,HostValue::String("00ff80c3a9:closed:None:true:true:tcp".into()));
}
#[test]
fn catalog_without_reactor_rejects_bad_arguments_and_fake_receivers() {
    let h = hosts();
    compiler::compile(
        "import {listen} from \"tcp\";fn main(){listen({port:0});}",
        &h,
    )
    .unwrap();
    for src in [
        "fn main(){tcp_connect(3);}",
        "import {connect} from \"tcp\";fn main(){connect({port:\"x\"});}",
        "import {listen} from \"tcp\";fn main(){listen({port:0,hostname:\"127.0.0.1\"});}",
        "import {listen} from \"tcp\";fn main(){const l=unwrap(listen({port:0})) or{return;};l.accept(1);}",
        "import {connect} from \"tcp\";async fn main(){const c=unwrap(await connect({port:1})) or{return;};c.write(\"bad\");}",
    ] {
        assert!(compiler::compile(src, &h).is_err(), "{src}");
    }
}
#[tokio::test]
async fn invalid_ports_listen_addresses_and_refused_connect_are_result_data() {
    let reservation = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = reservation.local_addr().unwrap().port();
    drop(reservation);
    assert_eq!(
        run(&format!(
            r#"
import {{connect,listen}} from "tcp";
async fn main() Promise<string>{{
 const a=match listen({{port:-1}}){{Ok(l)=>"bad",Err(e)=>"port"}};
 const b=match listen({{hostname:Some("not an IP"),port:0}}){{Ok(l)=>"bad",Err(e)=>"hostname"}};
 const c=match await connect({{port:0}}){{Ok(c)=>"bad",Err(e)=>"zero"}};
 const d=match await connect({{port:{port}}}){{Ok(c)=>"bad",Err(e)=>"refused"}};
 return a+":"+b+":"+c+":"+d;
}}"#
        ))
        .await,
        HostValue::String("port:hostname:zero:refused".into())
    );
}
#[tokio::test]
async fn pending_accept_close_and_drop_release_the_actual_bound_port() {
    let listener = tcp::Listener::bind("127.0.0.1", 0).unwrap();
    let addr = listener.addr();
    let mut waiting = Box::pin(listener.accept());
    std::future::poll_fn(|cx| {
        assert!(waiting.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    listener.close();
    assert!(waiting.await.err().unwrap().contains("closed"));
    let bound = std::net::TcpListener::bind(addr).unwrap();
    drop(bound);
    drop(listener);
    let listener = tcp::Listener::bind("127.0.0.1", addr.port()).unwrap();
    drop(listener);
    std::net::TcpListener::bind(addr).unwrap();
}
#[tokio::test]
async fn reads_do_not_block_writes_and_close_interrupts_all_pending_reads() {
    let listener = tcp::Listener::bind("127.0.0.1", 0).unwrap();
    let (a, b) = tokio::join!(
        tokio::net::TcpStream::connect(listener.addr()),
        listener.accept()
    );
    let client = tcp::Connection::new(a.unwrap()).unwrap();
    let server = b.unwrap();
    let mut read = Box::pin(client.read(8));
    std::future::poll_fn(|cx| {
        assert!(read.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    assert_eq!(client.write(&[0, 255]).await.unwrap(), 2);
    assert_eq!(server.read(8).await.unwrap(), Some(vec![0, 255]));
    client.close();
    assert!(read.await.unwrap_err().contains("closed"));
    assert!(client.write(&[]).await.is_err());
    assert!(client.read(1).await.is_err());
    assert_eq!(server.read(8).await.unwrap(), None);
    server.close();
    listener.close();
}
#[tokio::test]
async fn read_bounds_and_empty_writes_are_checked_without_touching_peer_data() {
    assert_eq!(
        run(r#"
import {connect,listen} from "tcp";
import {from_string} from "bytes";
async fn main() Promise<string>{
 const l=unwrap(listen({hostname:Some("127.0.0.1"),port:0})) or{return "l";};
 const pending=l.accept();
 const c=unwrap(await connect({port:l.addr.port})) or{return "c";};
 const peer=unwrap(await pending) or{return "p";};
 const a=match await c.read(0){Ok(b)=>"bad",Err(e)=>"zero"};
 const b=match await c.read(16777217){Ok(b)=>"bad",Err(e)=>"large"};
 const n=unwrap(await c.write(from_string(""))) or{return "w";};
 c.close();peer.close();l.close();return a+":"+b+":"+string(n);
}"#)
        .await,
        HostValue::String("zero:large:0".into())
    );
}
#[tokio::test]
async fn cancelling_vm_releases_a_listener_waiting_in_accept() {
    let (send, mut receive) = tokio::sync::mpsc::unbounded_channel();
    let mut h = hosts();
    h.register(HostOp::new(
        "bound_port",
        vec![HostType::Number],
        HostType::Unit,
        false,
        move |args| {
            let HostValue::Number(port) = args[0] else {
                unreachable!()
            };
            send.send(port as u16).unwrap();
            HostReply::Ready(Ok(HostValue::Unit))
        },
    ))
    .unwrap();
    let p = compiler::compile(
        r#"
import {listen} from "tcp";
import {bound_port} from "vm:host";
async fn main(){
 const l=unwrap(listen({hostname:Some("127.0.0.1"),port:0})) or{return;};
 bound_port(l.addr.port);await l.accept();
}"#,
        &h,
    )
    .unwrap();
    let mut vm = Vm::new(p, h).unwrap();
    let port = tokio::select! {
        result=vm.run()=>panic!("unexpected completion: {result:?}"),
        port=receive.recv()=>port.unwrap(),
    };
    vm.cancel().unwrap();
    assert_eq!(vm.stats().live, 0);
    std::net::TcpListener::bind(("127.0.0.1", port)).unwrap();
}
