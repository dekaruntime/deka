//! Tests for the net bridge (extracted from `net.rs`, deka#391).
use super::proto;
use super::net::{NetState, net_call_proto_impl_with, net_policy_target, tcp_connect_port};
use ::security::security_policy::SecurityPolicy;
use prost::Message;
use serde_json::json;
use std::io::{ErrorKind, Write};
use std::net::TcpListener;

/// Build a policy that allows exactly one net target. Tests pass this
/// in directly instead of installing a security context — per-test
/// contexts would race on the executing thread (deka#537).
fn test_net_policy(allow: &str) -> SecurityPolicy {
    let document = json!({ "security": { "allow": { "net": [allow] } } });
    ::security::security_policy::parse_deka_security_policy(&document).policy
}

#[test]
fn tcp_policy_target_preserves_manifest_port_scope() {
    assert_eq!(
        net_policy_target(&json!({ "host": "127.0.0.1", "port": 9418 })),
        Some("127.0.0.1:9418".to_string())
    );
}

#[test]
fn tcp_policy_target_does_not_invent_a_port() {
    assert_eq!(
        net_policy_target(&json!({ "host": "registry.tana.gg" })),
        Some("registry.tana.gg".to_string())
    );
}

#[test]
fn tcp_connect_port_rejects_overflow_values() {
    for port in [65_536, 65_537, u32::MAX as u64] {
        assert!(
            tcp_connect_port(&json!({ "port": port })).is_err(),
            "port {port}"
        );
    }
}

#[test]
fn tcp_proto_rejects_overflow_before_policy_or_socket_connect() {
    let mut state = NetState::new();
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind listener");
    listener
        .set_nonblocking(true)
        .expect("set listener nonblocking");
    let wrapped_port = listener.local_addr().expect("listener address").port() as u32;
    let overflow_port = wrapped_port + 65_536;

    let request = proto::bridge_v1::NetRequest {
        schema_version: 1,
        action: Some(proto::bridge_v1::net_request::Action::Connect(
            proto::bridge_v1::NetConnectRequest {
                host: "127.0.0.1".to_string(),
                port: overflow_port,
                timeout_ms: 100,
            },
        )),
    };
    let policy = test_net_policy(&format!("127.0.0.1:{overflow_port}"));
    let error = net_call_proto_impl_with(&mut state, &policy, &request.encode_to_vec())
        .expect_err("overflow rejected");
    assert!(error.to_string().contains("1..=65535"));
    assert!(matches!(listener.accept(), Err(error) if error.kind() == ErrorKind::WouldBlock));
}

#[test]
fn tcp_host_port_policy_allows_only_the_manifest_endpoint() {
    let policy = test_net_policy("127.0.0.1:9418");

    assert!(super::security::enforce_net_with(&policy, Some("127.0.0.1:9418")).is_ok());
    assert!(super::security::enforce_net_with(&policy, Some("127.0.0.1:9419")).is_err());
    assert!(super::security::enforce_net_with(&policy, Some("127.0.0.1")).is_err());
}

