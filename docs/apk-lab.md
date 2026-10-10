# APK Lab: Deterministic APK Analysis and Compatibility Toolkit

`apk-lab` is the repository's deterministic toolchain for reverse engineering Android applications, managing APK/APKM/XAPK artifacts, running fail-fast patch compatibility tests via Morphe Desktop, and managing private cloud test fixtures in Cloudflare R2.

---

## 1. Prerequisites and Installation

### Host Prerequisites
- **Python**: Version `3.12+` managed via `uv`.
- **Java**: JDK `21+` (Eclipse Temurin recommended; provides `java` and `keytool`).
- **Android SDK Build Tools**: Version `36.0.0` (`aapt2`, `apksigner`, `zipalign`, `dexdump`).
  - Path detected automatically from `ANDROID_HOME`, `ANDROID_SDK_ROOT`, or repository `local.properties` (`sdk.dir`).
- **Android Platform Tools**: `adb` for device deployment and runtime monitoring (detected from SDK `platform-tools/adb` or PATH).
- **uv**: Fast Python package installer and virtualenv manager.
### Setup & Toolchain Installation
Tools are pinned in `tools/apk-lab/tools.lock.json` with official download URLs and immutable SHA-256 digests. Only `apk-lab setup` downloads external tools.

```bash
# Sync Python virtual environment
uv sync --locked --project tools/apk-lab

# Install pinned analysis tools (Morphe Desktop 1.18.0, JADX 1.5.6, Apktool 3.0.3, baksmali 3.0.10, smali 3.0.10)
uv run --project tools/apk-lab apk-lab setup --profile analysis
# Install CI profile (includes apkeep on Linux x86-64)
uv run --project tools/apk-lab apk-lab setup --profile ci

# Verify host prerequisites, tool caches, and workspace permissions
uv run --project tools/apk-lab apk-lab doctor

# Doctor verification in CI mode (checks presence of credentials without leaking values)
uv run --project tools/apk-lab apk-lab doctor --ci --json
```

---

## 2. Command Reference

### `inspect` — Artifact Inspection
Validates archive security (path traversal, duplicate names, zip bomb ratio limits, encryption) and parses container structure, signing certificates, splits, DEX counts, native libraries, and assets by content rather than file suffix.

```bash
# Human readable inspection
uv run --project tools/apk-lab apk-lab inspect path/to/app.apkm

# JSON output (creates no persistent workspace)
uv run --project tools/apk-lab apk-lab inspect path/to/app.apkm --json -
```

### `analyze` — Managed Extraction & Decompilation
Extracts artifacts into a managed run directory under `.apk-lab/<package>/<versionCode>-<inputShaPrefix>/<configDigest>/` with `.marker.json` metadata.

```bash
# Default: metadata + smali disassembly
uv run --project tools/apk-lab apk-lab analyze path/to/app.apkm

# Include JADX Java decompilation
uv run --project tools/apk-lab apk-lab analyze path/to/app.apkm --jadx

# Include Apktool resource decoding
uv run --project tools/apk-lab apk-lab analyze path/to/app.apkm --apktool
```

The command extracts all native `.so` shared libraries across the base APK and architecture splits into `<workspace>/lib/<arch>/` (e.g. `lib/arm64-v8a/libil2cpp.so`), and prints the exact directory and matching cleanup command upon completion.

### `compare` — Artifact Diffing
Compares two APK/APKM artifacts across version codes, signing certificates, split members, DEX classes/methods delta, native libraries, and assets.

```bash
uv run --project tools/apk-lab apk-lab compare old_version.apkm new_version.apkm

# Save JSON diff report
uv run --project tools/apk-lab apk-lab compare old_version.apkm new_version.apkm --json report.json

# Decompile and diff specific classes with JADX (reports unchanged, modified, added, removed, or missing)
uv run --project tools/apk-lab apk-lab compare old_version.apkm new_version.apkm --class com.example.Foo
```

### `check` — Real Patch Compatibility Testing
Applies patch bundles (`.mpp`) to an artifact using Morphe Desktop in isolated runs. Runs the default option configuration and independently inverts every boolean option to test all boundary cases. Validates postconditions:
- Output is a valid APK.
- Package name, version name, and version code remain unchanged.
- Android SDK DEX structural verification passes (`dexdump -c`).
- Uncompressed members changed in output while unrelated sentinels remain byte-identical.
- Extension classes (`app/aidan/extension/...`) are inventoried.

