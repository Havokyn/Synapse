use anyhow::Context;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use synapse_action::sample_typing_schedule;
use synapse_core::{KeystrokeDynamics, KeystrokeNaturalParams, error_codes};
use synapse_test_utils::stdio_mcp_client::StdioMcpClient;
use tempfile::TempDir;

#[tokio::test]
async fn act_type_schema_defaults_recording_and_edges_fsv() -> anyhow::Result<()> {
    let log_dir = TempDir::new()?;
    let mut client = StdioMcpClient::launch_and_init_with_env(
        Some(log_dir.path()),
        &[("SYNAPSE_MCP_RECORDING_BACKEND", "1")],
    )
    .await?;
    let resp = client.tools_list().await?;
    let tools = resp
        .get("tools")
        .and_then(Value::as_array)
        .context("tools array missing")?;
    assert_act_type_schema(tools)?;
    let expected_ikis = call_act_type_happy_empty_and_edges(&mut client).await?;

    assert!(client.shutdown().await?.success());
    let logs = read_logs(log_dir.path())?;
    assert_recording_log_readbacks(&logs, &expected_ikis)?;
    Ok(())
}

fn assert_act_type_schema(tools: &[Value]) -> anyhow::Result<()> {
    let act_type = tools
        .iter()
        .find(|tool| tool.get("name") == Some(&Value::String("act_type".to_owned())))
        .context("act_type tool missing")?;
    let schema = &act_type["inputSchema"];
    println!(
        "source_of_truth=tools_list tool=act_type edge=schema before=tool_count:{}",
        tools.len()
    );
    println!(
        "source_of_truth=tools_list tool=act_type edge=defaults after=dynamics:{} linear_ms_per_char:{} use_scancodes:{} press_enter_after:{} backend:{} additionalProperties:{}",
        schema["properties"]["dynamics"]["default"],
        schema["properties"]["linear_ms_per_char"]["default"],
        schema["properties"]["use_scancodes"]["default"],
        schema["properties"]["press_enter_after"]["default"],
        schema["properties"]["backend"]["default"],
        schema["additionalProperties"]
    );
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(schema["properties"]["dynamics"]["default"], "natural");
    assert_eq!(schema["properties"]["linear_ms_per_char"]["default"], 30);
    assert_eq!(schema["properties"]["use_scancodes"]["default"], false);
    assert_eq!(schema["properties"]["press_enter_after"]["default"], false);
    assert_eq!(schema["properties"]["backend"]["default"], "auto");
    assert_backend_schema(schema);

    let projection = json!({
        "name": act_type["name"],
        "description": act_type["description"],
        "inputSchema": act_type["inputSchema"],
        "outputSchemaRoot": schema_root(act_type.get("outputSchema")),
    });
    insta::assert_json_snapshot!("m2_act_type_tool", projection);
    Ok(())
}

fn assert_backend_schema(schema: &Value) {
    let schema_text = schema.to_string();
    assert!(schema_text.contains("software"));
    assert!(schema_text.contains("hardware"));
    assert!(schema_text.contains("auto"));
    assert!(!schema_text.contains("vigem"));
}

async fn call_act_type_happy_empty_and_edges(
    client: &mut StdioMcpClient,
) -> anyhow::Result<Vec<u32>> {
    let expected_ikis = call_act_type_happy(client).await?;
    call_act_type_empty(client).await?;
    call_act_type_error_edges(client).await?;
    Ok(expected_ikis)
}

async fn call_act_type_happy(client: &mut StdioMcpClient) -> anyhow::Result<Vec<u32>> {
    let text = "Hello world.";
    let expected_ikis = natural_fast_ikis(text);
    println!("source_of_truth=mcp_act_type edge=happy before=text:{text:?}");
    let happy = client.tools_call("act_type", json!({"text": text})).await?;
    let response: ActTypeWireResponse = structured(&happy)?;
    println!(
        "source_of_truth=mcp_act_type edge=happy after=ok:{} chars_typed:{} elapsed_ms:{} expected_ikis:{expected_ikis:?}",
        response.ok, response.chars_typed, response.elapsed_ms
    );
    assert!(response.ok);
    assert_eq!(response.chars_typed, 12);
    Ok(expected_ikis)
}

