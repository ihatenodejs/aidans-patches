# APK Lab migration harness

This directory freezes the observable contracts of the Python `apk-lab` implementation
and records an independent baseline before any native Rust replacement is accepted.
The Python package remains the oracle for the entire migration; it is not removed or
weakened to make a comparison pass.

The auxiliary Rust binary is a test/benchmark tool. It is not the replacement CLI and
does not change production Python behavior.

## Scope and ownership

- `contracts.json` is the frozen contract inventory: commands, parser rules, exit
  classes, JSON wire fields, ordering guarantees, side effects, test destinations, and
  external consumers. Add a mapping whenever behavior is added or discovered.
- `src/migration/` contains the harness. `inventory.rs`, `prerequisites.rs`, and
  `benchmark.rs` own their commands; `oracle.rs` owns Python capture and differential
  verification. The modules are independently executable and must not share oracle and
  candidate result logic.
- Public repository files contain sanitized metadata and synthetic fixture generators,
  never proprietary APK bytes, account material, device serials, or private paths.

## Build and test gates

Run from the repository root. Keep the generated `Cargo.lock` with the harness source
in a completed phase commit; do not regenerate it as part of unrelated changes.

```sh
cargo test --locked --manifest-path tools/apk-lab/Cargo.toml
cargo build --release --locked --manifest-path tools/apk-lab/Cargo.toml --bin apk-lab-migrate
cargo fmt --manifest-path tools/apk-lab/Cargo.toml -- --check
cargo clippy --locked --manifest-path tools/apk-lab/Cargo.toml --all-targets -- -D warnings
```

The Python oracle gates are unchanged:

```sh
uv sync --locked --project tools/apk-lab
uv run --project tools/apk-lab pytest -q
```

### Dependency and lock rationale

The crate uses Rust edition 2021 and the stable host toolchain. Its four direct
dependencies are deliberately narrow: `serde` derive + `serde_json` for the capture
schema (`raw_value` preserves exact bytes around measured Morphe timing fields),
`sha2` for content/archive identities, and `tempfile` for isolated sandboxes.
`Cargo.lock` pins serde 1.0.229, serde_json 1.0.154, sha2 0.10.9, and tempfile 3.27.0
(32 packages total); direct licenses are MIT OR Apache-2.0, and the highest transitive
`rust-version` floor is 1.85 (`getrandom` 0.4.3), below the stable host's 1.97.1.
There are intentionally no ZIP, image, or HTTP client dependencies in the migration
harness: synthetic ZIP fixtures use a small local writer, and CI callback probes use a
loopback Python HTTP server. This keeps new dependency surface out of gated phases.
The lock metadata above was collected by orchestrator inspection, not as quality-gate
proof.

## Commands

```sh
MIG=tools/apk-lab/target/release/apk-lab-migrate
$MIG inventory --check
$MIG prerequisites --manifest .apk-lab-migration/prerequisites.local.json \
  --out .apk-lab-migration/prerequisites.json
$MIG capture --engine python --profile offline \
  --out .apk-lab-migration/python-contracts.json
$MIG verify --expected .apk-lab-migration/python-contracts.json \
  --actual .apk-lab-migration/python-contracts.json
$MIG bench --prepare --engine python --profile local \
  --manifest docs/benchmarks/apk-lab/corpus.local.json \
  --out .apk-lab-migration/output-pins.json
$MIG bench --engine python --profile local \
  --manifest docs/benchmarks/apk-lab/corpus.local.json \
  --out .apk-lab-migration/python-baseline.json
$MIG bench --sanitize --input .apk-lab-migration/python-baseline.json \
  --out docs/benchmarks/apk-lab/baseline.json
```

Each command exits `0` only after performing the requested work. Unsupported engines,
profiles, missing inputs, or incomplete reports are errors with a statement of what was
not run. Neither `capture` nor `verify` mutates production state, contacts a network
service, or touches a device.

