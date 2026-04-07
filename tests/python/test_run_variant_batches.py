#!/usr/bin/env python3
"""Tests for bench/corpus/run_variant_batches.py."""

import importlib.util
import json
from pathlib import Path

SCRIPT = Path(__file__).parents[2] / "bench" / "corpus" / "run_variant_batches.py"
SPEC = importlib.util.spec_from_file_location("run_variant_batches", SCRIPT)
assert SPEC and SPEC.loader
run_variant_batches = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(run_variant_batches)


def _write_manifest(path, repo_ids):
    """Write a minimal manifest.jsonl with the given repo IDs."""
    with open(path, "w") as f:
        for rid in repo_ids:
            f.write(json.dumps({"id": rid, "repo_url": f"https://example.com/{rid}", "sha": "abc"}) + "\n")


def test_repo_manifest_index_found(tmp_path):
    manifest = tmp_path / "manifest.jsonl"
    _write_manifest(manifest, ["repo_a", "repo_b", "repo_c"])
    assert run_variant_batches._repo_manifest_index("repo_a", manifest) == 0
    assert run_variant_batches._repo_manifest_index("repo_b", manifest) == 1
    assert run_variant_batches._repo_manifest_index("repo_c", manifest) == 2


def test_repo_manifest_index_not_found(tmp_path):
    manifest = tmp_path / "manifest.jsonl"
    _write_manifest(manifest, ["repo_a", "repo_b"])
    assert run_variant_batches._repo_manifest_index("repo_z", manifest) is None


def test_run_variant_batches_skips_repo_beyond_limit(tmp_path):
    """Repos beyond max_variant_repos are skipped."""
    manifest = tmp_path / "manifest.jsonl"
    _write_manifest(manifest, ["repo_0", "repo_1", "repo_2"])

    # repo_2 is at index 2, limit is 2 → should be skipped
    result = run_variant_batches.run_variant_batches(
        repo_dir=str(tmp_path),
        repo_id="repo_2",
        binary="/nonexistent",
        batches_dir=str(tmp_path / "batches"),
        results_dir=str(tmp_path / "results"),
        manifest=str(manifest),
        max_variant_repos=2,
    )
    assert result == []


def test_run_variant_batches_runs_repo_within_limit(tmp_path):
    """Repos within max_variant_repos are not skipped (but may return [] if no batches)."""
    manifest = tmp_path / "manifest.jsonl"
    _write_manifest(manifest, ["repo_0", "repo_1", "repo_2"])

    # repo_1 is at index 1, limit is 2 → should proceed (returns [] because no batch configs)
    result = run_variant_batches.run_variant_batches(
        repo_dir=str(tmp_path),
        repo_id="repo_1",
        binary="/nonexistent",
        batches_dir=str(tmp_path / "batches"),
        results_dir=str(tmp_path / "results"),
        manifest=str(manifest),
        max_variant_repos=2,
    )
    assert result == []


def test_run_variant_batches_no_limit_runs_all(tmp_path):
    """Without manifest/limit args, no repos are skipped."""
    # No manifest → proceeds to discover_batches (returns [] because no batch configs)
    result = run_variant_batches.run_variant_batches(
        repo_dir=str(tmp_path),
        repo_id="anything",
        binary="/nonexistent",
        batches_dir=str(tmp_path / "batches"),
        results_dir=str(tmp_path / "results"),
    )
    assert result == []


def test_parse_batch_style_map(tmp_path):
    """Parse variant batch YAML configs to extract cop -> style label."""
    batches_dir = tmp_path / "batches"
    batches_dir.mkdir()
    (batches_dir / "variant_batch_1.yml").write_text(
        "inherit_from: ../baseline.yml\n"
        "Style/TrailingCommaInHashLiteral:\n"
        "  EnforcedStyleForMultiline: comma\n"
        "Layout/DotPosition:\n"
        "  EnforcedStyle: trailing\n"
    )
    (batches_dir / "variant_batch_2.yml").write_text(
        "inherit_from: ../baseline.yml\n"
        "Style/TrailingCommaInHashLiteral:\n"
        "  EnforcedStyleForMultiline: consistent_comma\n"
    )
    result = run_variant_batches.parse_batch_style_map(batches_dir)
    assert "variant_batch_1" in result
    assert result["variant_batch_1"]["Style/TrailingCommaInHashLiteral"] == "comma"
    assert result["variant_batch_1"]["Layout/DotPosition"] == "trailing"
    assert result["variant_batch_2"]["Style/TrailingCommaInHashLiteral"] == "consistent_comma"


