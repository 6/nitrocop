#!/usr/bin/env python3
"""Resolve agent backend name to CLI, env vars, and log config.

Backend names map to a CLI tool and its configuration. Multiple backends
can share the same CLI (for example, both Codex variants use Codex CLI).

Usage:
    python3 resolve_backend.py <backend>

Outputs KEY=VALUE lines suitable for sourcing in shell or appending to
$GITHUB_OUTPUT. All values are shell-safe (no quoting needed).
"""
import os
import sys


def codex_backend(model: str, reasoning_effort: str, strength: str) -> dict:
    return {
        "family": "codex",
        "strength": strength,
        "model": model,
        "reasoning_effort": reasoning_effort,
        "display_label": f"codex / {strength}",
        "model_label": f"{model} ({reasoning_effort})",
        "cli": "codex",
        "setup_cmd": (
            'python3 scripts/workflows/guard_backend_secrets.py '
            '--from-env CODEX_AUTH_JSON '
            'emit-masks && '
            'python3 scripts/workflows/validate_codex_auth.py '
            '--from-env CODEX_AUTH_JSON '
            '--max-age-days 7 && '
            'npm install -g @openai/codex@latest && '
            'mkdir -p ~/.codex && '
            'chmod 700 ~/.codex && '
            'printf \'%s\' "$CODEX_AUTH_JSON" > ~/.codex/auth.json && '
            'chmod 600 ~/.codex/auth.json'
        ),
        "log_format": "codex",
        "log_pattern": "~/.codex/sessions/**/*.jsonl",
        "run_cmd": (
            f'( codex exec --dangerously-bypass-approvals-and-sandbox -m {model} '
            f'-c model_reasoning_effort={reasoning_effort} '
            '-c project_doc_max_bytes=0 '
            '--json '
            '-o "$AGENT_LAST_MESSAGE_FILE" '
            '- < "$FINAL_TASK_FILE" '
            '> "$AGENT_EVENTS_FILE" '
            '2> >(tee "$AGENT_LOG_FILE" >&2); '
            'STATUS=$?; '
            'python3 scripts/workflows/agent_logs.py summarize '
            '"$AGENT_EVENTS_FILE" '
            '"$AGENT_LAST_MESSAGE_FILE" '
            '> "$AGENT_RESULT_FILE" || true; '
            'exit $STATUS ) || true'
        ),
        "env": {},
        "secrets": {
            "CODEX_AUTH_JSON": "CODEX_AUTH_JSON",
        },
    }


MINIMAX_BACKEND = {
    "family": "minimax",
    "strength": "normal",
    "model": "MiniMax-M2.7",
    "reasoning_effort": "",
    "display_label": "minimax / normal",
    "model_label": "MiniMax-M2.7",
    "cli": "claude",
    "setup_cmd": (
        'python3 scripts/workflows/guard_backend_secrets.py '
        '--from-env ANTHROPIC_AUTH_TOKEN '
        'emit-masks && '
        'curl -fsSL https://claude.ai/install.sh | bash'
    ),
    "log_format": "claude",
    "log_pattern": "~/.claude/projects/**/*.jsonl",
    "run_cmd": (
        'claude -p --dangerously-skip-permissions '
        '--output-format json '
        '"$(cat "$FINAL_TASK_FILE")" '
        '> "$AGENT_RESULT_FILE" '
        '2> >(tee "$AGENT_LOG_FILE" >&2) || true'
    ),
    "env": {
        "ANTHROPIC_BASE_URL": "https://api.minimax.io/anthropic",
        "ANTHROPIC_MODEL": "MiniMax-M2.7",
        "ANTHROPIC_SMALL_FAST_MODEL": "MiniMax-M2.7",
        "API_TIMEOUT_MS": "300000",
        "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC": "1",
    },
    "secrets": {
        "ANTHROPIC_AUTH_TOKEN": "ANTHROPIC_AUTH_TOKEN",
    },
}


def claude_backend(strength: str, effort: str, display_label: str, model_label: str) -> dict:
    return {
        "family": "claude",
        "strength": strength,
        "model": "claude-opus-4-7",
        "reasoning_effort": effort,
        "display_label": display_label,
        "model_label": model_label,
        "cli": "claude",
        "setup_cmd": (
            'python3 scripts/workflows/guard_backend_secrets.py '
            '--from-env ANTHROPIC_API_KEY '
            'emit-masks && '
            'curl -fsSL https://claude.ai/install.sh | bash'
        ),
        "log_format": "claude",
        "log_pattern": "~/.claude/projects/**/*.jsonl",
        "run_cmd": (
            f'claude -p --dangerously-skip-permissions '
            f'--model claude-opus-4-7 --effort {effort} '
            f'--output-format json '
            f'"$(cat "$FINAL_TASK_FILE")" '
            f'> "$AGENT_RESULT_FILE" '
            f'2> >(tee "$AGENT_LOG_FILE" >&2) || true'
        ),
        "env": {
            "ANTHROPIC_MODEL": "claude-opus-4-7",
            "API_TIMEOUT_MS": "300000",
            "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC": "1",
        },
        "secrets": {
            "ANTHROPIC_API_KEY": "ANTHROPIC_API_KEY",
        },
    }


CLAUDE_NORMAL_BACKEND = claude_backend("normal", "high", "claude / normal", "Claude Opus 4.7 (high)")
CLAUDE_HARD_BACKEND = claude_backend("hard", "xhigh", "claude / hard", "Claude Opus 4.7 (xhigh)")


