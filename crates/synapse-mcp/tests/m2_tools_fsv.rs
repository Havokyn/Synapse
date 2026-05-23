use anyhow::Context;
use serde_json::{Value, json};
use synapse_test_utils::stdio_mcp_client::StdioMcpClient;

const EXPECTED_M2_TOOL_NAMES: &[&str] = &[
    "act_aim",
    "act_click",
    "act_clipboard",
    "act_drag",
    "act_pad",
    "act_press",
    "act_scroll",
    "act_type",
    "find",
    "health",
    "observe",
    "read_text",
    "release_all",
    "set_capture_target",
    "set_perception_mode",
];

#[tokio::test]
async fn m2_tools_list_contains_exact_sorted_surface_fsv() -> anyhow::Result<()> {
    let mut client = StdioMcpClient::launch_and_init().await?;
    let resp = client.tools_list().await?;
    let tools = resp
        .get("tools")
        .and_then(Value::as_array)
        .context("tools array missing")?;

    let mut names = tools
        .iter()
        .map(|tool| {
            tool.get("name")
                .and_then(Value::as_str)
                .context("tool name missing")
                .map(str::to_owned)
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    names.sort();
    println!("source_of_truth=tools_list edge=m2 final_names={names:?}");
    assert_eq!(names, EXPECTED_M2_TOOL_NAMES);

    let mut projection = tools
        .iter()
        .map(|tool| {
            let name = tool
                .get("name")
                .and_then(Value::as_str)
                .context("tool name missing")?;
            Ok((
                name.to_owned(),
                json!({
                    "name": tool["name"],
                    "description": tool["description"],
                    "inputSchema": tool["inputSchema"],
                    "outputSchema": tool.get("outputSchema").unwrap_or(&Value::Null),
                }),
            ))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    projection.sort_by(|left, right| left.0.cmp(&right.0));
    let schemas = projection
        .into_iter()
        .map(|(_name, schema)| schema)
        .collect::<Vec<_>>();
    insta::assert_json_snapshot!("m2_tools_list", schemas);

    assert!(client.shutdown().await?.success());
    Ok(())
}
