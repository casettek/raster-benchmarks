use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use eyre::{eyre, Context, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tar::{Archive, Builder};

const FIXTURE_PATH: &str = "runs/fixtures/l2-poc-synth-fixture.json";
const ROLLUP_CONFIG_PATH: &str = "fixtures/l2-poc/rollup-config-v1.json";
const WITNESS_BUNDLE_PATH: &str = "fixtures/l2-poc/synthetic-witness-bundle-v1.json";
const WITNESS_MANIFEST_PATH: &str = "fixtures/l2-poc/synthetic-witness-closure-manifest-v1.json";

pub fn build_canonical_input_package() -> Result<Vec<u8>> {
    let root = repo_root();
    let mut builder = Builder::new(Vec::new());

    let compact_kv_refs = compact_witness_kv_refs(&root)?;
    let compact_bundle = build_compact_witness_bundle(&root, &compact_kv_refs)?;
    let compact_manifest =
        build_compact_witness_manifest(&root, &compact_kv_refs, &compact_bundle)?;

    for relative in [FIXTURE_PATH, ROLLUP_CONFIG_PATH] {
        append_path(&mut builder, &root, relative)?;
    }

    append_bytes(&mut builder, WITNESS_BUNDLE_PATH, &compact_bundle)?;
    append_bytes(&mut builder, WITNESS_MANIFEST_PATH, &compact_manifest)?;

    for relative in compact_kv_refs {
        append_path(&mut builder, &root, &relative)?;
    }

    builder
        .finish()
        .wrap_err("failed to finalize input package tarball")?;
    builder
        .into_inner()
        .wrap_err("failed to extract input package tar bytes")
}

pub fn materialize_input_package(package_bytes: &[u8], destination_root: &Path) -> Result<()> {
    fs::create_dir_all(destination_root).wrap_err_with(|| {
        format!(
            "failed to create input package destination {}",
            destination_root.display()
        )
    })?;

    let cursor = Cursor::new(package_bytes);
    let mut archive = Archive::new(cursor);
    archive
        .unpack(destination_root)
        .wrap_err("failed to extract input package tarball")
}

pub fn canonical_fixture_json_from_root(root: &Path) -> Result<String> {
    let path = root.join(FIXTURE_PATH);
    fs::read_to_string(&path)
        .wrap_err_with(|| format!("failed to read materialized fixture {}", path.display()))
}

pub fn canonical_fixture_ref_root(root: &Path) -> PathBuf {
    root.to_path_buf()
}

pub fn canonical_fixture_relative_path() -> &'static str {
    FIXTURE_PATH
}

fn append_path(builder: &mut Builder<Vec<u8>>, root: &Path, relative: &str) -> Result<()> {
    let absolute = root.join(relative);
    if !absolute.exists() {
        return Err(eyre!("missing input package entry {}", absolute.display()));
    }

    if absolute.is_dir() {
        builder
            .append_dir_all(relative, &absolute)
            .wrap_err_with(|| format!("failed to append directory {}", absolute.display()))?;
    } else {
        builder
            .append_path_with_name(&absolute, relative)
            .wrap_err_with(|| format!("failed to append file {}", absolute.display()))?;
    }

    Ok(())
}

fn append_bytes(builder: &mut Builder<Vec<u8>>, relative: &str, contents: &[u8]) -> Result<()> {
    let mut header = tar::Header::new_gnu();
    header.set_size(u64::try_from(contents.len()).wrap_err("content length overflow")?);
    header.set_mode(0o644);
    header.set_cksum();
    builder
        .append_data(&mut header, relative, Cursor::new(contents))
        .wrap_err_with(|| format!("failed to append generated file {relative}"))
}

fn compact_witness_kv_refs(root: &Path) -> Result<Vec<String>> {
    let bundle = load_json(root.join(WITNESS_BUNDLE_PATH))?;
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

    let compact_ref = refs
        .last()
        .cloned()
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
