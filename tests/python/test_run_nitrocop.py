#!/usr/bin/env python3
"""Tests for bench/corpus/run_nitrocop.py."""

import importlib.util
import sys
import tempfile
from pathlib import Path

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
