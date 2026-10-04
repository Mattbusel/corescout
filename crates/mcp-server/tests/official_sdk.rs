//! The official Rust MCP SDK (rmcp) as the client: if it can complete the
//! handshake, list the tools and call one, so can MCP clients in general.
//! The server runs in process over a pipe with a stand-in backend, so no
//! CoreScout data is touched.

use std::io::BufReader;

use corescout_mcp_server::{Backend, Server, SUPPORTED_PROTOCOL_VERSIONS};
use rmcp::model::CallToolRequestParams;
use rmcp::ServiceExt;
use serde_json::{json, Value};
use tokio_util::io::SyncIoBridge;

struct StandIn;

impl Backend for StandIn {
    fn call(&self, method: &str, _params: &Value) -> Result<Value, String> {
        Ok(json!({ "answered_by": method }))
    }

    fn instructions(&self) -> String {
        "CoreScout test backend".into()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_official_sdk_can_use_corescout() {
    let (client_io, server_io) = tokio::io::duplex(256 * 1024);
    let (server_read, server_write) = tokio::io::split(server_io);
    let reader = BufReader::new(SyncIoBridge::new(server_read));
    let writer = SyncIoBridge::new(server_write);
    tokio::task::spawn_blocking(move || Server::new(StandIn).run(reader, writer));

    let client = ().serve(client_io).await.expect("handshake with the official SDK");
    let info = client.peer_info().expect("server info");
    let version = serde_json::to_value(&info.protocol_version).expect("version");
    assert!(
        SUPPORTED_PROTOCOL_VERSIONS
            .iter()
            .any(|v| version == json!(v)),
        "negotiated {version}"
    );
    assert_eq!(info.instructions.as_deref(), Some("CoreScout test backend"));

    let tools = client.list_all_tools().await.expect("tools/list");
    assert!(
        tools.iter().any(|t| t.name == "corescout_status"),
        "{} tools",
        tools.len()
    );
    for tool in &tools {
        assert!(
            tool.description.is_some(),
            "{} has no description",
            tool.name
        );
        assert_eq!(
            tool.input_schema.get("type").and_then(Value::as_str),
            Some("object"),
            "{}",
            tool.name
        );
    }

    let result = client
        .call_tool(CallToolRequestParams::new("corescout_status"))
        .await
        .expect("tools/call");
    assert_ne!(result.is_error, Some(true));
    let structured = result.structured_content.expect("structured content");
    assert!(structured.to_string().contains("status"), "{structured}");

    client.cancel().await.expect("close");
}
