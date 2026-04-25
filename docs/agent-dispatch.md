# Agent Dispatch for Corpus Conformance (Codex)

Automated system for fixing corpus conformance gaps by dispatching Codex agents to fix one cop at a time. The current flow is issue-backed: sync one tracker issue per diverging cop from the corpus, then fill a bounded queue of those issues into `agent-cop-fix`. Each cop runs in a GitHub Actions runner with Codex CLI, which edits the code, validates with `cargo test`, and opens a PR.

The recommended workflow is Codex-first:
- `gpt-5.5` with `high` effort for `difficulty:simple` initial cop-fix issues
- `gpt-5.5` with `xhigh` effort for `difficulty:medium|complex`, retries, and PR repairs
- Manual overrides for `claude` and `minimax` still exist for experiments.

## Architecture

```
You (any machine with gh CLI)
  │
  │  gh workflow run cop-issue-sync.yml
  │  gh workflow run cop-issue-dispatch.yml -f max_active=5
  ▼
GitHub Actions (agent-cop-fix.yml)
  │  1. Checkout repo + build Rust (cached, ~1 min)
  │  2. dispatch_cops.py task → self-contained task prompt
  │  3. codex exec --dangerously-bypass-approvals-and-sandbox
  │     → auto-routed to gpt-5.5 (high or xhigh effort)
  │  4. cargo test --lib → validate the fix compiles + tests pass
  │  5. Commit, push branch, open PR
  ▼
GitHub CI (on the PR)
  │  checks.yml: clippy, full test suite, corpus smoke test
  │  agent-cop-check.yml: per-cop count check vs corpus baseline
  ▼
You review + merge
```

No external cloud service needed. Everything runs in GHA with Codex CLI as the AI agent.

## One-Time Setup

### Step 1: Codex Auth Credentials

The workflow uses your ChatGPT Pro plan (flat rate, no per-token billing).

1. Install Codex CLI locally: `npm install -g @openai/codex@latest`
2. Run `codex login` — this opens a browser for ChatGPT OAuth
3. Copy the auth file: `cat ~/.codex/auth.json`
4. The content of this file becomes your `CODEX_AUTH_JSON` secret

Codex automatically refreshes tokens during normal runs, but GHA runners are ephemeral so the refreshed `auth.json` is lost. Use `scripts/refresh_codex_auth.sh` to rotate tokens locally and push the updated secret:

```bash
# First time: log in with the CI ChatGPT account
scripts/refresh_codex_auth.sh login

# Later: refresh stale tokens
scripts/refresh_codex_auth.sh refresh
```

The script uses `~/.codex-gha` as a dedicated `CODEX_HOME` so the CI account's auth never collides with your personal `~/.codex`. Override with `CODEX_GHA_HOME`.

**Important:** Use a dedicated ChatGPT subscription for CI dispatch — do not share with your personal Codex usage. Token refreshes from concurrent sessions will conflict and invalidate each other. A separate ChatGPT Plus ($20/mo) or Pro ($200/mo) account for CI keeps things clean.

