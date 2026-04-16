#!/usr/bin/env python3
"""Tests for resolve_backend.py."""
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parents[3] / "scripts" / "workflows"))
import resolve_backend

SCRIPT = Path(__file__).parents[3] / "scripts" / "workflows" / "resolve_backend.py"


def test_all_backends_resolve():
    """Every registered backend should resolve without error."""
    for name in resolve_backend.BACKENDS:
        config = resolve_backend.resolve(name)
        assert config["cli"] in ("claude", "codex", "claude-action"), f"{name}: unexpected cli"
        assert config["log_format"] in ("claude", "codex"), f"{name}: unexpected log_format"
        assert config["setup_cmd"], f"{name}: missing setup_cmd"
        if not config.get("action"):
            assert config["run_cmd"], f"{name}: missing run_cmd"
        assert config["log_pattern"], f"{name}: missing log_pattern"


def test_codex_normal_uses_codex():
    config = resolve_backend.resolve("codex-normal")
    assert config["cli"] == "codex"
    assert config["log_format"] == "codex"
    assert "guard_backend_secrets.py" in config["setup_cmd"]
    assert "validate_codex_auth.py" in config["setup_cmd"]
    assert "CODEX_AUTH_JSON" in config["setup_cmd"]
    assert "-m gpt-5.4" in config["run_cmd"]
    assert "model_reasoning_effort=high" in config["run_cmd"]


def test_minimax_uses_claude():
    config = resolve_backend.resolve("minimax")
    assert config["cli"] == "claude"
    assert config["log_format"] == "claude"
    assert "ANTHROPIC_BASE_URL" in config["env"]
    assert "guard_backend_secrets.py" in config["setup_cmd"]
    assert "ANTHROPIC_AUTH_TOKEN" in config["setup_cmd"]
    assert "claude.ai/install.sh" in config["setup_cmd"]


def test_choose_backend_outputs_family_strength_and_labels():
    result = subprocess.run(
        [sys.executable, str(SCRIPT), "choose", "codex", "normal"],
        capture_output=True, text=True,
    )
    assert result.returncode == 0
    fields = dict(line.split("=", 1) for line in result.stdout.strip().splitlines())
    assert fields["backend"] == "codex-normal"
    assert fields["family"] == "codex"
    assert fields["strength"] == "normal"
    assert fields["display_label"] == "codex / normal"
    assert fields["model_label"] == "gpt-5.4 (high)"


def test_claude_normal_uses_claude():
    config = resolve_backend.resolve("claude-normal")
    assert config["cli"] == "claude"
    assert config["log_format"] == "claude"
    assert "ANTHROPIC_BASE_URL" not in config["env"]
    assert "guard_backend_secrets.py" in config["setup_cmd"]
    assert "ANTHROPIC_API_KEY" in config["setup_cmd"]
    assert "claude.ai/install.sh" in config["setup_cmd"]


def test_codex_uses_codex():
    config = resolve_backend.resolve("codex-hard")
    assert config["cli"] == "codex"
    assert config["log_format"] == "codex"
    assert "CODEX_AUTH_JSON" in config["secrets"]
    assert "guard_backend_secrets.py" in config["setup_cmd"]
    assert "validate_codex_auth.py" in config["setup_cmd"]
    assert "@openai/codex@latest" in config["setup_cmd"]
    assert "chmod 700 ~/.codex" in config["setup_cmd"]
    assert "chmod 600 ~/.codex/auth.json" in config["setup_cmd"]
    assert "--dangerously-bypass-approvals-and-sandbox" in config["run_cmd"]
    assert "--json" in config["run_cmd"]
    assert '"$AGENT_EVENTS_FILE"' in config["run_cmd"]
    assert '"$FINAL_TASK_FILE"' in config["run_cmd"]
    assert '"$AGENT_RESULT_FILE"' in config["run_cmd"]
    assert "scripts/workflows/agent_logs.py summarize" in config["run_cmd"]


def test_claude_oauth_normal():
    config = resolve_backend.resolve("claude-oauth-normal")
    assert config["cli"] == "claude-action"
    assert config["action"] is True
    assert config["log_format"] == "claude"
    assert config["run_cmd"] == ""
    assert "CLAUDE_CODE_OAUTH_TOKEN" in config["setup_cmd"]
    assert "ANTHROPIC_API_KEY" not in config["setup_cmd"]
    assert "claude.ai/install.sh" not in config["setup_cmd"]
    assert config["env"]["ANTHROPIC_MODEL"] == "claude-opus-4-7"
    assert config["reasoning_effort"] == "high"
    assert "CLAUDE_CODE_OAUTH_TOKEN" in config["secrets"]


def test_claude_oauth_hard():
    config = resolve_backend.resolve("claude-oauth-hard")
    assert config["cli"] == "claude-action"
    assert config["action"] is True
    assert config["env"]["ANTHROPIC_MODEL"] == "claude-opus-4-7"
    assert config["reasoning_effort"] == "xhigh"
    assert "Opus 4.7" in config["model_label"]


def test_choose_claude_oauth():
    backend, strength, reason = resolve_backend.choose_backend("claude-oauth", "normal")
    assert backend == "claude-oauth-normal"
    assert strength == "normal"

    backend, strength, reason = resolve_backend.choose_backend("claude-oauth", "hard")
    assert backend == "claude-oauth-hard"
    assert strength == "hard"


def test_cli_output_includes_action_flag():
    """CLI should emit action=true for oauth backends, action=false for others."""
    result = subprocess.run(
        [sys.executable, str(SCRIPT), "claude-oauth-normal"],
        capture_output=True, text=True,
    )
    assert result.returncode == 0
    fields = dict(line.split("=", 1) for line in result.stdout.strip().splitlines())
    assert fields["action"] == "true"

    result = subprocess.run(
        [sys.executable, str(SCRIPT), "claude-normal"],
        capture_output=True, text=True,
    )
    assert result.returncode == 0
    fields = dict(line.split("=", 1) for line in result.stdout.strip().splitlines())
    assert fields["action"] == "false"


def test_unknown_backend_exits():
    result = subprocess.run(
        [sys.executable, str(SCRIPT), "unknown"],
        capture_output=True, text=True,
    )
    assert result.returncode != 0
    assert "Unknown backend" in result.stderr


def test_cli_output_format():
    """CLI should output key=value pairs."""
    result = subprocess.run(
        [sys.executable, str(SCRIPT), "codex-normal"],
        capture_output=True, text=True,
    )
    assert result.returncode == 0
    lines = result.stdout.strip().split("\n")
    for line in lines:
        assert "=" in line, f"Line missing '=': {line}"
    keys = [line_value.split("=", 1)[0] for line_value in lines]
    assert "cli" in keys
    assert "setup_cmd" in keys
    assert "log_format" in keys
    assert "run_cmd" in keys


def test_no_args_exits():
    result = subprocess.run(
        [sys.executable, str(SCRIPT)],
        capture_output=True, text=True,
    )
    assert result.returncode != 0


if __name__ == "__main__":
    test_all_backends_resolve()
    test_codex_normal_uses_codex()
    test_minimax_uses_claude()
    test_claude_normal_uses_claude()
    test_claude_oauth_normal()
    test_claude_oauth_hard()
    test_choose_claude_oauth()
    test_cli_output_includes_action_flag()
    test_choose_backend_outputs_family_strength_and_labels()
    test_codex_uses_codex()
    test_unknown_backend_exits()
    test_cli_output_format()
    test_no_args_exits()
    print("All tests passed.")