```bash
# Test specific patch with all option permutations
uv run --project tools/apk-lab apk-lab check path/to/app.apkm \
  --mpp patches/build/libs/patches-X.X.X.mpp \
  --package com.example.app \
  --patch "Patch Name"

# Test ALL compatible patches declared for the package
uv run --project tools/apk-lab apk-lab check path/to/app.apkm \
  --mpp patches/build/libs/patches-X.X.X.mpp \
  --package com.example.app \
  --all

# Retain workspace scratch files for inspection (ephemeral by default)
uv run --project tools/apk-lab apk-lab check path/to/app.apkm \
  --mpp patches/build/libs/patches-X.X.X.mpp \
  --package com.example.app \
  --all \
  --keep-workspace
```

### `clean` — Managed Workspace Cleanup
Safely removes managed run directories. Strictly refuses symlinks, directories missing `.marker.json`, and paths outside the workspace root.

```bash
# Clean specific run
uv run --project tools/apk-lab apk-lab clean --run .apk-lab/com.example/run-dir

# Clean all runs for an application package
uv run --project tools/apk-lab apk-lab clean --package com.example.app

# Clean runs older than N days
uv run --project tools/apk-lab apk-lab clean --stale 7

# Dry-run preview
uv run --project tools/apk-lab apk-lab clean --package com.example.app --dry-run
```
### `asm` — ARM64 Instruction Assembler & Branch Relocator
Encodes ARM64 machine instructions and computes 26-bit relative branch relocations (`b`, `bl`), returns (`ret`), and register moves (`mov`/`movz`). Deterministically formats output as space-separated hex bytes, integer opcodes, or Morphe Kotlin `byteArrayOf(...)` arrays with appropriate `.toByte()` casting for signed bytes.

```bash
# Calculate relative branch and encode to little-endian hex bytes
uv run --project tools/apk-lab apk-lab asm "bl 0x3c98ce4" --pc 0x1fcf6e8
# Output: 7f 25 73 94

# Encode return instruction directly to Kotlin byteArrayOf syntax
uv run --project tools/apk-lab apk-lab asm "ret" --format kotlin
# Output: byteArrayOf(0xc0.toByte(), 0x03, 0x5f, 0xd6.toByte())

# Encode immediate register move
uv run --project tools/apk-lab apk-lab asm "mov w1, #1"
# Output: 21 00 80 52
```
### `il2cpp` — Unity IL2CPP Metadata Extraction & Symbol Mapping
Extracts and parses Unity `assets/bin/Data/Managed/Metadata/global-metadata.dat` and companion native shared libraries (`lib/<arch>/libil2cpp.so`) across single APKs and multi-split container bundles (APKM, APKS, XAPK). Enables instant querying of stripped C# type definitions, method signatures, parameter counts, and namespaces without external tooling.

```bash
# Query symbols matching a method name substring
uv run --project tools/apk-lab apk-lab il2cpp path/to/game.apkm --query OpenShop

# Export all symbols or matches to JSON stdout or file
uv run --project tools/apk-lab apk-lab il2cpp path/to/game.apkm --query "Blackjack.*" --json symbols.json
```
### `unity` — Unity Serialized Asset & GameObject Inspector
Inspects Unity serialized asset files (`assets/bin/Data/*.assets`, `*.assets.split*`, `globalgamemanagers`) across single APKs and multi-split container bundles (APKM, APKS, XAPK). Locates `GameObject` records by name, reports their containing split chunk, and calculates the exact byte offset and value of the `m_IsActive` boolean property for direct Morphe raw resource patching.

```bash
# Locate GameObject and calculate active-state byte offset
uv run --project tools/apk-lab apk-lab unity path/to/game.apkm --gameobject Button_HelpCenter

# Output matches as JSON
uv run --project tools/apk-lab apk-lab unity path/to/game.apkm --gameobject Button_HelpCenter --json
```

### `deploy` — Deterministic Patch, Sign & Device Deployment
Applies Morphe patches from a `.mpp` bundle to an APK/APKM/XAPK/APKS artifact, aligns the output to 4-byte boundaries with `zipalign`, signs it with the standard Android debug keystore (`~/.android/debug.keystore`, auto-created with permissions `0600` if absent), streams it to an active ADB device/emulator, and optionally cold-launches the main launcher activity.

