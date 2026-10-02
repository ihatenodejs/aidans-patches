# Repository Guidelines

## Project Overview

This repository develops binary bytecode, resource, and asset patches for Android applications using the **Morphe Patching Framework** (`app.morphe.patches` Gradle plugin v1.3.4, Morphe Patcher v1.14.0).

The project patches four Android applications:
1. **Sezzle: Buy Now, Pay Later** (`com.sezzle.sezzlemobile`, target `5.3.9`): Hybrid React Native Fabric application compiled to **Hermes Bytecode v98**. Patches eliminate ads and tracking SDKs, suppress CodePush OTA updates and root/tamper checks, sanitize authentication (Google SSO only, native `ConsentGate` modal), restructure navigation (replace Shop with Home, remove Rewards, customize shortcuts, replace AI Discover), unblock features (receipt scanner, custom launcher icons), expose internal developer settings, and ensure 16 KB page size compatibility on Android 15+.
2. **SidelineSwap: Buy & Sell Gear** (`com.sidelineswap.android`, target `1.52.0`): Native Android (Kotlin/Java) marketplace app. Patches eliminate first-party and third-party tracking/analytics (Amplitude, Firebase Analytics, Crashlytics, Facebook App Events, Iterable, Braintree FPTI) and customize the primary brand accent color via Android XML resource modification.
3. **AfterShip: Package Tracker** (`com.aftership.AfterShip`, target `5.25.8`): Native Android (Kotlin/Java) tracking app with native C++ libraries (`libandroidsig-lib.so`). Patches neutralize native APK signature verification (`checkApkSha`) and remove login barriers (forcing permanent guest mode).
4. **Canvas Student** (`com.instructure.candroid`, target `8.10.0`): Native Android (Kotlin/Java) learning-management client. Patches remove Pendo behavioral tracking, Instructure Pandata pageview surveillance, first-party analytics, Firebase Crashlytics reporting, and Play Store rating redirects.

---

## Architecture & Patching Layers

Patches operate across five distinct architectural layers depending on target application requirements:
```
                                      Target APK / XAPK / APKM
                                                 |
       +-------------------+---------------------+--------------------+--------------------+
       |                   |                     |                    |                    |
       v                   v                     v                    v                    v
[ Dalvik / Smali ]  [ Hermes HBC v98 ]   [ Native Extension ]   [ XML Resource ]    [ Raw Binary/SO ]
  bytecodePatch      rawResourcePatch        extendWith           resourcePatch      rawResourcePatch
  Dexlib2 AST        Bytecode & string   Java DEX injection     Android DOM XML      ELF / SO patching
  rewriting          manipulation        (ConsentGate.java)     (colors, manifest)   (checkApkSha)
       |                   |                     |                    |                    |
       +-------------------+---------------------+--------------------+--------------------+
                                                 |
                                                 v
                                        Patched Output APK
```

### 1. Hermes Bytecode Layer (`rawResourcePatch` on HBC v98)
- **App**: Sezzle (`assets/index.android.bundle`, Hermes v98, magic `c6 1f bc 03 c1 03 19 1f`).
- **Editor**: `patches/src/main/kotlin/app/aidan/patches/sezzle/shared/HermesBundleEditor.kt`.
- **Mechanism**:
  - Direct opcode substitution (e.g., replacing function bodies with `LoadConstFalse r1` [`0x96 0x01`] / `Ret r1` [`0x76 0x01`]).
  - Array mutation and component unmounting (resizing container children arrays and setting promotional elements to `LoadConstUndefined` [`0x93`]).
  - Donor string recycling: Hermes cannot expand string tables without corrupting cross-section offsets. New text replaces unused donor strings (e.g. storybook debug strings) of equal or greater length, zero-padding remainder bytes.
  - Trailing SHA-1 digest recalculation via `editor.updateFooterHash()`.

### 2. Dalvik / Smali Bytecode Layer (`bytecodePatch`)
- **Apps**: Sezzle, SidelineSwap, AfterShip.
- **Engine**: Dexlib2 AST manipulation via Morphe DSL (`mutableClassDefBy`, `addInstructions`).
- **Mechanism**:
  - Neutralizes entrypoints by injecting early returns (`return-void`, `const/4 v0, 0x0 \n return v0`, dummy objects).
  - Intercepts method return values (e.g., overriding `CodePush.getJSBundleFile()` to return `null`, forcing fallback to embedded Hermes bundle).
  - Hooks lifecycles (e.g., injecting `ConsentGate.maybeShow(this)` into `MainActivity.onCreate`).

