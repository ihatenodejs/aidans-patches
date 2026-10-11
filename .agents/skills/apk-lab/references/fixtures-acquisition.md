# Artifact Acquisition & R2 Cloud Fixtures

Acquire Android application artifacts directly from Google Play, normalize split bundles to `.apkm`, and manage the two-slot private Cloudflare R2 test fixture architecture (`target` and `latest`).

---

## 1. `acquire` — Google Play Artifact Acquisition

Downloads APK/APKM artifacts from the Google Play Store using native tools (`apkeep` with `goopdl` fallback) and normalizes multi-split APK downloads into unified `.apkm` container bundles.

### Normalization Pipeline
1. Downloads base APK and configuration splits (`split_config.*.apk`).
2. Runs `aapt2 dump badging` across all downloaded APK files.
3. Enforces container invariants:
   - Exactly one base APK (manifest has no `split` attribute).
   - All split APKs have identical package name and version code.
   - Rejects `.dm` (Dex Metadata) or `.obb` files.
4. Packages splits into a canonical `.apkm` ZIP container with standard `info.json` metadata.

### CLI Invocation

```bash
# Acquire the latest version of an app from Google Play
uv run --project tools/apk-lab apk-lab acquire com.example.app

# Acquire a specific version to a target output directory
uv run --project tools/apk-lab apk-lab acquire com.example.app \
  --version 2.22.09 \
  --out-dir /tmp/artifacts
```

### CLI Arguments
- `package` (required positional): Application package name.
- `--version VERSION`: Desired version name.
- `--version-code INT`: Desired version code.
- `--out-dir DIR`: Directory to place the acquired artifact (defaults to system temporary directory).

### Exit Codes
- `0`: Acquisition and normalization succeeded.
- `4`: `INFRASTRUCTURE_FAILURE` — Downloader authentication error, download failure, or Google Play error.

---

## 2. `fixtures` — Private R2 Cloud Fixtures

Manages the repository's two-slot private Cloudflare R2 storage architecture (`aidans-patches-apk-fixtures`):
- `fixtures/<package>/target`: Currently supported production patch target APK.
- `fixtures/<package>/latest`: Newest version released on Google Play Store.

### Subcommands

#### `seed`
Uploads a local artifact to an R2 slot, calculating hashes and signing certificate metadata.
```bash
uv run --project tools/apk-lab apk-lab fixtures seed \
  --package com.example.app \
  --role target \
  --artifact path/to/app.apkm
```

#### `info`
Inspects fixture metadata stored in R2 (package name, version, container type, SHA-256, signer, timestamp).
```bash
# Check both slots (target and latest)
uv run --project tools/apk-lab apk-lab fixtures info --package com.example.app

# Check specific slot
uv run --project tools/apk-lab apk-lab fixtures info --package com.example.app --role target
```

#### `download`
Downloads an APK artifact from an R2 slot to local disk.
```bash
uv run --project tools/apk-lab apk-lab fixtures download \
  --package com.example.app \
  --role target \
  --out /tmp/target.apkm
```

### CLI Arguments
- `fixture_subcommand`: `seed`, `info`, or `download`.
- `--package PACKAGE` (required): Application package name.
- `--role {target,latest}`: Target slot role.
- `--artifact PATH`: Path to local artifact for `seed`.
- `--out PATH`: Destination path for `download`.

### Slot Rotation Logic (`rotate_slots_on_new_latest`)
When a new version is released on Google Play:
1. If the existing `latest` matches the repository's desired target version, it is copied to `target`.
2. The newly acquired version is uploaded to `latest`.
3. If `target` and `latest` have the same version and hash, `target` is cleared.

### Exit Codes
- `0`: Fixture operation succeeded.
- `2`: Missing credentials or invalid arguments.
- `3`: Artifact package mismatch or invalid container.
- `4`: R2 communication or S3 API error.

---

## 3. Python API Reference

```python
from pathlib import Path
from apk_lab.acquisition import acquire_artifact
from apk_lab.fixtures import R2FixtureManager, SlotMetadata

# Acquire from Google Play
acquired_path, source = acquire_artifact(
    package_name="com.example.app",
    version_name="2.22.09",
    out_dir=Path("/tmp/artifacts"),
)
print(f"Acquired artifact at {acquired_path} via {source}")

# Manage R2 Fixture Slots
fixture_mgr = R2FixtureManager()

# Inspect slot
target_meta = fixture_mgr.get_slot_metadata("com.example.app", role="target")
if target_meta:
    print(f"Target version: {target_meta.version_name} ({target_meta.version_code})")
    print(f"SHA-256: {target_meta.sha256}")

# Download fixture
downloaded = fixture_mgr.download_slot(
    package="com.example.app",
    role="target",
    dest_path=Path("/tmp/target.apkm"),
)

# Seed fixture from local file
seeded_meta = fixture_mgr.seed_fixture(
    package="com.example.app",
    role="target",
    artifact_path=Path("path/to/app.apkm"),
)
```
