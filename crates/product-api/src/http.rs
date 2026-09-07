//! The loopback server: a few hundred lines of HTTP/1.1 and nothing else.
//!
//! # Why this is not a framework
//!
//! CoreScout promises to be invisible when idle. A web framework brings an
//! async runtime, a thread pool and a dependency tree, all to serve a handful
//! of requests a second from one process on one machine to the window in front
//! of it. Thread-per-connection over `std::net` is the honest size of this
//! problem, and it keeps the whole product's dependency list at four crates.
//!
//! # What this server refuses
//!
//! It binds loopback only, so nothing off the machine can reach it at all. It
//! requires the token from the endpoint file on every request but `/health`.
//! It caps a request body, so a client that says it is sending a gigabyte does
//! not get to allocate one. And it never reads a file from disk in response to
//! a path, because it does not serve files: there are two routes and both take
//! JSON.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use corescout_core::error::{Error, Result};
use serde_json::{json, Value};

use crate::api::Api;

/// The most a request body may be.
pub const MAX_BODY: usize = 1024 * 1024;
/// How long a connection may sit idle mid-request.
const TIMEOUT: Duration = Duration::from_secs(30);

/// The local API server.
pub struct Server {
    listener: TcpListener,
    api: Api,
    token: String,
    running: Arc<AtomicBool>,
}

impl Server {
    /// Bind a port on loopback.
    ///
    /// Pass zero for the port to be given one, which is what the service does:
    /// a fixed port collides with whatever else is on the machine.
    pub fn bind(api: Api, token: impl Into<String>, port: u16) -> Result<Server> {
        let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        let listener = TcpListener::bind(address)
            .map_err(|source| Error::io(format!("127.0.0.1:{port}"), source))?;
        Ok(Server {
            listener,
            api,
            token: token.into(),
            running: Arc::new(AtomicBool::new(true)),
        })
    }

    /// The port actually bound.
    pub fn port(&self) -> u16 {
        self.listener
            .local_addr()
            .map(|address| address.port())
            .unwrap_or(0)
    }

    /// A handle that stops the server.
    pub fn stopper(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.running)
    }

    /// Serve until stopped.
    pub fn serve(&self) {
        for stream in self.listener.incoming() {
            if !self.running.load(Ordering::Relaxed) {
                break;
            }
            let Ok(stream) = stream else { continue };
            let api = self.api.clone();
            let token = self.token.clone();
            // One thread per connection. Clients are the window in front of
            // the user and a handful of agent processes, so the count is in
            // single figures and a pool would be machinery for its own sake.
            let _ = std::thread::Builder::new()
                .name("corescout-http".into())
                .spawn(move || {
                    let _ = handle(stream, &api, &token);
                });
        }
    }
}

/// One request.
#[derive(Debug, Default, PartialEq)]
struct Request {
    method: String,
    path: String,
    authorization: Option<String>,
    origin: Option<String>,
    body: Vec<u8>,
}

fn handle(stream: TcpStream, api: &Api, token: &str) -> std::io::Result<()> {
    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.set_write_timeout(Some(TIMEOUT))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let request = match read_request(&mut reader) {
        Ok(request) => request,
        Err(message) => return respond(stream, 400, &json!({ "error": message }), None),
    };

    // A browser tab could reach a loopback port. It cannot read the endpoint
    // file, so it cannot hold the token, and every route but the liveness
    // check requires one.
    let origin = request.origin.clone();
    if request.method == "OPTIONS" {
        return respond(stream, 204, &Value::Null, origin.as_deref());
    }
    if request.path == "/health" {
        return respond(
            stream,
            200,
            &json!({ "ok": true, "version": crate::engine::VERSION }),
            origin.as_deref(),
        );
    }
    if !authorised(&request, token) {
        return respond(
            stream,
            401,
            &json!({ "error": "CoreScout needs the token from its endpoint file" }),
            origin.as_deref(),
        );
    }

    match request.path.as_str() {
        "/methods" => respond(
            stream,
            200,
            &json!(crate::api::METHODS
                .iter()
                .map(|(name, about)| json!({ "method": name, "about": about }))
                .collect::<Vec<_>>()),
            origin.as_deref(),
        ),
        "/call" => {
            let parsed: Value = match serde_json::from_slice(&request.body) {
                Ok(value) => value,
                Err(error) => {
                    return respond(
                        stream,
                        400,
                        &json!({ "error": format!("that is not JSON: {error}") }),
                        origin.as_deref(),
                    )
                }
            };
            let method = parsed
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let params = parsed.get("params").cloned().unwrap_or(json!({}));
            match api.call(&method, &params) {
                Ok(result) => respond(stream, 200, &result, origin.as_deref()),
                Err(error) => respond(
                    stream,
                    400,
                    &json!({ "error": error.to_string() }),
                    origin.as_deref(),
                ),
            }
        }
        other => respond(
            stream,
            404,
            &json!({ "error": format!("no route {other}") }),
            origin.as_deref(),
        ),
    }
}