### 3. XML Resource Layer (`resourcePatch`)
- **Apps**: SidelineSwap (`res/values/colors.xml`, `res/values-night/colors.xml`), Sezzle (`AndroidManifest.xml`).
- **Engine**: Morphe `document(...)` XML DOM manipulation.
- **Mechanism**: Edits color element hex values based on patch options (`primaryColor`, `primaryDarkColor`) or sets manifest attributes like `android:debuggable="true"`.

### 4. Native Extension Layer (`extensions/extension.mpe`)
- **Module**: `:extensions:extension` compiles Java classes into a Morphe Patch Extension (`.mpe`).
- **Component**: `app.aidan.extension.sezzle.ConsentGate` presents an un-cancelable `AlertDialog` tracking acknowledgment in `SharedPreferences`.
- **Packaging**: Merged via `extendWith("extensions/extension.mpe")` and injected into the target APK's DEX.

### 5. Native Binary / Library Layer (`rawResourcePatch`)
- **App**: AfterShip (`lib/arm64-v8a/libandroidsig-lib.so`, `lib/armeabi-v7a/libandroidsig-lib.so`).
- **Mechanism**: Directly patches machine code instructions in shared ELF libraries (e.g., neutralizing `_Z11checkApkShaP7_JNIEnvP8_jobjectS2_h` so APK signature checks return success on custom-signed builds).

### Data Flow
1. **Build Time**: Gradle builds `:extensions:extension` into `.mpe`, compiles Kotlin patch definitions into `.mpp`, and executes `PatchListGeneratorKt` to emit `patches-list.json`.
2. **Patch Time (Morphe CLI / Desktop)**: Morphe unzips the target APK/APKM/XAPK, validates package/version compatibility, modifies Dalvik bytecode, merges `.mpe` classes into DEX, applies XML DOM edits, edits raw resources/ELF binaries/Hermes bundles, updates hashes, and repacks/signs the output APK.
3. **Runtime**: Patched classes, resources, native libraries, and Hermes bundles execute with tracking neutralized, security/integrity bypassed, and custom navigation/theming active.

---

## Key Directories

```
.
├── patches/                               # Core Morphe patch definitions module
│   ├── build.gradle.kts                   # Patch bundle metadata & task configuration
│   └── src/main/kotlin/
│       ├── app/aidan/patches/
│       │   ├── aftership/                 # AfterShip patch implementations
│       │   │   ├── auth/                  # Bypass native signature check, remove login
│       │   │   └── shared/                # AfterShip constants & compatibility
│       │   ├── sezzle/                    # Sezzle patch implementations
│       │   │   ├── ads/                   # HideBannerAdsPatch (13 SDKs neutralized)
│       │   │   ├── auth/                  # CleanAuthenticationPatch
│       │   │   ├── customization/         # UnlockCustomAppIconsPatch
│       │   │   ├── dev/                   # EnableAppDebuggingPatch, UnlockDevSettingsPatch
│       │   │   ├── features/              # UnlockReceiptScannerPatch
│       │   │   ├── navigation/            # ReplaceShopWithHome, RemoveRewards, ConfigureShortcuts, etc.
│       │   │   ├── security/              # SuppressUpdatesAndIntegrityPatch, PatchConsentScreenPatch
│       │   │   └── shared/                # Constants, compatibility, HermesBundleEditor
│       │   └── sidelineswap/              # SidelineSwap patch implementations
│       │       ├── customization/         # ChangeBrandColorPatch
│       │       ├── tracking/              # BlockTrackingAndTelemetryPatch
│       │       └── shared/                # SidelineSwap constants & compatibility
│       └── util/                          # Build-time utilities (PatchListGenerator.kt)
├── extensions/extension/                  # Native Android extension module
│   ├── build.gradle.kts                   # Compiles Java sources to extension.mpe
│   └── src/main/
│       ├── AndroidManifest.xml            # Minimal extension manifest
│       └── java/app/aidan/extension/      # Native Java code (ConsentGate.java)
├── docs/                                  # Reverse engineering specs & deep dive docs
│   ├── aftership/                         # Architecture & patch specifications for AfterShip
│   ├── sezzle/                            # Architecture, patch guide & hidden feature flags for Sezzle
│   └── sidelineswap/                      # Architecture & patch specifications for SidelineSwap
├── gradle/                                # Gradle wrapper and libs.versions.toml
└── .github/                               # CI/CD workflows, issue templates, release scripts

---

## Development Commands

### Building & Compilation
```bash
# Compile patch bundle (.mpp) and extension (.mpe)
./gradlew buildAndroid
# Output: patches/build/libs/patches-<version>.mpp

