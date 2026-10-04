#![cfg(all(feature = "compiler", feature = "host"))]
use deka_vm::*;
#[path = "support/http_server.rs"]
mod server;
fn hosts() -> Hosts {
    let mut h = Hosts::default();
    builtin_http::register(&mut h).unwrap();
    bytes::register(&mut h).unwrap();
    h
}
async fn run(source: &str) -> HostValue {
    let h = hosts();
    let p = compiler::compile(source, &h).unwrap();
    let p = serde_json::from_slice(&serde_json::to_vec(&p).unwrap()).unwrap();
    Vm::new(p, h).unwrap().run().await.unwrap()
}
#[tokio::test]
async fn parser_success_and_typed_throw_are_consumed_by_language_handlers() {
    assert_eq!(run(r#"import {parse_url as parse} from "http";
    fn main() string {
        const u=match parse("https://[::1]:8443/a?b=1") {Ok(u)=>u,Throw(e)=>{scheme:"",host:"",port:0,path:""}};
        const a=match parse("http://") {Ok(u)=>"bad",Throw(e)=>e.name+":"+e.message};
        let b="";try{const u=parse("[broken");b=u.host;}catch(e){b=e.message;}
        return u.scheme+";"+u.host+";"+string(u.port)+";"+u.path+";"+a+";"+b;
    }"#).await,HostValue::String("https;::1;8443;/a?b=1;Error:invalid url;invalid url".into()));
}
#[tokio::test]
async fn formatter_is_a_first_class_checked_function_and_counts_utf8_bytes() {
    assert_eq!(run(r#"import {format_request} from "http";import {from_string} from "bytes";
        fn main() string {const format=format_request;return format("post","","h",{content_type:"text/plain"},from_string("é"));}"#).await,
        HostValue::String("POST / HTTP/1.1\r\nContent-Type: text/plain\r\nHost: h\r\nContent-Length: 2\r\nConnection: close\r\n\r\n".into()));
}
#[tokio::test]
async fn network_get_post_request_and_http_statuses_are_real_results() {
    use std::io::Read;
    let server = server::Server::new(|path, stream| {
        if path == "/post" {
            let mut b = [0; 2];
            stream.read_exact(&mut b)?;
            if b != [0xc3, 0xa9] {
                return Err(std::io::Error::other("wrong posted bytes"));
            }
        }
        Ok(server::reply(
            if path == "/missing" {
                "404 Not Found"
            } else {
                "200 OK"
            },
            "",
            path.as_bytes(),
        ))
    })
    .unwrap();
    let source = format!(
        r#"import {{get,post,request}} from "http";import {{from_string,to_string}} from "bytes";
    fn main() string {{
        const a=unwrap(get("{}")) or{{return "get failed";}};
        const b=unwrap(post("{}","text/plain",from_string("é"))) or{{return "post failed";}};
        const c=unwrap(request("GET","{}",{{content_type:""}},from_string(""))) or{{return "request failed";}};
        return string(a.complete)+";"+string(a.status)+";"+(match to_string(a.body){{Ok(t)=>t,Err(e)=>e}})+";"+string(b.status)+";"+string(c.status)+";"+string(c.error);
    }}"#,
        server.url("/get"),
        server.url("/post"),
        server.url("/missing")
    );
    assert_eq!(
        run(&source).await,
        HostValue::String("true;200;/get;200;404;None".into())
    );
    assert_eq!(server.finish().unwrap(), ["/get", "/post", "/missing"]);
}
#[tokio::test]
async fn module_preserves_no_redirect_and_chunked_refusal_with_bounded_bodies() {
    let server=server::Server::new(|path,_|Ok(match path{
        "/redirect"=>server::reply("302 Found","Location: /done\r\n",b""),
        "/chunked"=>b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n1\r\nx\r\n0\r\n\r\n".to_vec(),
        _=>b"HTTP/1.1 200 OK\r\nContent-Length: 1048577\r\nConnection: close\r\n\r\n".to_vec(),
    })).unwrap();
    let source = format!(
        r#"import {{get}} from "http";
    fn main() string {{
        const a=match get("{}"){{Ok(r)=>string(r.status),Err(e)=>e}};
        const b=match get("{}"){{Ok(r)=>"bad",Err(e)=>e}};
        const c=match get("{}"){{Ok(r)=>"bad",Err(e)=>e}};
        return a+";"+b+";"+c;
    }}"#,
        server.url("/redirect"),
        server.url("/chunked"),
        server.url("/large")
    );
    assert_eq!(
        run(&source).await,
        HostValue::String(
            "302;chunked encoding not supported;response body exceeds 1048576 byte buffered limit"
                .into()
        )
    );
    assert_eq!(
        server.finish().unwrap(),
        ["/redirect", "/chunked", "/large"]
    );
}
#[test]
fn catalog_rejects_wrong_types_unknown_exports_and_async_exception_channels() {
    let h = hosts();
    for source in [
        "import {get} from \"http\";fn main(){get(7);}",
        "import {post} from \"http\";fn main(){post(\"x\",\"text/plain\",\"body\");}",
        "import {parse_response} from \"http\";fn main(){}",
        "import {format_request} from \"http\";fn main(){format_request(\"GET\",\"/\",\"h\",{},[]);}",
    ] {
        assert!(compiler::compile(source, &h).is_err(), "{source}");
    }
    let mut h = Hosts::default();
    assert!(
        h.register(
            HostOp::new("bad", vec![], HostType::Number, true, |_| HostReply::Ready(
                Ok(HostValue::Number(1.))
            ))
            .with_exception_channel()
        )
        .is_err()
    );
    assert!(
        h.register(
            HostOp::new(
                "bad",
                vec![],
                HostType::Number,
                false,
                |_| HostReply::Ready(Ok(HostValue::Number(1.)))
            )
            .with_exception_channel()
            .with_result_channel()
        )
        .is_err()
    );
}
#[tokio::test]
async fn result_network_errors_remain_data_and_uncaught_parser_throw_fails() {
    assert_eq!(run(r#"import {get} from "http";fn main() string {return match get("not a url"){Ok(r)=>"bad",Err(e)=>"error data"};}"#).await,HostValue::String("error data".into()));
    let h = hosts();
    let p = compiler::compile(
        r#"import {parse_url} from "http";fn main() Exception<{host:string,path:string,port:number,scheme:string},JsError> {return parse_url("http://");}"#,
        &h,
    )
    .unwrap();
    assert!(
        Vm::new(p, h)
            .unwrap()
            .run()
            .await
            .unwrap_err()
            .contains("uncaught Throw")
    );
}