/// Whether the request carries the token.
///
/// Compared in full rather than short-circuiting on the first differing byte.
/// The timing signal from a `==` on a local socket is not a realistic attack,
/// and writing the careful version costs one line.
fn authorised(request: &Request, token: &str) -> bool {
    let Some(header) = &request.authorization else {
        return false;
    };
    let offered = header.strip_prefix("Bearer ").unwrap_or(header).trim();
    if offered.len() != token.len() {
        return false;
    }
    offered
        .bytes()
        .zip(token.bytes())
        .fold(0u8, |differences, (a, b)| differences | (a ^ b))
        == 0
}

fn read_request(reader: &mut BufReader<TcpStream>) -> std::result::Result<Request, String> {
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .map_err(|error| format!("could not read the request: {error}"))?;
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();
    if method.is_empty() || path.is_empty() {
        return Err("that is not an HTTP request".into());
    }

    let mut request = Request {
        method,
        path,
        ..Request::default()
    };
    let mut length = 0usize;
    loop {
        let mut header = String::new();
        let read = reader
            .read_line(&mut header)
            .map_err(|error| format!("could not read the headers: {error}"))?;
        if read == 0 || header.trim().is_empty() {
            break;
        }
        let Some((name, value)) = header.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match name.trim().to_ascii_lowercase().as_str() {
            "authorization" => request.authorization = Some(value.to_string()),
            "origin" => request.origin = Some(value.to_string()),
            "content-length" => {
                length = value
                    .parse()
                    .map_err(|_| "content-length is not a number".to_string())?;
                if length > MAX_BODY {
                    // Refused on the header, before anything is allocated. A
                    // client that claims a gigabyte does not get to have one
                    // reserved for it.
                    return Err(format!("a request body may not exceed {MAX_BODY} bytes"));
                }
            }
            _ => {}
        }
    }
    if length > 0 {
        let mut body = vec![0u8; length];
        reader
            .read_exact(&mut body)
            .map_err(|error| format!("the body was shorter than it said: {error}"))?;
        request.body = body;
    }
    Ok(request)
}