async fn call_act_type_empty(client: &mut StdioMcpClient) -> anyhow::Result<()> {
    println!("source_of_truth=mcp_act_type edge=empty before=text:\"\"");
    let empty = client.tools_call("act_type", json!({"text": ""})).await?;
    let response: ActTypeWireResponse = structured(&empty)?;
    println!(
        "source_of_truth=mcp_act_type edge=empty after=ok:{} chars_typed:{} elapsed_ms:{}",
        response.ok, response.chars_typed, response.elapsed_ms
    );
    assert!(response.ok);
    assert_eq!(response.chars_typed, 0);
    Ok(())
}

async fn call_act_type_error_edges(client: &mut StdioMcpClient) -> anyhow::Result<()> {
    assert_error_code(
        client,
        "extra_property",
        "junk:true",
        json!({"text": "x", "junk": true}),
        error_codes::TOOL_PARAMS_INVALID,
    )
    .await?;
    assert_error_code(
        client,
        "scancodes",
        "use_scancodes:true",
        json!({"text": "x", "use_scancodes": true}),
        error_codes::ACTION_BACKEND_UNAVAILABLE,
    )
    .await?;
    assert_error_code(
        client,
        "into_element",
        "element_id:0x1:2a",
        json!({"text": "x", "into_element": "0x1:2a"}),
        error_codes::ACTION_BACKEND_UNAVAILABLE,
    )
    .await
}

async fn assert_error_code(
    client: &mut StdioMcpClient,
    edge: &str,
    before: &str,
    args: Value,
    expected_code: &'static str,
) -> anyhow::Result<()> {
    println!("source_of_truth=mcp_act_type edge={edge} before={before}");
    let error = client.tools_call_error("act_type", args).await?;
    println!("source_of_truth=mcp_act_type edge={edge} after={error}");
    assert_eq!(error_code(&error), Some(expected_code));
    Ok(())
}

fn assert_recording_log_readbacks(logs: &str, expected_ikis: &[u32]) -> anyhow::Result<()> {
    let readbacks = recording_readbacks(logs)?;
    let happy_readback = readbacks
        .iter()
        .find(|readback| readback.recorded_ikis == format!("{expected_ikis:?}"))
        .context("happy-path recording readback missing expected Natural::FAST IKIs")?;
    let empty_readback = readbacks
        .iter()
        .find(|readback| readback.recorded_ikis == "[]" && readback.new_event_count == 0)
        .context("empty-text recording readback missing zero-event proof")?;
    println!(
        "source_of_truth=recording_log tool=act_type edge=happy after_recorded_ikis={} new_event_count={}",
        happy_readback.recorded_ikis, happy_readback.new_event_count
    );
    println!(
        "source_of_truth=recording_log tool=act_type edge=empty after_recorded_ikis={} new_event_count={}",
        empty_readback.recorded_ikis, empty_readback.new_event_count
    );
    Ok(())
}

#[derive(serde::Deserialize)]
struct ActTypeWireResponse {
    ok: bool,
    chars_typed: u32,
    elapsed_ms: u32,
}

#[derive(Debug)]
struct RecordingReadback {
    recorded_ikis: String,
    new_event_count: u64,
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

fn schema_root(value: Option<&Value>) -> Value {
    let Some(value) = value else {
        return Value::Null;
    };
    json!({
        "title": value.get("title"),
        "type": value.get("type"),
        "required": value.get("required"),
        "additionalProperties": value.get("additionalProperties"),
    })
}

fn natural_fast_ikis(text: &str) -> Vec<u32> {
    sample_typing_schedule(
        text,
        &KeystrokeDynamics::Natural {
            params: KeystrokeNaturalParams::FAST,
        },
        None,
    )
    .into_iter()
    .filter_map(|event| (event.iki_ms_before > 0).then_some(event.iki_ms_before))
    .collect()
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

fn recording_readbacks(logs: &str) -> anyhow::Result<Vec<RecordingReadback>> {
    let mut readbacks = Vec::new();
    for line in logs.lines().filter(|line| !line.trim().is_empty()) {
        let value: Value = serde_json::from_str(line)?;
        let fields = &value["fields"];
        if fields.get("code").and_then(Value::as_str) != Some("M2_ACT_TYPE_RECORDING_READBACK") {
            continue;
        }
        let recorded_ikis = fields
            .get("recorded_ikis")
            .and_then(Value::as_str)
            .context("recording readback missing recorded_ikis")?
            .to_owned();
        let new_event_count = fields
            .get("new_event_count")
            .and_then(Value::as_u64)
            .context("recording readback missing new_event_count")?;
        readbacks.push(RecordingReadback {
            recorded_ikis,
            new_event_count,
        });
    }
    Ok(readbacks)
}