def test_merge_variant_results_with_style_labels(tmp_path):
    """When batches_dir is provided, merge filters to overridden cops and adds style_label."""
    batches_dir = tmp_path / "batches"
    batches_dir.mkdir()
    (batches_dir / "variant_batch_1.yml").write_text(
        "inherit_from: ../baseline.yml\n"
        "Style/Foo:\n"
        "  EnforcedStyle: bar\n"
    )

    f1 = tmp_path / "style-variant-variant_batch_1.json"
    f1.write_text(json.dumps({
        "summary": {"total_repos": 10},
        "by_cop": [
            {"cop": "Style/Foo", "matches": 50, "fp": 2, "fn": 1},
            {"cop": "Style/Other", "matches": 100, "fp": 0, "fn": 0},
        ],
        "by_repo_cop": {
            "repo_a__abc": {
                "Style/Foo": {"fp": 2, "fn": 1, "matches": 50},
                "Style/Other": {"fp": 0, "fn": 0, "matches": 100},
            },
            "repo_b__def": {
                "Style/Other": {"fp": 0, "fn": 0, "matches": 30},
            },
        },
    }))

    result = run_variant_batches.merge_variant_results([f1], batches_dir=batches_dir)
    assert len(result["batches"]) == 1
    batch = result["batches"][0]
    # Only Style/Foo should be included (it has an override), not Style/Other
    assert len(batch["by_cop"]) == 1
    assert batch["by_cop"][0]["cop"] == "Style/Foo"
    assert batch["by_cop"][0]["style_label"] == "bar"
    # by_repo_cop should be filtered to only Style/Foo
    assert "by_repo_cop" in batch
    assert "repo_a__abc" in batch["by_repo_cop"]
    assert "Style/Foo" in batch["by_repo_cop"]["repo_a__abc"]
    assert "Style/Other" not in batch["by_repo_cop"]["repo_a__abc"]
    # repo_b only has Style/Other, so it should be excluded entirely
    assert "repo_b__def" not in batch["by_repo_cop"]


def test_merge_variant_results_without_batches_dir(tmp_path):
    """Without batches_dir, all cops are included and no style_label is added."""
    f1 = tmp_path / "style-variant-variant_batch_1.json"
    f1.write_text(json.dumps({
        "summary": {"total_repos": 10},
        "by_cop": [
            {"cop": "Style/Foo", "matches": 50, "fp": 2, "fn": 1},
            {"cop": "Style/Other", "matches": 100, "fp": 0, "fn": 0},
        ],
    }))

    result = run_variant_batches.merge_variant_results([f1])
    batch = result["batches"][0]
    # All cops included, no style_label
    assert len(batch["by_cop"]) == 2
    assert "style_label" not in batch["by_cop"][0]


