#!/usr/bin/env python3
"""Tests for bench/corpus/run_nitrocop.py."""

import importlib.util
import subprocess
import sys
import tempfile
from pathlib import Path
from unittest import mock

SCRIPT = Path(__file__).parents[2] / "bench" / "corpus" / "run_nitrocop.py"
sys.path.insert(0, str(SCRIPT.parent))
SPEC = importlib.util.spec_from_file_location("run_nitrocop", SCRIPT)
assert SPEC and SPEC.loader
run_nitrocop = importlib.util.module_from_spec(SPEC)
sys.modules["run_nitrocop"] = run_nitrocop
SPEC.loader.exec_module(run_nitrocop)


def test_normalize_offenses_deduplicates_symlink_paths():
    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        repo = tmp / "repo"
        real_dir = repo / "plugins"
        alias_dir = repo / "baseplugins"
        real_dir.mkdir(parents=True)
        alias_dir.symlink_to("plugins")

        real_file = real_dir / "statistics_block_test.rb"
        real_file.write_text("[]\n")
        symlink_file = alias_dir / "statistics_block_test.rb"

        offenses = [
            {"path": str(symlink_file), "line": 4, "cop_name": "Style/WordArray"},
            {"path": str(real_file), "line": 4, "cop_name": "Style/WordArray"},
            {"path": str(real_file), "line": 11, "cop_name": "Style/WordArray"},
        ]

        normalized = run_nitrocop.normalize_offenses(offenses)

        assert normalized == [
            {
                "path": str(real_file.resolve()),
                "line": 4,
                "cop_name": "Style/WordArray",
            },
            {
                "path": str(real_file.resolve()),
                "line": 11,
                "cop_name": "Style/WordArray",
            },
        ]


def test_normalize_offenses_collapses_multi_column_same_line():
    """Multiple offenses at different columns on the same line collapse to one."""
    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        repo = tmp / "repo"
        repo.mkdir(parents=True)
        file = repo / "test.rb"
        file.write_text("def foo(a, b)\nend\n")

        offenses = [
            {"path": str(file), "line": 1, "cop_name": "Naming/MethodParameterName", "column": 9},
            {"path": str(file), "line": 1, "cop_name": "Naming/MethodParameterName", "column": 12},
        ]

        normalized = run_nitrocop.normalize_offenses(offenses)

        assert len(normalized) == 1, (
            "Same (path, line, cop) should collapse regardless of column"
        )


# ---------- resolve_repo_config ----------


def test_resolve_repo_config_default_falls_back_to_baseline():
    """When gen_repo_config.py fails, fall back to BASELINE_CONFIG."""
    with mock.patch("subprocess.run", side_effect=FileNotFoundError):
        result = run_nitrocop.resolve_repo_config("fake_repo", "/tmp/fake")
    assert result == str(run_nitrocop.BASELINE_CONFIG)


def test_resolve_repo_config_base_config_used_as_fallback():
    """When base_config is given and gen_repo_config.py fails, fall back to it."""
    custom = "/tmp/my_variant_config.yml"
    with mock.patch("subprocess.run", side_effect=FileNotFoundError):
        result = run_nitrocop.resolve_repo_config(
            "fake_repo", "/tmp/fake", base_config=custom,
        )
    assert result == custom


def test_resolve_repo_config_passes_base_config_to_subprocess():
    """When base_config is given, it's forwarded to gen_repo_config.py."""
    custom = "/tmp/my_variant_config.yml"
    fake_result = subprocess.CompletedProcess(
        args=[], returncode=0, stdout="/tmp/generated_overlay.yml\n",
    )
    with mock.patch("subprocess.run", return_value=fake_result) as mock_run:
        result = run_nitrocop.resolve_repo_config(
            "my_repo", "/tmp/my_repo", base_config=custom,
        )
    # The third positional arg to gen_repo_config.py should be our custom base
    call_args = mock_run.call_args[0][0]  # first positional arg = command list
    assert custom in call_args, (
        f"Expected base_config '{custom}' in subprocess args: {call_args}"
    )
    assert result == "/tmp/generated_overlay.yml"


def test_resolve_repo_config_without_base_config_uses_baseline():
    """Without base_config, BASELINE_CONFIG is passed to gen_repo_config.py."""
    fake_result = subprocess.CompletedProcess(
        args=[], returncode=0, stdout="/tmp/generated_overlay.yml\n",
    )
    with mock.patch("subprocess.run", return_value=fake_result) as mock_run:
        run_nitrocop.resolve_repo_config("my_repo", "/tmp/my_repo")
    call_args = mock_run.call_args[0][0]
    assert str(run_nitrocop.BASELINE_CONFIG) in call_args