#[test]
fn tcp_handle_operations_retain_the_allowed_connect_target() {
    let mut state = NetState::new();
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind listener");
    let address = listener.local_addr().expect("listener address");
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept connection");
        let mut buffer = [0_u8; 4];
        std::io::Read::read_exact(&mut stream, &mut buffer).expect("read request");
        std::io::Write::write_all(&mut stream, &buffer).expect("write response");
    });
    let allowed = format!("127.0.0.1:{}", address.port());
    let allowed_policy = test_net_policy(&allowed);

    let connect = super::net::net_action_payload_to_proto_request(
        "connect",
        &json!({ "host": "127.0.0.1", "port": address.port() }),
    )
    .expect("encode connect");
    let connect_response =
        net_call_proto_impl_with(&mut state, &allowed_policy, &connect.encode_to_vec())
            .expect("connect");
    let connect_response = proto::bridge_v1::NetResponse::decode(connect_response.as_slice())
        .expect("decode connect response");
    let handle = match connect_response.action.expect("connect action") {
        proto::bridge_v1::net_response::Action::Connect(response) => response.handle,
        other => panic!("unexpected connect response: {other:?}"),
    };

    for (action, payload) in [
        ("write", json!({ "handle": handle, "data": "ping" })),
        ("read", json!({ "handle": handle, "max_bytes": 4 })),
    ] {
        let request = super::net::net_action_payload_to_proto_request(action, &payload)
            .expect("encode handle operation");
        assert!(
            net_call_proto_impl_with(&mut state, &allowed_policy, &request.encode_to_vec())
                .is_ok(),
            "{action} must use the allowed connect target"
        );
    }

    let denied_policy = test_net_policy("127.0.0.1:1");
    let denied_write = super::net::net_action_payload_to_proto_request(
        "write",
        &json!({ "handle": handle, "data": "nope" }),
    )
    .expect("encode denied write");
    assert!(
        net_call_proto_impl_with(&mut state, &denied_policy, &denied_write.encode_to_vec())
            .is_err()
    );

    let denied_connect = super::net::net_action_payload_to_proto_request(
        "connect",
        &json!({ "host": "127.0.0.1", "port": address.port() }),
    )
    .expect("encode denied connect");
    assert!(
        net_call_proto_impl_with(&mut state, &denied_policy, &denied_connect.encode_to_vec())
            .is_err()
    );

    let close =
        super::net::net_action_payload_to_proto_request("close", &json!({ "handle": handle }))
            .expect("encode close");
    assert!(
        net_call_proto_impl_with(&mut state, &allowed_policy, &close.encode_to_vec()).is_ok()
    );
    server.join().expect("server join");
}

#[test]
fn tcp_proto_binary_roundtrip_preserves_non_utf8_bytes() {
    let mut state = NetState::new();
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind listener");
    let address = listener.local_addr().expect("listener address");
    let payload = vec![0_u8, 1, 2, 127, 128, 200, 255];
    let server = std::thread::spawn({
        let payload = payload.clone();
        move || {
            let (mut stream, _) = listener.accept().expect("accept connection");
            let mut buf = vec![0_u8; payload.len()];
            std::io::Read::read_exact(&mut stream, &mut buf).expect("read request");
            assert_eq!(buf, payload, "server received unexpected bytes");
            std::io::Write::write_all(&mut stream, &buf).expect("write response");
        }
    });
    let allowed = format!("127.0.0.1:{}", address.port());
    let policy = test_net_policy(&allowed);

    let connect = super::net::net_action_payload_to_proto_request(
        "connect",
        &json!({ "host": "127.0.0.1", "port": address.port() }),
    )
    .expect("encode connect");
    let connect_response =
        net_call_proto_impl_with(&mut state, &policy, &connect.encode_to_vec())
            .expect("connect");
    let connect_response = proto::bridge_v1::NetResponse::decode(connect_response.as_slice())
        .expect("decode connect response");
    let handle = match connect_response.action.expect("connect action") {
        proto::bridge_v1::net_response::Action::Connect(response) => response.handle,
        other => panic!("unexpected connect response: {other:?}"),
    };

    let write = super::net::net_action_payload_to_proto_request(
        "write",
        &json!({
            "handle": handle,
            "data": payload.iter().map(|b| *b as u64).collect::<Vec<_>>()
        }),
    )
    .expect("encode write");
    let write_response =
        net_call_proto_impl_with(&mut state, &policy, &write.encode_to_vec()).expect("write");
    let write_response = proto::bridge_v1::NetResponse::decode(write_response.as_slice())
        .expect("decode write response");
    assert_eq!(
        write_response
            .action
            .and_then(|a| match a {
                proto::bridge_v1::net_response::Action::Write(w) => Some(w.written),
                _ => None,
            })
            .unwrap_or(0) as usize,
        payload.len()
    );

    let read = super::net::net_action_payload_to_proto_request(
        "read",
        &json!({ "handle": handle, "max_bytes": payload.len() as u64 }),
    )
    .expect("encode read");
    let read_response =
        net_call_proto_impl_with(&mut state, &policy, &read.encode_to_vec()).expect("read");
    let read_response = proto::bridge_v1::NetResponse::decode(read_response.as_slice())
        .expect("decode read response");
    let read_bytes = match read_response.action.expect("read action") {
        proto::bridge_v1::net_response::Action::Read(r) => r.data,
        other => panic!("unexpected read response: {other:?}"),
    };
    assert_eq!(read_bytes, payload);

    let close =
        super::net::net_action_payload_to_proto_request("close", &json!({ "handle": handle }))
            .expect("encode close");
    assert!(net_call_proto_impl_with(&mut state, &policy, &close.encode_to_vec()).is_ok());
    server.join().expect("server join");
}