def test_run_variant_batches_passes_only_flag(tmp_path):
    """Variant runs pass --only with only the cops overridden in that batch."""
    batches_dir = tmp_path / "batches"
    batches_dir.mkdir()
    (batches_dir / "variant_batch_1.yml").write_text(
        "inherit_from: ../baseline.yml\n"
        "Style/Foo:\n"
        "  EnforcedStyle: bar\n"
        "Layout/Baz:\n"
        "  EnforcedStyle: qux\n"
    )
    (batches_dir / "variant_batch_2.yml").write_text(
        "inherit_from: ../baseline.yml\n"
        "Style/Foo:\n"
        "  EnforcedStyle: baz\n"
    )

    # Capture commands passed to _run_tool
    cmds = []
    original_run_tool = run_variant_batches._run_tool

    def fake_run_tool(*, cmd, env, timeout, stdout_path, stderr_path, label):
        cmds.append({"cmd": cmd, "label": label})
        # Write minimal valid JSON so the tool "succeeds"
        stdout_path.write_text('{"offenses": [], "files": []}')
        return True

    run_variant_batches._run_tool = fake_run_tool
    try:
        run_variant_batches.run_variant_batches(
            repo_dir=str(tmp_path),
            repo_id="test_repo",
            binary="/fake/nitrocop",
            batches_dir=str(batches_dir),
            results_dir=str(tmp_path / "results"),
        )
    finally:
        run_variant_batches._run_tool = original_run_tool

    # 2 batches × 2 tools (nitrocop + rubocop) = 4 commands
    assert len(cmds) == 4

    # Batch 1: --only should include both Style/Foo and Layout/Baz
    nc_batch1 = cmds[0]["cmd"]
    rc_batch1 = cmds[1]["cmd"]
    assert "--only" in nc_batch1
    only_idx = nc_batch1.index("--only")
    only_val = nc_batch1[only_idx + 1]
    assert "Layout/Baz" in only_val
    assert "Style/Foo" in only_val
    # Rubocop also gets --only
    assert "--only" in rc_batch1

    # Batch 2: --only should include only Style/Foo
    nc_batch2 = cmds[2]["cmd"]
    only_idx = nc_batch2.index("--only")
    only_val = nc_batch2[only_idx + 1]
    assert only_val == "Style/Foo"

    # Labels should show cop counts
    assert "2 cops" in cmds[0]["label"]
    assert "1 cops" in cmds[2]["label"]


def test_discover_batches_empty(tmp_path):
    """No batches in empty directory."""
    assert run_variant_batches.discover_batches(tmp_path) == []


def test_discover_batches_finds_files(tmp_path):
    """Discovers variant_batch_*.yml files, sorted."""
    (tmp_path / "variant_batch_2.yml").write_text("x: 1\n")
    (tmp_path / "variant_batch_1.yml").write_text("x: 1\n")
    (tmp_path / "other.yml").write_text("x: 1\n")  # should be ignored
    batches = run_variant_batches.discover_batches(tmp_path)
    assert len(batches) == 2
    assert batches[0].name == "variant_batch_1.yml"
    assert batches[1].name == "variant_batch_2.yml"


def test_discover_batches_nonexistent_dir():
    """Returns empty list for nonexistent directory."""
    assert run_variant_batches.discover_batches("/nonexistent/path") == []


def test_merge_variant_results_empty():
    """Merging empty list produces empty batches."""
    result = run_variant_batches.merge_variant_results([])
    assert result == {"batches": []}


def test_merge_variant_results(tmp_path):
    """Merge multiple result files into combined summary."""
    f1 = tmp_path / "style-variant-variant_batch_1.json"
    f1.write_text(json.dumps({
        "summary": {"total_repos": 10, "matches": 100, "fp": 5, "fn": 3},
        "by_cop": [{"cop": "Style/Foo", "matches": 50, "fp": 2, "fn": 1}],
    }))
    f2 = tmp_path / "style-variant-variant_batch_2.json"
    f2.write_text(json.dumps({
        "summary": {"total_repos": 10, "matches": 90, "fp": 8, "fn": 2},
        "by_cop": [{"cop": "Style/Foo", "matches": 40, "fp": 5, "fn": 0}],
    }))

    result = run_variant_batches.merge_variant_results([f1, f2])
    assert len(result["batches"]) == 2
    assert result["batches"][0]["name"] == "variant_batch_1"
    assert result["batches"][0]["total_repos"] == 10
    assert result["batches"][0]["fp"] == 5
    assert result["batches"][1]["name"] == "variant_batch_2"
    assert "by_cop" in result["batches"][0]


def test_merge_variant_results_skips_bad_json(tmp_path):
    """Bad JSON files are skipped, not crashed on."""
    good = tmp_path / "style-variant-variant_batch_1.json"
    good.write_text(json.dumps({"summary": {"fp": 0}}))
    bad = tmp_path / "style-variant-variant_batch_2.json"
    bad.write_text("not json")

    result = run_variant_batches.merge_variant_results([good, bad])
    assert len(result["batches"]) == 1
