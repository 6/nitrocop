# NitroCop

Fast Ruby linter in Rust targeting RuboCop compatibility.

> [!NOTE]
> 🚧 Early-stage: Detection is high-fidelity on most codebases but edge cases remain. Autocorrect is not yet complete. Expect bugs.

Benchmark on the [rubygems.org repo](https://github.com/rubygems/rubygems.org) (1,227 files, Ruby 4.0), Apple Silicon:

| Scenario | nitrocop | RuboCop | Speedup |
|----------|-------:|--------:|--------:|
| Local dev (50 files changed) | **75ms** | 1.30s | **17.3x** |
| CI (no cache) | **279ms** | 14.86s | **53.4x** |

**Features**

- **915 cops** from 7 RuboCop gems (rubocop, rubocop-rails, rubocop-performance, rubocop-rspec, rubocop-rspec_rails, rubocop-factory_bot, rubocop-rake)
- Tested on [**1 open-source repos**](docs/corpus.md):
  - **284 of 915** cops match RuboCop exactly with default config
  - **282 of 915** match across all `EnforcedStyle` variants
  - Across **8.34K** offenses compared, **8.33K** (99.95%) match exactly with default config
- **Autocorrect** (`-a`/`-A`) is partial — work in progress
- Reads your existing `.rubocop.yml` — no migration needed
- Uses [Prism](https://github.com/ruby/prism) (Ruby's official parser) via `ruby-prism` crate
- Parallel file processing with [rayon](https://github.com/rayon-rs/rayon)

## Quick Start (Work in progress 🚧)

Requires Rust 1.85+ (edition 2024).

```bash
cargo install nitrocop   # not yet published — build from source for now
```

Then run it in your Ruby project:

```bash
nitrocop
```

## Configuration

nitrocop reads `.rubocop.yml` with full support for:

- **`inherit_from`** — local files, recursive
- **`inherit_gem`** — resolves gem paths via `bundle info`
- **`inherit_mode`** — merge/override for arrays
- **Department-level config** — `RSpec:`, `Rails:` Include/Exclude/Enabled
- **`AllCops`** — `NewCops`, `DisabledByDefault`, `Exclude`, `Include`
- **`Enabled: pending`** tri-state
- **Per-cop options** — `EnforcedStyle`, `Max`, `AllowedMethods`, `AllowedPatterns`, etc.

Config auto-discovery walks up from the target directory to find `.rubocop.yml`.

## Cops

<!-- corpus-cops:start -->
Compared with RuboCop on [**1 open-source repos**](docs/corpus.md) (0k Ruby files, 8.3K offenses compared).

**Default** = default RuboCop config. **All variants** = every supported `EnforcedStyle` (e.g. `EnforcedStyle: comma`).

**[rubocop](https://github.com/rubocop/rubocop)** `1.84.2` (588 cops)

| Department | Cops | Exact match (default) | Exact match (all variants) | No data |
|------------|-----:|----------------------:|---------------------------:|--------:|
| Layout | 100 | 44 | 44 | 55 |
| Lint | 148 | 35 | 35 | 112 |
| Style | 287 | 106 | 104 | 180 |
| Metrics | 10 | 6 | 6 | 4 |
| Naming | 19 | 8 | 8 | 11 |
| Security | 6 | 0 | 0 | 6 |
| Bundler | 7 | 3 | 3 | 4 |
| Gemspec | 10 | 3 | 3 | 7 |
| Migration | 1 | 0 | 0 | 1 |
| **Total** | **588** | **205 (34.8%)** | **203 (34.5%)** | **380** |

**[rubocop-rails](https://github.com/rubocop/rubocop-rails)** `2.34.3` (138 cops)

| Department | Cops | Exact match (default) | Exact match (all variants) | No data |
|------------|-----:|----------------------:|---------------------------:|--------:|
| Rails | 138 | 58 | 58 | 80 |

**[rubocop-performance](https://github.com/rubocop/rubocop-performance)** `1.26.1` (52 cops)

| Department | Cops | Exact match (default) | Exact match (all variants) | No data |
|------------|-----:|----------------------:|---------------------------:|--------:|
| Performance | 52 | 11 | 11 | 41 |

**[rubocop-rspec](https://github.com/rubocop/rubocop-rspec)** `3.9.0` (113 cops)

| Department | Cops | Exact match (default) | Exact match (all variants) | No data |
|------------|-----:|----------------------:|---------------------------:|--------:|
| RSpec | 113 | 8 | 8 | 105 |

**[rubocop-rspec_rails](https://github.com/rubocop/rubocop-rspec_rails)** `2.32.0` (8 cops)

| Department | Cops | Exact match (default) | Exact match (all variants) | No data |
|------------|-----:|----------------------:|---------------------------:|--------:|
| RSpecRails | 8 | 1 | 1 | 7 |

**[rubocop-factory_bot](https://github.com/rubocop/rubocop-factory_bot)** `2.28.0` (11 cops)

| Department | Cops | Exact match (default) | Exact match (all variants) | No data |
|------------|-----:|----------------------:|---------------------------:|--------:|
| FactoryBot | 11 | 1 | 1 | 10 |

**[rubocop-rake](https://github.com/rubocop/rubocop-rake)** `0.7.1` (5 cops)

| Department | Cops | Exact match (default) | Exact match (all variants) | No data |
|------------|-----:|----------------------:|---------------------------:|--------:|
| Rake | 5 | 0 | 0 | 5 |

See [docs/corpus.md](docs/corpus.md) for the full corpus breakdown.
<!-- corpus-cops:end -->

Every cop reads its RuboCop YAML config options and has fixture-based test coverage.

## Hybrid Mode

Use `--rubocop-only` to run nitrocop alongside RuboCop for cops it doesn't cover yet:

```bash
#!/usr/bin/env bash
# bin/lint — fast hybrid linter
nitrocop "$@"

REMAINING=$(nitrocop --rubocop-only)
if [ -n "$REMAINING" ]; then
  bundle exec rubocop --only "$REMAINING" "$@"
fi
```

## CLI

```
nitrocop [OPTIONS] [PATHS]...

Arguments:
  [PATHS]...    Files or directories to lint [default: .]

Options:
  -a, --autocorrect         Autocorrect offenses (safe cops only)
  -A, --autocorrect-all     Autocorrect offenses (all cops, including unsafe)
  -c, --config <PATH>       Path to .rubocop.yml
  -f, --format <FORMAT>     Output format: text, json [default: text]
      --only <COPS>         Run only specified cops (comma-separated)
      --except <COPS>       Skip specified cops (comma-separated)
      --rubocop-only        Print cops NOT covered by nitrocop
      --stdin <PATH>        Read source from stdin, use PATH for display
      --debug               Print timing and debug info
      --list-cops           List all registered cops
      --ignore-disable-comments  Ignore all # rubocop:disable inline comments
      --cache <true|false>  Enable/disable file-level result caching [default: true]
      --cache-clear         Clear the result cache and exit
      --init                Resolve gem paths and write lockfile to cache directory, then exit
      --fail-level <SEV>    Minimum severity for non-zero exit (convention/warning/error/fatal)
  -F, --fail-fast           Stop after first file with offenses
      --force-exclusion     Apply AllCops.Exclude to explicitly-passed files
  -L, --list-target-files   Print files that would be linted, then exit
      --force-default-config  Ignore all config files, use built-in defaults
  -h, --help                Print help
```

## Local Development

```bash
cargo check          # fast compile check
cargo test           # run all tests (2,700+)
cargo run -- .       # lint current directory

# Quality checks (must pass — zero tolerance)
cargo test config_audit     # all YAML config keys implemented
cargo test prism_pitfalls   # no missing node type handling

# Benchmarks
cargo run --release --bin bench_nitrocop          # full: setup + bench + conform
cargo run --release --bin bench_nitrocop -- bench # timing only
```

## How It Works

1. **Config resolution** — Walks up from target to find `.rubocop.yml`, resolves `inherit_from`/`inherit_gem` chains, merges layers
2. **File discovery** — Uses the `ignore` crate for .gitignore-aware traversal, applies AllCops.Exclude/Include patterns
3. **Parallel linting** — Each rayon worker thread parses files with Prism (`ParseResult` is `!Send`), runs all enabled cops per file
4. **Cop execution** — Three check phases per file: `check_lines` (raw text), `check_source` (bytes + CodeMap), `check_node` (AST walk via batched dispatch table)
5. **Output** — RuboCop-compatible text format or JSON

## Limitations

These cops are registered but cannot be exercised under current Ruby versions:

- `Lint/ItWithoutArgumentsInBlock` — `it` is a block parameter in Ruby 3.4+, making this cop obsolete
- `Lint/NonDeterministicRequireOrder` — `Dir` results are sorted since Ruby 3.0
- `Lint/NumberedParameterAssignment` — assigning to `_1` is a syntax error in Ruby 3.4+
- `Lint/UselessElseWithoutRescue` — syntax error in Ruby 3.4+
- `Security/YAMLLoad` — `YAML.load` is safe since Ruby 3.1 (cop has max Ruby 3.0)

These cops are excluded from corpus reporting counts.
