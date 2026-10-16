# APK Lab Toolchain & Workspace Management

Manage host prerequisites, pinned reverse engineering binaries, diagnostics, and managed scratch workspaces for APK analysis.

---

## 1. Prerequisites & Host Requirements

APK Lab orchestrates external tools pinned with immutable SHA-256 digests in `tools/apk-lab/tools.lock.json`. Before running analysis, verify the host environment:

| Requirement | Supported Versions | Resolution / Detection Order |
|---|---|---|
| **Python** | `3.12+` | Managed via `uv` (`tools/apk-lab/pyproject.toml`) |
| **Java / JDK** | `21+` (Eclipse Temurin recommended) | PATH (`java`) |
| **Android Build Tools** | `36.0.0` (`aapt2`, `apksigner`, `zipalign`, `dexdump`) | `$ANDROID_HOME/build-tools/`, `$ANDROID_SDK_ROOT/build-tools/`, `local.properties` (`sdk.dir`) |
| **Android Platform Tools** | `adb` | `$ANDROID_HOME/platform-tools/adb`, `$ANDROID_SDK_ROOT/platform-tools/adb`, PATH (runtime only; not probed by `doctor`) |
| **uv** | Modern | PATH (`uv run --project tools/apk-lab`) |

External tools cached in `~/.cache/aidans-patches/apk-lab/tools/`:
- **Morphe Desktop**: `1.18.0` (JAR)
- **JADX**: `1.5.6` (ZIP with bin/jadx)
- **Apktool**: `3.0.3` (JAR)
- **baksmali**: `3.0.10` (JAR)
- **smali**: `3.0.10` (JAR)
- **apkeep**: `1.1.0` (native binary, Linux x86_64 only; CI profile)

---

## 2. CLI Reference
Global options (e.g. `--workspace-root PATH`, default: `.apk-lab`) precede the subcommand:
```bash
uv run --project tools/apk-lab apk-lab [--workspace-root PATH] <subcommand> [flags]
```
### `setup` — Toolchain Download & Verification

Downloads and verifies all pinned tools for the specified profile using SHA-256 checksums from `tools.lock.json`.

```bash
# Install analysis tools (Morphe Desktop, JADX, Apktool, baksmali, smali)
uv run --project tools/apk-lab apk-lab setup --profile analysis

# Install CI tools (analysis profile + apkeep on Linux x86_64)
uv run --project tools/apk-lab apk-lab setup --profile ci
```

- **Arguments**:
  - `--profile {analysis,ci}`: Tool profile (default: `analysis`).
- **Exit Codes**:
  - `0`: All tools installed and passed version probe.
  - `2`: Download, verification, probe, or unsupported platform failure.

### `doctor` — Diagnostics & Prerequisite Check

Validates host tools, cache status, workspace write access, and optional CI credential presence.

```bash
# Check host readiness and cached tool status
uv run --project tools/apk-lab apk-lab doctor

# Check in CI mode with JSON output to stdout
uv run --project tools/apk-lab apk-lab doctor --ci --json -

# Save diagnostic report to file
uv run --project tools/apk-lab apk-lab doctor --json report.json
```

- **Arguments**:
  - `--ci`: Check CI credentials (`APKEEP_ACCOUNTS`, `R2_ACCOUNT_ID`, `R2_ACCESS_KEY_ID`, `R2_SECRET_ACCESS_KEY`, `COMPATIBILITY_STATUS_SECRET`) as boolean flags without printing values.
  - `--profile {analysis,ci}`: Check prerequisites for specified profile.
  - `--json [PATH|-]`: Output structured JSON (stdout if `-` or flag without path).
- Note: To customize the workspace root, pass global `--workspace-root PATH` before `doctor`.
- **Exit Codes**:
  - `0`: All prerequisites and tools satisfied.
  - `2`: Missing prerequisite, uninstalled tool, or unwritable workspace.

### `clean` — Safe Managed Workspace Cleanup

Safely removes run directories created under `.apk-lab/`. **NEVER** use raw `rm -rf` on workspace folders; `clean` enforces safety checks.

```bash
# Clean a specific run directory
uv run --project tools/apk-lab apk-lab clean --run .apk-lab/com.example.app/100-abc123456789/def987654321

# Clean all runs for a specific package
uv run --project tools/apk-lab apk-lab clean --package com.example.app

# Clean all runs older than 7 days
uv run --project tools/apk-lab apk-lab clean --stale 7

# Dry-run preview of what would be removed
uv run --project tools/apk-lab apk-lab clean --package com.example.app --dry-run
```

- **Arguments**:
  - `--run PATH`: Clean specific run path.
  - `--package NAME`: Clean all runs for application package.
  - `--stale DAYS`: Clean runs older than non-negative float days.
  - `--dry-run`: List targets without deleting files.
- **Safety Invariants**:
  - Validates that target path is inside workspace root.
  - Rejects symlinks and unresolved path escapes.
  - Requires valid schema-1 `.marker.json` inside run directories.
  - `--run PATH`: strictly rejects targets with missing, corrupted, or unmanaged markers (returns exit code `2`).
  - `--package NAME` and `--stale DAYS`: safely skip invalid, unreadable, or unmarked directories and proceed with valid targets (returns exit code `0`).
  - Automatically removes empty parent package directories after cleaning runs.
- **Exit Codes**:
  - `0`: Cleanup succeeded (or completed skipping invalid candidates in package/stale mode).
  - `2`: Target not found, unsafe path, or invalid workspace marker in `--run` mode.

---

## 3. Workspace Structure & Marker Contract

Analysis runs create isolated, reproducible workspaces under `.apk-lab/`:
```text
.apk-lab/
  └── <package_name>/
      └── <version_code>-<sha256_prefix_12>/
          └── <config_and_tool_digest_12>/
              ├── .marker.json
              ├── base/
              ├── splits/
              ├── lib/
              └── smali/
```

### Marker Format (`.marker.json`)
```json
{
  "schemaVersion": 1,
  "inputCanonicalPath": "/path/to/app.apkm",
  "inputSha256": "3a7b...",
  "command": "analyze",
  "configDigest": "def987654321",
  "toolVersions": {
    "baksmali": "3.0.10",
    "jadx": "1.5.6"
  },
  "createdAt": "2026-10-10T12:00:00Z"
}
```

---

## 4. Python API Reference

```python
from pathlib import Path
from apk_lab.tools import ToolManager, check_host_prerequisites
from apk_lab.workspace import WorkspaceManager

# Toolchain Management
tool_mgr = ToolManager()
is_ready = tool_mgr.is_tool_installed("baksmali")
baksmali_path = tool_mgr.get_executable_path("baksmali")
res = tool_mgr.run_tool_cmd("baksmali", ["--version"])

# Host Diagnostics
doctor_report = check_host_prerequisites(ci_mode=False, workspace_root=".apk-lab")
print(f"All ready: {doctor_report.all_ready}")

# Workspace Management
ws_mgr = WorkspaceManager(root=Path(".apk-lab"))
run_dir = ws_mgr.create_run_dir(
    package_name="com.example.app",
    version_code=100,
    input_sha256="abc...",
    input_path="app.apk",
    command="custom_task",
    config={"flag": True},
    tool_versions={"morphe": "1.18.0"},
)

# Safe ephemeral workspace context manager
with ws_mgr.ephemeral_run(
    package_name="com.example.app",
    version_code=100,
    input_sha256="abc...",
    input_path="app.apk",
    command="task",
    config={},
    tool_versions={},
    keep_workspace=False,
) as temp_run_dir:
    # Process files inside temp_run_dir; deleted automatically on exit
    pass
```
