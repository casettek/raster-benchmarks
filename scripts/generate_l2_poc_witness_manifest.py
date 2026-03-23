#!/usr/bin/env python3

import argparse
import hashlib
import json
from pathlib import Path


EXCLUDED_DB_BASENAMES = {"LOCK", "LOG"}


def sha256_hex(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def load_json(path: Path):
    return json.loads(path.read_text())


def normalized_relpath(path: Path, root: Path) -> str:
    return path.relative_to(root).as_posix()


def should_hash_db_file(path: Path) -> bool:
    name = path.name
    if name in EXCLUDED_DB_BASENAMES:
        return False
    if name.startswith("LOG.old"):
        return False
    if name.endswith(".log"):
        return False
    return True


def hash_db_tree(db_dir: Path, root: Path):
    files = []
    for file_path in sorted(db_dir.rglob("*")):
        if not file_path.is_file() or not should_hash_db_file(file_path):
            continue
        digest = sha256_hex(file_path.read_bytes())
        files.append(
            {
                "path": normalized_relpath(file_path, root),
                "sha256": digest,
                "size_bytes": file_path.stat().st_size,
            }
        )

    payload = json.dumps(files, sort_keys=True, separators=(",", ":")).encode("utf-8")
    return {
        "db_ref": normalized_relpath(db_dir, root),
        "file_count": len(files),
        "sha256": sha256_hex(payload),
        "files": files,
    }


def build_manifest(repo_root: Path, fixture_path: Path, bundle_path: Path):
    fixture = load_json(fixture_path)
    bundle = load_json(bundle_path)

    kv_refs = bundle.get("kv_store_refs") or [bundle["kv_store_ref"]]
    kv_refs = sorted(kv_refs)

    db_digests = []
    for ref in kv_refs:
        db_path = repo_root / ref
        db_digests.append(hash_db_tree(db_path, repo_root))

    identity = {
        "fixture_id": fixture["fixture_id"],
        "batch_hash": fixture["batch_hash"],
        "tx_hashes": [tx["hash"] for tx in fixture["transactions"]],
        "supplemental_tx_hashes": [
            tx["hash"] for tx in fixture.get("supplemental_transactions", [])
        ],
        "start_block": fixture["start_block"],
        "end_block": fixture["end_block"],
        "parent_header_hash": fixture["pre_checkpoint"]["parent_header_hash"],
        "parent_block_number": fixture["pre_checkpoint"]["parent_block_number"],
        "message_passer_storage_root": fixture["output_root_witness"][
            "message_passer_storage_root"
        ],
    }

    manifest = {
        "manifest_id": "l2-poc-witness-closure-v1",
        "fixture_ref": normalized_relpath(fixture_path, repo_root),
        "witness_bundle_ref": normalized_relpath(bundle_path, repo_root),
        "identity": identity,
        "rollup_config_ref": fixture["pre_checkpoint"]["rollup_config_ref"],
        "kv_store_refs": kv_refs,
        "fixture_sha256": sha256_hex(fixture_path.read_bytes()),
        "witness_bundle_sha256": sha256_hex(bundle_path.read_bytes()),
        "kv_store_digests": db_digests,
    }

    canonical = json.dumps(manifest, sort_keys=True, separators=(",", ":")).encode("utf-8")
    manifest["manifest_sha256"] = sha256_hex(canonical)
    return manifest


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--fixture",
        default="runs/fixtures/l2-poc-synth-fixture.json",
    )
    parser.add_argument(
        "--bundle",
        default="fixtures/l2-poc/synthetic-witness-bundle-v1.json",
    )
    parser.add_argument(
        "--output",
        default="fixtures/l2-poc/synthetic-witness-closure-manifest-v1.json",
    )
    args = parser.parse_args()

    repo_root = Path(__file__).resolve().parents[1]
    fixture_path = repo_root / args.fixture
    bundle_path = repo_root / args.bundle
    output_path = repo_root / args.output

    manifest = build_manifest(repo_root, fixture_path, bundle_path)
    output_path.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
    print(output_path.relative_to(repo_root).as_posix())


if __name__ == "__main__":
    main()
