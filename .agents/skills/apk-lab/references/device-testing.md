# Device Deployment, Keystore Signing & Runtime Health Monitoring

Apply Morphe patches, align, sign with debug keystore, stream to connected ADB devices, cold-launch activities, and monitor runtime process health, CPU spin, ANRs, and crashes.

---

## 1. `deploy` — Deterministic Patch, Sign & Install

Applies Morphe patches from a `.mpp` bundle to an APK or split container (APKM/XAPK/APKS), aligns with `zipalign`, signs with the Android debug keystore, installs to an ADB device/emulator, and optionally cold-launches the app.

### Deployment Pipeline & Invariants
1. **Patch Application**: Executes pinned Morphe Desktop in a temporary scratch workspace.
2. **Alignment**: Runs `zipalign -f -p 4` to align 32-bit and 64-bit uncompressed shared libraries to 4-byte boundaries.
3. **Debug Signing**: Signs the aligned APK using `apksigner` and the standard Android debug keystore (`~/.android/debug.keystore`). If absent, automatically generates the keystore with secure `0600` permissions.
4. **ADB Streaming Install**:
   - Default / `--reinstall`: Runs `adb install -r` (preserves existing app data and sandbox).
   - `--clean-install`: Runs `adb uninstall <package>` first (clears app data), then `adb install`.
5. **Launcher Activity Execution**: When `--launch` is passed, parses the launchable activity from manifest badging and invokes `adb shell am start -n <package>/<activity>`.
6. **Atomic File Staging**: Staging occurs inside a temporary directory in the final output's parent folder, ensuring `os.replace` is same-filesystem atomic. Failures leave existing output files untouched.

### CLI Invocation

```bash
# Deploy all default-compatible patches to the connected device and cold-launch
uv run --project tools/apk-lab apk-lab deploy path/to/app.apkm \
  --mpp patches/build/libs/patches-X.X.X.mpp \
  --package com.example.app \
  --all \
  --launch

# Deploy specific patch with options to a specific emulator
uv run --project tools/apk-lab apk-lab deploy path/to/app.apkm \
  --mpp patches/build/libs/patches-X.X.X.mpp \
  --package com.example.app \
  -e "Change Brand Color" \
  -O primaryColor=#FF5722 \
  --device emulator-5554 \
  --out /tmp/deployed.apk

# Clean install (destructively clears existing app data before installing)
uv run --project tools/apk-lab apk-lab deploy path/to/app.apkm \
  --mpp patches/build/libs/patches-X.X.X.mpp \
  --package com.example.app \
  --all \
  --clean-install
```

### CLI Arguments
- `artifact` (required positional): Path to APK/APKM/XAPK/APKS.
- `--mpp MPP` (required): Path to `.mpp` patch bundle.
- `--package PACKAGE` (required): Package name.
- `--all` or `-e` / `--enable PATCH ...` (one required, mutually exclusive): Select all default patches or specific patches.
- `-O` / `--option KEY=VALUE ...`: Configuration options for selected patches.
- `--device SERIAL`: Target specific ADB device serial (required only if multiple devices connected).
- `--launch`: Cold-launch the main activity after installation.
- `--reinstall`: Reinstall preserving data (`adb install -r`, default).
- `--clean-install`: Uninstall before install (`adb uninstall && adb install`).
- `--out PATH`: Copy the patched, signed APK to a specific output path.

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
- `2`: Device not connected, multiple devices without `--device`, or package not running.
- `5`: `RUNTIME_FAILURE` — High CPU spin loop, ANR, crash, or fatal exit-info reason.

---

## 3. Python API Reference

```python
from pathlib import Path
from apk_lab.device import AdbClient, select_device, monitor_application
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
    requested_patches=None,
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
```
