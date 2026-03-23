#!/usr/bin/env python3

import argparse
import copy
import json
import shutil
from pathlib import Path

from generate_l2_poc_witness_manifest import build_manifest


def load_json(path: Path):
    return json.loads(path.read_text())


def write_json(path: Path, payload):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n")


def copy_tree(src: Path, dst: Path, force: bool):
    if not src.exists():
        raise FileNotFoundError(f"missing source witness store: {src}")
    if dst.exists():
        if not force:
            raise FileExistsError(
                f"destination already exists: {dst} (pass --force to replace generated artifacts)"
            )
        shutil.rmtree(dst)
    shutil.copytree(src, dst)


def rewrite_transaction_ids(transactions, replacement_ids):
    if len(transactions) != len(replacement_ids):
        raise ValueError(
            "transaction id override count does not match transaction list length"
        )
    rewritten = copy.deepcopy(transactions)
    for tx, new_id in zip(rewritten, replacement_ids, strict=True):
        tx["id"] = new_id
    return rewritten


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--seed",
        default="fixtures/l2-poc/synthetic-fixture-seed-v1.json",
    )
    parser.add_argument(
        "--force",
        action="store_true",
        help="replace previously generated synthetic witness stores and artifacts",
    )
    args = parser.parse_args()

    repo_root = Path(__file__).resolve().parents[1]
    seed_path = repo_root / args.seed
    seed = load_json(seed_path)

    source_fixture_path = repo_root / seed["source_fixture_ref"]
    source_bundle_path = repo_root / seed["source_bundle_ref"]
    output_fixture_path = repo_root / seed["output_fixture_ref"]
    output_bundle_path = repo_root / seed["output_bundle_ref"]
    output_manifest_path = repo_root / seed["output_manifest_ref"]

    source_fixture = load_json(source_fixture_path)
    source_bundle = load_json(source_bundle_path)

    fixture = copy.deepcopy(source_fixture)
    fixture["fixture_id"] = seed["fixture_id"]
    fixture["description"] = seed["description"]
    fixture["pre_checkpoint"]["witness_bundle_ref"] = seed["output_bundle_ref"]
    fixture["transactions"] = rewrite_transaction_ids(
        fixture["transactions"], seed["tracked_transaction_ids"]
    )
    fixture["supplemental_transactions"] = rewrite_transaction_ids(
        fixture["supplemental_transactions"], seed["supplemental_transaction_ids"]
    )
    fixture["accounts"] = seed.get("accounts", {})
    fixture["generation"] = seed.get("package_metadata", {})

    bundle = copy.deepcopy(source_bundle)
    bundle["bundle_id"] = seed["bundle_id"]
    bundle["description"] = seed["bundle_description"]
    bundle["source"] = "generated locally from repo-owned synthetic seed metadata and vendored witness snapshots"
    bundle["closure_manifest_ref"] = seed["output_manifest_ref"]

    ref_map = {entry["from"]: entry["to"] for entry in seed["witness_store_mappings"]}
    bundle["kv_store_ref"] = ref_map[bundle["kv_store_ref"]]
    bundle["kv_store_refs"] = [ref_map[ref] for ref in bundle.get("kv_store_refs", [])]

    for entry in seed["witness_store_mappings"]:
        copy_tree(repo_root / entry["from"], repo_root / entry["to"], args.force)

    write_json(output_fixture_path, fixture)
    write_json(output_bundle_path, bundle)

    manifest = build_manifest(repo_root, output_fixture_path, output_bundle_path)
    write_json(output_manifest_path, manifest)

    print(output_fixture_path.relative_to(repo_root).as_posix())
    print(output_bundle_path.relative_to(repo_root).as_posix())
    print(output_manifest_path.relative_to(repo_root).as_posix())


if __name__ == "__main__":
    main()
