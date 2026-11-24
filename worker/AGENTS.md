# Repository Guidelines

Scope: the Cloudflare Worker under `worker/`.

## Architecture & Data Flow

- `src/index.ts`: fetch/cron entrypoint, authority-mediated app-record transitions, status aggregation, compatibility dispatch/result APIs, badge routes, and statistics chart endpoints.
- `src/apps.ts`: imports root `../patches-list.json` and derives monitored packages, targets, signers, package types, and patch names.
- `src/scraper.ts`: Play Store version acquisition.
- `src/comparator.ts`: version comparison.
- `src/badges.ts`: deterministic SVG badge rendering.
- `src/download-statistics.ts`: GitHub release download aggregation and history persistence.
- `src/statistics-charts.ts`: deterministic SVG download chart rendering.
- `src/svg.ts`: shared SVG XML escaping helper.
- `src/app-record.ts`: pure record merge/reconciliation helpers, including scrape-owned field application.
- `src/app-state.ts`: per-package `AppStateDurableObject` authority, authority client helpers, and KV migration/mirror logic.
- `src/types.ts`: binding, payload, and download statistics contracts.
- `wrangler.jsonc`: Worker name/routes, compatibility settings, six-hour freshness cron, daily statistics cron, `PATCHES_KV` and `APP_STATE` bindings, and the Durable Object migration.
- `test/index.spec.ts`: Vitest Worker-pool integration suite.
- `test/download-statistics.spec.ts`: Vitest download statistics collection and chart integration suite.

Flow: scheduled version checks (every 6 hours) scrape Play metadata → scrape-owned fields are submitted to the per-package Durable Object → compatibility CI dispatch/results update the same authority → public API/badge responses read the authority. Scheduled download statistics (daily at 00:15 UTC) aggregate GitHub release assets into KV → public chart routes render SVG download trends. Root patch metadata determines which apps and versions exist.

## App state authority and migration

- `app_version:<package>` records are owned by one `AppStateDurableObject` per package (`idFromName(packageName)`). Every read-modify-write transition (run-start, result, Play metadata, scrape application) executes inside that object as one transactional storage commit. Do not read-modify-write these records through KV in Worker code.
- KV remains the migration source and rollback mirror. On first access, the authority seeds itself from the existing `app_version:` KV value; every subsequent authority write also mirrors the record to KV. KV is never read to make an authoritative decision after initialization.
- Seeding requires the migration gate key `app_state_authority:v1` in `PATCHES_KV`. The value must be an ISO timestamp from after old-version traffic drained and KV propagation completed. App-state routes and authority seeding return `503` until the gate exists and is at least two minutes old, so a routine automatic deploy cannot silently cut over on its own. Keep the gate key in place permanently.
- A successful authority commit whose KV mirror write fails is reported as `mirrored: false` (run-start/result and metadata batch) or logged for scheduled scrapes. The authority state is accepted; the rollback mirror is stale and must be reconciled before any rollback.
- Scraping and media mirroring run outside the authority. Scrapes submit only scrape-owned fields; compatibility callbacks own `targetCompatibility`/`outstandingRequest` and authenticated version acquisition. Do not hold an authority transaction across network work.
- Status, version, and badge routes read the authority on every request; there is no app-record memory cache to go stale.

### Deployment and rollback prerequisites

- Deploy the Worker code and `wrangler.jsonc` together so Wrangler applies the `v1` `new_sqlite_classes` migration for `AppStateDurableObject`. Do not create or delete KV namespaces.
- A deploy alone does **not** cut over. App-state routes return `503` until the operator writes `app_state_authority:v1` to `PATCHES_KV` (namespace `5c62cbcdc49749368cc3daba5aa87759`). The value is an ISO timestamp. This gate is enforced in code, so an automatic `deploy-worker.yml` run cannot seed the authority on its own.
- Cutover sequence: (1) deploy and accept that app-state routes are `503`; (2) wait for all old-version in-flight scheduled scrapes and callbacks to finish; (3) confirm the latest `app_version:` values have propagated; (4) write the gate timestamp with Wrangler's KV key put for `PATCHES_KV`; (5) wait at least two minutes for the settle window, then confirm routes recover. Do not seed by writing a gate during active old-version traffic.
- The supported rollback path is roll-forward. Before the gate is written, no authority state exists and rolling back to KV-only code is safe. After seeding, any mutation with `mirrored: false` (or a logged scheduled-scrape mirror failure) means the KV mirror is stale; reconcile that record before considering rollback.
- Never describe KV last-write-wins, isolate memory, or a KV key as a lock or compare-and-swap. New app-record fields must be assigned to an owner and routed through `app-state.ts`.

### Play metadata batch semantics

- Phase 1 parses and validates every item, duplicate package names, package existence, and `listingUpdatedAt` before any record is written. Validation failures (400/409) leave all records unchanged.
- Phase 2 applies each item through its per-package authority, which re-checks `listingUpdatedAt`. KV provides no multi-record transaction, so a fully applied batch returns `200 { success: true, updatedPackages }`; a partial batch returns `409` (listing changed) or `500` (authority failure) with `success: false`, the committed `updatedPackages`, and per-package `failures`. If an authority commit succeeded but its KV mirror write failed, the item still counts as committed and the response adds `mirrored: false` plus `mirrorWarnings`. Callers must not treat partial or `mirrored: false` responses as all-or-nothing.

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
