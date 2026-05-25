use anyhow::Context;
use serde_json::{Value, json};
use synapse_test_utils::stdio_mcp_client::StdioMcpClient;
use tempfile::TempDir;

#[tokio::test]
async fn subscribe_schema_defaults_and_edges() -> anyhow::Result<()> {
    let logs = TempDir::new()?;
    let mut client = StdioMcpClient::launch_and_init_with_log_dir(Some(logs.path())).await?;

    let tools = client.tools_list().await?;
    let tools = tools
        .get("tools")
        .and_then(Value::as_array)
        .context("tools array missing")?;
    let subscribe_tool = tools
        .iter()
        .find(|tool| tool["name"] == "subscribe")
        .context("subscribe tool missing")?;
    assert_subscribe_schema(subscribe_tool);

    let response = client.tools_call("subscribe", json!({})).await?;
    let first = structured(&response)?;
    assert!(
        first["subscription_id"]
            .as_str()
            .is_some_and(|id| !id.is_empty())
    );
    assert!(first["started_at"].as_str().is_some());

    let bad_buffer = client
        .tools_call_error("subscribe", json!({"buffer_size": 4097}))
        .await?;
    assert_eq!(bad_buffer["data"]["code"], "TOOL_PARAMS_INVALID");

    let bad_filter = client
        .tools_call_error("subscribe", json!({"filter": {"op": "and", "args": []}}))
        .await?;
    assert_eq!(bad_filter["data"]["code"], "TOOL_PARAMS_INVALID");

    for _ in 1..64 {
        let response = client.tools_call("subscribe", json!({})).await?;
        let payload = structured(&response)?;
        assert!(
            payload["subscription_id"]
                .as_str()
                .is_some_and(|id| !id.is_empty())
        );
    }
    let capped = client.tools_call_error("subscribe", json!({})).await?;
    assert_eq!(capped["data"]["code"], "SUBSCRIPTION_CAP_REACHED");

    let status = client.shutdown().await?;
    assert!(status.success());
    Ok(())
}

fn structured(response: &Value) -> anyhow::Result<Value> {
    if let Some(value) = response.get("structuredContent") {
        return Ok(value.clone());
    }

    let text = response
        .get("content")
        .and_then(Value::as_array)
        .and_then(|content| content.first())
        .and_then(|content| content.get("text"))
        .and_then(Value::as_str)
        .context("structured content missing")?;
    serde_json::from_str(text).context("parse text content")
}

fn assert_subscribe_schema(tool: &Value) {
    let shape = json!({
        "name": tool.get("name").cloned().unwrap_or(Value::Null),
        "inputSchema": tool.get("inputSchema").cloned().unwrap_or(Value::Null),
        "outputSchema": tool.get("outputSchema").cloned().unwrap_or(Value::Null),
    });
    assert_eq!(shape["inputSchema"]["additionalProperties"], false);
    assert_eq!(
        shape["inputSchema"]["properties"]["kinds"]["default"],
        json!([])
    );
    assert_eq!(
        shape["inputSchema"]["properties"]["snapshot_first"]["default"],
        false
    );
    assert_eq!(
        shape["inputSchema"]["properties"]["buffer_size"]["default"],
        4096
    );
    insta::assert_json_snapshot!("m3_subscribe_tool", shape);
}