fn respond(
    mut stream: TcpStream,
    status: u16,
    body: &Value,
    origin: Option<&str>,
) -> std::io::Result<()> {
    let text = if body.is_null() {
        String::new()
    } else {
        serde_json::to_string(body).unwrap_or_else(|_| "{}".into())
    };
    let reason = match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        _ => "Error",
    };
    let mut head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
         Connection: close\r\n",
        text.len()
    );
    // The desktop shell runs on its own origin and talks to this port. Echoing
    // the origin rather than allowing everything keeps the reply specific;
    // either way the token is what actually guards the API, and a page in a
    // browser cannot read the file it lives in.
    if let Some(origin) = origin {
        head.push_str(&format!("Access-Control-Allow-Origin: {origin}\r\n"));
        head.push_str("Access-Control-Allow-Headers: authorization, content-type\r\n");
        head.push_str("Access-Control-Allow-Methods: POST, GET, OPTIONS\r\n");
        head.push_str("Vary: Origin\r\n");
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;
    stream.write_all(text.as_bytes())?;
    stream.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Engine;
    use corescout_storage::{Ring, Store};
    use std::sync::Mutex;

    fn server() -> (tempfile::TempDir, Server, String) {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let store = Store::open(&dir.path().join("t.redb")).expect("a store");
        let ring = Ring::open(&dir.path().join("t.ring"), 512).expect("a ring");
        let engine = Engine::open(store, ring).expect("an engine");
        let token = crate::endpoint::mint_token();
        let server =
            Server::bind(Api::new(Arc::new(Mutex::new(engine))), token.clone(), 0).expect("bind");
        (dir, server, token)
    }

    /// Send a raw request and read the whole reply.
    fn send(port: u16, request: &str) -> String {
        let mut stream =
            TcpStream::connect((Ipv4Addr::LOCALHOST, port)).expect("connect to the server");
        stream.write_all(request.as_bytes()).expect("write");
        stream.flush().expect("flush");
        let mut reply = String::new();
        stream.read_to_string(&mut reply).expect("read");
        reply
    }

    fn spawn(server: Server) -> u16 {
        let port = server.port();
        std::thread::spawn(move || server.serve());
        // The listener is already bound before the thread starts, so a
        // connection cannot be refused; nothing needs waiting for.
        port
    }

    #[test]
    fn the_server_binds_loopback_and_nothing_else() {
        let (_dir, server, _token) = server();
        let address = server.listener.local_addr().expect("an address");
        assert_eq!(address.ip().to_string(), "127.0.0.1");
        assert!(server.port() > 0);
    }

    #[test]
    fn health_needs_no_token_and_everything_else_does() {
        // The liveness check has to work before a client has read the endpoint
        // file, and it says nothing a stranger could use.
        let (_dir, server, _token) = server();
        let port = spawn(server);

        let health = send(port, "GET /health HTTP/1.1\r\nHost: x\r\n\r\n");
        assert!(health.starts_with("HTTP/1.1 200"), "{health}");

        let refused = send(
            port,
            "POST /call HTTP/1.1\r\nHost: x\r\nContent-Length: 22\r\n\r\n{\"method\":\"status\"}\r\n\r\n",
        );
        assert!(refused.starts_with("HTTP/1.1 401"), "{refused}");
        assert!(refused.contains("token"));
    }

    #[test]
    fn a_wrong_token_is_refused() {
        let (_dir, server, _token) = server();
        let port = spawn(server);
        let body = "{\"method\":\"status\"}";
        let reply = send(
            port,
            &format!(
                "POST /call HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer wrong\r\nContent-Length: \
                 {}\r\n\r\n{body}",
                body.len()
            ),
        );
        assert!(reply.starts_with("HTTP/1.1 401"), "{reply}");
    }

    #[test]
    fn a_correct_token_gets_an_answer() {
        let (_dir, server, token) = server();
        let port = spawn(server);
        let body = "{\"method\":\"status\",\"params\":{}}";
        let reply = send(
            port,
            &format!(
                "POST /call HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {token}\r\nContent-Length: \
                 {}\r\n\r\n{body}",
                body.len()
            ),
        );
        assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
        assert!(reply.contains("\"running\":true"), "{reply}");
    }

    #[test]
    fn an_oversized_body_is_refused_on_the_header() {
        // Before anything is allocated: a client claiming a gigabyte must not
        // get a gigabyte reserved for it.
        let (_dir, server, token) = server();
        let port = spawn(server);
        let reply = send(
            port,
            &format!(
                "POST /call HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {token}\r\nContent-Length: \
                 999999999\r\n\r\n"
            ),
        );
        assert!(reply.starts_with("HTTP/1.1 400"), "{reply}");
        assert!(reply.contains("may not exceed"), "{reply}");
    }

    #[test]
    fn an_unknown_route_is_a_404_and_not_a_file_read() {
        // This server has two routes and serves no files. A path traversal has
        // nothing to traverse to, and this is the test that keeps it that way.
        let (_dir, server, token) = server();
        let port = spawn(server);
        for path in ["/../../etc/passwd", "/index.html", "/"] {
            let reply = send(
                port,
                &format!("GET {path} HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {token}\r\n\r\n"),
            );
            assert!(reply.starts_with("HTTP/1.1 404"), "{path}: {reply}");
        }
    }

    #[test]
    fn a_body_that_is_not_json_gets_a_readable_error() {
        let (_dir, server, token) = server();
        let port = spawn(server);
        let body = "not json at all";
        let reply = send(
            port,
            &format!(
                "POST /call HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {token}\r\nContent-Length: \
                 {}\r\n\r\n{body}",
                body.len()
            ),
        );
        assert!(reply.starts_with("HTTP/1.1 400"), "{reply}");
        assert!(reply.contains("not JSON"), "{reply}");
    }

    #[test]
    fn garbage_on_the_socket_does_not_take_the_server_down() {
        let (_dir, server, token) = server();
        let port = spawn(server);
        send(port, "\r\n\r\n");
        // Complete requests that are not requests. A *partial* one is a
        // different case: the server holds it open until its read timeout,
        // which is the correct thing to do and would make this test take that
        // long to run.
        send(port, "GARBAGE\r\n\r\n");
        send(port, "GET\r\n\r\n");
        send(
            port,
            "POST /call HTTP/1.1\r\nContent-Length: nonsense\r\n\r\n",
        );
        // Still answering afterwards.
        let reply = send(
            port,
            &format!("GET /methods HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {token}\r\n\r\n"),
        );
        assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
    }

    #[test]
    fn the_method_list_is_served_and_matches_the_api() {
        let (_dir, server, token) = server();
        let port = spawn(server);
        let reply = send(
            port,
            &format!("GET /methods HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {token}\r\n\r\n"),
        );
        let body = reply.split("\r\n\r\n").nth(1).unwrap_or_default();
        let methods: Vec<Value> = serde_json::from_str(body).expect("a list");
        assert_eq!(methods.len(), crate::api::METHODS.len());
    }

    #[test]
    fn a_preflight_is_answered_without_a_token() {
        let (_dir, server, _token) = server();
        let port = spawn(server);
        let reply = send(
            port,
            "OPTIONS /call HTTP/1.1\r\nHost: x\r\nOrigin: tauri://localhost\r\n\r\n",
        );
        assert!(reply.starts_with("HTTP/1.1 204"), "{reply}");
        assert!(
            reply.contains("Access-Control-Allow-Origin: tauri://localhost"),
            "{reply}"
        );
    }

    #[test]
    fn tokens_are_compared_in_full_rather_than_up_to_the_first_difference() {
        let request = |header: &str| Request {
            authorization: Some(header.to_string()),
            ..Request::default()
        };
        assert!(authorised(&request("Bearer abc"), "abc"));
        assert!(authorised(&request("abc"), "abc"));
        assert!(!authorised(&request("Bearer abd"), "abc"));
        assert!(!authorised(&request("Bearer ab"), "abc"));
        assert!(!authorised(&Request::default(), "abc"));
    }
}