`capture` selects Python in this order: `--python PATH`, `APK_LAB_PYTHON`,
`tools/apk-lab/.venv/bin/python`, then `python3`. The interpreter must be 3.12 or newer.
If the binary is not run from inside the repository, pass `--repo-root PATH` (the
directory containing `tools/apk-lab/apk_lab/cli.py`). It records the interpreter
implementation/version (not its absolute path), the Python source digest, and the
best-effort Git revision/dirty flag. The source digest is the authority when the
worktree differs from `HEAD`.

## Independent Python capture

`capture --engine python --profile offline` builds deterministic synthetic containers
in a fresh temporary sandbox and runs the real Python module
(`python -B -m apk_lab.cli`) once per case with an isolated working directory and
`HOME`/`XDG_CONFIG_HOME`/`TMPDIR` inside that sandbox. The fixture generator produces:

- a normal APK with two DEX files, resources, assets, native library, Unicode strings,
  a callback marker, and a large integer version code, plus a dedicated fixture whose
  version code exceeds the 53-bit JSON integer precision boundary;
- a split APK and flat/nested APKM containers;
- an older base APK for comparison/order cases;
- traversal, absolute-path, duplicate-entry, and truncated archives for rejection cases;
- synthetic Unity serialized assets and v29 IL2CPP metadata, including a corrupt table;
- a synthetic patches list for Morphe independent-boolean and compatibility-CI probes;
- an isolated fake `aapt2`/`apksigner` under a temporary `ANDROID_HOME`, used only to
  exercise offline inspection/manifest/string/resource interfaces. A separate case
  removes the tool to assert the missing-tool exit class.

No generated APK fixture bytes are committed and no private corpus is used by capture. Cases cover parser
errors, valid and invalid assembly/Smali, JSON to stdout and to a file, split ordering,
nested splits, nullable JSON fields, booleans, missing artifacts, archive boundary
rejections, large integer version codes, malformed IL2CPP offsets/table sizes, Unicode,
missing tools, TTY and non-TTY diff rendering against a real PTY versus a real pipe,
broken-pipe behavior through a consumer-closed OS pipe with the attempted payload
recorded, resource-dump parsing, ADB log/device parsing, Morphe independent
boolean generation, CI matrix generation, and CI run-start/result callbacks against an
isolated local HTTP server. Callback persistence is asserted for both HTTP success
(marker written) and HTTP failure (nonzero exit, no success marker), and the request
paths, bearer header, request IDs, submission body, and side-effect files are checked.
Managed-workspace cases create a schema-1 run marker and assert that `clean --run`
deletes only the valid contained run while rejecting a malformed marker.

`capture` validates every mandatory case and fixture invariant before writing a
document: a broken import, wrong exit class, missing JSON field, lost ordering/Unicode,
missing callback marker, or missing TTY/broken-pipe evidence fails capture itself.
Running Python tests against the production tests remains a separate gate; fixture
replay is contract evidence, not proof that a live Morphe/R2/Play/ADB integration
worked.

### Normalization

Only explicit path volatility is normalized:

- sandbox-root prefixes (`<sandbox>`);
- repository-root prefixes (`<repo>`);
- the Python executable path (`<python>`).

ANSI escapes are **not** stripped: each capture retains and compares the exact output
of its own terminal mode. ANSI may be normalized only when an explicit case compares
different terminal modes, and no such case exists. Timestamps, durations, and tool
banners are omitted from captured cases rather than normalized. No JSON key is treated
as volatile by name; a future measured field must be declared as an explicit
case + JSON-pointer pair in `APPROVED_VOLATILE_JSON_PATHS`. Result status, exit classes,
split/member ordering, matched offsets, error text, content hashes, boolean cases,
callback markers, and Unicode are never normalized.

## Differential verification

`verify` first rejects an invalid schema version, kind, engine/profile, empty fixture
list, missing/duplicate mandatory case, or unexpected case—even when comparing a
document with itself. It then compares metadata, fixture hashes, case order, argv, exit
code, exact stdout/stderr bytes, structured JSON (including missing keys versus `null`,
array order, booleans, callback markers, and Unicode), and the provisional side-effect
file tree with normalized content hashes. Differences are reported with the case and
JSON path. Self-comparison only proves serialization mechanics; the committed unit
tests additionally mutate exit code, remove a JSON null, change an archive hash,
reorder significant results, omit a boolean, alter a callback marker, corrupt Unicode,
and alter a nested `durationMs`/`processingNanos` field, and require each mutation to
fail.