#[test]
fn tcp_listen_accept_roundtrip() {
    let mut state = NetState::new();

    let raw_listener = TcpListener::bind("127.0.0.1:0").expect("reserve listener");
    let addr = raw_listener.local_addr().expect("listener address");
    let allowed = format!("127.0.0.1:{}", addr.port());
    let policy = test_net_policy(&allowed);
    drop(raw_listener);

    let listen = super::net::net_action_payload_to_proto_request(
        "listen",
        &json!({
            "host": "127.0.0.1",
            "port": addr.port(),
            "backlog": 5,
        }),
    )
    .expect("encode listen");
    let listen_response =
        net_call_proto_impl_with(&mut state, &policy, &listen.encode_to_vec()).expect("listen");
    let listen_response = proto::bridge_v1::NetResponse::decode(listen_response.as_slice())
        .expect("decode listen response");
    let listen_handle = match listen_response.action.expect("listen action") {
        proto::bridge_v1::net_response::Action::Listen(response) => response.handle,
        other => panic!("unexpected listen response: {other:?}"),
    };

    let client = std::thread::spawn(move || {
        let mut stream = std::net::TcpStream::connect(addr).expect("client connect");
        stream.write_all(b"ping").expect("client write");
        let mut buf = [0_u8; 4];
        std::io::Read::read_exact(&mut stream, &mut buf).expect("client read");
        assert_eq!(&buf, b"pong");
    });

    let accept = super::net::net_action_payload_to_proto_request(
        "accept",
        &json!({ "handle": listen_handle }),
    )
    .expect("encode accept");
    let accept_response =
        net_call_proto_impl_with(&mut state, &policy, &accept.encode_to_vec()).expect("accept");
    let accept_response = proto::bridge_v1::NetResponse::decode(accept_response.as_slice())
        .expect("decode accept response");
    let conn_handle = match accept_response.action.expect("accept action") {
        proto::bridge_v1::net_response::Action::Accept(response) => response.handle,
        other => panic!("unexpected accept response: {other:?}"),
    };

    let read = super::net::net_action_payload_to_proto_request(
        "read",
        &json!({ "handle": conn_handle, "max_bytes": 4 }),
    )
    .expect("encode read");
    let read_response =
        net_call_proto_impl_with(&mut state, &policy, &read.encode_to_vec()).expect("read");
    let read_response = proto::bridge_v1::NetResponse::decode(read_response.as_slice())
        .expect("decode read response");
    let read_bytes = match read_response.action.expect("read action") {
        proto::bridge_v1::net_response::Action::Read(r) => r.data,
        other => panic!("unexpected read response: {other:?}"),
    };
    assert_eq!(read_bytes, b"ping");

    let write = super::net::net_action_payload_to_proto_request(
        "write",
        &json!({
            "handle": conn_handle,
            "data": b"pong".iter().map(|b| *b as u64).collect::<Vec<_>>()
        }),
    )
    .expect("encode write");
    net_call_proto_impl_with(&mut state, &policy, &write.encode_to_vec()).expect("write");

    client.join().expect("client join");
}

