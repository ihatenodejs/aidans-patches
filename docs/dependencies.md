# Dependency Freshness and Upgrades

This repository uses a unified dependency checker and updater orchestrator (`scripts/deps.ts`), designed to check and apply dependency updates across all monorepo languages and pinned tooling without needing to invoke each ecosystem's package manager manually.

## Commands

Root `package.json` delegates to the Bun script:

```bash
# Check all repository dependencies against upstream registries (read-only)
bun run deps:check

# Filter check to specific dependency ID(s)
bun run deps:check --only 'npm:worker/package.json#devDependencies/wrangler'

# Output structured JSON for automation or review
bun run deps:check --json > /tmp/dependencies.json

# Interactive update (prompts with proposed changes on a TTY)
bun run deps:update

# Agent non-interactive approved update (requires exact proposal digest)
bun run deps:update --plan /tmp/dependencies.json --approve <proposal.digest>

# Audit integration (opt-in freshness phase)
bun scripts/audit.ts deps
bun run audit --deps
```

## Exit Codes

| Exit Code | Meaning                                                                                                   |
| --------- | --------------------------------------------------------------------------------------------------------- |
| `0`       | Success: all checked dependencies are up-to-date or held, or update was cleanly applied.                  |
| `1`       | Error: registry failure, network error, policy violation, stale/tampered plan, or tool execution failure. |
| `2`       | Outdated: actionable updates are available.                                                               |
| `3`       | Declined: user cancelled the interactive update prompt.                                                   |

## Monorepo Coverage

The updater covers all direct software dependencies and version-sensitive toolchain configurations in the repository:

| Ecosystem               | Manifest / Source                                                                                                                       | Tracked Lockfiles                                                   |
| ----------------------- | --------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------- |
| JavaScript / Web        | `package.json`, `worker/package.json`, `site/package.json`                                                                              | `bun.lock`, `worker/bun.lock`, `site/bun.lock`, `package-lock.json` |
| Rust / Crates           | `tools/apk-lab/Cargo.toml`, `tools/play-metadata/Cargo.toml`, `tools/asset-prep/Cargo.toml`, `extensions/nhl-adblock-native/Cargo.toml` | Corresponding standalone `Cargo.lock` files                         |
| Gradle Plugins          | `settings.gradle.kts`, `gradle/libs.versions.toml`                                                                                      | Gradle metadata                                                     |
| External APK Binaries   | `tools/apk-lab/tools.lock.json`                                                                                                         | Download URL, SHA-256, executable path, and version probe           |
| Code Quality Tooling    | `scripts/audit.ts` (`KTLINT_VERSION`)                                                                                                   | Upstream release asset SHA-256                                      |
| GitHub Actions          | `.github/workflows/*.yml`                                                                                                               | Release tags                                                        |
| Toolchain Runtimes      | Workflow setup inputs, `rust-toolchain.toml`, `build.gradle.kts` (NDK), `tools/apk-lab/src/tools.rs` (Build Tools)                      | Pinned versions across coordinated files                            |
| Embedded In-Repo Assets | `OsmMapView.java` (Leaflet CDN)                                                                                                         | Coupled asset URLs                                                  |
| C++ / Native            | `extensions/adobescan-pdf-native/CMakeLists.txt`                                                                                        | CMake FetchContent archives and integrity hashes                    |

Gradle wrapper, ktlint, APK Lab tools, and NDK updates carry their artifact
checksum (and, for APK tools, the new URL, executable path, and version probe)
inside the proposal. `deps:update` writes all coupled fields together; it never
leaves a new version paired with an old hash or URL.

All four registered Rust projects regenerate their own `Cargo.lock` during
`deps:update`, including `tools/apk-lab` and `tools/asset-prep`. Floating
dependency forms such as `@v7` for GitHub Actions and Cargo ranges like `1.0`
are compared as padded semver, so patch releases are still detected.

Git dependencies that track a branch (for example
`@MorpheApp/changelog#bundle`) are reported as `moving`, with the branch's
current commit in the report. They are intentionally excluded from
`deps:update`; refresh them with the package manager after reviewing the new
commit. Workflow actions pinned to a floating major tag are updated to the
latest exact release tag.

