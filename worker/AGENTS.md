# Repository Guidelines

Scope: the Cloudflare Worker under `worker/`.

## Architecture & Data Flow

- `src/index.ts`: fetch/cron entrypoint, KV reads/writes, status aggregation, compatibility dispatch/result APIs, badge routes, and statistics chart endpoints.
- `src/apps.ts`: imports root `../patches-list.json` and derives monitored packages, targets, signers, package types, and patch names.
- `src/scraper.ts`: Play Store version acquisition.
- `src/comparator.ts`: version comparison.
- `src/badges.ts`: deterministic SVG badge rendering.
- `src/download-statistics.ts`: GitHub release download aggregation and history persistence.
- `src/statistics-charts.ts`: deterministic SVG download chart rendering.
- `src/svg.ts`: shared SVG XML escaping helper.
- `src/types.ts`: binding, payload, and download statistics contracts.
- `wrangler.jsonc`: Worker name/routes, compatibility settings, six-hour freshness cron, daily statistics cron, and `PATCHES_KV` binding.
- `test/index.spec.ts`: Vitest Worker-pool integration suite.
- `test/download-statistics.spec.ts`: Vitest download statistics collection and chart integration suite.

Flow: scheduled version checks (every 6 hours) scrape Play metadata → records persist in KV → compatibility CI dispatch/results update package state → public API/badge responses read the aggregated payload. Scheduled download statistics (daily at 00:15 UTC) aggregate GitHub release assets into KV → public chart routes render SVG download trends. Root patch metadata determines which apps and versions exist.

## Runtime & Commands

Use Bun and the local Wrangler dependency:

```bash
bun install --frozen-lockfile --cwd worker
bun run --cwd worker dev
bun run --cwd worker types
bun run --cwd worker check
bun run --cwd worker test
bun run --cwd worker deploy  # only when explicitly requested
```

`check` is `tsc --noEmit`; tests use Vitest with `@cloudflare/vitest-pool-workers` and the real Wrangler config. Do not treat a unit/integration test as a live deployment check.

## Code Patterns

- Keep service bindings explicit through `env: WorkerEnv`. Pass dependencies/state into helpers; do not import hidden binding singletons.
- Await every KV, fetch, and dispatch operation whose result affects the response or durable state. Use `ctx.waitUntil(...)` only for intentionally post-response work with independent failure semantics.
- Validate untrusted JSON structurally before use. Compatibility result/start payloads are authenticated state transitions; preserve timing-safe secret comparison, replay/identity checks, valid statuses, and failure-stage validation.
- KV is durable shared state. The module-level memory cache is only a bounded TTL fallback/optimization and must never become authoritative.
- Preserve key builders and payload types when changing storage. Coordinate migrations or backward-compatible reads before changing persisted shapes.
- Keep badge functions deterministic and escape all user/external text before SVG emission.
- Return explicit HTTP status codes and safe error bodies. Log operational context, never authentication values or Cloudflare secrets.
- Use `Promise.all` only for independent operations; avoid unbounded fan-out to Play, GitHub, KV, or dispatch APIs.
- Update `src/types.ts` and regenerate `worker-configuration.d.ts` with `bun run types` after binding changes. Do not hand-edit generated binding types.

## Metadata and Deployment Boundaries

- `src/apps.ts` derives app configuration from root `patches-list.json`; do not duplicate package/version lists manually.
- `deploy-worker.yml` checks TypeScript and Vitest before deployment and runs for Worker/metadata changes on `main` or release handoff.
- Never deploy, create/delete KV namespaces, mutate production data, or rotate secrets as routine verification.
- Changes to public endpoints, auth contracts, compatibility payloads, badge semantics, cron cadence, or bindings must update tests and relevant site/CI consumers.

## Testing & QA

Add tests in `test/index.spec.ts` using `cloudflare:test` bindings. Cover observable behavior: routes, auth rejection, payload validation, replay handling, state transitions, KV persistence, status aggregation, and SVG escaping. Keep tests deterministic and independent of live Play/GitHub/Cloudflare services.

```bash
bun run --cwd worker check
bun run --cwd worker test
bun run audit
```

For route/cron changes, also run `bun run --cwd worker dev` and exercise the changed request or scheduled path locally. Report production-only limits rather than deploying for proof.
