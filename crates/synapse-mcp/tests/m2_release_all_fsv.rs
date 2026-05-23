use anyhow::Context;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use synapse_core::error_codes;
use synapse_test_utils::stdio_mcp_client::StdioMcpClient;
use tempfile::TempDir;

#[tokio::test]
async fn release_all_schema_empty_response_and_edges_fsv() -> anyhow::Result<()> {
    let log_dir = TempDir::new()?;
    let mut client = StdioMcpClient::launch_and_init_with_log_dir(Some(log_dir.path())).await?;
    let resp = client.tools_list().await?;
    let tools = resp
        .get("tools")
        .and_then(Value::as_array)
        .context("tools array missing")?;
    assert_release_all_schema(tools)?;
    call_release_all_empty_and_edges(&mut client).await?;

    assert!(client.shutdown().await?.success());
    let logs = read_logs(log_dir.path())?;
    assert_release_all_log_readbacks(&logs);
    Ok(())
}

fn assert_release_all_schema(tools: &[Value]) -> anyhow::Result<()> {
    let release_all = tools
        .iter()
        .find(|tool| tool.get("name") == Some(&Value::String("release_all".to_owned())))
        .context("release_all tool missing")?;
    let schema = &release_all["inputSchema"];
    println!(
        "source_of_truth=tools_list tool=release_all edge=schema before=tool_count:{}",
        tools.len()
    );
    println!(
        "source_of_truth=tools_list tool=release_all edge=schema after=additionalProperties:{} properties:{} required:{}",
        schema["additionalProperties"],
        schema.get("properties").unwrap_or(&Value::Null),
        schema.get("required").unwrap_or(&Value::Null)
    );
    assert_eq!(schema["additionalProperties"], false);
    if let Some(properties) = schema.get("properties") {
        assert_eq!(properties, &json!({}));
    }
    assert!(schema.get("required").is_none_or(Value::is_null));

    let projection = json!({
        "name": release_all["name"],
        "description": release_all["description"],
        "inputSchema": release_all["inputSchema"],
        "outputSchema": release_all["outputSchema"],
    });
    insta::assert_json_snapshot!("m2_release_all_tool", projection);
    Ok(())
}

async fn call_release_all_empty_and_edges(client: &mut StdioMcpClient) -> anyhow::Result<()> {
    println!("source_of_truth=mcp_release_all edge=empty before=expected_counts:0/0/0");
    let empty = client.tools_call("release_all", json!({})).await?;
    let response: ReleaseAllWireResponse = structured(&empty)?;
    println!(
        "source_of_truth=mcp_release_all edge=empty after=released_keys:{} released_buttons:{} neutralized_pads:{}",
        response.released_keys, response.released_buttons, response.neutralized_pads
    );
    assert_eq!(response.released_keys, 0);
    assert_eq!(response.released_buttons, 0);
    assert_eq!(response.neutralized_pads, 0);

    assert_error_code(
        client,
        "extra_property",
        "unexpected:true",
        json!({"unexpected": true}),
        error_codes::TOOL_PARAMS_INVALID,
    )
    .await?;
    assert_empty_after_error(client, "extra_property").await?;

    assert_error_code(
        client,
        "output_field_as_input",
        "released_keys:0",
        json!({"released_keys": 0}),
        error_codes::TOOL_PARAMS_INVALID,
    )
    .await?;
    assert_empty_after_error(client, "output_field_as_input").await?;

    assert_error_code(
        client,
        "verb_field_as_input",
        "verb:release_all",
        json!({"verb": "release_all"}),
        error_codes::TOOL_PARAMS_INVALID,
    )
    .await?;
    assert_empty_after_error(client, "verb_field_as_input").await
}

async fn assert_error_code(
    client: &mut StdioMcpClient,
    edge: &str,
    before: &str,
    args: Value,
    expected_code: &'static str,
) -> anyhow::Result<()> {
    println!("source_of_truth=mcp_release_all edge={edge} before={before}");
    let error = client.tools_call_error("release_all", args).await?;
    println!("source_of_truth=mcp_release_all edge={edge} after={error}");
    assert_eq!(error_code(&error), Some(expected_code));
    Ok(())
}

async fn assert_empty_after_error(client: &mut StdioMcpClient, edge: &str) -> anyhow::Result<()> {
    let readback = client.tools_call("release_all", json!({})).await?;
    let response: ReleaseAllWireResponse = structured(&readback)?;
    println!(
        "source_of_truth=mcp_release_all edge={edge} after_state_readback=released_keys:{} released_buttons:{} neutralized_pads:{}",
        response.released_keys, response.released_buttons, response.neutralized_pads
    );
    assert_eq!(response.released_keys, 0);
    assert_eq!(response.released_buttons, 0);
    assert_eq!(response.neutralized_pads, 0);
    Ok(())
}

fn assert_release_all_log_readbacks(logs: &str) {
    let safety = logs
        .lines()
        .filter_map(parse_log_fields)
        .filter(|fields| {
            fields.get("code").and_then(Value::as_str)
                == Some(error_codes::SAFETY_RELEASE_ALL_FIRED)
                && fields.get("reason").and_then(Value::as_str) == Some("tool_invocation")
        })
        .count();
    let readbacks = logs
        .lines()
        .filter_map(parse_log_fields)
        .filter(|fields| {
            fields.get("code").and_then(Value::as_str) == Some("M2_RELEASE_ALL_READBACK")
        })
        .count();
    println!(
        "source_of_truth=daemon_log tool=release_all after_safety_tool_invocation_count={safety} after_readback_count={readbacks}"
    );
    assert!(
        safety >= 4,
        "expected release_all tool_invocation safety logs for empty plus edge readbacks"
    );
    assert!(
        readbacks >= 4,
        "expected release_all source-of-truth readback logs"
    );
}

#[derive(serde::Deserialize)]
struct ReleaseAllWireResponse {
    released_keys: u32,
    released_buttons: u32,
    neutralized_pads: u32,
}

fn structured<T: DeserializeOwned>(resp: &Value) -> anyhow::Result<T> {
    serde_json::from_value(resp["structuredContent"].clone()).context("decode structuredContent")
}

fn error_code(error: &Value) -> Option<&str> {
    error
        .get("data")
        .and_then(|data| data.get("code"))
        .and_then(Value::as_str)
}

fn read_logs(path: &std::path::Path) -> anyhow::Result<String> {
    let mut logs = String::new();
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        if entry.metadata()?.is_file() {
            logs.push_str(&std::fs::read_to_string(entry.path())?);
        }
    }
    Ok(logs)
}

fn parse_log_fields(line: &str) -> Option<Value> {
    let value: Value = serde_json::from_str(line).ok()?;
    Some(value.get("fields")?.clone())
}