def claude_oauth_backend(strength: str, effort: str, display_label: str, model_label: str) -> dict:
    return {
        "family": "claude-oauth",
        "strength": strength,
        "model": "claude-opus-4-7",
        "reasoning_effort": effort,
        "display_label": display_label,
        "model_label": model_label,
        "cli": "claude-action",
        "action": True,
        "setup_cmd": (
            'python3 scripts/workflows/guard_backend_secrets.py '
            '--from-env CLAUDE_CODE_OAUTH_TOKEN '
            'emit-masks'
        ),
        "log_format": "claude",
        "log_pattern": "~/.claude/projects/**/*.jsonl",
        "run_cmd": "",
        "env": {
            "ANTHROPIC_MODEL": "claude-opus-4-7",
            "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC": "1",
        },
        "secrets": {
            "CLAUDE_CODE_OAUTH_TOKEN": "CLAUDE_CODE_OAUTH_TOKEN",
        },
    }


CODEX_54_HIGH_BACKEND = codex_backend("gpt-5.4", "high", "normal")
CODEX_54_XHIGH_BACKEND = codex_backend("gpt-5.4", "xhigh", "hard")

CLAUDE_OAUTH_NORMAL_BACKEND = claude_oauth_backend(
    "normal", "high", "claude-oauth / normal", "Claude Opus 4.7 (OAuth, high)",
)
CLAUDE_OAUTH_HARD_BACKEND = claude_oauth_backend(
    "hard", "xhigh", "claude-oauth / hard", "Claude Opus 4.7 (OAuth, xhigh)",
)


BACKENDS = {
    "minimax": MINIMAX_BACKEND,
    "claude-normal": CLAUDE_NORMAL_BACKEND,
    "claude-hard": CLAUDE_HARD_BACKEND,
    "codex-normal": CODEX_54_HIGH_BACKEND,
    "codex-hard": CODEX_54_XHIGH_BACKEND,
    "claude-oauth-normal": CLAUDE_OAUTH_NORMAL_BACKEND,
    "claude-oauth-hard": CLAUDE_OAUTH_HARD_BACKEND,
}


def choose_backend(family: str, strength: str, default_strength: str = "hard") -> tuple[str, str, str]:
    if family == "auto":
        raise ValueError("auto family cannot be resolved without external routing")

    if family == "minimax":
        return "minimax", "normal", "minimax ignores strength and uses its single supported model"

    resolved_strength = default_strength if strength == "auto" else strength
    if resolved_strength not in {"normal", "hard"}:
        raise ValueError(f"unknown strength: {strength}")

    if family == "codex":
        backend = "codex-hard" if resolved_strength == "hard" else "codex-normal"
        return backend, resolved_strength, f"workflow override ({family}/{resolved_strength})"

    if family in ("claude", "claude-oauth"):
        # Autodetect: prefer OAuth token, fall back to API key
        has_oauth = bool(os.environ.get("CLAUDE_CODE_OAUTH_TOKEN"))
        has_api_key = bool(os.environ.get("ANTHROPIC_API_KEY"))
        if has_oauth or family == "claude-oauth":
            backend = "claude-oauth-hard" if resolved_strength == "hard" else "claude-oauth-normal"
            method = "oauth"
        elif has_api_key:
            backend = "claude-hard" if resolved_strength == "hard" else "claude-normal"
            method = "api-key"
        else:
            backend = "claude-oauth-hard" if resolved_strength == "hard" else "claude-oauth-normal"
            method = "oauth (default, no secret detected at resolve time)"
        return backend, resolved_strength, f"claude/{resolved_strength} via {method}"

    raise ValueError(f"unknown backend family: {family}")


def resolve(backend: str) -> dict:
    """Resolve a backend name to its full config."""
    if backend not in BACKENDS:
        print(f"Unknown backend: {backend}", file=sys.stderr)
        print(f"Available: {', '.join(BACKENDS)}", file=sys.stderr)
        sys.exit(1)
    return BACKENDS[backend]


def main():
    if len(sys.argv) >= 2 and sys.argv[1] == "choose":
        if len(sys.argv) not in {4, 5}:
            print(f"Usage: {sys.argv[0]} choose <family> <strength> [default_strength]", file=sys.stderr)
            sys.exit(1)
        family = sys.argv[2]
        strength = sys.argv[3]
        default_strength = sys.argv[4] if len(sys.argv) == 5 else "normal"
        backend, resolved_strength, reason = choose_backend(family, strength, default_strength)
        config = resolve(backend)
        print(f"backend={backend}")
        print(f"family={config['family']}")
        print(f"strength={resolved_strength}")
        print(f"display_label={config['display_label']}")
        print(f"model_label={config['model_label']}")
        print(f"reason={reason}")
        return

    if len(sys.argv) != 2:
        print(f"Usage: {sys.argv[0]} <backend>", file=sys.stderr)
        sys.exit(1)

    backend = sys.argv[1]
    config = resolve(backend)

    # Output key=value pairs
    print(f"cli={config['cli']}")
    print(f"setup_cmd={config['setup_cmd']}")
    print(f"log_format={config['log_format']}")
    print(f"log_pattern={config['log_pattern']}")
    print(f"run_cmd={config['run_cmd']}")
    print(f"family={config['family']}")
    print(f"strength={config['strength']}")
    print(f"model={config['model']}")
    print(f"reasoning_effort={config['reasoning_effort']}")
    print(f"display_label={config['display_label']}")
    print(f"model_label={config['model_label']}")
    print(f"action={'true' if config.get('action') else 'false'}")

    # Output env vars
    for key, val in config["env"].items():
        print(f"env_{key}={val}")

    # Output secret mappings
    for secret_name, env_var in config["secrets"].items():
        print(f"secret_{secret_name}={env_var}")


if __name__ == "__main__":
    main()