#[test]
fn tls_listen_accept_and_connect_tls_roundtrip() {
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()])
        .expect("generate self-signed cert");
    let cert_pem = cert.cert.pem().into_bytes();
    let key_pem = cert.key_pair.serialize_pem().into_bytes();

    let mut state = NetState::new();

    let raw_listener = TcpListener::bind("127.0.0.1:0").expect("reserve listener");
    let addr = raw_listener.local_addr().expect("listener address");
    let allowed = format!("127.0.0.1:{}", addr.port());
    let policy = test_net_policy(&allowed);
    drop(raw_listener);

    let listen = super::net::net_action_payload_to_proto_request(
        "listen_tls",
        &json!({
            "host": "127.0.0.1",
            "port": addr.port(),
            "backlog": 5,
            "cert": cert_pem.iter().map(|b| *b as u64).collect::<Vec<_>>(),
            "key": key_pem.iter().map(|b| *b as u64).collect::<Vec<_>>(),
        }),
    )
    .expect("encode listen_tls");
    let listen_response =
        net_call_proto_impl_with(&mut state, &policy, &listen.encode_to_vec())
            .expect("listen_tls");
    let listen_response = proto::bridge_v1::NetResponse::decode(listen_response.as_slice())
        .expect("decode listen response");
    let listen_handle = match listen_response.action.expect("listen action") {
        proto::bridge_v1::net_response::Action::ListenTls(response) => response.handle,
        other => panic!("unexpected listen response: {other:?}"),
    };

    let client_ca_cert = cert_pem.clone();
    let client_policy = policy.clone();
    let client = std::thread::spawn(move || {
        let mut client_state = NetState::new();
        let connect_tls = super::net::net_action_payload_to_proto_request(
            "connect_tls",
            &json!({
                "host": "127.0.0.1",
                "port": addr.port(),
                "server_name": "localhost",
                "timeout_ms": 5000,
                "ca_cert": client_ca_cert.iter().map(|b| *b as u64).collect::<Vec<_>>(),
            }),
        )
        .expect("encode connect_tls");
        let connect_response = net_call_proto_impl_with(
            &mut client_state,
            &client_policy,
            &connect_tls.encode_to_vec(),
        )
        .expect("connect_tls");
        let connect_response =
            proto::bridge_v1::NetResponse::decode(connect_response.as_slice())
                .expect("decode connect response");
        let handle = match connect_response.action.expect("connect action") {
            proto::bridge_v1::net_response::Action::ConnectTls(response) => response.handle,
            other => panic!("unexpected connect response: {other:?}"),
        };

        let write = super::net::net_action_payload_to_proto_request(
            "write",
            &json!({
                "handle": handle,
                "data": b"ping".iter().map(|b| *b as u64).collect::<Vec<_>>()
            }),
        )
        .expect("encode write");
        net_call_proto_impl_with(&mut client_state, &client_policy, &write.encode_to_vec())
            .expect("write");

        let read = super::net::net_action_payload_to_proto_request(
            "read",
            &json!({ "handle": handle, "max_bytes": 4 }),
        )
        .expect("encode read");
        let read_response =
            net_call_proto_impl_with(&mut client_state, &client_policy, &read.encode_to_vec())
                .expect("read");
        let read_response = proto::bridge_v1::NetResponse::decode(read_response.as_slice())
            .expect("decode read response");
        let read_bytes = match read_response.action.expect("read action") {
            proto::bridge_v1::net_response::Action::Read(r) => r.data,
            other => panic!("unexpected read response: {other:?}"),
        };
        assert_eq!(read_bytes, b"pong");

        let close =
            super::net::net_action_payload_to_proto_request("close", &json!({ "handle": handle }))
                .expect("encode close");
        net_call_proto_impl_with(&mut client_state, &client_policy, &close.encode_to_vec())
            .expect("close");
    });

    let accept = super::net::net_action_payload_to_proto_request(
        "accept",
        &json!({ "handle": listen_handle }),
    )
    .expect("encode accept");
    let accept_response =
        net_call_proto_impl_with(&mut state, &policy, &accept.encode_to_vec()).expect("accept");
    let accept_response = proto::bridge_v1::NetResponse::decode(accept_response.as_slice())
        .expect("decode accept response");
    let (conn_handle, peer_addr) = match accept_response.action.expect("accept action") {
        proto::bridge_v1::net_response::Action::Accept(response) => {
            (response.handle, response.peer_addr)
        }
        other => panic!("unexpected accept response: {other:?}"),
    };
    assert!(!peer_addr.is_empty());

    let read = super::net::net_action_payload_to_proto_request(
        "read",
        &json!({ "handle": conn_handle, "max_bytes": 4 }),
    )
    .expect("encode read");
    let read_response =
        net_call_proto_impl_with(&mut state, &policy, &read.encode_to_vec()).expect("read");
    let read_response = proto::bridge_v1::NetResponse::decode(read_response.as_slice())
        .expect("decode read response");
    let read_bytes = match read_response.action.expect("read action") {
        proto::bridge_v1::net_response::Action::Read(r) => r.data,
        other => panic!("unexpected read response: {other:?}"),
    };
    assert_eq!(read_bytes, b"ping");

    let write = super::net::net_action_payload_to_proto_request(
        "write",
        &json!({
            "handle": conn_handle,
            "data": b"pong".iter().map(|b| *b as u64).collect::<Vec<_>>()
        }),
    )
    .expect("encode write");
    net_call_proto_impl_with(&mut state, &policy, &write.encode_to_vec()).expect("write");

    client.join().expect("client join");
}