# Build only the patches module
./gradlew :patches:buildAndroid

# Clean build verification (matches CI test step)
./gradlew :patches:buildAndroid clean --no-daemon

# Standard Gradle build (:patches:build finalizes with buildAndroid)
./gradlew build
```

### Metadata & Documentation Tasks
```bash
# Generate patches-list.json from compiled .mpp artifacts
./gradlew generatePatchesList

# Synchronize README.md patch tables with patches-list.json
python3 .github/scripts/generate_patches_readme.py <owner/repo> <branch> patches-list.json README.md
```

### Patch Application (Local Verification)
```bash
# Apply compiled patches to a base APK using Morphe Desktop CLI
java -jar morphe-desktop.jar patch \
  --patches patches/build/libs/patches-X.X.X.mpp \
  --out sezzle-patched.apk \
  base.apk
```

### Release Pipeline (Local Dry-Run)
```bash
# Install release automation dependencies
npm install

# Dry-run semantic-release pipeline
npx semantic-release --dry-run
```

---

## Code Conventions & Common Patterns

Do not use legacy `@Patch` or `@CompatiblePackage` annotations. Define patches as top-level Kotlin values using `bytecodePatch`, `rawResourcePatch`, or `resourcePatch`:

```kotlin
// Dalvik bytecode patch
val sampleBytecodePatch = bytecodePatch(
    name = "Patch Display Name",
    description = "Concise description of the modifications.",
    default = true
) {
    compatibleWith(COMPATIBILITY_OBJECT)
    extendWith("extensions/extension.mpe") // Optional: include native extension
    dependsOn(anotherPatch)               // Optional: dependencies

    execute {
        // Dalvik bytecode manipulation via Dexlib2 AST
    }
}

// Raw binary / asset patch (Hermes bundle, ELF shared library)
val sampleRawResourcePatch = rawResourcePatch(
    name = "Asset Patch Name",
    description = "Modifies embedded asset or binary.",
    default = true
) {
    compatibleWith(COMPATIBILITY_OBJECT)

    execute {
        val file = get("assets/index.android.bundle") // or "lib/arm64-v8a/libfoo.so"
        // In-place byte modifications
    }
}

