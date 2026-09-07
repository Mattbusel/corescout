//! Talking to a running service.
//!
//! Used by the CLI and by the MCP bridge. Both are short-lived processes that
//! make a handful of calls, so this opens a connection per call and closes it,
//! which is simpler than keeping one alive and indistinguishable in cost at
//! this rate.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use corescout_core::error::{Error, Result};
use serde_json::{json, Value};

use crate::endpoint::Endpoint;

/// How long to wait on a call.
///
/// Generous, because a call can run a capability, and a build is a legitimate
/// thing for one to be waiting on.
const TIMEOUT: Duration = Duration::from_secs(600);

/// A connection to the local service.
#[derive(Clone, Debug)]
pub struct Client {
    endpoint: Endpoint,
}

impl Client {
    /// Find the running service.
    pub fn connect() -> Result<Client> {
        Ok(Client {
            endpoint: Endpoint::discover()?,
        })
    }

    /// Use a known endpoint.
    pub fn at(endpoint: Endpoint) -> Client {
        Client { endpoint }
    }

    /// Where it is.
    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    /// Whether the service answers.
    pub fn is_alive(&self) -> bool {
        self.request("GET", "/health", None).is_ok()
    }

    /// Call a method.
    pub fn call(&self, method: &str, params: Value) -> Result<Value> {
        let body = json!({ "method": method, "params": params });
        let text = serde_json::to_string(&body)
            .map_err(|error| Error::invalid(format!("could not send that: {error}")))?;
        let (status, reply) = self.request("POST", "/call", Some(&text))?;
        let value: Value = serde_json::from_str(&reply).unwrap_or(Value::Null);
        if status == 200 {
            return Ok(value);
        }
        // The service's own message, not an HTTP status. A user who typed a
        // command wrong should be told what was wrong with it.
        Err(Error::invalid(
            value
                .get("error")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| format!("CoreScout answered {status}")),
        ))
    }

    fn request(&self, verb: &str, path: &str, body: Option<&str>) -> Result<(u16, String)> {
        let address = format!("{}:{}", self.endpoint.host, self.endpoint.port);
        let mut stream = TcpStream::connect(&address)
            .map_err(|source| Error::io(format!("{address} (is CoreScout running?)"), source))?;
        stream
            .set_read_timeout(Some(TIMEOUT))
            .and_then(|_| stream.set_write_timeout(Some(TIMEOUT)))
            .map_err(|source| Error::io(&address, source))?;

        let mut request = format!(
            "{verb} {path} HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {}\r\nConnection: \
             close\r\n",
            self.endpoint.token
        );
        match body {
            Some(text) => {
                request.push_str("Content-Type: application/json\r\n");
                request.push_str(&format!("Content-Length: {}\r\n\r\n", text.len()));
                request.push_str(text);
            }
            None => request.push_str("\r\n"),
        }

        stream
            .write_all(request.as_bytes())
            .and_then(|_| stream.flush())
            .map_err(|source| Error::io(&address, source))?;

        let mut reply = String::new();
        stream
            .read_to_string(&mut reply)
            .map_err(|source| Error::io(&address, source))?;
        parse(&reply)
    }
}

/// Split a reply into its status and its body.
fn parse(reply: &str) -> Result<(u16, String)> {
    let status = reply
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or_else(|| {
            Error::invalid("CoreScout sent something that is not a reply".to_string())
        })?;
    let body = reply
        .split_once("\r\n\r\n")
        .map(|(_, body)| body.to_string())
        .unwrap_or_default();
    Ok((status, body))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::Api;
    use crate::engine::Engine;
    use crate::http::Server;
    use corescout_storage::{Ring, Store};
    use std::sync::{Arc, Mutex};

    fn running() -> (tempfile::TempDir, Client) {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let store = Store::open(&dir.path().join("t.redb")).expect("a store");
        let ring = Ring::open(&dir.path().join("t.ring"), 512).expect("a ring");
        let engine = Engine::open(store, ring).expect("an engine");
        let token = crate::endpoint::mint_token();
        let server =
            Server::bind(Api::new(Arc::new(Mutex::new(engine))), token.clone(), 0).expect("bind");
        let endpoint = Endpoint::new(server.port(), token);
        std::thread::spawn(move || server.serve());
        (dir, Client::at(endpoint))
    }

    #[test]
    fn a_client_reaches_a_running_service() {
        let (_dir, client) = running();
        assert!(client.is_alive());
        let status = client.call("status", json!({})).expect("status");
        assert_eq!(status["running"], true);
    }

    #[test]
    fn the_service_error_reaches_the_caller_rather_than_a_status_code() {
        // Someone who typed a command wrong should be told what was wrong with
        // it, not shown a 400.
        let (_dir, client) = running();
        let error = client
            .call("autonomy", json!({ "mode": "full" }))
            .expect_err("should fail")
            .to_string();
        assert!(error.contains("autopilot"), "{error}");
    }

    #[test]
    fn calling_a_method_that_does_not_exist_says_which() {
        let (_dir, client) = running();
        let error = client
            .call("nonsense", json!({}))
            .expect_err("should fail")
            .to_string();
        assert!(error.contains("nonsense"), "{error}");
    }

    #[test]
    fn a_client_pointed_at_nothing_says_corescout_may_not_be_running() {
        let client = Client::at(Endpoint::new(1, "t"));
        assert!(!client.is_alive());
        let error = client.call("status", json!({})).expect_err("should fail");
        assert!(
            error.to_string().contains("is CoreScout running?"),
            "{error}"
        );
    }

    #[test]
    fn a_reply_is_split_into_its_status_and_its_body() {
        let (status, body) =
            parse("HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}").expect("parse");
        assert_eq!(status, 200);
        assert_eq!(body, "{}");
    }

    #[test]
    fn something_that_is_not_a_reply_is_refused_rather_than_parsed() {
        assert!(parse("hello there").is_err());
        assert!(parse("").is_err());
    }
}
