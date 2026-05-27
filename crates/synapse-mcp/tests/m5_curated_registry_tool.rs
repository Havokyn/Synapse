use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::Context;
use serde_json::{Value, json};
use synapse_storage::cf;
use synapse_test_utils::stdio_mcp_client::StdioMcpClient;
use tempfile::TempDir;

#[tokio::test]
async fn curated_package_install_writes_seed_target_row() -> anyhow::Result<()> {
    let logs = TempDir::new()?;
    let db = TempDir::new()?;
    let db_path = db.path().join("db");
    let db_path_string = db_path.to_string_lossy().to_string();
    let mut client = StdioMcpClient::launch_and_init_with_env(
        Some(logs.path()),
        &[("SYNAPSE_DB", db_path_string.as_str())],
    )
    .await?;

    let before = structured(&client.tools_call("storage_inspect", json!({})).await?)?;
    assert_eq!(before["cf_row_counts"][cf::CF_PROFILES], 0);

    let manifests = TempDir::new()?;
    let manifest_path = install_curated_manifest(&mut client, manifests.path()).await?;
    assert_curated_search_and_inspect(&mut client).await?;
    assert_duplicate_is_idempotent(&mut client, &manifest_path).await?;
    assert_edge_manifests_fail_closed(&mut client, manifests.path()).await?;

    let after = structured(&client.tools_call("storage_inspect", json!({})).await?)?;
    assert_eq!(after["cf_row_counts"][cf::CF_PROFILES], 6);
    assert_eq!(after["cf_row_counts"][cf::CF_KV], 1);

    let status = client.shutdown().await?;
    assert!(status.success());
    Ok(())
}

async fn install_curated_manifest(
    client: &mut StdioMcpClient,
    manifest_dir: &Path,
) -> anyhow::Result<PathBuf> {
    let manifest_path = prepare_manifest(
        "docs/computergames/fixtures/curated_starter_registry/curated_luanti_package_manifest.toml",
        manifest_dir,
        "curated.toml",
    )?;
    let install = structured(
        &client
            .tools_call(
                "profile_registry_install",
                json!({"manifest_path": manifest_path.display().to_string()}),
            )
            .await?,
    )?;
    assert_eq!(install["wrote_rows"], true);
    assert_eq!(install["idempotent"], false);
    let row_keys = install["cf_profile_row_keys"]
        .as_array()
        .context("cf_profile_row_keys missing")?;
    assert!(row_keys.iter().any(|key| {
        key.as_str() == Some("profile_registry/v1/curated_target/starter.v1/luanti.minetest")
    }));
    Ok(manifest_path)
}

async fn assert_curated_search_and_inspect(client: &mut StdioMcpClient) -> anyhow::Result<()> {
    let search = structured(
        &client
            .tools_call(
                "profile_registry_search",
                json!({"row_kind": "curated_profile_target"}),
            )
            .await?,
    )?;
    assert_eq!(search["total_matched"], 1);
    assert_eq!(
        search["rows"][0]["key"],
        "profile_registry/v1/curated_target/starter.v1/luanti.minetest"
    );
    let inspect = structured(
        &client
            .tools_call(
                "profile_registry_inspect",
                json!({"row_key": "profile_registry/v1/curated_target/starter.v1/luanti.minetest"}),
            )
            .await?,
    )?;
    let value = &inspect["row"]["value"];
    assert_eq!(value["row_kind"], "curated_profile_target");
    assert_eq!(value["seed_set_id"], "starter.v1");
    assert_eq!(value["target_id"], "luanti.minetest");
    assert_eq!(value["profile_id"], "luanti.minetest");
    assert_eq!(value["backlog_issue"], "#471,#472,#473,#474,#475,#476");
    let fsv = value["minimum_manual_fsv"]
        .as_array()
        .context("minimum_manual_fsv array missing")?;
    assert_eq!(fsv.len(), 8);
    Ok(())
}

async fn assert_duplicate_is_idempotent(
    client: &mut StdioMcpClient,
    manifest_path: &Path,
) -> anyhow::Result<()> {
    let duplicate = structured(
        &client
            .tools_call(
                "profile_registry_install",
                json!({"manifest_path": manifest_path.display().to_string()}),
            )
            .await?,
    )?;
    assert_eq!(duplicate["idempotent"], true);
    assert_eq!(duplicate["wrote_rows"], false);
    Ok(())
}

async fn assert_edge_manifests_fail_closed(
    client: &mut StdioMcpClient,
    manifest_dir: &Path,
) -> anyhow::Result<()> {
    let unknown_scope_path = prepare_manifest(
        "docs/computergames/fixtures/curated_starter_registry/edge_unknown_use_scope_manifest.toml",
        manifest_dir,
        "unknown-scope.toml",
    )?;
    let unknown_scope = client
        .tools_call_error(
            "profile_registry_install",
            json!({"manifest_path": unknown_scope_path.display().to_string()}),
        )
        .await?;
    assert_eq!(unknown_scope["data"]["code"], "PROFILE_PARSE_ERROR");
    let missing_compat_path = prepare_manifest(
        "docs/computergames/fixtures/curated_starter_registry/edge_missing_compatibility_manifest.toml",
        manifest_dir,
        "missing-compat.toml",
    )?;
    let missing_compat = client
        .tools_call_error(
            "profile_registry_install",
            json!({"manifest_path": missing_compat_path.display().to_string()}),
        )
        .await?;
    assert_eq!(missing_compat["data"]["code"], "PROFILE_PARSE_ERROR");
    Ok(())
}

fn prepare_manifest(
    fixture_relative_path: &str,
    output_dir: &Path,
    output_name: &str,
) -> anyhow::Result<PathBuf> {
    let root = repo_root()?;
    let profile_toml = root
        .join("crates/synapse-profiles/profiles/luanti.minetest.toml")
        .canonicalize()
        .context("canonicalize Luanti profile TOML")?;
    let source = fs::read_to_string(root.join(fixture_relative_path))
        .with_context(|| format!("read fixture manifest {fixture_relative_path}"))?;
    let rewritten = source.replace(
        "profile_toml = \"crates/synapse-profiles/profiles/luanti.minetest.toml\"",
        &format!("profile_toml = \"{}\"", toml_path(&profile_toml)),
    );
    let path = output_dir.join(output_name);
    fs::write(&path, rewritten)?;
    Ok(path)
}

fn repo_root() -> anyhow::Result<PathBuf> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .context("canonicalize repo root")
}

fn toml_path(path: &Path) -> String {
    path.display().to_string().replace('\\', "\\\\")
}

fn structured(response: &Value) -> anyhow::Result<Value> {
    let content = response
        .get("content")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .context("content[0] missing")?;
    let text = content
        .get("text")
        .and_then(Value::as_str)
        .context("content[0].text missing")?;
    serde_json::from_str(text).context("parse tool response json")
}