```bash
# Deploy all default-compatible patches to the single connected device and launch
uv run --project tools/apk-lab apk-lab deploy path/to/app.apkm \
  --mpp patches/build/libs/patches-X.X.X.mpp \
  --package com.example.app \
  --all \
  --launch

# Deploy explicit patches with options to a specific device emulator
uv run --project tools/apk-lab apk-lab deploy path/to/app.apkm \
  --mpp patches/build/libs/patches-X.X.X.mpp \
  --package com.example.app \
  -e "Patch Name" \
  -O primaryColor=#FF5722 \
  --device emulator-5554 \
  --reinstall \
  --out /tmp/deployed.apk

# Perform a clean install (destructively uninstalls first, clearing app data)
uv run --project tools/apk-lab apk-lab deploy path/to/app.apkm \
  --mpp patches/build/libs/patches-X.X.X.mpp \
  --package com.example.app \
  --all \
  --clean-install
```

**Key Invariants:**
- `--all` selects all compatible patches marked `default: true` for the package, transitively resolving dependencies.
- Installs use data-preserving `adb install -r` by default (and with explicit `--reinstall`). Only `--clean-install` uninstalls first.
- Staging occurs in a temporary directory inside the final output's parent directory, ensuring `os.replace` is same-filesystem atomic. Any failure before replacement leaves existing files untouched byte-for-byte.

### `monitor` — Runtime Process Health, Spin & ANR/Crash Watcher
Hooks into a running or freshly launched app process on an ADB device, monitors main thread CPU utilization, detects infinite spin loops, catches ANRs (`BIND APPLICATION ANR`), and surfaces stack traces from `ApplicationExitInfo`.

```bash
# Monitor application health for 10 seconds
uv run --project tools/apk-lab apk-lab monitor \
  --package com.example.app \
  --timeout 10.0

# Monitor with automatic thread dump capture on ANR or spin
uv run --project tools/apk-lab apk-lab monitor \
  --package com.example.app \
  --device emulator-5554 \
  --dump-threads
```

**Diagnostics & Exit Codes:**
- Samples `/proc/<pid>/task/<pid>/stat` (TID == PID) and `/proc/stat` at 500ms intervals.
- Flags a **High CPU Spin Warning** and exits with code `5` (`RUNTIME_FAILURE`) if the main thread sustains State `R` and $\ge 90\%$ CPU for 3.0 consecutive seconds (heuristic spin detection).
- Live ANRs (`ANR in <pkg>`, `BIND APPLICATION ANR`) and crashes (fatal exceptions, native SIGSEGV) immediately exit with code `5` (`RUNTIME_FAILURE`).
- On process termination, inspects `dumpsys activity exit-info`. Reasons `4` (`CRASH`), `5` (`CRASH_NATIVE`), and `6` (`ANR`) return exit code `5`. Clean exits return exit code `0`.

### `inspect-code` — Targeted Class and Method Bytecode/AST Inspection & Diff
Extracts or decompiles *only a single class or method* directly from an APK without decompiling the entire multi-gigabyte application, and provides side-by-side unified diffs between unpatched and patched APKs.

```bash
# Disassemble specific class to Smali
uv run --project tools/apk-lab apk-lab inspect-code path/to/app.apkm \
  --class com.example.app.MainActivity \
  --format smali

# Extract specific method using bare name or Dalvik descriptor
uv run --project tools/apk-lab apk-lab inspect-code path/to/app.apkm \
  --class com.example.app.MainActivity \
  --method "onCreate(Landroid/os/Bundle;)V" \
  --format smali

# Compare decompiled Java method between unpatched and patched APKs
uv run --project tools/apk-lab apk-lab inspect-code patched.apk \
  --class com.example.app.MainActivity \
  --method onCreate \
  --format java \
  --compare unpatched.apk
```

