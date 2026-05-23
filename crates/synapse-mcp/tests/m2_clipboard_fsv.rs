use anyhow::Context;
#[cfg(windows)]
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use synapse_core::error_codes;
use synapse_test_utils::stdio_mcp_client::StdioMcpClient;

#[tokio::test]
async fn act_clipboard_schema_platform_and_edges_fsv() -> anyhow::Result<()> {
    let mut client = StdioMcpClient::launch_and_init_with_env(None, &[]).await?;
    let resp = client.tools_list().await?;
    let tools = resp
        .get("tools")
        .and_then(Value::as_array)
        .context("tools array missing")?;
    assert_act_clipboard_schema(tools)?;

    #[cfg(windows)]
    call_act_clipboard_windows_happy_and_edges(&mut client).await?;
    #[cfg(not(windows))]
    call_act_clipboard_non_windows_edges(&mut client).await?;

    assert!(client.shutdown().await?.success());
    Ok(())
}

fn assert_act_clipboard_schema(tools: &[Value]) -> anyhow::Result<()> {
    let act_clipboard = tools
        .iter()
        .find(|tool| tool.get("name") == Some(&Value::String("act_clipboard".to_owned())))
        .context("act_clipboard tool missing")?;
    let schema = &act_clipboard["inputSchema"];
    println!(
        "source_of_truth=tools_list tool=act_clipboard edge=schema before=tool_count:{}",
        tools.len()
    );
    println!(
        "source_of_truth=tools_list tool=act_clipboard edge=defaults after=format:{} additionalProperties:{} required:{}",
        schema["properties"]["format"]["default"],
        schema["additionalProperties"],
        schema["required"]
    );
    assert_eq!(tools.len(), 15);
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(schema["properties"]["format"]["default"], "unicode");
    assert_eq!(schema["required"], json!(["verb"]));
    let schema_text = schema.to_string();
    for value in ["read", "write", "clear", "text", "unicode"] {
        assert!(schema_text.contains(value));
    }

    let projection = json!({
        "name": act_clipboard["name"],
        "description": act_clipboard["description"],
        "inputSchema": act_clipboard["inputSchema"],
        "outputSchemaRoot": schema_root(act_clipboard.get("outputSchema")),
    });
    insta::assert_json_snapshot!("m2_act_clipboard_tool", projection);
    Ok(())
}

#[cfg(not(windows))]
async fn call_act_clipboard_non_windows_edges(client: &mut StdioMcpClient) -> anyhow::Result<()> {
    assert_error_code(
        client,
        "linux_read_unavailable",
        "verb:read format:unicode",
        json!({"verb": "read"}),
        error_codes::ACTION_BACKEND_UNAVAILABLE,
    )
    .await?;
    assert_error_code(
        client,
        "linux_write_unavailable",
        "verb:write text:synapse-m2",
        json!({"verb": "write", "text": "synapse-m2"}),
        error_codes::ACTION_BACKEND_UNAVAILABLE,
    )
    .await?;
    assert_error_code(
        client,
        "linux_clear_unavailable",
        "verb:clear",
        json!({"verb": "clear"}),
        error_codes::ACTION_BACKEND_UNAVAILABLE,
    )
    .await?;
    assert_error_code(
        client,
        "write_missing_text",
        "verb:write text:<missing>",
        json!({"verb": "write"}),
        error_codes::TOOL_PARAMS_INVALID,
    )
    .await?;
    assert_error_code(
        client,
        "read_rejects_text",
        "verb:read text:ignored",
        json!({"verb": "read", "text": "ignored"}),
        error_codes::TOOL_PARAMS_INVALID,
    )
    .await?;
    assert_error_code(
        client,
        "text_format_non_ascii",
        "verb:write format:text text:lambda",
        json!({"verb": "write", "format": "text", "text": "lambda-λ"}),
        error_codes::TOOL_PARAMS_INVALID,
    )
    .await?;
    assert_error_code(
        client,
        "extra_property",
        "junk:true",
        json!({"verb": "read", "junk": true}),
        error_codes::TOOL_PARAMS_INVALID,
    )
    .await?;
    assert_error_code(
        client,
        "invalid_verb",
        "verb:paste",
        json!({"verb": "paste"}),
        error_codes::TOOL_PARAMS_INVALID,
    )
    .await
}

