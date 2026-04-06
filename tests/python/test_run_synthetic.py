#!/usr/bin/env python3
"""Tests for bench/synthetic/run_synthetic.py."""

import importlib.util
import json
import os
import sys
from pathlib import Path
from unittest import mock

SCRIPT = Path(__file__).parents[2] / "bench" / "synthetic" / "run_synthetic.py"
SPEC = importlib.util.spec_from_file_location("run_synthetic", SCRIPT)
assert SPEC and SPEC.loader
run_synthetic = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(run_synthetic)

# Register the module so mock.patch("run_synthetic.xxx") works.
sys.modules["run_synthetic"] = run_synthetic


# ── Pure function tests ──


def test_normalize_path_strips_project_prefix():
    assert run_synthetic.normalize_path("/a/b/project/foo.rb", "/a/b/project") == "foo.rb"


def test_normalize_path_strips_project_slash_prefix():
    assert run_synthetic.normalize_path("project/foo.rb", "/irrelevant") == "foo.rb"


def test_normalize_path_strips_dot_slash():
    assert run_synthetic.normalize_path("./foo.rb", "/irrelevant") == "foo.rb"


def test_normalize_path_already_relative():
    assert run_synthetic.normalize_path("lib/bar.rb", "/some/dir") == "lib/bar.rb"


def test_trunc4_rounds_down():
    assert run_synthetic.trunc4(0.99999) == 0.9999
    assert run_synthetic.trunc4(1.0) == 1.0
    # 0.12345 → floor(1234.5) = 1234 → 0.1234
    assert run_synthetic.trunc4(0.12345) == 0.1234


def test_parse_nitrocop_json():
    data = {
        "offenses": [
            {"path": "project/foo.rb", "line": 10, "cop_name": "Style/Foo"},
            {"path": "project/bar.rb", "line": 20, "cop_name": "Lint/Bar"},
            {"path": "", "line": 1, "cop_name": "Style/Bad"},  # empty path, skipped
        ]
    }
    result = run_synthetic.parse_nitrocop_json(data, "/whatever")
    assert ("foo.rb", 10, "Style/Foo") in result
    assert ("bar.rb", 20, "Lint/Bar") in result
    assert len(result) == 2


def test_parse_rubocop_json():
    data = {
        "files": [
            {
                "path": "project/foo.rb",
                "offenses": [
                    {"location": {"line": 5}, "cop_name": "Style/X"},
                    {"location": {"line": 8}, "cop_name": "Lint/Y"},
                ],
            },
        ]
    }
    result = run_synthetic.parse_rubocop_json(data, "/whatever")
    assert ("foo.rb", 5, "Style/X") in result
    assert ("foo.rb", 8, "Lint/Y") in result
    assert len(result) == 2


def test_parse_rubocop_json_empty():
    assert run_synthetic.parse_rubocop_json({"files": []}, "/x") == set()


# ── _run_single_variant tests ──


def _fake_run_factory(nc_output, rc_output):
    """Create a fake subprocess.run that returns nc/rc output."""
    def fake_run(cmd, **kwargs):
        result = mock.MagicMock()
        if "nitrocop" in str(cmd[0]):
            result.stdout = nc_output
        else:
            result.stdout = rc_output
        return result
    return fake_run


def test_run_single_variant_perfect(tmp_path):
    """When both tools agree, variant result is perfect."""
    project_dir = str(tmp_path / "project")
    os.makedirs(project_dir)

    nc_output = json.dumps({
        "offenses": [
            {"path": "foo.rb", "line": 1, "cop_name": "Style/A"},
        ]
    })
    rc_output = json.dumps({
        "files": [{"path": "foo.rb", "offenses": [
            {"location": {"line": 1}, "cop_name": "Style/A"},
        ]}]
    })

    with mock.patch("run_synthetic.subprocess.run",
                     side_effect=_fake_run_factory(nc_output, rc_output)):
        batch = run_synthetic._run_single_variant(
            config_path=str(tmp_path / "config.yml"),
            override_cops={"Style/A"},
            target_set={"Style/A"},
            nitrocop_binary="nitrocop",
            project_dir=project_dir,
            gemfile=str(tmp_path / "Gemfile"),
            style_label="variant_batch_1",
            verbose=False,
        )

    assert len(batch["by_cop"]) == 1
    cop = batch["by_cop"][0]
    assert cop["cop"] == "Style/A"
    assert cop["matches"] == 1
    assert cop["fp"] == 0
    assert cop["fn"] == 0
    assert cop["perfect_match"] is True
    assert cop["style_label"] == "variant_batch_1"


def test_run_single_variant_diverging(tmp_path):
    """When tools disagree, variant result shows divergence."""
    project_dir = str(tmp_path / "project")
    os.makedirs(project_dir)

    nc_output = json.dumps({
        "offenses": [
            {"path": "foo.rb", "line": 1, "cop_name": "Style/A"},
            {"path": "foo.rb", "line": 5, "cop_name": "Style/A"},  # FP
        ]
    })
    rc_output = json.dumps({
        "files": [{"path": "foo.rb", "offenses": [
            {"location": {"line": 1}, "cop_name": "Style/A"},
            {"location": {"line": 10}, "cop_name": "Style/A"},  # FN
        ]}]
    })

    with mock.patch("run_synthetic.subprocess.run",
                     side_effect=_fake_run_factory(nc_output, rc_output)):
        batch = run_synthetic._run_single_variant(
            config_path=str(tmp_path / "config.yml"),
            override_cops={"Style/A"},
            target_set={"Style/A"},
            nitrocop_binary="nitrocop",
            project_dir=project_dir,
            gemfile=str(tmp_path / "Gemfile"),
            style_label="variant_batch_1",
            verbose=False,
        )

    cop = batch["by_cop"][0]
    assert cop["fp"] == 1
    assert cop["fn"] == 1
    assert cop["matches"] == 1
    assert cop["diverging"] is True
    assert cop["perfect_match"] is False


