use anyhow::{Context, ensure};
use serde_json::{Value, json};
use synapse_core::error_codes;
use synapse_test_utils::stdio_mcp_client::StdioMcpClient;
use tempfile::TempDir;

#[tokio::test]
async fn act_drag_advertises_velocity_profile_and_handles_legacy_curve_alias() -> anyhow::Result<()>
{
    let log_dir = TempDir::new()?;
    let db = TempDir::new()?;
    let db_path = db.path().join("db");
    let db_path_string = db_path.to_string_lossy().to_string();
    let mut client = StdioMcpClient::launch_and_init_with_env(
        Some(log_dir.path()),
        &[
            ("SYNAPSE_DB", db_path_string.as_str()),
            ("SYNAPSE_MCP_SYNTHETIC_FIXTURE", "notepad"),
            ("SYNAPSE_MCP_RECORDING_BACKEND", "1"),
        ],
    )
    .await?;
    client
        .tools_call("profile_activate", json!({"profile_id": "notepad"}))
        .await?;

    let tools = client.tools_list().await?;
    let drag = tool_by_name(&tools, "act_drag")?;
    let description = drag
        .get("description")
        .and_then(Value::as_str)
        .context("act_drag description missing")?;
    ensure!(
        description.contains("velocity_profile controls timing only")
            && description.contains("act_stroke.path"),
        "act_drag description must separate timing from spatial path: {description}"
    );
    ensure!(
        value_at(drag, "inputSchema.properties.velocity_profile").is_ok(),
        "act_drag schema must advertise velocity_profile"
    );
    ensure!(
        value_at(drag, "inputSchema.properties.curve").is_err(),
        "act_drag schema must not advertise legacy curve"
    );

    let modern = structured(
        &client
            .tools_call(
                "act_drag",
                json!({
                    "from": {"x": 10, "y": 20},
                    "to": {"x": 70, "y": 80},
                    "velocity_profile": "linear",
                    "duration_ms": 80,
                    "backend": "software"
                }),
            )
            .await?,
    )?;
    println!("readback=mcp_act_drag edge=velocity_profile after={modern}");
    assert_eq!(modern["velocity_profile_used"], "linear");
    assert_eq!(modern["deprecated_curve_alias_used"], false);

    let legacy = structured(
        &client
            .tools_call(
                "act_drag",
                json!({
                    "from": {"x": 10, "y": 20},
                    "to": {"x": 70, "y": 80},
                    "curve": "ease_in_out",
                    "duration_ms": 80,
                    "backend": "software"
                }),
            )
            .await?,
    )?;
    println!("readback=mcp_act_drag edge=legacy_curve_alias after={legacy}");
    assert_eq!(legacy["velocity_profile_used"], "ease_in_out");
    assert_eq!(legacy["deprecated_curve_alias_used"], true);
    assert!(
        legacy["deprecation"]
            .as_str()
            .is_some_and(|message| message.contains("use velocity_profile"))
    );

    let bezier = client
        .tools_call_error(
            "act_drag",
            json!({
                "from": {"x": 10, "y": 20},
                "to": {"x": 70, "y": 80},
                "curve": "bezier",
                "duration_ms": 80,
                "backend": "software"
            }),
        )
        .await?;
    println!("readback=mcp_act_drag edge=legacy_curve_bezier after_error={bezier}");
    assert_eq!(error_code(&bezier), Some(error_codes::TOOL_PARAMS_INVALID));
    assert!(
        bezier
            .get("message")
            .and_then(Value::as_str)
            .is_some_and(|message| message.contains("act_stroke with path.kind=cubic_bezier"))
    );

    assert!(client.shutdown().await?.success());
    Ok(())
}

fn structured(resp: &Value) -> anyhow::Result<Value> {
    resp.get("structuredContent")
        .cloned()
        .context("structuredContent missing")
}

fn tool_by_name<'a>(tools_response: &'a Value, name: &str) -> anyhow::Result<&'a Value> {
    tools_response
        .get("tools")
        .and_then(Value::as_array)
        .context("tools array missing")?
        .iter()
        .find(|tool| tool.get("name").and_then(Value::as_str) == Some(name))
        .with_context(|| format!("tool missing from tools/list: {name}"))
}

fn value_at<'a>(value: &'a Value, path: &str) -> anyhow::Result<&'a Value> {
    let mut current = value;
    for segment in path.split('.') {
        current = current
            .get(segment)
            .with_context(|| format!("missing path {path}"))?;
    }
    Ok(current)
}

fn error_code(error: &Value) -> Option<&str> {
    error
        .get("data")
        .and_then(|data| data.get("code"))
        .and_then(Value::as_str)
}