#[test]
fn tcp_proto_read_until_finds_delimiter_and_eof() {
    let mut state = NetState::new();
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind listener");
    let address = listener.local_addr().expect("listener address");
    let payload = b"hello\r\nworld";
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept connection");
        std::io::Write::write_all(&mut stream, payload).expect("write payload");
    });

    let policy = test_net_policy(&format!("127.0.0.1:{}", address.port()));

    let connect = super::net::net_action_payload_to_proto_request(
        "connect",
        &json!({ "host": "127.0.0.1", "port": address.port() }),
    )
    .expect("encode connect");
    let connect_response =
        net_call_proto_impl_with(&mut state, &policy, &connect.encode_to_vec())
            .expect("connect");
    let connect_response = proto::bridge_v1::NetResponse::decode(connect_response.as_slice())
        .expect("decode connect response");
    let handle = match connect_response.action.expect("connect action") {
        proto::bridge_v1::net_response::Action::Connect(response) => response.handle,
        other => panic!("unexpected connect response: {other:?}"),
    };

    let read_until = super::net::net_action_payload_to_proto_request(
        "read_until",
        &json!({
            "handle": handle,
            "delimiter": [b'\r', b'\n'],
            "max_bytes": 128
        }),
    )
    .expect("encode read_until");
    let read_response =
        net_call_proto_impl_with(&mut state, &policy, &read_until.encode_to_vec())
            .expect("read_until");
    let read_response = proto::bridge_v1::NetResponse::decode(read_response.as_slice())
        .expect("decode read_until response");
    let (data, found, eof) = match read_response.action.expect("read_until action") {
        proto::bridge_v1::net_response::Action::ReadUntil(r) => (r.data, r.found, r.eof),
        other => panic!("unexpected read_until response: {other:?}"),
    };
    assert_eq!(data, b"hello");
    assert!(found);
    assert!(!eof);

    let close =
        super::net::net_action_payload_to_proto_request("close", &json!({ "handle": handle }))
            .expect("encode close");
    net_call_proto_impl_with(&mut state, &policy, &close.encode_to_vec()).expect("close");
    server.join().expect("server join");
}