#[cfg(windows)]
async fn call_act_clipboard_windows_happy_and_edges(
    client: &mut StdioMcpClient,
) -> anyhow::Result<()> {
    println!("source_of_truth=windows_clipboard edge=clear_before before=unknown");
    let clear = client
        .tools_call("act_clipboard", json!({"verb": "clear"}))
        .await?;
    let clear_response: ActClipboardWireResponse = structured(&clear)?;
    let direct_after_clear =
        synapse_action::read_clipboard_text(synapse_action::ClipboardFormat::Unicode)?;
    println!(
        "source_of_truth=windows_clipboard edge=clear_before after=ok:{} cleared:{} direct_text:{direct_after_clear:?}",
        clear_response.ok, clear_response.cleared
    );
    assert!(clear_response.ok);
    assert!(clear_response.cleared);
    assert!(direct_after_clear.is_empty());

    println!("source_of_truth=windows_clipboard edge=write before=text:synapse-m2");
    let write = client
        .tools_call(
            "act_clipboard",
            json!({"verb": "write", "text": "synapse-m2"}),
        )
        .await?;
    let write_response: ActClipboardWireResponse = structured(&write)?;
    let direct_after_write =
        synapse_action::read_clipboard_text(synapse_action::ClipboardFormat::Unicode)?;
    println!(
        "source_of_truth=windows_clipboard edge=write after=ok:{} written:{} text_len:{:?} direct_text:{direct_after_write:?}",
        write_response.ok, write_response.written, write_response.text_len
    );
    assert!(write_response.ok);
    assert!(write_response.written);
    assert_eq!(write_response.text_len, Some("synapse-m2".len()));
    assert_eq!(direct_after_write, "synapse-m2");

    println!("source_of_truth=windows_clipboard edge=read before=direct_text:synapse-m2");
    let read = client
        .tools_call("act_clipboard", json!({"verb": "read"}))
        .await?;
    let read_response: ActClipboardWireResponse = structured(&read)?;
    println!(
        "source_of_truth=windows_clipboard edge=read after=ok:{} text:{:?} text_len:{:?}",
        read_response.ok, read_response.text, read_response.text_len
    );
    assert!(read_response.ok);
    assert_eq!(read_response.text.as_deref(), Some("synapse-m2"));
    assert_eq!(read_response.text_len, Some("synapse-m2".len()));

    println!("source_of_truth=windows_clipboard edge=clear_after before=direct_text:synapse-m2");
    let clear = client
        .tools_call("act_clipboard", json!({"verb": "clear"}))
        .await?;
    let clear_response: ActClipboardWireResponse = structured(&clear)?;
    let direct_after_clear =
        synapse_action::read_clipboard_text(synapse_action::ClipboardFormat::Unicode)?;
    println!(
        "source_of_truth=windows_clipboard edge=clear_after after=ok:{} cleared:{} direct_text:{direct_after_clear:?}",
        clear_response.ok, clear_response.cleared
    );
    assert!(clear_response.ok);
    assert!(clear_response.cleared);
    assert!(direct_after_clear.is_empty());

    assert_error_code(
        client,
        "write_missing_text",
        "verb:write text:<missing>",
        json!({"verb": "write"}),
        error_codes::TOOL_PARAMS_INVALID,
    )
    .await?;
    assert_error_code(
        client,
        "read_rejects_text",
        "verb:read text:ignored",
        json!({"verb": "read", "text": "ignored"}),
        error_codes::TOOL_PARAMS_INVALID,
    )
    .await?;
    assert_error_code(
        client,
        "text_format_non_ascii",
        "verb:write format:text text:lambda",
        json!({"verb": "write", "format": "text", "text": "lambda-λ"}),
        error_codes::TOOL_PARAMS_INVALID,
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
    println!("source_of_truth=mcp_act_clipboard edge={edge} before={before}");
    let error = client.tools_call_error("act_clipboard", args).await?;
    println!("source_of_truth=mcp_act_clipboard edge={edge} after={error}");
    assert_eq!(error_code(&error), Some(expected_code));
    Ok(())
}

#[cfg(windows)]
#[derive(serde::Deserialize)]
struct ActClipboardWireResponse {
    ok: bool,
    written: bool,
    cleared: bool,
    text: Option<String>,
    text_len: Option<usize>,
}

#[cfg(windows)]
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
