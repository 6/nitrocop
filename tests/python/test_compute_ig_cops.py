#!/usr/bin/env python3
"""Tests for bench/corpus/compute_ig_cops.py.

Guards the auto-derived include-gated cop list. A plugin bump that adds a
new non-**/ Include pattern should pick up the new cop automatically; this
test fails if the regex of department-level Include inheritance breaks.
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

SCRIPT = Path(__file__).parents[2] / "bench" / "corpus" / "compute_ig_cops.py"


def derive() -> set[str]:
    result = subprocess.run(
        [sys.executable, str(SCRIPT)],
        capture_output=True, text=True, check=True,
    )
    assert result.stdout.strip(), f"script produced no output: {result.stderr}"
    return set(result.stdout.strip().split(","))


def test_all_rake_cops_included_via_department_inherit():
    """The Rake department has `Include: [Rakefile, **/*.rake]` at the top
    level. `Rakefile` has no **/ prefix, so every Rake/* cop that doesn't
    override Include must inherit this and appear in the IG list."""
    cops = derive()
    assert "Rake/DuplicateTask" in cops
    assert "Rake/DuplicateNamespace" in cops
    assert "Rake/MethodDefinitionInTask" in cops
    assert "Rake/ClassDefinitionInTask" in cops
    assert "Rake/Desc" in cops


def test_rails_db_cops_included():
    """Rails cops with `db/**/*.rb` Include (no **/ prefix)."""
    cops = derive()
    assert "Rails/AddColumnIndex" in cops
    assert "Rails/BulkChangeTable" in cops
    assert "Rails/CreateTableWithTimestamps" in cops
    assert "Rails/Output" in cops


def test_rails_spec_test_cops_included():
    """Rails cops with `spec/**/*.rb`/`test/**/*.rb` Include."""
    cops = derive()
    assert "Rails/I18nLocaleAssignment" in cops
    assert "Rails/TimeZoneAssignment" in cops
    assert "Rails/HttpPositionalArguments" in cops


def test_glob_prefixed_cops_excluded():
    """Cops whose Include starts with `**/` resolve correctly in the main
    pipeline and should NOT be in the IG list."""
    cops = derive()
    assert "Rails/ActionControllerTestCase" not in cops  # **/test/**/*.rb
    assert "Rails/EnumSyntax" not in cops  # **/app/models/**/*.rb
    assert "Rails/HttpStatusNameConsistency" not in cops  # **/app/controllers/**/*.rb


def test_core_rubocop_parses_despite_ruby_tags():
    """rubocop-core's default.yml has `!ruby/regexp` nodes. The derivation
    script must not crash on them — it uses a loader that ignores unknown
    Ruby-specific tags."""
    result = subprocess.run(
        [sys.executable, str(SCRIPT)],
        capture_output=True, text=True,
    )
    assert result.returncode == 0
    assert "warning: failed to parse" not in result.stderr