## Baseline provenance and reconstruction

The plan-001 baseline is collected by the Rust harness, never by hand:

- `bench --prepare` runs every selected workload once outside the timing region to
  validate exit status, required output tokens, expected files, and the approved
  Morphe timing-only normalization, then publishes observed output pins. Preparation
  is not a performance sample.
- `bench` then executes the full pre-registered workload matrix with the declared
  warmup/sample classes and writes a result document.
- `bench --sanitize` derives the publishable result from the private one: engine paths,
  archive locations, and corpus paths become fixed placeholders derived from declared
  hashes, `argvTemplate` becomes a `command`/`flags` description, the host name becomes
  a stable `<host:digest>` identity, storage mount/device become placeholders, and every
  other byte of measured data is preserved. Sanitizing rejects unknown shapes and any
  surviving private path marker, and writes nothing on rejection.

Private artifacts live under the ignored `.apk-lab-migration/` root: the raw result,
the runnable corpus manifest, the verified tool-cache staging, and
`baseline-source/<revision12>-<digest12>/`. That archive contains `manifest.json`
(per-file sha256), `dirty.patch` (evidence of the pre-existing worktree difference),
and the executed source: the oracle package/tests/locks, the legacy `.github/scripts`
generators, the release/icon/CI workflow inputs, `.releaserc`,
`.pre-commit-config.yaml`, `scripts/audit.ts`, `patches/build.gradle.kts`,
`gradle/libs.versions.toml`, `patches-list.json`, the root/site package manifests and
locks, and the native PDF fixture generator. Credential-bearing local files, corpora,
virtual environments, and caches are never archived.

To reproduce a baseline: restore the archive exactly (do **not** apply `dirty.patch`;
the archive already holds the executed bytes), verify every file against
`manifest.json`, run `capture` and `bench --prepare`/`bench` with the recorded
interpreter and policy, and compare against the recorded digests. `source.worktreeDigest`
covers executed-source identity only; policy and manifest digests are separate fields,
so changing a manifest expectation cannot masquerade as a source change.

Morphe workloads run with `--jobs 1`. Parallel cases intermittently failed inside
Morphe's archive reader (`Failed to find end record`) in one timed sample, while two
default-jobs reruns passed 12/12. Serial execution keeps every case and produced the
same prepared output hashes, so the baseline stays reproducible without dropping
coverage.

Host metric coverage is recorded rather than assumed. On Linux the driver samples
`/proc` for CPU ticks, RSS, and read/write bytes with `getconf`-derived units. On macOS
it samples `ps` for process-tree CPU and RSS and reports no per-process byte counters,
because none is available without unsafe FFI. Metric acceptance is derived from the
pre-registered policy: a metric whose budget sets `blockOnWorse` must be collected,
while a non-blocking one stays `null` with a reason plus an explanation-required
limitation. Release-binary size and build cost are measured in plan 008, when a native
release CLI exists; the Python baseline records interpreter and environment identity
instead.

## Mapping lifecycle and cutover checklist

The inventory is a living contract, not a snapshot: adding or discovering a command,
parser branch, wire field, side effect, consumer, or test requires a mapping entry in
`contracts.json` before the change lands. `inventory --check` re-discovers the current
Python surface and fails on unmapped, stale, or drifted entries; approved exceptions
(the verified broken-pipe correction and the future icon compression-only no-op) are
listed explicitly instead of being normalized away.

Python may be removed only after all of the following hold, in order:

1. Every mapped behavior has native coverage and all parity groups report zero
   unapproved differences on the same host and settings.
2. `bench --prepare` output pins and the full measured baseline exist, with the raw
   sanitized result and unmodified performance policy committed.
3. Real tool/device/cloud/Play/CI scenarios have actual evidence; fixture replay or a
   missing resource never counts as a pass.
4. The asset, README, and hook replacements are adopted with their own parity gates and
   no Python caller remains.
5. `TO-RUST.md` reports measured, reproducible results with Python-generated charts
   produced outside the checkout.
