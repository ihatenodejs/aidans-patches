# Device Deployment, Keystore Signing & Runtime Health Monitoring

Apply Morphe patches, align, sign with debug keystore, stream to connected ADB devices, cold-launch activities, and monitor runtime process health, CPU spin, ANRs, and crashes.

---

## 1. `deploy` — Deterministic Patch, Sign & Install

Applies Morphe patches from a `.mpp` bundle to an APK or split container (APKM/XAPK/APKS), aligns with `zipalign`, signs with the negotiated keystore, installs to an ADB device/emulator, and optionally cold-launches the app.

### Deployment Pipeline & Invariants
1. **Patch Application**: Executes pinned Morphe Desktop in a temporary scratch workspace.
2. **Alignment**: Runs `zipalign -f 4` to align 32-bit and 64-bit uncompressed shared libraries to 4-byte boundaries.
3. **Keystore Negotiation & Signing**:
   - Signs the aligned APK using `apksigner`.
   - **Automatic Keystore Negotiation**: If `--keystore` is omitted, `apk-lab` inspects the installed package signature on the target device via ADB. If the installed app signature matches Morphe's bundled key (`6959a7790bff42cc...`), Morphe's cached keystore (`morphe-data/morphe.keystore`) is used automatically with alias `Morphe` and password `Morphe`, avoiding `INSTALL_FAILED_UPDATE_INCOMPATIBLE`. Otherwise, it defaults to the standard Android debug keystore (`~/.android/debug.keystore`, auto-created with secure `0600` permissions if absent).
   - Explicit credentials can be provided via `--keystore PATH`, `--ks-alias ALIAS`, `--ks-pass PASS`, and `--key-pass PASS`.
4. **ADB Streaming Install**:
   - Default / `--reinstall`: Runs `adb install -r` (preserves existing app data and sandbox).
   - `--clean-install`: Attempts `adb uninstall <package>` first (clears app data; uninstall failure is ignored), then `adb install`.
5. **Launcher Activity Execution**: When `--launch` is passed, parses the launchable activity from manifest badging and invokes `adb shell am start -n <package>/<activity>`.
6. **Atomic File Staging & Input Preservation**: Staging occurs inside a temporary directory in the final output's parent folder, ensuring `os.replace` is same-filesystem atomic. Output paths are checked upfront to guarantee they never alias or overwrite the source artifact. Failures before destination replacement leave existing output files untouched; installation or launch failures occur after the destination file has already been replaced.

### CLI Invocation

```bash
# Deploy all default-compatible patches to the connected device and cold-launch
uv run --project tools/apk-lab apk-lab deploy path/to/app.apkm \
  --mpp patches/build/libs/patches-X.X.X.mpp \
  --package com.example.app \
  --all \
  --launch

# Automatically compile patch bundle via Gradle and deploy
uv run --project tools/apk-lab apk-lab deploy path/to/app.apkm \
  --build \
  --package com.example.app \
  --all

# Deploy with custom keystore credentials
uv run --project tools/apk-lab apk-lab deploy path/to/app.apkm \
  --mpp patches/build/libs/patches-X.X.X.mpp \
  --package com.example.app \
  --all \
  --keystore /path/to/custom.keystore \
  --ks-alias mykey \
  --ks-pass secretpass \
  --key-pass secretpass

# Clean install (destructively clears existing app data before installing)
uv run --project tools/apk-lab apk-lab deploy path/to/app.apkm \
  --mpp patches/build/libs/patches-X.X.X.mpp \
  --package com.example.app \
  --all \
  --clean-install
```

