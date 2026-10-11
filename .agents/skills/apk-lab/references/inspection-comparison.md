# Artifact Inspection & Version Comparison

Inspect single APKs and multi-split container bundles (APKM, XAPK, APKS), parse metadata securely, extract signing certificates and DEX metrics, and compare changes across application versions.

---

## 1. `inspect` — Secure Artifact Inspection

Inspects container architecture, manifest badging, DEX metrics, and native components without creating persistent workspaces.

### Security Invariants & Container Classification
- **Safe ZIP Validation**:
  - Rejects directory traversal (`..`), absolute paths, and POSIX symlinks.
  - Rejects archive bombs: max 100,000 entries, max 4 GiB total expanded size, max 100.0 compression ratio on entries > 1 MiB.
  - Rejects duplicate member names.
- **Container Classification**:
  - `APKS`: Contains `toc.pb`.
  - `XAPK`: Contains `manifest.json`.
  - `APK`: Contains root `AndroidManifest.xml`.
  - `APKM`: Contains `info.json` or nested `.apk` split files.
- **Split Invariants**:
  - Requires exactly one base APK (member with no `split` attribute in manifest).
  - Enforces identical package name and version code across all split APKs.
  - Enforces identical signing certificate SHA-256 across all splits.

### CLI Invocation

```bash
# Human-readable inspection summary
uv run --project tools/apk-lab apk-lab inspect path/to/app.apkm

# JSON output printed to stdout
uv run --project tools/apk-lab apk-lab inspect path/to/app.apkm --json -

# Write JSON inspection report to file
uv run --project tools/apk-lab apk-lab inspect path/to/app.apkm --json /tmp/inspection.json
```

### CLI Arguments
- `artifact` (required positional): Path to `.apk`, `.apkm`, `.xapk`, or `.apks` file.
- `--json [PATH|-]`: Output structured JSON report. Omitting value or passing `-` writes to stdout; otherwise writes to given path.

### Output Fields (`ArtifactInspection`)
- `filePath`: Canonical path to artifact.
- `containerType`: `APK`, `APKM`, `XAPK`, `APKS`, or `UNKNOWN`.
- `fileSize`, `sha256`: Container file size and SHA-256 digest.
- `packageName`, `versionName`, `versionCode`: Android manifest identity.
- `minSdk`, `targetSdk`: API level requirements.
- `signingCertificateSha256`: Hex SHA-256 digest of signing certificate (from `apksigner`).
- `splits`: List of split APK metadata (`filename`, `splitName`, `sha256`, `size`, `isBase`).
- `dexClassesCount`, `dexMethodsCount`: Aggregate counts across all DEX files.
- `dexFiles`, `nativeLibraries`, `resources`, `assets`: Inventory lists.
- `warnings`: Diagnostic warnings encountered during inspection.

### Exit Codes
- `0`: Inspection succeeded.
- `2`: Artifact file does not exist or build tools missing.
- `3`: Invalid artifact (archive security violation, inconsistent splits, corrupt manifest).

---

## 2. `compare` — Application Version Diffing

Compares two APK/APKM artifacts across metadata, split architecture, native libraries, permissions, and decompiled class bytecode.

### Diff Capabilities
- **Metadata**: Version name, version code delta, signing certificate changes.
- **Splits**: Added, removed, or modified split APKs (based on content hash).
- **DEX Metrics**: Net change in class count and method count.
- **Components & Permissions**: Added and removed activities, services, receivers, providers, and manifest permissions.
- **Native Libraries & Assets**: Added, removed, and modified files under `lib/` and `assets/`.
- **Targeted Class Decompilation Diff**: Uses pinned JADX to decompile specified classes in both artifacts and generates a unified diff. Diff output is truncated at 500 lines if large.

### CLI Invocation

```bash
# Compare two application versions
uv run --project tools/apk-lab apk-lab compare old_app.apkm new_app.apkm

# Compare with targeted Java class diffing via JADX
uv run --project tools/apk-lab apk-lab compare old_app.apkm new_app.apkm \
  --class com.example.app.MainActivity \
  --class com.example.app.data.SessionManager

# Export comparison diff to JSON
uv run --project tools/apk-lab apk-lab compare old_app.apkm new_app.apkm --json diff_report.json
```

### CLI Arguments
- `old_artifact` (required positional): Path to baseline artifact.
- `new_artifact` (required positional): Path to target/updated artifact.
- `--class CLASS`: Repeatable fully-qualified class name to decompile and diff with JADX.
- `--json [PATH|-]`: Output structured JSON report.

### Exit Codes
- `0`: Comparison succeeded.
- `2`: Artifact not found, or JADX failed on requested class comparison.
- `3`: Invalid artifact format or archive security error.

---

## 3. Python API Reference

```python
from pathlib import Path
from apk_lab.inspection import inspect_artifact
from apk_lab.comparison import compare_artifacts, format_comparison_summary
from apk_lab.tools import ToolManager

# Inspect an artifact
inspection = inspect_artifact("path/to/app.apkm")
print(f"Package: {inspection.package_name} v{inspection.version_name} ({inspection.version_code})")
print(f"DEX classes: {inspection.dex_classes_count}, methods: {inspection.dex_methods_count}")
print(f"Signer: {inspection.signing_certificate_sha256}")

# Export inspection to dictionary
data = inspection.to_dict()

# Compare two versions
tool_mgr = ToolManager()
comparison = compare_artifacts(
    old_path="old.apkm",
    new_path="new.apkm",
    requested_classes=["com.example.app.MainActivity"],
    tool_mgr=tool_mgr,
)

print(format_comparison_summary(comparison))

# Inspect specific class diff
if "com.example.app.MainActivity" in comparison.classes_diff:
    diff_info = comparison.classes_diff["com.example.app.MainActivity"]
    status = diff_info["status"]  # unchanged, modified, added, removed, missing
    if status == "modified":
        print("".join(diff_info["diff"]))
```