**Method Selector Rules & Limitations:**
- Multi-DEX localization searches `classes*.dex` across all splits using `dexdump` and identifies the containing location (`split:classesN.dex`).
- Disassembles Smali using pinned baksmali (`--classes`, `--code-offsets`); decompiles Java using pinned JADX (`--single-class`, `--show-bad-code`, `--no-res`).
- Method slicing: Smali extracts `.method` to `.end method`. Java uses a lexical brace-balancing scanner that skips string/char literals and comments.
- Dalvik descriptors (e.g. `foo(I)V`) are accepted only with `--format smali`. Passing descriptors with `--format java` fails fast with exit code `2` (`USAGE_OR_TOOL_ERROR`).
- Unified diffs truncate at 500 lines (`... diff truncated at 500 lines ...`) and apply ANSI colors only when stdout is a TTY.

### `validate-smali` — Pre-Injection Register & CFG Validator
Statically validates Smali instruction snippets prior to injecting them into Morphe Kotlin DSL `addInstructions` / `replaceInstruction` blocks.

```bash
# Validate instruction format register widths
uv run --project tools/apk-lab apk-lab validate-smali \
  --snippet "instance-of v0, p1, Lcom/example/Target;" \
  --locals 20 \
  --params 2

# Validate static method snippet with branch loop
uv run --project tools/apk-lab apk-lab validate-smali \
  --snippet "const/4 v0, 1\n:loop\nadd-int/lit8 v0, v0, 1\ngoto :loop" \
  --locals 2 \
  --params 0 \
  --is-static
```

**Validation Capabilities:**
- **Register Mapping**: Maps `p0..pN` registers to absolute indices (`v<locals + idx>`). On instance methods, `p0` is `this`, and `p1` is the first parameter.
- **Opcode Register Widths**: Validates Dalvik opcode bit widths:
  - Format `22c` (`instance-of`, `iget`, `iput`): 4-bit operands (`v0..v15`). Overflow emits remediation guidance (e.g. `move-object/from16`).
  - Format `21c` (`check-cast`, `new-instance`): 8-bit destination (`v0..v255`).
  - Format `11n` (`const/4`): signed 4-bit literal range (`-8..7`).
  - Format `35c` (`invoke-*`): 4-bit register list with `/range` remediation for registers $> 15$.
- **CFG Loop & Reachability Analysis**: Detects duplicate labels, missing branch targets, backward jumps (`BACKWARD_BRANCH` warning), unreachable code (`UNREACHABLE_CODE` warning), and closed cycles lacking exit or return (`POSSIBLE_CLOSED_CYCLE` warning).
- **Assembler Oracle**: Synthesizes a class wrapper and invokes pinned `smali assemble` to verify Dalvik syntax.

### `res` — Cross-Split Resource Table & Raw Member Inspector
Resolves resources and raw members across base APKs and configuration splits (`split_config.xxhdpi.apk`), reports duplicate statuses, and advises the correct Morphe patch mode.

```bash
# Query by hex resource ID
uv run --project tools/apk-lab apk-lab res path/to/app.apkm \
  --query 0x7f08028e

# Query by type/name or bare name
uv run --project tools/apk-lab apk-lab res path/to/app.apkm \
  --query @drawable/map_preview_trails

# Query by file path and extract matched drawables to disk
uv run --project tools/apk-lab apk-lab res path/to/app.apkm \
  --query res/drawable-xxhdpi/map_preview_trails.webp \
  --extract /tmp/extracted_drawables
```

**Resource Modes & Extraction Safety:**
- **Morphe Mode Guidance**:
  - `AndroidManifest.xml` and files under `res/...` require `resourcePatch` (`document(...)` for XML, `get(...)` for binary files).
  - Files under `assets/...` and `lib/...` require `rawResourcePatch` (raw zip entries).
- **Duplicate Status**: Grouped across splits as `unique` (single entry), `equivalent` (identical SHA-256 or scalar value), or `conflict` (differing hashes/values).
- **Extraction Preflight**: `--extract` preflights all destinations into safe `<slug>-<hash8>/...` subdirectories, refusing path escapes, symlinks, duplicate destination collisions, or pre-existing files before writing any file to disk.

### `fixtures` — Private R2 Cloud Fixtures
Manages the two-slot private R2 storage architecture (`fixtures/<package>/latest` and `fixtures/<package>/target`).

```bash
# Seed a target slot from a verified local artifact
uv run --project tools/apk-lab apk-lab fixtures seed \
  --package com.example.app \
  --role target \
  --artifact path/to/app.apkm

# Inspect slot metadata in R2
uv run --project tools/apk-lab apk-lab fixtures info --package com.example.app

# Download a fixture slot
uv run --project tools/apk-lab apk-lab fixtures download \
  --package com.example.app \
  --role target \
  --out /tmp/target.apk
```