### CLI Arguments
- `artifact` (required positional): Path to APK/APKM/XAPK/APKS.
- `--mpp MPP`: Path to `.mpp` patch bundle (optional if `--build` is specified).
- `--build`: Automatically compile newest `.mpp` patch bundle via Gradle (`./gradlew :patches:buildAndroid`).
- `--package PACKAGE` (required): Package name.
- `--all` or `-e` / `--enable PATCH ...` (one required, mutually exclusive): Select all default patches or specific patches.
- `-O` / `--option KEY=VALUE ...`: Configuration options for selected patches.
- `--device SERIAL`: Target specific ADB device serial (required only if multiple devices connected).
- `--launch`: Cold-launch the main activity after installation.
- `--reinstall`: Reinstall preserving data (`adb install -r`, default).
- `--clean-install`: Uninstall before install (`adb uninstall && adb install`).
- `--keystore PATH`: Custom signing keystore (must use alias `androiddebugkey` and password `android`).
- `--out PATH`: Copy the patched, signed APK to a specific output path.
- `--keystore PATH`: Custom signing keystore.
- `--ks-alias ALIAS`: Keystore key alias.
- `--ks-pass PASS`: Keystore password.
- `--key-pass PASS`: Key password.

### Exit Codes
- `0`: Deployment succeeded.
- `2`: Usage error, missing files, or device selection error.
- `4`: Infrastructure error (ADB failure, Morphe failure, signing error).

---

## 2. `monitor` — Runtime Process Health, Spin & ANR Watcher

Hooks into a running or freshly launched app process via ADB, monitors main thread CPU utilization, detects infinite spin loops, catches ANRs and native crashes, and retrieves diagnostic stack traces.

### Monitoring & Detection Rules
- **Main Thread CPU Spin Detection**:
  - Samples `/proc/<pid>/task/<pid>/stat` (TID == PID) and `/proc/stat` at 500ms intervals.
  - Flags a **High CPU Spin Warning** and exits with code `5` (`RUNTIME_FAILURE`) if the main thread sustains State `R` (running) and $\ge 90\%$ CPU for 3.0 consecutive seconds (6 samples).
- **ANR Detection**:
  - Monitors live logcat for `ANR in <package>` and `BIND APPLICATION ANR`.
  - Immediately exits with code `5` (`RUNTIME_FAILURE`).
- **Crash Detection**:
  - Catches unhandled Java exceptions and native crashes (e.g. `SIGSEGV`, `SIGABRT`).
  - Immediately exits with code `5` (`RUNTIME_FAILURE`).
- **Process Exit & Diagnostics**:
  - On process termination, inspects `dumpsys activity exit-info`.
  - Reasons `4` (`CRASH`), `5` (`CRASH_NATIVE`), and `6` (`ANR`) return exit code `5`.
  - Clean exits return exit code `0`.
- **Thread Dump Capture**:
  - When `--dump-threads` is enabled, sends `kill -3` to the target PID upon detecting ANR or high CPU spin and extracts the thread traces from logcat.

### CLI Invocation

```bash
# Monitor application process for 10 seconds
uv run --project tools/apk-lab apk-lab monitor \
  --package com.example.app \
  --timeout 10.0

# Monitor with automatic thread dump capture on ANR or spin
uv run --project tools/apk-lab apk-lab monitor \
  --package com.example.app \
  --device emulator-5554 \
  --dump-threads
```

### CLI Arguments
- `--package PACKAGE` (required): Package name to monitor.
- `--device SERIAL`: Specific ADB device serial.
- `--timeout SECONDS`: Monitoring duration (default: monitor until exit or Ctrl+C).
- `--dump-threads`: Capture thread dump on spin or ANR.

### Exit Codes
- `0`: Healthy monitoring session or clean process termination.
- `2`: Device not connected or multiple devices without `--device`.
- `5`: `RUNTIME_FAILURE` — Package did not start before `--timeout`, high CPU spin loop, ANR, crash, or fatal exit-info reason.


## 3. `crash-trace` — Smali-Aware Crash Harvester