// Android XML resource patch
val sampleResourcePatch = resourcePatch(
    name = "Resource Patch Name",
    description = "Modifies XML resources or manifest.",
    default = true
) {
    compatibleWith(COMPATIBILITY_OBJECT)

    execute {
        document("res/values/colors.xml").use { doc ->
            // DOM manipulation
        }
    }
}
```
### 2. Dalvik Bytecode Helpers
Keep bytecode injection logic reusable and safe:
- Always check that the target method has an implementation (`method.implementation != null`) before injecting instructions.
- Use localized helper methods for stubbing out SDK calls:
  - `disableVoidMethods(classDescriptor, vararg methodNames)` -> injects `return-void`.
  - `returnBoolean(classDescriptor, methodName, value)` -> injects `const/4 v0, 0x0 \n return v0`.
  - `returnConstString(classDescriptor, methodName, value)` -> injects `const-string v0, "..." \n return-object v0`.

### 3. Hermes Bytecode Editing Principles
- **Guard Before Writing**: Use `editor.matchesBytes(offset, expected)` or `editor.patchBytesIfMatches(offset, expected, replacement)` to prevent corrupting mismatched bundle versions.
- **Fixed Size Invariant**: Never append bytes or reallocate sections; modify opcodes and operands in-place.
- **Donor String Replacement**: To insert custom text, locate an unused string of equal or greater length (e.g. storybook paths) and re-point the target string table entry using `replaceStringUsingDonor(...)`.
- **Mandatory Rehash**: Always call `editor.updateFooterHash()` prior to bundle export.

### 4. Error Handling
- Throw `PatchException("Descriptive reason")` when a target class, method, or byte offset cannot be located.
- Prefer fail-fast checks (`require(...)`, `check(...)`, `singleOrNull ?: throw PatchException(...)`) over silent failure or try/catch suppression.

### 5. Formatting & Code Style
- Kotlin official style enforced via `.editorconfig` (`ktlint_code_style = intellij_idea`).
- Wildcard imports are disabled in lint rules (`ktlint_standard_no-wildcard-imports = disabled`).
- 4-space indentation for Kotlin/Java; 2-space indentation for Gradle KTS, YAML, and JSON.

---

## Important Files

| File Path | Description |
| --- | --- |
| `patches/src/main/kotlin/app/aidan/patches/sezzle/shared/Constants.kt` | Sezzle package name (`com.sezzle.sezzlemobile`), version codes, and Morphe `Compatibility` object. |
| `patches/src/main/kotlin/app/aidan/patches/sezzle/shared/HermesBundleEditor.kt` | Binary parser and in-place bytecode/string editor for Hermes Bytecode (HBC v98+). |
| `patches/src/main/kotlin/app/aidan/patches/sezzle/security/SuppressUpdatesAndIntegrityPatch.kt` | Security patches suppressing CodePush OTA updates, RootBeer/JailMonkey, Hermes update sagas, and Play Store redirects. |
| `patches/src/main/kotlin/app/aidan/patches/sezzle/security/PatchConsentScreenPatch.kt` | Dalvik patch injecting `ConsentGate.maybeShow(this)` into `MainActivity.onCreate`. |
| `patches/src/main/kotlin/app/aidan/patches/sezzle/compatibility/PageSizeCompatibilityPatch.kt` | Resource patch enabling 16 KB page size compatibility, native library extraction, and 16 KB ZIP alignment in Sezzle. |
| `patches/src/main/kotlin/app/aidan/patches/sidelineswap/shared/Constants.kt` | SidelineSwap package name (`com.sidelineswap.android`), signatures, and Morphe `Compatibility` object. |
| `patches/src/main/kotlin/app/aidan/patches/sidelineswap/tracking/BlockTrackingAndTelemetryPatch.kt` | Dalvik patch neutralizing all analytics, tracking, telemetry, and AAID in SidelineSwap. |
| `patches/src/main/kotlin/app/aidan/patches/sidelineswap/customization/ChangeBrandColorPatch.kt` | XML resource patch customizing SidelineSwap brand colors. |
| `patches/src/main/kotlin/app/aidan/patches/aftership/shared/Constants.kt` | AfterShip package name (`com.aftership.AfterShip`) and Morphe `Compatibility` object. |
| `patches/src/main/kotlin/app/aidan/patches/aftership/auth/RemoveLoginPatch.kt` | Patches bypassing native signature check in `libandroidsig-lib.so` and enforcing permanent guest mode. |
| `patches/src/main/kotlin/app/aidan/patches/aftership/account/RemoveAfterShipAccountPageLinksPatch.kt` | Dalvik patch removing About the app, Share the app, and Feedback links from the Account screen. |
| `patches/src/main/kotlin/app/aidan/patches/aftership/feedback/RemoveFeedbackPatch.kt` | Dalvik patch removing feedback prompts, Feedback buttons, and star rating component on shipments. |
| `patches/src/main/kotlin/app/aidan/patches/aftership/sync/RemoveShipmentSyncPatch.kt` | Dalvik patch removing email shipment synchronization, prompts, banners, dialogs, and entry points. |
| `patches/src/main/kotlin/app/aidan/patches/canvas/shared/Constants.kt` | Canvas Student package name (`com.instructure.candroid`), signature, APKM type, and Morphe `Compatibility` object. |
| `patches/src/main/kotlin/app/aidan/patches/canvas/tracking/RemoveTrackingAndAnalyticsPatch.kt` | Dalvik patch neutralizing Pendo, Pandata, first-party analytics, Crashlytics, and rating redirects in Canvas Student. |
| `extensions/extension/src/main/java/app/aidan/extension/sezzle/ConsentGate.java` | Native Android Java component rendering the user consent modal dialog. |
| `patches/src/main/kotlin/util/PatchListGenerator.kt` | JavaExec reflection utility generating `patches-list.json` from `.mpp` archives. |
| `settings.gradle.kts` | Multi-project setup, plugin management, and GitHub Packages repository declarations. |
| `patches/build.gradle.kts` | Patch metadata, gson classpath setup, and `generatePatchesList` task definition. |
| `gradle/libs.versions.toml` | Version catalog for `morphe-patcher` (1.14.0), `smali`, and `gson`. |
| `.releaserc` | Semantic-release configuration managing version bumps, changelog bundling, and backmerges. |
| `.github/workflows/release.yml` | CI/CD release workflow with Java 21, build provenance attestation, and fallback build checks. |
| `docs/sezzle/architecture_and_patches.md` | Reverse engineering specification for Sezzle v5.3.9 Hermes bytecode and Dalvik structures. |
| `docs/sezzle/hidden_feature_flags.md` | Catalog of Sezzle hidden feature flags, cohorts, and debugger hooks. |
| `docs/sidelineswap/architecture.md` | Reverse engineering specification for SidelineSwap architecture and telemetry. |
| `docs/sidelineswap/patches.md` | Patch specification for SidelineSwap tracking neutralization and brand color customization. |
| `docs/aftership/architecture.md` | Reverse engineering specification for AfterShip architecture, native libs, and auth. |
| `docs/aftership/patches.md` | Patch specification for AfterShip signature bypass and login removal. |
| `docs/canvas/architecture.md` | Reverse engineering specification for Canvas Student telemetry architecture. |
| `docs/canvas/patches.md` | Patch specification for Canvas Student tracking and analytics removal. |

---

## Runtime/Tooling Preferences

- **Java / JDK**:
  - JDK 17+ is required for local builds; JDK 27 is tested and supported.
  - CI uses **Eclipse Temurin JDK 21**.
- **Android SDK**:
  - Required to compile `:extensions:extension` (`ConsentGate.java`).
  - Configure path via `ANDROID_HOME` environment variable or `sdk.dir=/path/to/sdk` in `local.properties`.
- **Gradle**:
  - Use the bundled wrapper `./gradlew` (pinned to **Gradle 9.7.1** with SHA-256 verification).
  - Parallel execution and build caching are enabled in `gradle.properties`.
- **Node.js & npm**:
  - Node.js LTS (`lts/*`) with standard `npm`.
  - Used exclusively for semantic-release and changelog tooling (`package.json`). Do not introduce runtime JS dependencies into the patches.
- **Python**:
  - Python 3 is required to run `.github/scripts/generate_patches_readme.py`.
- **Repository Authentication**:
  - GitHub Packages registry (`maven.pkg.github.com/MorpheApp/registry`) requires authentication via `GITHUB_TOKEN` / `GITHUB_ACTOR` or `gpr.key` / `gpr.user` in `~/.gradle/gradle.properties`.

---

## Testing & QA

### Testing Status
- **Automated Unit / Integration Tests**: None. There are no test sources in `patches/src/test` or `extensions/extension/src/test`.
- **Test Dependencies**: `gradle/libs.versions.toml` contains no testing frameworks (no JUnit, MockK, Kotest, or Robolectric). Running `./gradlew test` executes 0 tasks.
- **Rationale**: Patches transform proprietary, closed-source APK binaries (`base.apk` and embedded Hermes bundles). Synthetic unit testing without real target binaries provides little value compared to build-time invariants and real-world APK testing.

### Quality Assurance Strategy
1. **Compilation Verification**:
   - Primary CI validation (`release.yml`):
     ```bash
     ./gradlew :patches:buildAndroid clean --no-daemon
     ```
   - Validates that Kotlin sources, Java extension code, and `.mpp` packaging compile cleanly.
2. **Metadata Verification**:
   - Run `./gradlew generatePatchesList` to verify that all patches instantiate cleanly, register valid compatibility objects, and serialize to `patches-list.json`.
3. **Defensive Patch-Time Invariants**:
   - All patches must enforce strict preconditions. If class names, method signatures, or byte sequences differ from the expected target version, the patch must immediately throw `PatchException` rather than producing a corrupt APK.
4. **Local Artifact & Bytecode Inspection**:
   - Apply the `.mpp` bundle to a target Sezzle APK using `morphe-desktop.jar`.
   - Disassemble the output APK with `jadx` or `baksmali` to verify Dalvik method injections.
   - Inspect `assets/index.android.bundle` using Hermes disassemblers (`hbctool` or `hermes-dec`) to verify opcode and string edits.
5. **Device Smoke Testing**:
   - Install the patched APK on an emulator or device (`adb install -r sezzle-patched.apk`).
   - Verify that `ConsentGate` blocks interaction until accepted.
   - Verify that CodePush does not trigger OTA downloads over the network.
   - Verify UI: "Shop" tab is titled "Home", store feed is empty, Rewards tab is unmounted, and Google sign-in works.
   - Monitor `adb logcat` to confirm ad and tracking SDK initializations are neutralized without throwing unhandled exceptions.
