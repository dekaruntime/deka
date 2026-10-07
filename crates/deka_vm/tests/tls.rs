#![cfg(all(feature = "compiler", feature = "host"))]
use deka_vm::*;
use std::{future::Future, task::Poll, time::Duration};
mod support {
    pub mod tls;
}
fn hosts() -> Hosts {
    let mut h = Hosts::default();
    tcp::register(&mut h).unwrap();
    tls::register(&mut h).unwrap();
    bytes::register(&mut h).unwrap();
    h
}
async fn bounded<T>(future: impl Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(10), future)
        .await
        .unwrap()
}
#[tokio::test]
async fn actual_language_guide_verifies_and_echoes_through_bytecode() {
    let i = support::tls::identity();
    let guide = include_str!("../../../docs/dekascript/native/tls.mdx")
        .split("```ds\n")
        .nth(1)
        .unwrap()
        .split("```")
        .next()
        .unwrap();
    let source = support::tls::program(guide, &i)
        .replace("async fn main(){", "async fn main() Promise<string>{")
        .replace("or{return;}", "or{return \"decode failed\";}")
        .replace(
            "console.log(await roundTrip(cert,key,ca));",
            "return await roundTrip(cert,key,ca);",
        );
    let h = hosts();
    let p = compiler::compile(&source, &h).unwrap();
    let p = serde_json::from_slice(&serde_json::to_vec(&p).unwrap()).unwrap();
    let mut vm = Vm::new(p, h).unwrap();
    assert_eq!(
        bounded(vm.run()).await.unwrap(),
        HostValue::String("Deka".into())
    );
    assert_eq!(vm.stats().live, 0);
}
async fn pair() -> (tls::Connection, tls::Connection, tls::Listener) {
    let i = support::tls::identity();
    let l =
        tls::Listener::bind("127.0.0.1", 0, tls::server_config(&i.cert, &i.key).unwrap()).unwrap();
    let (client, server) = bounded(async {
        tokio::join!(
            tls::connect(
                "127.0.0.1".into(),
                l.addr().port(),
                tls::client_config(&[i.ca]).unwrap()
            ),
            l.accept()
        )
    })
    .await;
    (client.unwrap(), server.unwrap(), l)
}
#[tokio::test]
async fn full_duplex_binary_flush_and_clean_eof() {
    let (c, s, l) = pair().await;
    let mut waiting = Box::pin(c.read(8));
    std::future::poll_fn(|cx| {
        assert!(waiting.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    assert_eq!(bounded(c.write(&[0, 255, 128])).await.unwrap(), 3);
    assert_eq!(bounded(s.read(8)).await.unwrap(), Some(vec![0, 255, 128]));
    assert_eq!(bounded(s.write(&[1, 2])).await.unwrap(), 2);
    assert_eq!(bounded(waiting).await.unwrap(), Some(vec![1, 2]));
    bounded(s.close_write()).await.unwrap();
    assert_eq!(bounded(c.read(8)).await.unwrap(), None);
    let alias = c.clone();
    let mut waiting = Box::pin(s.read(8));
    std::future::poll_fn(|cx| {
        assert!(waiting.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    s.close();
    assert!(waiting.await.unwrap_err().contains("closed"));
    alias.close();
    assert!(c.write(&[]).await.is_err());
    assert!(c.read(1).await.is_err());
    l.close();
}
#[tokio::test]
async fn upgrade_invalidates_raw_aliases_and_rejects_busy_reads_without_consuming() {
    let i = support::tls::identity();
    let l =
        tls::Listener::bind("127.0.0.1", 0, tls::server_config(&i.cert, &i.key).unwrap()).unwrap();
    let raw =
        tcp::Connection::new(tokio::net::TcpStream::connect(l.addr()).await.unwrap()).unwrap();
    let alias = raw.clone();
    let mut read = Box::pin(raw.read(1));
    std::future::poll_fn(|cx| {
        assert!(read.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    let err = tls::start_tls(
        raw.clone(),
        "127.0.0.1".into(),
        tls::client_config(std::slice::from_ref(&i.ca)).unwrap(),
    )
    .await
    .err()
    .unwrap();
    assert!(err.contains("pending reads"));
    drop(read);
    let (c, s) = bounded(async {
        tokio::join!(
            tls::start_tls(
                raw.clone(),
                "127.0.0.1".into(),
                tls::client_config(&[i.ca]).unwrap()
            ),
            l.accept()
        )
    })
    .await;
    let (c, s) = (c.unwrap(), s.unwrap());
    assert!(alias.write(&[1]).await.unwrap_err().contains("closed"));
    c.write(&[0, 255]).await.unwrap();
    assert_eq!(s.read(8).await.unwrap(), Some(vec![0, 255]));
    c.close();
    s.close();
    l.close();
}
#[tokio::test]
async fn untrusted_and_wrong_hostname_are_verification_errors() {
    for (hostname, trust) in [("127.0.0.1", false), ("wrong.invalid", true)] {
        let i = support::tls::identity();
        let l = tls::Listener::bind("127.0.0.1", 0, tls::server_config(&i.cert, &i.key).unwrap())
            .unwrap();
        let raw =
            tcp::Connection::new(tokio::net::TcpStream::connect(l.addr()).await.unwrap()).unwrap();
        let alias = raw.clone();
        let roots = if trust { vec![i.ca] } else { vec![] };
        let (c, s) = bounded(async {
            tokio::join!(
                tls::start_tls(raw, hostname.into(), tls::client_config(&roots).unwrap()),
                l.accept()
            )
        })
        .await;
        assert!(
            c.err()
                .unwrap()
                .starts_with("TLS certificate verification:")
        );
        assert!(s.is_err());
        assert!(alias.read(1).await.is_err());
        l.close();
    }
}
#[tokio::test]
async fn pending_handshake_close_and_cancel_release_resources() {
    let i = support::tls::identity();
    let l =
        tls::Listener::bind("127.0.0.1", 0, tls::server_config(&i.cert, &i.key).unwrap()).unwrap();
    let addr = l.addr();
    let mut accept = Box::pin(l.accept());
    std::future::poll_fn(|cx| {
        assert!(accept.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    let raw = tokio::net::TcpStream::connect(addr).await.unwrap();
    std::future::poll_fn(|cx| {
        assert!(accept.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    l.close();
    assert!(bounded(accept).await.err().unwrap().contains("closed"));
    drop(raw);
    drop(l);
    std::net::TcpListener::bind(addr).unwrap();
    let raw_l = tcp::Listener::bind("127.0.0.1", 0).unwrap();
    let (c, s) = tokio::join!(tokio::net::TcpStream::connect(raw_l.addr()), raw_l.accept());
    let c = tcp::Connection::new(c.unwrap()).unwrap();
    let alias = c.clone();
    let mut upgrade = Box::pin(tls::start_tls(
        c,
        "127.0.0.1".into(),
        tls::client_config(&[]).unwrap(),
    ));
    std::future::poll_fn(|cx| {
        assert!(upgrade.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    drop(upgrade);
    assert!(alias.write(&[1]).await.is_err());
    assert!(s.unwrap().read(16384).await.unwrap().is_some());
    raw_l.close();
}
#[test]
fn config_and_catalog_are_checked_without_reactor() {
    let i = support::tls::identity();
    assert!(tls::server_config(b"bad", &i.key).is_err());
    assert!(tls::server_config(&i.cert, b"bad").is_err());
    assert!(tls::client_config(&[b"bad".to_vec()]).is_err());
    let h = hosts();
    compiler::compile(
        "import {connectTls} from \"tls\";fn main(){connectTls({port:443});}",
        &h,
    )
    .unwrap();
    for s in [
        "import {connectTls} from \"tls\";fn main(){connectTls({port:443,caCerts:Some([\"bad\"])});}",
        "import {listenTls} from \"tls\";fn main(){listenTls({port:0,cert:\"bad\",key:\"bad\"});}",
    ] {
        assert!(compiler::compile(s, &h).is_err());
    }
}
#[tokio::test]
async fn language_start_tls_transfers_opaque_tcp_ownership() {
    let i = support::tls::identity();
    let l =
        tls::Listener::bind("127.0.0.1", 0, tls::server_config(&i.cert, &i.key).unwrap()).unwrap();
    let source = format!(
        r#"
import {{connect}} from "tcp";
import {{startTls}} from "tls";
import {{from_hex,to_hex}} from "bytes";
async fn main() Promise<string>{{
 const ca=unwrap(from_hex("{}")) or{{return "ca";}};
 const raw=unwrap(await connect({{port:{}}})) or{{return "tcp";}};
 const old=raw;
 const secure=unwrap(await startTls(raw,{{caCerts:Some([ca])}})) or{{return "tls";}};
 const closed=match await old.write(ca){{Ok(n)=>"bad",Err(e)=>"closed"}};
 const data=unwrap(from_hex("00ff")) or{{return "hex";}};
 const count=unwrap(await secure.write(data)) or{{return "write";}};
 const read=unwrap(await secure.read(8)) or{{return "read";}};
 const bytes=unwrap(read) or{{return "eof";}};
 secure.close();return closed+":"+to_hex(bytes);
}}"#,
        support::tls::hex(&i.ca),
        l.addr().port()
    );
    let h = hosts();
    let p = compiler::compile(&source, &h).unwrap();
    let p = serde_json::from_slice(&serde_json::to_vec(&p).unwrap()).unwrap();
    let mut vm = Vm::new(p, h).unwrap();
    let server = async {
        let c = l.accept().await.unwrap();
        let b = c.read(8).await.unwrap().unwrap();
        c.write(&b).await.unwrap();
        c.close_write().await.unwrap();
    };
    let (result, ()) = bounded(async { tokio::join!(vm.run(), server) }).await;
    assert_eq!(result.unwrap(), HostValue::String("closed:00ff".into()));
    assert_eq!(vm.stats().live, 0);
    l.close();
}
#[tokio::test]
async fn cancelling_vm_drops_tls_listener_and_pending_accept() {
    let i = support::tls::identity();
    let (send, mut recv) = tokio::sync::mpsc::unbounded_channel();
    let mut h = hosts();
    h.register(HostOp::new(
        "bound_port",
        vec![HostType::Number],
        HostType::Unit,
        false,
        move |args| {
            let HostValue::Number(p) = args[0] else {
                unreachable!()
            };
            send.send(p as u16).unwrap();
            HostReply::Ready(Ok(HostValue::Unit))
        },
    ))
    .unwrap();
    let source = format!(
        r#"
import {{listenTls}} from "tls";
import {{from_hex}} from "bytes";
import {{bound_port}} from "vm:host";
async fn main(){{
 const cert=unwrap(from_hex("{}")) or{{return;}};
 const key=unwrap(from_hex("{}")) or{{return;}};
 const l=unwrap(listenTls({{hostname:Some("127.0.0.1"),port:0,cert:cert,key:key}})) or{{return;}};
 bound_port(l.addr.port);await l.accept();
}}"#,
        support::tls::hex(&i.cert),
        support::tls::hex(&i.key)
    );
    let p = compiler::compile(&source, &h).unwrap();
    let mut vm = Vm::new(p, h).unwrap();
    let port = bounded(async {
        tokio::select! {result=vm.run()=>panic!("unexpected {result:?}"),p=recv.recv()=>p.unwrap()}
    })
    .await;
    let peer = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    let mut running = Box::pin(vm.run());
    std::future::poll_fn(|cx| {
        assert!(running.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    drop(running);
    vm.cancel().unwrap();
    assert_eq!(vm.stats().live, 0);
    drop(peer);
    std::net::TcpListener::bind(("127.0.0.1", port)).unwrap();
}