6. A Python-free clean environment runs the full gate set, including a golden verifier
   that exercises the native CLI rather than static JSON.

Until then the Python oracle stays intact and independently executable, and no native
command may delegate to it or return placeholder success.

## Release gates

1. Existing Python tests pass; added consumer-visible characterization passes.
2. Every public command, parser branch, wire field, exit class, side effect, test, and
   external consumer appears in `contracts.json`; `inventory --check` reports no gaps.
3. Differential verification rejects the mutation set above and accepts only the named
   volatile differences.
4. Sanitized baseline raw samples, schema, environment, input hashes, and provenance
   exist; no performance improvement is claimed before paired measurement.
5. `cargo fmt --check` and `cargo clippy -- -D warnings` pass, with a locked Cargo build.
6. Real/live scenarios remain explicitly pending until actually executed with operator
   authorization. Pending mandatory scenarios block final migration acceptance.
7. The prerequisite report covers host tools, corpus, disposable device/emulator, Play
   accounts/packages, isolated R2/Worker/compatibility workflow resources, with
   availability and authorization evidence. Any missing resource is a blocker for the
   main rewrite plan.

## Known blockers and pending work

- **Plan 001 is complete.** Verified: 285 Python tests; 110 native tests across four
  suites; clippy with warnings denied; `--locked` release build; `inventory --check`;
  an accepted compiled capture (41 cases, 16 fixtures) cross-compared exactly; 17
  output pins from real preparation runs; and a measured baseline at
  [`baseline.json`](../../../docs/benchmarks/apk-lab/baseline.json) with valid
  acceptance. The approved `fix_python` exception is recorded in `contracts.json` as
  `approved-verified`, retains the pre-correction `main == 0` / process exit `120`
  evidence, and did not relax the capture contract.
- **Baseline shape.** Seventeen workloads: 15 local with 5 warmups and 30 samples, 2
  Morphe with 2 and 10. CPU and descendant RSS are sampled every 5 ms via `ps`; process
  I/O is unsupported on macOS and recorded as a non-blocking, explanation-required
  limitation, so plan 008 must explain it rather than compare or zero-fill it. Morphe
  workloads run with `--jobs 1` after one timed default-jobs sample hit an internal
  Morphe archive race; all cases still run and prepared hashes are unchanged. The
  measured code re-hashes exactly against the archived source.
- **Five defects found by integration gates are fixed with regressions:** an inventory
  call-offset bug, an expired-window fixture that described a malformed interval,
  generated benchmark evidence classified as an executable consumer, lost failed-row
  output diagnostics, and the macOS `ps` accumulating-minute `TIME` parse that had
  silently nulled every sampled CPU/RSS value.
- **Mislabeled container preserved.** The original AfterShip 5.25.8 artifact is APKM
  content with an `.apk` suffix and fails 12/12 cases with manifest-null errors; it is
  kept in
  [`mislabeled-container-observation.json`](../../../docs/benchmarks/apk-lab/mislabeled-container-observation.json)
  as a negative control and planned native parity case. The positive workload uses a
  byte-identical `.apkm` copy with the same patch selections, and no production
  compatibility behavior was changed.
- Live device, Google Play, R2, Worker callback, and compatibility-workflow scenarios
  are **pending**: this harness has no execution authorization for mutations and the
  offline profile must not pretend they passed. Each is listed with an owner and safe
  probe in the prerequisite catalog; acceptance mode (`prerequisites --check`) fails
  until every required resource is available, authorized, and independently observed.
- The private corpus, raw result, and prerequisite manifest are intentionally absent
  from the public checkout. Capture and harness tests need no private corpus; the
  publishable baseline is derived from the private one by `bench --sanitize`. No
  paired comparison or speedup claim exists, because no native candidate has been
  measured.
- `Cargo.lock` and release artifacts are produced by the integration build. This crate
  is not yet registered in `scripts/audit.ts`; plan 002 owns that narrow registration
  alongside the production core.
- `contracts.json` inventory coverage and benchmark budget results are owned by their
  respective plan steps; this README records the gates, not unverified success.
