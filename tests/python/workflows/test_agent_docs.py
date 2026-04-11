#!/usr/bin/env python3
"""Tests for AGENTS/CLAUDE doc wiring."""

from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).parents[3]
AGENTS = ROOT / "AGENTS.md"
CLAUDE = ROOT / "CLAUDE.md"
AGENT_CI = ROOT / "docs" / "agent-ci.md"
CORPUS = ROOT / "docs" / "corpus-workflow.md"


def test_agents_and_claude_are_wired_correctly():
    text = AGENTS.read_text()
    assert "If `GITHUB_ACTIONS` is set" in text
    assert "docs/agent-ci.md" in text
    assert "docs/corpus-workflow.md" in text
    assert "cargo check                 # fast compile check" in text
    assert "cargo run -- -A .           # lint + autocorrect (all cops, including unsafe)" in text
    assert "cargo run -- --format json . # emit JSON diagnostics" in text
    assert "uv run ruff check ." in text
    assert "## Corpus Quick Reference" in text
    assert "Do not use `git diff` to discover changed files" in text
    assert "file-drop noise" in text


def test_claude_points_at_agents():
    assert CLAUDE.read_text().strip() == "@AGENTS.md"


def test_ci_doc_contains_required_rules():
    text = AGENT_CI.read_text()
    assert "Do not create extra branches or `git worktree`s." in text
    assert "Do not use `git stash`." in text
    assert "Do not revert the branch to `origin/main`" in text
    assert "Stay within the file scope implied by the workflow route." in text


def test_supporting_docs_exist():
    assert CORPUS.is_file()


if __name__ == "__main__":
    test_agents_and_claude_are_wired_correctly()
    test_claude_points_at_agents()
    test_ci_doc_contains_required_rules()
    test_supporting_docs_exist()
    print("All tests passed.")
