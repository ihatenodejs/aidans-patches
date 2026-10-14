# Repository Guidelines

Scope: root automation scripts, currently the Bun audit orchestrator `audit.ts`.

## Command Contract

Root `package.json` delegates to `scripts/audit.ts`:

| Command | Behavior |
| --- | --- |
| `bun run format` | Mutating Prettier, Ruff format, ktlint format, and cargo fmt on configured scopes. |
| `bun run format:check` | Non-mutating formatter checks (Prettier, Ruff, ktlint, cargo fmt). |
| `bun run lint` | Oxlint, Ruff, ktlint, and clippy checks. |
| `bun run lint:fix` | Mutating supported lint fixes (Oxlint, Ruff, ktlint, clippy --fix). |
| `bun run typecheck` | Worker `tsc`, Astro check, APK Lab mypy, Kotlin/Java compile, Rust cargo check. |
| `bun run test` | APK Lab pytest, Worker Vitest, site tests, cargo test, icon consistency check. |
| `bun run audit` | Format check → lint → typecheck → test. |

This script is the source of truth. Root/nested guides should describe its public commands rather than duplicate every internal argument.

## Implementation Rules

- Use Bun/TypeScript and `spawn` with argument arrays (`shell: false`) for commands. Do not concatenate untrusted shell strings.
- Preserve per-step cwd, exit code, stdout/stderr, duration, diagnostic parsing, and final non-zero status when any step fails.
- Keep mutating (`format`, `lint:fix`) and check-only commands visibly separate.
- Tool acquisition must be pinned and checksum-verified. The ktlint fallback downloads only the pinned version into ignored `.tools/`; do not add unverified download execution.
- Prefer existing repository package scripts and `uv`/Gradle wrappers over duplicating tool setup.
- Keep output concise but retain enough raw diagnostics to reproduce failures. Never suppress a failed subcommand to make the aggregate green.
- When adding a language/module, update format, lint, typecheck, and test phases deliberately; do not claim coverage in docs before the orchestrator actually runs it.

## Verification

Run the directly changed phase first, then the aggregate:

```bash
bun run format:check
bun run lint
bun run typecheck
bun run test
bun run audit
```

A change to command orchestration must be smoke-tested on both success and a controlled failing command when practical, verifying the reported diagnostic and process exit status. Do not use mutating format/fix modes as the only proof.
