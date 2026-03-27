use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use base64::Engine;
use eyre::{eyre, Context, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const FIXTURE_PATH: &str = "runs/fixtures/l2-poc-synth-fixture.json";
const ROLLUP_CONFIG_PATH: &str = "fixtures/l2-poc/rollup-config-v1.json";
const WITNESS_BUNDLE_PATH: &str = "fixtures/l2-poc/synthetic-witness-bundle-v1.json";
const WITNESS_MANIFEST_PATH: &str = "fixtures/l2-poc/synthetic-witness-closure-manifest-v1.json";

pub fn build_canonical_input_package() -> Result<Vec<u8>> {
    let root = repo_root();
    let compact_kv_refs = compact_witness_kv_refs(&root)?;
    let compact_bundle = build_compact_witness_bundle(&root, &compact_kv_refs)?;
    let compact_manifest =
        build_compact_witness_manifest(&root, &compact_kv_refs, &compact_bundle)?;

    let mut fixture = load_json(root.join(FIXTURE_PATH))?;
    let mut inline_assets = BTreeMap::<String, Value>::new();
    inline_assets.insert(
        ROLLUP_CONFIG_PATH.to_string(),
        inline_asset_value(&fs::read(root.join(ROLLUP_CONFIG_PATH))?),
    );
    inline_assets.insert(
        WITNESS_BUNDLE_PATH.to_string(),
        inline_asset_value(&compact_bundle),
    );
    inline_assets.insert(
        WITNESS_MANIFEST_PATH.to_string(),
        inline_asset_value(&compact_manifest),
    );

    for kv_ref in compact_kv_refs {
        append_directory_assets(&root, &kv_ref, &mut inline_assets)?;
    }

    fixture["inline_assets"] = json!({
        "encoding": "base64",
        "files": inline_assets,
    });

    serde_json::to_vec_pretty(&fixture).wrap_err("failed to encode canonical input package json")
}

fn append_directory_assets(
    root: &Path,
    relative_dir: &str,
    inline_assets: &mut BTreeMap<String, Value>,
) -> Result<()> {
    let absolute_dir = root.join(relative_dir);
    if !absolute_dir.is_dir() {
        return Err(eyre!(
            "expected witness asset directory {}, but it was missing",
            absolute_dir.display()
        ));
    }

    let mut files = fs::read_dir(&absolute_dir)
        .wrap_err_with(|| {
            format!(
                "failed to read witness asset dir {}",
                absolute_dir.display()
            )
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    files.sort_by_key(|entry| entry.file_name());

    for entry in files {
        let path = entry.path();
        if path.is_file() {
            let relative = path
                .strip_prefix(root)
                .wrap_err("failed to relativize witness asset path")?
                .to_string_lossy()
                .replace('\\', "/");
            inline_assets.insert(relative, inline_asset_value(&fs::read(&path)?));
        }
    }

    Ok(())
}

fn inline_asset_value(bytes: &[u8]) -> Value {
    json!({
        "bytes_b64": base64::engine::general_purpose::STANDARD.encode(bytes),
    })
}

fn compact_witness_kv_refs(root: &Path) -> Result<Vec<String>> {
    let bundle = load_json(root.join(WITNESS_BUNDLE_PATH))?;
    let fixture = load_json(root.join(FIXTURE_PATH))?;
    let refs = bundle
        .get("kv_store_refs")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .filter(|items| !items.is_empty())
        .or_else(|| {
            bundle
                .get("kv_store_ref")
                .and_then(Value::as_str)
                .map(|value| vec![value.to_string()])
        })
        .ok_or_else(|| eyre!("witness bundle missing kv store references"))?;

    let start_block = fixture
        .get("start_block")
        .and_then(Value::as_u64)
        .ok_or_else(|| eyre!("fixture missing start_block"))?;
    let preferred_suffix = format!("block-{start_block}");
    let compact_ref = refs
        .iter()
        .find(|value| value.ends_with(&preferred_suffix))
        .cloned()
        .or_else(|| refs.last().cloned())
        .ok_or_else(|| eyre!("witness bundle did not include any kv refs"))?;

    Ok(vec![compact_ref])
}

fn build_compact_witness_bundle(root: &Path, compact_kv_refs: &[String]) -> Result<Vec<u8>> {
    let mut bundle = load_json(root.join(WITNESS_BUNDLE_PATH))?;
    let compact_ref = compact_kv_refs
        .first()
        .cloned()
        .ok_or_else(|| eyre!("compact witness bundle requires at least one kv ref"))?;
    bundle["kv_store_ref"] = Value::String(compact_ref);
    bundle["kv_store_refs"] = json!(compact_kv_refs);
    serde_json::to_vec_pretty(&bundle).wrap_err("failed to encode compact witness bundle")
}

fn build_compact_witness_manifest(
    root: &Path,
    compact_kv_refs: &[String],
    compact_bundle: &[u8],
) -> Result<Vec<u8>> {
    let mut manifest = load_json(root.join(WITNESS_MANIFEST_PATH))?;
    let filtered_digests = manifest
        .get("kv_store_digests")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|entry| {
            entry
                .get("db_ref")
                .and_then(Value::as_str)
                .is_some_and(|db_ref| compact_kv_refs.iter().any(|value| value == db_ref))
        })
        .collect::<Vec<_>>();

    manifest["kv_store_refs"] = json!(compact_kv_refs);
    manifest["kv_store_digests"] = Value::Array(filtered_digests);
    manifest["fixture_sha256"] = Value::String(file_sha256(root.join(FIXTURE_PATH))?);
    manifest["witness_bundle_sha256"] = Value::String(sha256_hex(compact_bundle));

    let manifest_without_self = serde_json::to_vec_pretty(&manifest)
        .wrap_err("failed to encode compact witness manifest")?;
    manifest["manifest_sha256"] = Value::String(sha256_hex(&manifest_without_self));
    serde_json::to_vec_pretty(&manifest).wrap_err("failed to finalize compact witness manifest")
}

fn load_json(path: PathBuf) -> Result<Value> {
    let raw = fs::read_to_string(&path)
        .wrap_err_with(|| format!("failed to read json file {}", path.display()))?;
    serde_json::from_str(&raw)
        .wrap_err_with(|| format!("failed to parse json file {}", path.display()))
}

fn file_sha256(path: PathBuf) -> Result<String> {
    let bytes = fs::read(&path)
        .wrap_err_with(|| format!("failed to read file for sha256 {}", path.display()))?;
    Ok(sha256_hex(&bytes))
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_input_package_is_json_with_inline_assets() {
        let bytes = build_canonical_input_package().expect("input package should build");
        let value: Value = serde_json::from_slice(&bytes).expect("package should be valid json");

        assert!(value.get("transactions").is_some());
        let inline_assets = value
            .get("inline_assets")
            .expect("package should include inline assets");
        assert_eq!(
            inline_assets.get("encoding").and_then(Value::as_str),
            Some("base64")
        );
        assert!(inline_assets
            .get("files")
            .and_then(Value::as_object)
            .is_some_and(|files| files.contains_key(ROLLUP_CONFIG_PATH)));
    }
}