Harvests recent Android logcat crash reports for a package, extracting fatal exceptions, ANRs, and verification errors. When an analysis workspace exists at `.apk-lab/<package>`, stack trace frames are cross-referenced with Smali bytecode across multi-dex directories (`smali/`, `smali_classes2/`, etc.) to show exact file locations, method signatures, and instruction snippets.

### CLI Invocation

```bash
# Harvest recent crash reports for a package
uv run --project tools/apk-lab apk-lab crash-trace --package com.example.app

# Output structured JSON including resolved Smali frames
uv run --project tools/apk-lab apk-lab crash-trace \
  --package com.example.app \
  --device emulator-5554 \
  --lines 10000 \
  --json
```

### CLI Arguments
- `--package PACKAGE` (required): Target application package name.
- `--device SERIAL`: Specific ADB device serial.
- `--lines N`: Number of logcat lines to inspect (default: `5000`).
- `--json`: Output structured JSON reports including `resolved_frames` (`smali_path`, `method_signature`, `code_snippet`).

---

## 4. `dev` — Rapid Iteration Workflow

Automates the inner development loop for patch development. Executes a complete iteration cycle with a single command:
1. **Repo Root Discovery**: Resolves the root of the Git repository containing patch sources.
2. **Patch Compilation**: Executes `./gradlew :patches:buildAndroid` to compile the newest `.mpp` patch bundle.
3. **Artifact Discovery**: Locates the matching target APK/APKM artifact for the given application slug/name.
4. **Package Inspection**: Inspects the artifact manifest to extract the target package name.
5. **Keystore Negotiation & Deployment**: Evaluates target device signatures, resolves the appropriate keystore (Morphe vs. debug), applies patches, zipaligns, signs, installs to device, and cold-launches the application.

### CLI Invocation

```bash
# Iterate on an app: compile patches, locate artifact, deploy, and launch
uv run --project tools/apk-lab apk-lab dev geocaching

# Target specific emulator with clean installation
uv run --project tools/apk-lab apk-lab dev sezzle --device emulator-5554 --clean-install
```

### CLI Arguments
- `app` (required positional): Application name or package identifier (e.g. `geocaching`, `sezzle`).
- `--device SERIAL`: Specific ADB device serial.
- `--launch`: Launch main activity after installation (default: `True`).
- `--all`: Apply all compatible patches (default: `True`).
- `--reinstall`: Reinstall preserving data (`adb install -r`, default).
- `--clean-install`: Clean install by uninstalling first.

---

## 5. Python API Reference

```python
from pathlib import Path
from apk_lab.device import AdbClient, select_device, monitor_application, harvest_recent_crashes
from apk_lab.deployment import deploy_artifact
from apk_lab.tools import ToolManager

# ADB device selection
adb = AdbClient()
devices = adb.devices()
serial = select_device(devices, explicit_serial=None)  # Auto-selects single online device

# Deploy patched artifact
tool_mgr = ToolManager()
result = deploy_artifact(
    artifact_path=Path("app.apkm"),
    mpp_path=Path("patches/build/libs/patches-1.0.0.mpp"),
    package_name="com.example.app",
    all_defaults=True,
    enabled_patches=[],
    raw_options=[],
    device_serial=serial,
    launch=True,
    install_mode="reinstall",
    output_path=Path("/tmp/patched.apk"),
    tool_manager=tool_mgr,
)

print(f"Deployed {result.package_name} to {result.device_serial}")
print(f"Patches applied: {result.applied_patches}")

# Monitor process health for 10 seconds
exit_code = monitor_application(
    client=adb,
    package="com.example.app",
    serial=serial,
    timeout=10.0,
    dump_threads=True,
)

if exit_code == 0:
    print("Application running healthy!")
else:
    print("Application encountered spin, ANR, or crash!")

# Harvest recent crash reports from logcat
crashes = harvest_recent_crashes(package="com.example.app", device=serial, lines=5000)
for crash in crashes:
    print(f"Crash: {crash.exception_class}: {crash.message}")
```
