//! The HTTP builtins against a local server that echoes each request back as
//! JSON: `{"method", "target", "headers", "body"}`. A request for
//! `/status/N` gets status N, and `/redirect` redirects to `/landed`.

#![cfg(feature = "net")]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;

use dog4::{Runtime, Value};

/// Starts the echo server and returns its base URL.
fn server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let mut parts = line.split_whitespace();
            let method = parts.next().unwrap_or_default().to_string();
            let target = parts.next().unwrap_or_default().to_string();
            let mut headers = serde_json::Map::new();
            let mut length = 0;
            loop {
                let mut header = String::new();
                reader.read_line(&mut header).unwrap();
                let header = header.trim_end();
                if header.is_empty() {
                    break;
                }
                let (name, value) = header.split_once(':').unwrap();
                let (name, value) = (name.to_ascii_lowercase(), value.trim().to_string());
                if name == "content-length" {
                    length = value.parse().unwrap();
                }
                headers.insert(name, value.into());
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            let echo = serde_json::json!({
                "method": method,
                "target": target,
                "headers": headers,
                "body": String::from_utf8(body).unwrap(),
            })
            .to_string();
            let (status, extra) = if let Some(code) = target.strip_prefix("/status/") {
                (code.to_string(), String::new())
            } else if target == "/redirect" {
                ("302".to_string(), "Location: /landed\r\n".to_string())
            } else {
                ("200".to_string(), String::new())
            };
            let mut stream = stream;
            let _ = write!(
                stream,
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nX-Echo: yes\r\n\
                 {extra}Content-Length: {}\r\nConnection: close\r\n\r\n{echo}",
                echo.len()
            );
        }
    });
    base
}

fn run(src: &str) -> Result<String, String> {
    let mut rt = Runtime::builder()
        .global("base", Value::str(server()))
        .build();
    rt.run("main", src)
        .map(|v| v.to_string())
        .map_err(|e| e.message().to_string())
}

#[test]
fn get_and_post_return_the_body() {
    let out = run(
        r#"r = { json.parse { get "$base/a" [query = [q = "a b", tag = [x y]]] } };
           pln $r.method $r.target;
           r = { json.parse { post "$base/p" [name = Ana, tags = [1 2]] } };
           pln $r.method $r.headers["content-type"] $r.body;
           r = { json.parse { post "$base/p" "plain" 5 } };
           pln $r.body ($r.headers["content-type"] ?? none)"#,
    )
    .unwrap();
    assert_eq!(
        out,
        "GET /a?q=a%20b&tag=x&tag=y\n\
         POST application/json {\"name\":\"Ana\",\"tags\":[1,2]}\n\
         plain none\n"
    );
}

#[test]
fn fetch_returns_the_whole_response() {
    let out = run(
        r#"r = { fetch "$base/x" [
               method = put,
               headers = [Authorization = "Bearer t", content-type = text/csv],
               body = [a = 1],
           ] };
           e = json.parse($r.body);
           pln $r.status $r.ok $r.headers["x-echo"] $e.method $e.headers.authorization $e.headers["content-type"] $e.body;
           r = { fetch "$base/f" [form = [a = "1 2", b = "é"]] };
           e = json.parse($r.body);
           pln $e.method $e.headers["content-type"] $e.body;
           r = fetch("$base/status/404");
           pln $r.status $r.ok;
           pln { fetch "$base/redirect" }.url;
           pln { fetch "$base/redirect" [redirects = false] }.status"#,
    )
    .unwrap();
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "200 true yes PUT Bearer t text/csv {\"a\":1}");
    assert_eq!(
        lines[1],
        "POST application/x-www-form-urlencoded a=1%202&b=%C3%A9"
    );
    assert_eq!(lines[2], "404 false");
    assert!(lines[3].ends_with("/landed"), "{}", lines[3]);
    assert_eq!(lines[4], "302");
}

#[test]
fn errors() {
    assert_eq!(
        run(r#"get "$base/status/500""#).unwrap_err(),
        "request failed with status 500 X"
    );
    assert!(
        run(r#"fetch $base [hedaers = [a = b]]"#)
            .unwrap_err()
            .starts_with("unknown request option `hedaers`")
    );
    assert_eq!(
        run(r#"fetch $base [headers = [a = "x\ny"]]"#).unwrap_err(),
        "invalid header `a`"
    );
    assert_eq!(
        run(r#"fetch $base [method = "GET /x"]"#).unwrap_err(),
        "\"GET /x\" is not an HTTP method"
    );
    assert_eq!(
        run(r#"post $base a [body = b]"#).unwrap_err(),
        "`post` takes its body as an argument, not in the options"
    );
}