The release workflow uses root `package-lock.json` as its lock owner: it
installs with `npm ci --ignore-scripts` and runs the locked local
`semantic-release` executable. `bun.lock` remains canonical for Bun CI
commands, so a release dependency change must update both lockfiles and the
`@semantic-release/*` versions must satisfy `package.json`.

Repository hooks use Bun (`scripts/hooks.ts` and `scripts/install-hooks.ts`), not the
Python pre-commit framework. `toml-eslint-parser` is required for TOML 1.0 syntax
validation, including integers beyond JavaScript's safe-number range; Bun's built-in
TOML reader is not a compatible validator. YAML checks use Bun's syntax parser and
retain custom-tag/multiple-document acceptance. Pre-commit hook version tracking is
retired with the framework; repository hooks are plain Bun scripts.

Asset preparation is native Rust; normal icon workflows no longer install Pillow.
The retired Python/Pillow environment is preserved only as private migration
oracle evidence, not an ordinary build/test dependency of `asset-prep`. ZIP reader
features retain Stored, Deflate, Bzip2 and LZMA; Cargo defaults that add unrelated
codecs/crypto are disabled. The hash-pinned libjpeg-turbo 3.1.1 wrapper is deliberate
exact-pixel compatibility, not the separate native PDF engine's codec version.

### Non-Dependency Exclusions

The dependency checker strictly inspects software dependency releases. It intentionally excludes:

- Target application compatibility versions in patch `Constants.kt`. (App versions require device and reverse-engineering verification via APK Lab).
- Fixture metadata and APK artifact hashes.
- In-tree font resources and adblock filter list sources.
- Repository semantic release version in `gradle.properties`.

## Dependency Policy and Holds

Intentional dependency holds are maintained in root `dependency-policy.json`. Every hold requires a mandatory text `reason` explaining why the package cannot be updated.

```json
{
  "schemaVersion": 1,
  "holds": [
    {
      "id": "npm:package.json#devDependencies/conventional-changelog-conventionalcommits",
      "value": "^9.3.1",
      "reason": "Preserve release-note generation compatibility; see https://github.com/conventional-changelog/conventional-changelog/issues/1495"
    }
  ]
}
```

- When a dependency is held in `dependency-policy.json`, `deps:check` marks it as `held` (displaying the reason and the latest available upstream release), but does not include it in actionable update proposals.
- An update cannot override a held dependency without explicitly removing or updating the hold entry in `dependency-policy.json`.
- Validated toolchain baselines are held as well: Temurin JDK 21 (`tool:java`), NDK r28c (`tool:ndk`), and Build Tools 36.0.0 (`tool:android-build-tools`). The checker still reports their upstream latest values so an intentional upgrade is visible.

## Safety & Approval Model

1. **Read-Only Checks**: `deps:check` only fetches upstream metadata over HTTP (registries, GitHub release metadata, and the Android SDK repository index). It never modifies manifests, runs lifecycle scripts, or downloads binary tools.
2. **Deterministic Proposals**: Each report containing actionable updates generates a content-addressed proposal digest (SHA-256 over canonical JSON of sorted inputs and planned updates). The digest binds the exact versions, artifact hashes, URLs, executable paths, and workflow labels that will be written; it is an integrity check, not a signature.
3. **Interactive Confirmation**: On a TTY terminal, `deps:update` displays all candidate updates, highlights major version bumps, and prompts for confirmation before modifying any file.
4. **Agent Programmatic Mode**: In non-interactive environments (e.g. CI, subagents), `deps:update` refuses to run without `--plan <file>` and `--approve <digest>`. If any input file has changed since the plan was generated, or if the digest does not match, the transaction is rejected immediately.
5. **Transactional Staging & Atomic Publish**: Updates are applied and lockfiles regenerated inside an isolated temporary staging directory (`os.tmpdir()`). Every Rust project updates its own `Cargo.lock`, and Bun/npm lockfiles are regenerated for the manifests that changed. Any proposal entry without a matching apply strategy, or whose target file/content cannot be found, aborts the transaction instead of reporting a successful no-op. Only after lock regeneration succeeds are files published; a write failure rolls back every changed file.
