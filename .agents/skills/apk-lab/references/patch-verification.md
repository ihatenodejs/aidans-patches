# Morphe Patch Compatibility & Verification

Execute real patch compatibility testing using Morphe Desktop, test boolean option inversions, verify DEX structural integrity, and automate CI test matrix generation.

---

## 1. `check` — Real Patch Compatibility Testing

Applies patch bundles (`.mpp`) to an APK or split bundle (APKM/XAPK) using pinned Morphe Desktop in isolated ephemeral workspaces.

### Why Real Patch Testing is Mandatory
A patch cannot be marked compatible in `Constants.kt` from a metadata diff or code inspection alone. A version is supported **ONLY** when `apk-lab check ... --all` executes every patch baseline and independent boolean-option inversion, and passes all verification gates:
1. **Morphe Execution**: Morphe Desktop executes without throwing unhandled exceptions or failing AST anchors.
2. **Identity Invariance**: Package name and version code remain unchanged in the output APK.
3. **Android SDK DEX Verification**: Runs `dexdump -c` across all generated DEX files to verify structural validity and bytecode formatting.
4. **Member Delta Verification**: At least one patchable member (DEX, manifest, resources, assets, native libs) is modified in the output APK.
5. **Extension Verification**: Classes injected from `extensions/extension.mpe` (`app/aidan/extension/...`) are inventoried in the output APK when present.

### Automated Option Permutation Testing
When a patch declares boolean configuration options, `apk-lab check` generates and executes:
- **Baseline test case**: Default patch configuration.
- **Inverted test cases**: One independent test case per boolean option with its value inverted (e.g. `primaryColor`, `filterMarketplace=true`).

### CLI Invocation

```bash
# Test a specific patch with all option permutations
uv run --project tools/apk-lab apk-lab check path/to/app.apkm \
  --mpp patches/build/libs/patches-X.X.X.mpp \
  --package com.example.app \
  --patch "Patch Name"

# Test ALL compatible patches declared for the package
uv run --project tools/apk-lab apk-lab check path/to/app.apkm \
  --mpp patches/build/libs/patches-X.X.X.mpp \
  --package com.example.app \
  --all

# Force check to establish failing baseline during app updates
uv run --project tools/apk-lab apk-lab check path/to/app.apkm \
  --mpp patches/build/libs/patches-X.X.X.mpp \
  --package com.example.app \
  --all \
  --force

# Retain workspace scratch files for post-mortem debugging
uv run --project tools/apk-lab apk-lab check path/to/app.apkm \
  --mpp patches/build/libs/patches-X.X.X.mpp \
  --package com.example.app \
  --all \
  --keep-workspace

# Output structured compatibility report to JSON
uv run --project tools/apk-lab apk-lab check path/to/app.apkm \
  --mpp patches/build/libs/patches-X.X.X.mpp \
  --package com.example.app \
  --all \
  --json report.json
```

### CLI Arguments
- `artifact` (required positional): Path to APK or split bundle.
- `--mpp MPP` (required): Path to compiled Morphe patch bundle (`.mpp`).
- `--package PACKAGE`: Package name (inferred from artifact manifest if omitted).
- `--all`: Test all compatible patches declared for this package in `patches-list.json`.
- `--patch NAME ...`: Repeatable name of specific patch(es) to test.
- `--force`: Bypass target version compatibility check in Morphe metadata (vital for app updates to establish failing baseline).
- `--keep-workspace`: Retain ephemeral workspace under `.apk-lab/` instead of cleaning up on exit.
- `--json [PATH|-]`: Output structured JSON report (`PatchCompatibilityReport`).

### Exit Codes
- `0`: All patch test cases passed.
- `1`: `COMPATIBILITY_FAILURE` — One or more test cases failed to apply, violated postconditions, or failed DEX structural verification.
- `2`: `USAGE_OR_TOOL_ERROR` — Missing files or invalid patch name.
- `4`: `INFRASTRUCTURE_FAILURE` — Uncaught exceptions, malformed artifacts, missing Morphe binary, or workspace errors.
---

## 2. CI Automation Module (`apk_lab.compatibility_ci`)

Orchestrates automated patch testing and R2 fixture reconciliation in GitHub Actions workflows (`.github/workflows/apk-compatibility.yml`).

### Subcommands
1. **`matrix`**: Analyzes Git diff between commits to generate a dynamic matrix of affected packages.
   ```bash
   python -m apk_lab.compatibility_ci matrix --changed-files "patches/src/main/kotlin/app/aidan/patches/sezzle/..."
   ```
2. **`start`**: Registers a test run with dispatch metadata.
   ```bash
   python -m apk_lab.compatibility_ci start --package com.example.app --request-id 12345 --git-revision abc1234
   ```
3. **`run`**: Downloads test fixtures from R2, verifies patch bundle compatibility, checks targets, and updates status.
   ```bash
   python -m apk_lab.compatibility_ci run --package com.example.app --mpp patches/build/libs/patches-1.0.0.mpp
   ```

---

## 3. Python API Reference

```python
from pathlib import Path
from apk_lab.morphe import (
    load_patches_list,
    get_compatible_patches,
    run_compatibility_check,
    verify_sdk_dex,
)
from apk_lab.tools import ToolManager
from apk_lab.workspace import WorkspaceManager

# Load patch definitions from patches-list.json
patches_data = load_patches_list()
compatible = get_compatible_patches("com.example.app", patches_data, all_patches=True)
for p in compatible:
    print(f"Patch: {p.name} (default={p.default}, deps={p.dependencies})")

# Run full compatibility check programmatically
report, exit_code = run_compatibility_check(
    artifact_path=Path("path/to/app.apkm"),
    mpp_path=Path("patches/build/libs/patches-1.0.0.mpp"),
    expected_package="com.example.app",
    all_patches=True,
    force=False,
    keep_workspace=False,
)

print(f"Status: {report.overall_status} ({report.passed_cases}/{report.total_cases} passed)")
for case in report.test_cases:
    print(f" - {case.patch_name} {case.options}: {'PASS' if case.success else 'FAIL: ' + str(case.error_message)}")

# Direct SDK DEX structural verification
dex_valid, error = verify_sdk_dex(Path("output.apk"))
print(f"DEX Valid: {dex_valid} (Error: {error})")
```