**Usage limits:** ChatGPT Pro allows 300-1500 messages per 5-hour window. Each cop fix uses ~10-30 internal messages. Dispatch in small batches (5-10 cops at a time) and monitor usage at [chatgpt.com/codex/settings/usage](https://chatgpt.com/codex/settings/usage).

**This is a supported use case.** OpenAI explicitly documents running Codex in CI/CD:
- [Non-interactive mode](https://developers.openai.com/codex/noninteractive) — `codex exec` is "designed for CI/CD jobs"
- [CI/CD auth guide](https://developers.openai.com/codex/auth/ci-cd-auth) — shows how to use ChatGPT-managed auth on CI runners
- [Auth docs](https://developers.openai.com/codex/auth) — documents both API key and ChatGPT OAuth auth methods

### Step 2: GitHub Repository Secrets

Go to **Settings > Secrets and variables > Actions** and add:

| Secret | Value |
|--------|-------|
| `CODEX_AUTH_JSON` | Contents of `~/.codex/auth.json` from Step 1 |

### Step 3: Branch Protection (optional)

To prevent accidental merges of bad fixes:

Go to **Settings > Rules > Rulesets > New ruleset**:

- Name: `agent-pr-required`
- Enforcement: Active
- Targets: `main` branch
- Bypass list: Add your GitHub username
- Rules:
  - [x] Require status checks to pass (add `build-and-test`)

## Operator Workflow

### Phase 1: Triage / Issue Sync (5 min)

```bash
python3 scripts/dispatch_cops.py tiers
gh workflow run cop-issue-sync.yml
```

### Phase 2: Dispatch

```bash
# Dry run the bounded queue first
gh workflow run cop-issue-dispatch.yml -f max_active=5 -f dry_run=true

# Dispatch backlog issues into agent-cop-fix
gh workflow run cop-issue-dispatch.yml -f max_active=5

# Or force one Codex model across the queue
gh workflow run cop-issue-dispatch.yml -f max_active=5 -f backend_override=codex -f strength_override=normal
gh workflow run cop-issue-dispatch.yml -f max_active=5 -f backend_override=codex -f strength_override=hard
```

Wait ~10-15 min (build + Codex agent + validation). Check the PR:

```bash
gh pr list --search "Fix in:title" --state open
```

**What to verify:**
- Does CI pass?
- Is the fix correct (read the diff)?
- Did it add a test case + doc comment?

### Phase 3: Retry Failures

```bash
gh workflow run agent-cop-fix.yml -f cop="Style/VariableInterpolation" -f mode=retry
gh workflow run agent-cop-fix.yml -f cop="Style/VariableInterpolation" \
  -f mode=retry -f extra_context="The FN is a global variable interpolation"
```

Retry mode auto-discovers all prior failed PRs, includes their diffs and CI failure logs in the prompt, and closes stale PRs before dispatching. `agent-pr-repair.yml` also reacts automatically to failed deterministic PR checks.

### Phase 4: Validate

After merging ~20-50 PRs:

```bash
gh workflow run corpus-oracle.yml
```

## How It Works

### Task Packet

`dispatch_cops.py task` produces a markdown prompt containing:
- Focused instructions (TDD workflow, fixture format)
- The cop's Rust source
- RuboCop's Ruby implementation (ground truth)
- RuboCop spec excerpts
- Current test fixtures
- Corpus FP/FN examples with source context
- Prism pitfall notes (if relevant)

### Built-in Validation

The dispatch workflow validates BEFORE creating a PR:
1. `cargo test --lib --no-run` — compiles
2. `cargo test --lib -- cop::<dept>::<name>` — cop tests pass

If validation fails, no PR is created and the workflow fails visibly.

### CI Validation

On the PR, two additional workflows run:
- **checks.yml**: clippy, full test suite, corpus smoke test
- **agent-cop-check.yml**: per-cop count check against corpus baseline

### Cop Tiers

| Tier | FP+FN | Count | Est. Cost/Cop |
|------|-------|-------|---------------|
| 1 | 1-50 | ~319 | ~$0.30-1.00 |
| 2 | 51-1000 | ~88 | ~$1-3 |
| 3 | 1001+ | ~61 | ~$3-10 (or use `/fix-department` locally) |

## Scripts

| Script | Purpose |
|--------|---------|
| `scripts/dispatch_cops.py` | Dispatch helper CLI: task generation, tiers, changed cops, prior attempts, issue sync, issue dispatch, and backend routing |

## Workflows

| Workflow | Trigger | Purpose |
|----------|---------|---------|
| `cop-issue-sync.yml` | `workflow_dispatch` | Sync/update one tracker issue per diverging cop from corpus |
| `cop-issue-dispatch.yml` | `workflow_dispatch` | Fill bounded queue from tracker issues into `agent-cop-fix` |
| `agent-cop-fix.yml` | `workflow_dispatch` | Generate prompt → agent fixes → validate → PR (mode: fix/retry) |
| `agent-pr-repair.yml` | failed `Checks` / `workflow_dispatch` | Repair existing bot PRs after deterministic CI failures |
| `agent-build-cache.yml` | `workflow_dispatch` | Pre-build Rust cache (optional optimization) |

## Security

- `workflow_dispatch` requires **write access** — public users can see but not trigger
- `OPENAI_API_KEY` is never exposed to forks or external PRs
- Branch protection prevents pushing directly to `main`

## Advantages over Kilo/external cloud agents

- **No external service dependency** — just GHA + OpenAI API
- **Full Rust build environment** — GHA has 14GB+ disk, Rust cache, no disk issues
- **Built-in validation** — Codex edits are tested before PR creation
- **Parallel execution** — GHA runs up to 20 jobs concurrently
- **No container bugs** — GHA runners are reliable and well-documented
- **Consistent branches** — we control the branch name, not a random cloud agent
