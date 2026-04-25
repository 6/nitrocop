#!/usr/bin/env python3
"""Tests for exclude_agent_context.py."""

from __future__ import annotations

import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parents[3] / "scripts" / "workflows"))
import exclude_agent_context


def test_configure_claude_creates_settings(tmp_path: Path):
    claude_dir = tmp_path / ".claude"
    claude_dir.mkdir()
    cfg = exclude_agent_context.configure_claude(tmp_path)

    settings = json.loads((claude_dir / "settings.local.json").read_text())
    assert settings["claudeMdExcludes"] == ["**/CLAUDE.md", "**/AGENTS.md"]
    assert "Skill" in settings["permissions"]["deny"]
    assert cfg == settings


def test_configure_claude_preserves_existing(tmp_path: Path):
    claude_dir = tmp_path / ".claude"
    claude_dir.mkdir()
    existing = {"permissions": {"allow": ["Bash(cargo:*)"], "deny": ["WebSearch"]}}
    (claude_dir / "settings.local.json").write_text(json.dumps(existing))

    cfg = exclude_agent_context.configure_claude(tmp_path)
    assert cfg["permissions"]["allow"] == ["Bash(cargo:*)"]
    assert "WebSearch" in cfg["permissions"]["deny"]
    assert "Skill" in cfg["permissions"]["deny"]


def test_configure_claude_idempotent(tmp_path: Path):
    claude_dir = tmp_path / ".claude"
    claude_dir.mkdir()
    exclude_agent_context.configure_claude(tmp_path)
    exclude_agent_context.configure_claude(tmp_path)

    settings = json.loads((claude_dir / "settings.local.json").read_text())
    assert settings["permissions"]["deny"].count("Skill") == 1
    # Hook should also not be duplicated
    bash_hooks = [
        h for h in settings["hooks"]["PreToolUse"] if h["matcher"] == "Bash"
    ]
    assert len(bash_hooks) == 1


def test_configure_claude_adds_pretooluse_hook(tmp_path: Path):
    claude_dir = tmp_path / ".claude"
    claude_dir.mkdir()
    cfg = exclude_agent_context.configure_claude(tmp_path)

    assert "hooks" in cfg
    hooks = cfg["hooks"]["PreToolUse"]
    assert len(hooks) == 1
    assert hooks[0]["matcher"] == "Bash"
    sub = hooks[0]["hooks"][0]
    assert "deny-git-commit-push.sh" in sub["command"]
    assert sub["timeout"] == 5


def test_configure_codex_disables_skills(tmp_path: Path):
    skills_dir = tmp_path / ".agents" / "skills"
    for name in ["fix-department", "triage"]:
        d = skills_dir / name
        d.mkdir(parents=True)
        (d / "SKILL.md").write_text(f"# {name}\n")

    codex_home = tmp_path / "codex-home"
    count = exclude_agent_context.configure_codex(tmp_path, codex_home=codex_home)

    assert count == 2
    cfg_text = (codex_home / "config.toml").read_text()
    assert cfg_text.count("[[skills.config]]") == 2
    assert "enabled = false" in cfg_text
    assert "fix-department" in cfg_text
    assert "triage" in cfg_text


def test_configure_codex_preserves_existing_config(tmp_path: Path):
    skills_dir = tmp_path / ".agents" / "skills" / "test-skill"
    skills_dir.mkdir(parents=True)
    (skills_dir / "SKILL.md").write_text("# test\n")

    codex_home = tmp_path / "codex-home"
    codex_home.mkdir()
    (codex_home / "config.toml").write_text('model = "gpt-5.5"\n')

    exclude_agent_context.configure_codex(tmp_path, codex_home=codex_home)

    cfg_text = (codex_home / "config.toml").read_text()
    assert 'model = "gpt-5.5"' in cfg_text
    assert "[[skills.config]]" in cfg_text


def test_configure_codex_no_skills_dir(tmp_path: Path):
    codex_home = tmp_path / "codex-home"
    count = exclude_agent_context.configure_codex(tmp_path, codex_home=codex_home)

    assert count == 0
    assert not (codex_home / "config.toml").exists()


def test_configure_codex_empty_skills_dir(tmp_path: Path):
    (tmp_path / ".agents" / "skills").mkdir(parents=True)
    codex_home = tmp_path / "codex-home"
    count = exclude_agent_context.configure_codex(tmp_path, codex_home=codex_home)

    assert count == 0