def test_run_single_variant_filters_to_override_cops(tmp_path):
    """Only cops whose style was overridden are included in results."""
    project_dir = str(tmp_path / "project")
    os.makedirs(project_dir)

    nc_output = json.dumps({
        "offenses": [
            {"path": "foo.rb", "line": 1, "cop_name": "Style/A"},
            {"path": "foo.rb", "line": 2, "cop_name": "Style/Other"},
        ]
    })
    rc_output = json.dumps({
        "files": [{"path": "foo.rb", "offenses": [
            {"location": {"line": 1}, "cop_name": "Style/A"},
            {"location": {"line": 2}, "cop_name": "Style/Other"},
        ]}]
    })

    with mock.patch("run_synthetic.subprocess.run",
                     side_effect=_fake_run_factory(nc_output, rc_output)):
        batch = run_synthetic._run_single_variant(
            config_path=str(tmp_path / "config.yml"),
            override_cops={"Style/A"},
            target_set={"Style/A", "Style/Other"},
            nitrocop_binary="nitrocop",
            project_dir=project_dir,
            gemfile=str(tmp_path / "Gemfile"),
            style_label="variant_batch_1",
            verbose=False,
        )

    cop_names = [c["cop"] for c in batch["by_cop"]]
    assert "Style/A" in cop_names
    assert "Style/Other" not in cop_names


def test_run_single_variant_empty_output(tmp_path):
    """When both tools produce no offenses for the override cop, it's not exercised."""
    project_dir = str(tmp_path / "project")
    os.makedirs(project_dir)

    empty_nc = json.dumps({"offenses": []})
    empty_rc = json.dumps({"files": []})

    with mock.patch("run_synthetic.subprocess.run",
                     side_effect=_fake_run_factory(empty_nc, empty_rc)):
        batch = run_synthetic._run_single_variant(
            config_path=str(tmp_path / "config.yml"),
            override_cops={"Style/A"},
            target_set={"Style/A"},
            nitrocop_binary="nitrocop",
            project_dir=project_dir,
            gemfile=str(tmp_path / "Gemfile"),
            style_label="variant_batch_1",
            verbose=False,
        )

    assert len(batch["by_cop"]) == 1
    cop = batch["by_cop"][0]
    assert cop["exercised"] is False
    assert cop["perfect_match"] is False


# ── run_variant_batches tests ──


def test_run_variant_batches_generates_config_and_cleans_up(tmp_path):
    """Variant config is written to project dir and cleaned up after."""
    project_dir = str(tmp_path / "project")
    os.makedirs(project_dir)
    rubocop_yml = str(tmp_path / "rubocop.yml")
    Path(rubocop_yml).write_text("AllCops:\n  Enabled: true\n")

    variant_styles = [
        {"cop": "Style/Foo", "key": "EnforcedStyle", "default": "a",
         "alternatives": ["b"]},
    ]

    configs_seen = []

    def fake_run(cmd, **kwargs):
        for i, arg in enumerate(cmd):
            if arg == "--config" and i + 1 < len(cmd):
                configs_seen.append(cmd[i + 1])
        result = mock.MagicMock()
        result.stdout = json.dumps({"offenses": [], "files": []})
        return result

    with mock.patch("run_synthetic.subprocess.run", side_effect=fake_run):
        result = run_synthetic.run_variant_batches(
            variant_styles, {"Style/Foo"},
            "nitrocop", rubocop_yml, project_dir,
            str(tmp_path / "Gemfile"), False,
        )

    assert len(result["batches"]) == 1
    # Config should have been cleaned up
    for cfg in configs_seen:
        assert not os.path.exists(cfg), f"Temp config {cfg} was not cleaned up"


def test_run_variant_batches_multiple_alternatives(tmp_path):
    """Cops with multiple alternatives produce multiple batches."""
    project_dir = str(tmp_path / "project")
    os.makedirs(project_dir)

    variant_styles = [
        {"cop": "Style/A", "key": "EnforcedStyle", "default": "x",
         "alternatives": ["y", "z"]},
    ]

    def fake_run(cmd, **kwargs):
        result = mock.MagicMock()
        result.stdout = json.dumps({"offenses": [], "files": []})
        return result

    with mock.patch("run_synthetic.subprocess.run", side_effect=fake_run):
        result = run_synthetic.run_variant_batches(
            variant_styles, {"Style/A"},
            "nitrocop", "config.yml", project_dir,
            "Gemfile", False,
        )

    assert len(result["batches"]) == 2
    assert result["batches"][0]["style_label"] == "variant_batch_1"
    assert result["batches"][1]["style_label"] == "variant_batch_2"


def test_run_variant_batches_empty_styles():
    """No variants → empty batches."""
    result = run_synthetic.run_variant_batches(
        [], set(), "x", "y", "z", "g", False,
    )
    assert result == {"batches": []}