### `acquire` — Artifact Acquisition from Google Play
Downloads and normalizes Android application artifacts directly from the Google Play Store using `apkeep` (native Play Store downloader) or `goopdl` fallback. Validates downloaded package signatures, container integrity, and version metadata.

```bash
# Acquire latest version of an application package
uv run --project tools/apk-lab apk-lab acquire com.example.app

# Acquire specific version to an explicit output directory
uv run --project tools/apk-lab apk-lab acquire com.example.app --version 2.22.08 --out-dir /tmp/artifacts
```

---

## 3. Standard Workflows

### Authoring Patches for a New Application
1. **Inspect**: Run `apk-lab inspect <artifact>` to determine package name, version, signing cert SHA-256, and container structure.
2. **Analyze**: Run `apk-lab analyze <artifact> --smali` to extract smali and native shared libraries (`lib/<arch>/*.so`) into a managed run.
3. **Native & Unity Inspection** (if applicable):
   - For Unity IL2CPP applications: Run `apk-lab il2cpp <artifact> --query <Symbol>` to locate stripped C# classes and method signatures in `global-metadata.dat`.
   - For Unity serialized assets: Run `apk-lab unity <artifact> --gameobject <Name>` to find GameObject entries and calculate `m_IsActive` byte offsets across asset splits.
   - For ARM64 binary patches: Run `apk-lab asm "<instruction>" --pc <pc> --format kotlin` to calculate 26-bit branch offsets and emit Morphe Kotlin `byteArrayOf(...)` hooks.
4. **Implement**: Define fail-fast Morphe bytecode and resource patches in Kotlin using `bytecodePatch`, `resourcePatch`, or `rawResourcePatch`. Register `Compatibility(name, packageName, apkFileType, signatures, targets)`.
5. **Compile**: Run `./gradlew :patches:buildAndroid --no-daemon`.
6. **Verify**: Run `apk-lab check <artifact> --mpp patches/build/libs/patches-*.mpp --package <pkg> --all`.
7. **Device Smoke Test**: Install patched APK on an emulator or test device (`adb install -r output.apk`) and smoke test user flows.
8. **Clean**: Clean the workspace with `apk-lab clean --run <path>`.

### Migrating Existing Patches on App Updates
1. **Acquire or Ingest**: Download update via `apk-lab acquire <pkg>` or load from local artifact.
2. **Compare**: Run `apk-lab compare <old_artifact> <new_artifact>` to identify changed DEX method counts, split changes, native libraries, and resource modifications.
3. **Forced Baseline Check**: Run `apk-lab check <new_artifact> --mpp <bundle> --package <pkg> --all --force` to capture the failing-before baseline and pinpoint obsolete anchors.
4. **Analyze & Remap**: Run `apk-lab analyze <new_artifact> --smali` to rediscover changed classes, obfuscated descriptors, and instructions using stable landmarks. For native/Unity patches, rerun `apk-lab il2cpp` and `apk-lab unity` to update shifted offsets.
5. **Update Source**: Update patch definitions, extension classes, and bump target in `Constants.kt`.
6. **Matrix Check**: Rebuild and run `apk-lab check <new_artifact> --mpp <bundle> --package <pkg> --all`. Require 100% pass rate.
7. **Documentation & List**: Update reverse engineering specs in `docs/<app>/`, run `./gradlew generatePatchesList`, and sync README with `generate_patches_readme.py`.

---

## 4. Invariants & Storage Rules

- **Durable vs. Disposable**: Durable repository changes are patch Kotlin sources, Java extension sources, compatibility metadata, docs, tests, and web UI. APK files, decompiled smali/java trees, patched APK outputs, and downloader tokens are **disposable and never committed**.
- **Private Fixtures**: APK fixture bytes exist only in runner temp memory or the private `aidans-patches-apk-fixtures` R2 bucket. Public worker endpoints and status APIs only return metadata and status badges, never proprietary binaries.
- **Fail-Fast Invariant**: All patches enforce strict preconditions; if bytecode shapes differ, patches throw `PatchException` rather than generating corrupted APKs.
