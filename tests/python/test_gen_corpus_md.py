#!/usr/bin/env python3
"""Tests for bench/corpus/gen_corpus_md.py."""

import importlib.util
from pathlib import Path

SCRIPT = Path(__file__).parents[2] / "bench" / "corpus" / "gen_corpus_md.py"
SPEC = importlib.util.spec_from_file_location("gen_corpus_md", SCRIPT)
assert SPEC and SPEC.loader
gen_corpus_md = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(gen_corpus_md)


def test_generate_md_basic():
    """Generates valid markdown from minimal corpus data."""
    data = {
        "summary": {
            "total_repos": 10, "repos_perfect": 5, "repos_error": 0,
            "total_offenses_compared": 100, "matches": 95, "fp": 3, "fn": 2,
            "registered_cops": 5, "perfect_cops": 3, "diverging_cops": 2,
            "inactive_cops": 0, "overall_match_rate": 0.95,
            "total_files_inspected": 500, "rubocop_files_dropped": 0,
        },
        "by_department": [{
            "department": "Style", "cops": 5, "perfect_cops": 3,
            "diverging_cops": 2, "inactive_cops": 0,
            "matches": 95, "fp": 3, "fn": 2, "match_rate": 0.95,
        }],
        "by_cop": [
            {"cop": "Style/Foo", "matches": 50, "fp": 3, "fn": 0,
             "match_rate": 0.9433, "exercised": True, "perfect_match": False,
             "diverging": True, "fp_examples": ["repo_a: a.rb:1"], "fn_examples": []},
            {"cop": "Style/Bar", "matches": 45, "fp": 0, "fn": 2,
             "match_rate": 0.9574, "exercised": True, "perfect_match": False,
             "diverging": True, "fp_examples": [], "fn_examples": ["repo_a: b.rb:5"]},
        ],
        "by_repo": [{"repo": "repo_a", "status": "ok", "match_rate": 0.95,
                      "matches": 95, "fp": 3, "fn": 2, "files_inspected": 50}],
        "by_repo_cop": {},
    }
    md = gen_corpus_md.generate_md(data, {})
    assert "# Corpus Oracle Results" in md
    assert "## Summary" in md
    assert "## Diverging Cops" in md
    assert "Style/Foo" in md
    assert "Match rate (default config)" in md


def test_generate_md_with_variants():
    """Variant data adds variant rows and match rate."""
    data = {
        "summary": {
            "total_repos": 10, "repos_perfect": 5, "repos_error": 0,
            "total_offenses_compared": 100, "matches": 100, "fp": 0, "fn": 0,
            "registered_cops": 1, "perfect_cops": 1, "diverging_cops": 0,
            "inactive_cops": 0, "overall_match_rate": 1.0,
            "total_files_inspected": 500, "rubocop_files_dropped": 0,
        },
        "by_department": [{
            "department": "Style", "cops": 1, "perfect_cops": 1,
            "diverging_cops": 0, "inactive_cops": 0,
            "matches": 100, "fp": 0, "fn": 0, "match_rate": 1.0,
        }],
        "by_cop": [
            {"cop": "Style/Foo", "matches": 100, "fp": 0, "fn": 0,
             "match_rate": 1.0, "exercised": True, "perfect_match": True,
             "diverging": False, "fp_examples": [], "fn_examples": []},
        ],
        "by_repo": [], "by_repo_cop": {},
    }
    variant_by_cop = {
        "Style/Foo": [
            {"style_label": "bar", "matches": 80, "fp": 10, "fn": 5},
        ],
    }
    md = gen_corpus_md.generate_md(data, variant_by_cop)
    assert "Match rate (all variants)" in md
    assert "### All Variants" in md
    assert "All variants %" in md
    assert "Variant-only divergence" in md
    assert "Style/Foo (bar)" in md


def test_generate_md_no_variant_column_without_data():
    """Without variant data, no variant column appears."""
    data = {
        "summary": {
            "total_repos": 1, "repos_perfect": 1, "repos_error": 0,
            "total_offenses_compared": 10, "matches": 10, "fp": 0, "fn": 0,
            "registered_cops": 1, "perfect_cops": 1, "diverging_cops": 0,
            "inactive_cops": 0, "overall_match_rate": 1.0,
            "total_files_inspected": 5, "rubocop_files_dropped": 0,
        },
        "by_department": [{
            "department": "Style", "cops": 1, "perfect_cops": 1,
            "diverging_cops": 0, "inactive_cops": 0,
            "matches": 10, "fp": 0, "fn": 0, "match_rate": 1.0,
        }],
        "by_cop": [], "by_repo": [], "by_repo_cop": {},
    }
    md = gen_corpus_md.generate_md(data, {})
    assert "All variants %" not in md
    assert "Match %" in md
