#!/usr/bin/env python3
"""Tests for check_cop.py."""
import importlib.util
import json
import sys
import tempfile
from pathlib import Path

SCRIPT = Path(__file__).parents[2] / "scripts" / "check_cop.py"
sys.path.insert(0, str(SCRIPT.parent))
SPEC = importlib.util.spec_from_file_location("check_cop", SCRIPT)
assert SPEC and SPEC.loader
check_cop = importlib.util.module_from_spec(SPEC)
sys.modules["check_cop"] = check_cop
SPEC.loader.exec_module(check_cop)


def write_manifest(path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    entry = {
        "id": "demo-repo",
        "repo_url": "https://example.com/demo.git",
        "sha": "deadbeef",
    }
    path.write_text(json.dumps(entry) + "\n")


def test_clone_repos_for_cop_creates_temp_dir_for_zero_divergence():
    original_manifest_path = check_cop.MANIFEST_PATH
    try:
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            check_cop.MANIFEST_PATH = tmp_path / "bench" / "corpus" / "manifest.jsonl"
            write_manifest(check_cop.MANIFEST_PATH)

            result = check_cop.clone_repos_for_cop("Style/MixinUsage", {"by_repo_cop": {}})
            # Returns a temp dir with repos/ subdirectory
            assert (result / "repos").exists()
    finally:
        check_cop.MANIFEST_PATH = original_manifest_path


def test_relevant_repos_for_cop_unions_activity_and_divergence():
    data = {
        "cop_activity_repos": {
            "Style/MixinUsage": ["repo-active"],
        },
        "by_repo_cop": {
            "repo-diverging": {
                "Style/MixinUsage": {"matches": 0, "fp": 1, "fn": 0},
            },
        },
    }
    assert check_cop.relevant_repos_for_cop("Style/MixinUsage", data) == {
        "repo-active",
        "repo-diverging",
    }


def test_run_nitrocop_per_repo_skips_missing_corpus_when_no_relevant_repos():
    with tempfile.TemporaryDirectory() as tmp:
        tmp_path = Path(tmp)
        original_corpus_dir = check_cop.CORPUS_DIR
        try:
            check_cop.CORPUS_DIR = tmp_path / "vendor" / "corpus"
            result = check_cop.run_nitrocop_per_repo(
                "Style/MixinUsage",
                relevant_repos=set(),
            )
            assert result == {}
        finally:
            check_cop.CORPUS_DIR = original_corpus_dir


def test_run_nitrocop_per_repo_errors_on_missing_required_repos():
    with tempfile.TemporaryDirectory() as tmp:
        tmp_path = Path(tmp)
        original_corpus_dir = check_cop.CORPUS_DIR
        try:
            check_cop.CORPUS_DIR = tmp_path / "vendor" / "corpus"
            check_cop.CORPUS_DIR.mkdir(parents=True, exist_ok=True)
            try:
                check_cop.run_nitrocop_per_repo(
                    "Style/MixinUsage",
                    relevant_repos={"missing-repo"},
                )
                raise AssertionError("expected FileNotFoundError")
            except FileNotFoundError as exc:
                assert "missing-repo" in str(exc)
                assert str(check_cop.CORPUS_DIR) in str(exc)
        finally:
            check_cop.CORPUS_DIR = original_corpus_dir


def test_clone_repos_for_cop_uses_shared_clone_module():
    """clone_repos_for_cop delegates to the shared clone_repos module."""
    original_manifest_path = check_cop.MANIFEST_PATH
    original_clone = check_cop._clone_repos
    try:
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            check_cop.MANIFEST_PATH = tmp_path / "manifest.jsonl"
            write_manifest(check_cop.MANIFEST_PATH)

            calls = []
            check_cop._clone_repos = lambda dest, manifest, repo_ids=None, parallel=3: calls.append(
                {"dest": str(dest), "ids": repo_ids}
            ) or 0

            result = check_cop.clone_repos_for_cop(
                "Style/MixinUsage",
                {"cop_activity_repos": {"Style/MixinUsage": ["demo-repo"]}, "by_repo_cop": {}},
            )

            assert len(calls) == 1
            assert calls[0]["ids"] == {"demo-repo"}
            assert (result / "repos").parent == result
    finally:
        check_cop.MANIFEST_PATH = original_manifest_path
        check_cop._clone_repos = original_clone


def test_rerun_local_per_repo_always_uses_per_repo_mode():
    original_ensure_binary_fresh = check_cop.ensure_binary_fresh
    original_clear_file_cache = check_cop.clear_file_cache
    original_run_nitrocop_per_repo = check_cop.run_nitrocop_per_repo
    try:
        calls = []

        check_cop.ensure_binary_fresh = lambda: calls.append("fresh")
        check_cop.clear_file_cache = lambda: calls.append("clear")

        def fake_per_repo(_cop_name, relevant_repos=None, **_kw):
            calls.append(("per_repo", relevant_repos))
            return {"repo-a": 2}

        check_cop.run_nitrocop_per_repo = fake_per_repo

        result = check_cop.rerun_local_per_repo(
            "Style/MixinUsage",
            {
                "cop_activity_repos": {"Style/MixinUsage": ["repo-a"]},
                "by_repo_cop": {},
            },
            quick=True,
            has_activity_index=True,
        )

        assert result == {"repo-a": 2}
        assert ("per_repo", {"repo-a"}) in calls
    finally:
        check_cop.ensure_binary_fresh = original_ensure_binary_fresh
        check_cop.clear_file_cache = original_clear_file_cache
        check_cop.run_nitrocop_per_repo = original_run_nitrocop_per_repo


def _compute_gate(by_repo_cop, cop, per_repo):
    """Replicate the gate logic from check_cop.py for testing.

    Returns dict with: new_fp, new_fn, resolved_fp, resolved_fn, net_fp, net_fn,
    total_baseline_fp, total_baseline_fn, total_local_fp, total_local_fn.
    """
    oracle_nitrocop_counts = {}
    oracle_rubocop_counts = {}
    oracle_location_fp = {}
    oracle_location_fn = {}
    for repo_id, cops in by_repo_cop.items():
        if cop in cops:
            entry = cops[cop]
            matches = entry.get("matches", 0)
            fp = entry.get("fp", 0)
            fn = entry.get("fn", 0)
            oracle_nitrocop_counts[repo_id] = matches + fp
            oracle_rubocop_counts[repo_id] = matches + fn
            oracle_location_fp[repo_id] = fp
            oracle_location_fn[repo_id] = fn

    new_fp, new_fn = 0, 0
    resolved_fp, resolved_fn = 0, 0
    total_baseline_fp, total_baseline_fn = 0, 0
    total_local_fp, total_local_fn = 0, 0
    total_count_baseline_fp, total_count_baseline_fn = 0, 0
    for repo_id, local_count in per_repo.items():
        bl_nc = oracle_nitrocop_counts.get(repo_id)
        bl_rc = oracle_rubocop_counts.get(repo_id)
        if bl_nc is None or bl_rc is None:
            continue
        baseline_fp = oracle_location_fp.get(repo_id, 0)
        baseline_fn = oracle_location_fn.get(repo_id, 0)
        total_baseline_fp += baseline_fp
        total_baseline_fn += baseline_fn
        count_bl_fp = max(0, bl_nc - bl_rc)
        count_bl_fn = max(0, bl_rc - bl_nc)
        total_count_baseline_fp += count_bl_fp
        total_count_baseline_fn += count_bl_fn
        local_fp = max(0, local_count - bl_rc)
        local_fn = max(0, bl_rc - local_count)
        total_local_fp += local_fp
        total_local_fn += local_fn
        fp_increase = max(0, local_fp - baseline_fp)
        fn_increase = max(0, local_fn - baseline_fn)
        fp_decrease = max(0, baseline_fp - local_fp)
        fn_decrease = max(0, baseline_fn - local_fn)
        new_fp += fp_increase
        new_fn += fn_increase
        resolved_fp += fp_decrease
        resolved_fn += fn_decrease
    net_fp = new_fp - resolved_fp
    net_fn = new_fn - resolved_fn
    return {
        "new_fp": new_fp, "new_fn": new_fn,
        "resolved_fp": resolved_fp, "resolved_fn": resolved_fn,
        "net_fp": net_fp, "net_fn": net_fn,
        "total_baseline_fp": total_baseline_fp, "total_baseline_fn": total_baseline_fn,
        "total_local_fp": total_local_fp, "total_local_fn": total_local_fn,
        "total_count_baseline_fp": total_count_baseline_fp,
        "total_count_baseline_fn": total_count_baseline_fn,
    }


def test_gate_preexisting_fn_does_not_regress():
    """Pre-existing FN (already on main) should not be flagged."""
    by_repo_cop = {
        "repo-a": {
            # Oracle: nitrocop=12 (10+2fp), rubocop=13 (10+3fn) → baseline FN=1
            "Style/Foo": {"matches": 10, "fp": 2, "fn": 3},
        },
    }
    # Local produces same as oracle nitrocop → no regression
    g = _compute_gate(by_repo_cop, "Style/Foo", {"repo-a": 12})
    assert g["net_fp"] <= 0 and g["net_fn"] <= 0


def test_gate_improvement_passes():
    """Moving closer to rubocop is not a regression."""
    by_repo_cop = {
        "repo-a": {
            # Oracle: nitrocop=12, rubocop=13 → baseline FN=1
            "Style/Foo": {"matches": 10, "fp": 2, "fn": 3},
        },
    }
    # Local=13 matches rubocop exactly (fixed the FN)
    g = _compute_gate(by_repo_cop, "Style/Foo", {"repo-a": 13})
    assert g["net_fp"] <= 0 and g["net_fn"] <= 0


def test_gate_new_fp_detected():
    """New FP beyond baseline is flagged."""
    by_repo_cop = {
        "repo-a": {
            # Oracle: nitrocop=10, rubocop=10 → baseline FP=0
            "Style/Foo": {"matches": 10, "fp": 0, "fn": 0},
        },
    }
    # Local=12 → 2 more FP than rubocop, baseline had 0 FP
    g = _compute_gate(by_repo_cop, "Style/Foo", {"repo-a": 12})
    assert g["new_fp"] == 2
    assert g["net_fp"] == 2  # no resolved FP to offset


def test_gate_new_fn_detected():
    """New FN beyond baseline is flagged."""
    by_repo_cop = {
        "repo-a": {
            # Oracle: nitrocop=10, rubocop=10 → baseline FN=0
            "Style/Foo": {"matches": 10, "fp": 0, "fn": 0},
        },
    }
    # Local=7 → FN=3 vs rubocop, baseline had FN=0
    g = _compute_gate(by_repo_cop, "Style/Foo", {"repo-a": 7})
    assert g["new_fn"] == 3
    assert g["net_fn"] == 3  # no resolved FN to offset


def test_gate_location_swap_visible_in_baseline():
    """Location swaps (equal FP and FN) should be visible, not cancel to 0.

    This is the key case: the oracle found nitrocop fires at different lines
    than RuboCop (e.g., outer unless vs inner if). Both produce the same count,
    but the oracle's location-level comparison shows fp=2, fn=2. The old
    count-based baseline computed max(0, 10-10)=0, hiding the divergence.
    """
    by_repo_cop = {
        "repo-a": {
            # Oracle: 8 locations match, 2 nitrocop-only (FP), 2 rubocop-only (FN)
            # nc=10, rc=10 — counts match, but locations differ
            "Style/Next": {"matches": 8, "fp": 2, "fn": 2},
        },
    }
    # Baseline should reflect the location-level FP/FN
    g = _compute_gate(by_repo_cop, "Style/Next", {"repo-a": 10})
    assert g["total_baseline_fp"] == 2  # was 0 with count-based
    assert g["total_baseline_fn"] == 2  # was 0 with count-based

    # Local count matches rubocop (10=10), so local FP/FN = 0
    assert g["total_local_fp"] == 0
    assert g["total_local_fn"] == 0

    # The fix resolved the location mismatches
    assert g["resolved_fp"] == 2
    assert g["resolved_fn"] == 2
    assert g["net_fp"] == -2
    assert g["net_fn"] == -2


def test_gate_exact_match_no_regression():
    """Cop with zero baseline divergence — same count passes."""
    by_repo_cop = {
        "repo-a": {
            "Style/Foo": {"matches": 50, "fp": 0, "fn": 0},
        },
    }
    g = _compute_gate(by_repo_cop, "Style/Foo", {"repo-a": 50})
    assert g["net_fp"] <= 0 and g["net_fn"] <= 0


def test_gate_net_improvement_passes():
    """Per-repo FN regressions offset by improvements elsewhere should pass."""
    by_repo_cop = {
        # repo-a: nitrocop=5, rubocop=10 → baseline FN=5
        "repo-a": {"Style/Foo": {"matches": 5, "fp": 0, "fn": 5}},
        # repo-b: nitrocop=0, rubocop=20 → baseline FN=20
        "repo-b": {"Style/Foo": {"matches": 0, "fp": 0, "fn": 20}},
    }
    per_repo = {
        "repo-a": 3,   # worse: FN=7, was 5 → +2 new FN
        "repo-b": 15,  # better: FN=5, was 20 → resolved 15 FN
    }
    g = _compute_gate(by_repo_cop, "Style/Foo", per_repo)
    assert g["new_fn"] == 2        # repo-a regressed by 2
    assert g["resolved_fn"] == 15  # repo-b improved by 15
    assert g["net_fn"] < 0         # net improvement
    assert g["net_fp"] <= 0


def test_gate_net_regression_fails():
    """Per-repo regressions exceeding improvements should fail."""
    by_repo_cop = {
        # repo-a: nitrocop=10, rubocop=10 → baseline FN=0
        "repo-a": {"Style/Foo": {"matches": 10, "fp": 0, "fn": 0}},
        # repo-b: nitrocop=0, rubocop=3 → baseline FN=3
        "repo-b": {"Style/Foo": {"matches": 0, "fp": 0, "fn": 3}},
    }
    per_repo = {
        "repo-a": 0,  # worse: FN=10, was 0 → +10 new FN
        "repo-b": 3,  # better: FN=0, was 3 → resolved 3 FN
    }
    g = _compute_gate(by_repo_cop, "Style/Foo", per_repo)
    assert g["new_fn"] == 10
    assert g["resolved_fn"] == 3
    assert g["net_fn"] == 7  # net regression


def test_gate_net_zero_passes():
    """Regressions exactly offset by improvements should pass (net=0)."""
    by_repo_cop = {
        "repo-a": {"Style/Foo": {"matches": 10, "fp": 0, "fn": 0}},
        "repo-b": {"Style/Foo": {"matches": 0, "fp": 0, "fn": 5}},
    }
    per_repo = {
        "repo-a": 5,  # worse: FN=5, was 0 → +5 new FN
        "repo-b": 5,  # better: FN=0, was 5 → resolved 5 FN
    }
    g = _compute_gate(by_repo_cop, "Style/Foo", per_repo)
    assert g["new_fn"] == 5
    assert g["resolved_fn"] == 5
    assert g["net_fn"] == 0  # exactly offset — should pass (gate checks > threshold)


def test_gate_fp_net_improvement_passes():
    """Per-repo FP regressions offset by FP improvements should pass."""
    by_repo_cop = {
        # repo-a: nitrocop=15, rubocop=10 → baseline FP=5
        "repo-a": {"Style/Foo": {"matches": 10, "fp": 5, "fn": 0}},
        # repo-b: nitrocop=12, rubocop=10 → baseline FP=2
        "repo-b": {"Style/Foo": {"matches": 10, "fp": 2, "fn": 0}},
    }
    per_repo = {
        "repo-a": 13,  # FP=3, was 5 → resolved 2 FP
        "repo-b": 11,  # FP=1, was 2 → resolved 1 FP (improved)
    }
    g = _compute_gate(by_repo_cop, "Style/Foo", per_repo)
    assert g["new_fp"] == 0
    assert g["resolved_fp"] == 3
    assert g["net_fp"] < 0
    assert g["net_fn"] <= 0


def test_gate_strict_fails_on_any_regression():
    """Without --allow-net-improvement, any per-repo regression fails."""
    by_repo_cop = {
        "repo-a": {"Style/Foo": {"matches": 5, "fp": 0, "fn": 5}},
        "repo-b": {"Style/Foo": {"matches": 0, "fp": 0, "fn": 20}},
    }
    per_repo = {
        "repo-a": 3,   # worse: +2 new FN
        "repo-b": 15,  # better: resolved 15 FN
    }
    g = _compute_gate(by_repo_cop, "Style/Foo", per_repo)
    # Net is an improvement
    assert g["net_fn"] < 0
    # But strict mode (default) uses new_fn, not net_fn
    assert g["new_fn"] == 2  # would FAIL in strict mode (agents)
    # --allow-net-improvement uses net_fn which is negative → PASS in CI


def test_summary_emits_total_local_fp_fn_not_regressions():
    """SUMMARY line should report total local FP/FN, not just regressions."""
    by_repo_cop = {
        # repo-a: oracle nitrocop=1064, rubocop=1061 → baseline FP=3
        "repo-a": {"Metrics/MethodLength": {"matches": 1061, "fp": 3, "fn": 0}},
    }
    # Local produces same as oracle nitrocop → no regression, but FP=3 remains
    g = _compute_gate(by_repo_cop, "Metrics/MethodLength", {"repo-a": 1064})
    assert g["new_fp"] == 0           # no regression
    assert g["total_baseline_fp"] == 3
    assert g["total_local_fp"] == 3   # should report actual FP, not 0
    # SUMMARY should use total_local_fp (3), not new_fp (0)


def test_summary_shows_improvement_correctly():
    """When local FP is lower than baseline, SUMMARY should reflect that."""
    by_repo_cop = {
        "repo-a": {"Style/Foo": {"matches": 10, "fp": 5, "fn": 0}},
    }
    # Local=12 → FP=2, baseline FP=5 → improvement
    g = _compute_gate(by_repo_cop, "Style/Foo", {"repo-a": 12})
    assert g["total_baseline_fp"] == 5
    assert g["total_local_fp"] == 2
    assert g["resolved_fp"] == 3
    assert g["new_fp"] == 0


def test_count_level_baseline_detects_location_shift():
    """When location-level FP increases but count-level doesn't, it's a location shift."""
    by_repo_cop = {
        # Oracle: nitrocop=100 (95 match + 5 FP), rubocop=98 (95 match + 3 FN)
        # Location-level: FP=5, FN=3. Count-level: FP=max(0,100-98)=2.
        "repo-a": {"Style/Foo": {"matches": 95, "fp": 5, "fn": 3}},
    }
    # Local produces 100 (same count as oracle nitrocop) but at different locations.
    # Location-level FP could be higher, but count-level FP = max(0,100-98) = 2.
    g = _compute_gate(by_repo_cop, "Style/Foo", {"repo-a": 100})
    assert g["total_baseline_fp"] == 5       # location-level from oracle
    assert g["total_count_baseline_fp"] == 2  # count-level from oracle
    assert g["total_local_fp"] == 2           # count-level from local
    # No count-level regression: local count FP (2) <= count baseline FP (2)
    # Even though location-level baseline was 5, count tells us no extra offenses.


def test_count_level_baseline_detects_real_regression():
    """When both location and count-level FP increase, it's a real regression."""
    by_repo_cop = {
        # Oracle: nitrocop=10 (10 match + 0 FP), rubocop=10 (10 match + 0 FN)
        "repo-a": {"Style/Foo": {"matches": 10, "fp": 0, "fn": 0}},
    }
    # Local produces 15 → 5 more than rubocop → real FP
    g = _compute_gate(by_repo_cop, "Style/Foo", {"repo-a": 15})
    assert g["total_count_baseline_fp"] == 0
    assert g["total_local_fp"] == 5
    assert g["new_fp"] == 5  # real regression


# ── sampling tests for relevant_repos_for_cop ──


def test_sample_caps_when_fewer_diverging_than_sample():
    """When diverging < sample, fill remaining slots by offense count."""
    data = {
        "cop_activity_repos": {
            "Style/Foo": [f"repo-{i}" for i in range(20)],
        },
        "by_repo_cop": {
            "repo-0": {"Style/Foo": {"matches": 5, "fp": 2, "fn": 0}},
            "repo-1": {"Style/Foo": {"matches": 10, "fp": 0, "fn": 3}},
            # repo-2..19 have activity but no divergence
            **{
                f"repo-{i}": {"Style/Foo": {"matches": 100 - i, "fp": 0, "fn": 0}}
                for i in range(2, 20)
            },
        },
    }
    result = check_cop.relevant_repos_for_cop("Style/Foo", data, sample=5)
    assert len(result) == 5
    # Both diverging repos must be included
    assert "repo-0" in result
    assert "repo-1" in result


def test_sample_caps_diverging_repos_when_exceeding_sample():
    """When diverging repos exceed sample, pick top N by FP+FN."""
    # 30 repos, all diverging with different FP+FN counts
    activity = [f"repo-{i}" for i in range(30)]
    by_repo_cop = {
        f"repo-{i}": {"Style/Foo": {"matches": 10, "fp": i, "fn": i}}
        for i in range(30)
    }
    data = {
        "cop_activity_repos": {"Style/Foo": activity},
        "by_repo_cop": by_repo_cop,
    }
    result = check_cop.relevant_repos_for_cop("Style/Foo", data, sample=5)
    # Must be exactly 5, not 30
    assert len(result) == 5
    # The top 5 by FP+FN are repo-29..repo-25
    for i in range(25, 30):
        assert f"repo-{i}" in result


def test_sample_none_returns_all_relevant():
    """Without sample, all relevant repos are returned."""
    data = {
        "cop_activity_repos": {
            "Style/Foo": [f"repo-{i}" for i in range(50)],
        },
        "by_repo_cop": {
            f"repo-{i}": {"Style/Foo": {"matches": 10, "fp": 1, "fn": 1}}
            for i in range(50)
        },
    }
    result = check_cop.relevant_repos_for_cop("Style/Foo", data, sample=None)
    assert len(result) == 50


def test_sample_equal_to_relevant_returns_all():
    """When sample >= relevant count, return all."""
    data = {
        "cop_activity_repos": {"Style/Foo": ["repo-a", "repo-b"]},
        "by_repo_cop": {},
    }
    result = check_cop.relevant_repos_for_cop("Style/Foo", data, sample=10)
    assert result == {"repo-a", "repo-b"}


# ── --style flag tests ──


def test_generate_style_override_config_creates_valid_yaml():
    """generate_style_override_config creates a YAML file with correct structure."""
    import os
    path = check_cop.generate_style_override_config(
        "Style/TrailingCommaInHashLiteral",
        "EnforcedStyleForMultiline",
        "comma",
    )
    try:
        content = Path(path).read_text()
        assert "inherit_from:" in content
        assert "Style/TrailingCommaInHashLiteral:" in content
        assert "EnforcedStyleForMultiline: comma" in content
    finally:
        os.unlink(path)


def test_generate_style_override_config_different_cops():
    """Each call produces a file specific to the cop and param."""
    import os
    path1 = check_cop.generate_style_override_config("Layout/DotPosition", "EnforcedStyle", "trailing")
    path2 = check_cop.generate_style_override_config("Style/AndOr", "EnforcedStyle", "always")
    try:
        c1 = Path(path1).read_text()
        c2 = Path(path2).read_text()
        assert "Layout/DotPosition:" in c1
        assert "EnforcedStyle: trailing" in c1
        assert "Style/AndOr:" in c2
        assert "EnforcedStyle: always" in c2
    finally:
        os.unlink(path1)
        os.unlink(path2)


def test_style_config_override_threads_to_run_one_repo():
    """_STYLE_CONFIG_OVERRIDE is passed through to _run_corpus_nitrocop."""
    original = check_cop._run_corpus_nitrocop
    calls = []

    def fake_run(repo_dir, *, cop=None, binary=None, timeout=120, cwd=None, config_override=None):
        calls.append(config_override)
        return {"offenses": [], "count": 0, "error": None}

    check_cop._run_corpus_nitrocop = fake_run
    old_override = check_cop._STYLE_CONFIG_OVERRIDE
    try:
        check_cop._STYLE_CONFIG_OVERRIDE = "/tmp/test_override.yml"
        check_cop._run_one_repo(("Style/Foo", "/tmp/fake_repo"))
        assert calls == ["/tmp/test_override.yml"]
    finally:
        check_cop._run_corpus_nitrocop = original
        check_cop._STYLE_CONFIG_OVERRIDE = old_override


# ── variant discovery tests ──


def test_variant_styles_for_cop_finds_overrides(tmp_path):
    """Discovers which variant batches include a given cop and returns style labels."""
    batches = tmp_path / "batches"
    batches.mkdir()
    (batches / "variant_batch_1.yml").write_text(
        "inherit_from: ../baseline.yml\n"
        "Style/TrailingCommaInHashLiteral:\n"
        "  EnforcedStyleForMultiline: comma\n"
        "Layout/DotPosition:\n"
        "  EnforcedStyle: trailing\n"
    )
    (batches / "variant_batch_2.yml").write_text(
        "inherit_from: ../baseline.yml\n"
        "Style/TrailingCommaInHashLiteral:\n"
        "  EnforcedStyleForMultiline: consistent_comma\n"
    )

    result = check_cop.variant_styles_for_cop(
        "Style/TrailingCommaInHashLiteral", str(batches))
    assert len(result) == 2
    assert result[0]["batch_config"] == str(batches / "variant_batch_1.yml")
    assert result[0]["style_label"] == "comma"
    assert result[1]["style_label"] == "consistent_comma"


def test_variant_styles_for_cop_returns_empty_for_non_overridden(tmp_path):
    """Cops not overridden in any batch return empty list."""
    batches = tmp_path / "batches"
    batches.mkdir()
    (batches / "variant_batch_1.yml").write_text(
        "inherit_from: ../baseline.yml\n"
        "Style/Foo:\n"
        "  EnforcedStyle: bar\n"
    )

    result = check_cop.variant_styles_for_cop("Style/Other", str(batches))
    assert result == []


def test_variant_styles_for_cop_missing_dir():
    """Missing batches directory returns empty list."""
    result = check_cop.variant_styles_for_cop("Style/Foo", "/nonexistent/path")
    assert result == []


# ── variant check result tests ──


def test_run_variant_checks_returns_per_variant_results(tmp_path):
    """run_variant_checks runs nitrocop + rubocop per variant and returns results."""
    batches = tmp_path / "batches"
    batches.mkdir()
    (batches / "variant_batch_1.yml").write_text(
        "inherit_from: ../baseline.yml\n"
        "Style/Foo:\n"
        "  EnforcedStyle: bar\n"
    )
    (batches / "variant_batch_2.yml").write_text(
        "inherit_from: ../baseline.yml\n"
        "Style/Foo:\n"
        "  EnforcedStyle: baz\n"
    )

    # Mock the nitrocop and rubocop runners
    nc_calls = []
    rc_calls = []

    def fake_nc(repo_dir, *, cop=None, binary=None, timeout=120, cwd=None, config_override=None):
        nc_calls.append({"cop": cop, "config": config_override})
        return {"count": 10, "offenses": [], "error": None}

    def fake_rc(repo_dir, *, cop=None, config=None, timeout=120):
        rc_calls.append({"cop": cop, "config": config})
        return {"count": 8}

    results = check_cop.run_variant_checks(
        cop_name="Style/Foo",
        repo_dirs=[str(tmp_path / "repo_a"), str(tmp_path / "repo_b")],
        batches_dir=str(batches),
        run_nitrocop_fn=fake_nc,
        run_rubocop_fn=fake_rc,
    )

    # Should have 2 variants (batch 1 and 2)
    assert len(results) == 2
    assert results[0]["style_label"] == "bar"
    assert results[1]["style_label"] == "baz"
    # 2 repos × 2 variants = 4 nitrocop calls, 4 rubocop calls
    assert len(nc_calls) == 4
    assert len(rc_calls) == 4
    # Each variant: 2 repos × 10 nitrocop = 20, 2 repos × 8 rubocop = 16
    assert results[0]["nitrocop_total"] == 20
    assert results[0]["rubocop_total"] == 16
    assert results[0]["fp"] == 4  # 20 - 16
    assert results[0]["fn"] == 0


def test_run_variant_checks_skips_cops_not_in_batch(tmp_path):
    """Cops not overridden in any batch get empty results."""
    batches = tmp_path / "batches"
    batches.mkdir()
    (batches / "variant_batch_1.yml").write_text(
        "inherit_from: ../baseline.yml\n"
        "Style/Other:\n"
        "  EnforcedStyle: bar\n"
    )

    results = check_cop.run_variant_checks(
        cop_name="Style/NotInBatch",
        repo_dirs=[str(tmp_path / "repo_a")],
        batches_dir=str(batches),
        run_nitrocop_fn=lambda *a, **kw: {"count": 0, "offenses": [], "error": None},
        run_rubocop_fn=lambda *a, **kw: {"count": 0},
    )
    assert results == []


def test_load_variant_baselines_from_file(tmp_path):
    """Loads per-style baselines from style-variant-results.json."""
    import tempfile
    # Write a fake variant results file where the cache would expect it
    cache_dir = Path(tempfile.gettempdir()) / "nitrocop-corpus-cache"
    cache_dir.mkdir(parents=True, exist_ok=True)
    cache_file = cache_dir / "style-variant-results-99999.json"
    cache_file.write_text(json.dumps({
        "batches": [
            {
                "name": "variant_batch_1",
                "by_cop": [
                    {"cop": "Style/Foo", "style_label": "bar", "matches": 80, "fp": 5, "fn": 3},
                    {"cop": "Style/Other", "style_label": "x", "matches": 10, "fp": 0, "fn": 0},
                ],
            },
            {
                "name": "variant_batch_2",
                "by_cop": [
                    {"cop": "Style/Foo", "style_label": "baz", "matches": 60, "fp": 10, "fn": 7},
                ],
            },
        ]
    }))
    try:
        result = check_cop.load_variant_baselines("Style/Foo", 99999)
        assert "bar" in result
        assert result["bar"]["fp"] == 5
        assert result["bar"]["fn"] == 3
        assert "baz" in result
        assert result["baz"]["fp"] == 10
        assert "x" not in result  # Style/Other, not Style/Foo
    finally:
        cache_file.unlink(missing_ok=True)


def test_load_variant_baselines_no_run_id():
    """Returns empty dict when run_id is None."""
    assert check_cop.load_variant_baselines("Style/Foo", None) == {}


def test_variant_batch_configs_have_resolvable_inherit_from(tmp_path):
    """Variant batch configs must use inherit_from paths that resolve from the config's directory.

    Regression test: when configs were generated in a temp dir, inherit_from: ../baseline_rubocop.yml
    pointed to nothing, causing rubocop to silently return 0 offenses and making all nitrocop
    offenses look like FP.
    """
    import yaml

    # Generate batches in the standard location
    batches_dir = Path(__file__).parents[2] / "bench" / "corpus" / "variant_batches"
    if not batches_dir.exists():
        import sys
        sys.path.insert(0, str(Path(__file__).parents[2] / "bench" / "corpus"))
        from gen_variant_batches import generate_batches
        generate_batches(batches_dir)

    class _Loader(yaml.SafeLoader):
        pass
    _Loader.add_multi_constructor("!", lambda loader, suffix, node: loader.construct_scalar(node))

    for batch_file in sorted(batches_dir.glob("variant_batch_*.yml")):
        data = yaml.load(batch_file.read_text(), Loader=_Loader)
        inherit = data.get("inherit_from", "")
        if inherit:
            resolved = (batch_file.parent / inherit).resolve()
            assert resolved.exists(), (
                f"{batch_file.name}: inherit_from '{inherit}' resolves to "
                f"{resolved} which does not exist"
            )


def test_run_variant_checks_handles_rubocop_errors(tmp_path):
    """When rubocop returns negative count (error), still produces results."""
    batches = tmp_path / "batches"
    batches.mkdir()
    (batches / "variant_batch_1.yml").write_text(
        "inherit_from: ../baseline.yml\n"
        "Style/Foo:\n"
        "  EnforcedStyle: bar\n"
    )

    results = check_cop.run_variant_checks(
        cop_name="Style/Foo",
        repo_dirs=[str(tmp_path / "repo_a")],
        batches_dir=str(batches),
        run_nitrocop_fn=lambda *a, **kw: {"count": 5, "offenses": [], "error": None},
        run_rubocop_fn=lambda *a, **kw: {"count": -1},  # error
    )
    assert len(results) == 1
    assert results[0]["rubocop_errors"] == 1


# ── spot_check_examples tests ──


def test_spot_check_returns_zeros_for_missing_cop():
    """When the cop isn't in the data, all counts should be zero."""
    data = {"by_cop": []}
    result = check_cop.spot_check_examples("Style/Missing", data)
    assert result == (0, 0, 0, 0, 0, 0)


def test_spot_check_returns_zeros_for_empty_examples():
    """When the cop has no FP/FN examples, all counts should be zero."""
    data = {"by_cop": [{"cop": "Style/Foo", "fp_examples": [], "fn_examples": []}]}
    result = check_cop.spot_check_examples("Style/Foo", data)
    assert result == (0, 0, 0, 0, 0, 0)


def test_spot_check_unchecked_when_repo_not_cloned():
    """Examples from repos that don't exist in corpus dir are 'unchecked'."""
    original_clone_dir = check_cop._CLONE_DIR
    try:
        with tempfile.TemporaryDirectory() as tmp:
            # Point corpus dir at an empty temp dir — no repos exist
            check_cop._CLONE_DIR = Path(tmp)
            data = {
                "by_cop": [{
                    "cop": "Rails/Output",
                    "fp_examples": [
                        {"loc": "missing-repo: app/models/foo.rb:10", "msg": "FP msg"},
                        {"loc": "also-missing: db/seeds.rb:5", "msg": "FP msg"},
                    ],
                    "fn_examples": [
                        {"loc": "gone-repo: lib/bar.rb:20", "msg": "FN msg"},
                    ],
                }],
            }
            result = check_cop.spot_check_examples("Rails/Output", data)
            fp_remain, fp_resolved, fn_remain, fn_resolved, fp_unchecked, fn_unchecked = result
            assert fp_remain == 0
            assert fp_resolved == 0
            assert fn_remain == 0
            assert fn_resolved == 0
            assert fp_unchecked == 2
            assert fn_unchecked == 1
    finally:
        check_cop._CLONE_DIR = original_clone_dir


def test_spot_check_mixed_cloned_and_missing_repos():
    """Cloned repos are checked; missing repos are unchecked."""
    original_clone_dir = check_cop._CLONE_DIR
    original_nitrocop_bin = check_cop.NITROCOP_BIN
    try:
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            check_cop._CLONE_DIR = tmp_path

            # Create one repo dir with a file (but no real nitrocop to run)
            repo_dir = tmp_path / "cloned-repo"
            repo_dir.mkdir()
            (repo_dir / "app").mkdir()
            (repo_dir / "app" / "foo.rb").write_text("puts 'hello'\n")

            # Use a fake binary that outputs no offenses
            fake_bin = tmp_path / "fake_nitrocop"
            fake_bin.write_text('#!/bin/sh\necho \'{"offenses":[]}\'\n')
            fake_bin.chmod(0o755)
            check_cop.NITROCOP_BIN = fake_bin

            data = {
                "by_cop": [{
                    "cop": "Rails/Output",
                    "fp_examples": [
                        # This repo is cloned — will be checked
                        {"loc": "cloned-repo: app/foo.rb:1", "msg": "FP"},
                        # This repo is NOT cloned — unchecked
                        {"loc": "missing-repo: app/bar.rb:5", "msg": "FP"},
                    ],
                    "fn_examples": [
                        {"loc": "missing-repo: lib/baz.rb:10", "msg": "FN"},
                    ],
                }],
            }
            result = check_cop.spot_check_examples("Rails/Output", data)
            fp_remain, fp_resolved, fn_remain, fn_resolved, fp_unchecked, fn_unchecked = result
            # cloned-repo: nitrocop found no offenses → FP "resolved"
            assert fp_resolved == 1
            assert fp_remain == 0
            # missing-repo: not cloned → unchecked
            assert fp_unchecked == 1
            assert fn_unchecked == 1
            assert fn_remain == 0
            assert fn_resolved == 0
    finally:
        check_cop._CLONE_DIR = original_clone_dir
        check_cop.NITROCOP_BIN = original_nitrocop_bin


# ── _run_rubocop_for_variant filtering/dedup tests ──


def test_run_rubocop_for_variant_filters_to_requested_cop(monkeypatch):
    """_run_rubocop_for_variant must only count offenses for the requested cop,
    not Lint/Syntax or other cops that RuboCop emits alongside --only."""
    rubocop_json = json.dumps({
        "files": [{
            "path": "/tmp/repo/app.rb",
            "offenses": [
                {"cop_name": "Style/Foo", "location": {"line": 1}},
                {"cop_name": "Lint/Syntax", "location": {"line": 2}},
                {"cop_name": "Lint/Syntax", "location": {"line": 3}},
                {"cop_name": "Style/Foo", "location": {"line": 5}},
            ],
        }],
    })

    def fake_run(cmd, **kwargs):
        class Result:
            stdout = rubocop_json
        return Result()

    monkeypatch.setattr("subprocess.run", fake_run)
    result = check_cop._run_rubocop_for_variant(
        "/tmp/repo", cop="Style/Foo", config="/tmp/config.yml")
    assert result["count"] == 2, (
        "Should count only Style/Foo offenses, not Lint/Syntax")


def test_run_rubocop_for_variant_deduplicates_by_path_line(monkeypatch):
    """_run_rubocop_for_variant must deduplicate by (path, line) to match
    the dedup that run_nitrocop applies on the nitrocop side."""
    rubocop_json = json.dumps({
        "files": [{
            "path": "/tmp/repo/app.rb",
            "offenses": [
                {"cop_name": "Style/Foo", "location": {"line": 1}},
                {"cop_name": "Style/Foo", "location": {"line": 1}},
                {"cop_name": "Style/Foo", "location": {"line": 5}},
            ],
        }],
    })

    def fake_run(cmd, **kwargs):
        class Result:
            stdout = rubocop_json
        return Result()

    monkeypatch.setattr("subprocess.run", fake_run)
    result = check_cop._run_rubocop_for_variant(
        "/tmp/repo", cop="Style/Foo", config="/tmp/config.yml")
    assert result["count"] == 2, (
        "Should deduplicate: two offenses at line 1 count as one")
