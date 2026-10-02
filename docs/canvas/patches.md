# Canvas Student Patch Specifications

## Overview

This document specifies patches for **Canvas Student** (`com.instructure.candroid`).

| Patch Name | Type | Default | Description |
|---|---|---|---|
| [Fix 16 KB Page Compatibility](#fix-16-kb-page-compatibility) | `rawResourcePatch` | `true` | Repairs native-library ZIP alignment and removes Canvas's incompatible GNU RELRO declarations. |
| [Remove Tracking and Analytics](#remove-tracking-and-analytics) | `bytecodePatch` | `true` | Removes Pendo behavioral tracking, Pandata student activity surveillance, first-party analytics, Firebase Crashlytics reporting, and Play Store rating redirects. Depends on the 16 KB compatibility fix. |

## Patch: Fix 16 KB Page Compatibility

- **Name:** Fix 16 KB Page Compatibility
- **Target Package:** `com.instructure.candroid`
- **Supported Versions:** `8.10.0`
- **Default State:** `true` (Enabled by default)
- **Type:** Raw Binary / Asset Patch (`rawResourcePatch`)
- **Dependencies:** None

### 1. Motivation & Purpose

Canvas Student packages three arm64 native shared libraries: `libandroidx.graphics.path.so`, `libdatastore_shared_counter.so`, and `libpspdfkit.so`. While Morphe aligns uncompressed `.so` entries to 16 KB within the APK ZIP container, Android 15+ kernels running in 16 KB page-size mode also validate internal ELF program headers.

Specifically, Android's dynamic linker requires GNU RELRO program headers (`PT_GNU_RELRO`) to satisfy:
```text
(p_vaddr + p_memsz) % 16384 == 0
```
All three prebuilt libraries in Canvas Student 8.10.0 fail this condition. Android displays an "App Compatibility: This app isn't 16 KB compatible" dialog even when APK ZIP alignment succeeds.

### 2. Technical Implementation & Binary Modifications

1. **ELF Header Validation:**
   - Validates ELF magic (`0x7F 'E' 'L' 'F'`).
   - Verifies 64-bit architecture (`EI_CLASS == 2`) and little-endian byte ordering (`EI_DATA == 1`).
   - Validates program header table bounds against file size.
2. **Segment Inspection & Neutralization:**
   - Iterates program headers looking for `p_type == PT_GNU_RELRO` (`0x6474e552`).
   - Reads `p_vaddr` at offset 16 and `p_memsz` at offset 40.
   - If `(p_vaddr + p_memsz) % 16384 != 0`, overwrites `p_type` at offset 0 with `PT_NULL` (`0`).
   - Enforces invariant: exactly one incompatible RELRO segment must be neutralized per library; otherwise fails fast with `PatchException`.
3. **16 KB ZIP Packaging Alignment:**
   - Dynamically reconfigures Morphe's `ApkUtils.zFileOptions` using reflection to apply `AlignmentRules.constantForSuffix(".so", 16384)`, ensuring all `.so` entries are aligned to 16 KB boundaries in the output APKM.

---

## Patch: Remove Tracking and Analytics

- **Name:** Remove Tracking and Analytics
- **Target Package:** `com.instructure.candroid`
- **Supported Versions:** `8.10.0`
- **Default State:** `true` (Enabled by default)
- **Type:** Dalvik Bytecode Patch (`bytecodePatch`)
- **Dependencies:** `Fix 16 KB Page Compatibility`

### 1. Motivation & Purpose

Canvas Student records detailed learning activity beyond functional Canvas API requests. The application initializes Pendo with a hash of the student's UUID and account UUID, tracks app events and screen activity, logs Canvas URLs and time spent in courses/groups to a local database, and uploads those records to Instructure through a background worker. It also enables Crashlytics crash reporting and links its Help screen to a Play Store rating flow.

The patch preserves course access, login, synchronization, document viewing, submission, messaging, and notification behavior. It intercepts only telemetry/reporting paths and returns normal success results where WorkManager requires one.

### 2. Technical Implementation & Injection Points

#### Layer 1: Pendo SDK (`Lsdk/pendo/io/Pendo;`)
- **Void methods stubbed to `return-void`:** `setup`, `startSession`, `track`, `screenContentChanged`, `setAccountData`, `setVisitorData`, `dismissVisibleGuides`, `pauseGuides`, `resumeGuides`, `endSession`.
- **Boolean return stubbed to `false`:** `sendClickAnalytic`.
- **Object returns stubbed to `null`:** `getAccountId`, `getVisitorId`, `getDeviceId`.

#### Layer 2: Pendo Consent Handlers (`Lcom/instructure/pandautils/features/cookieconsent/AnalyticsConsentHandler;`)
- **Methods stubbed to `return-void`:** `onConsentGranted`, `onConsentRevoked`.

#### Layer 3: First-Party Event Dispatch & Token Reporting
- **`Lcom/instructure/canvasapi2/utils/Analytics;`:** Stubs `logEvent` and `setUserProperty` to `return-void`; stubs `isSessionActive` to return `false`.
- **`Lcom/instructure/student/util/Analytics;`:** Stubs `trackAppFlow`, `trackBookmarkCreated`, `trackBookmarkSelected`, `trackButtonPressed`, `trackUnsupportedFeature`, and `trackWidgetFlow` to `return-void`.
- **`Lcom/instructure/canvasapi2/AppManager;`:** Stubs `logTokenAnalytics` to `return-void`.

#### Layer 4: ScreenView & Pandata Pageview Surveillance
- **`Lcom/instructure/pandautils/analytics/ScreenViewAnnotationProcessor;`:** Stubs `processScreenView` to `return-void`.
- **`Lcom/instructure/pandautils/analytics/PageViewAnnotationProcessor;`:** Stubs `processPageView` to `return-void`.
- **`Lcom/instructure/pandautils/features/pageview/PageViewUtils;`:** Stubs `saveSingleEvent` and `stopEvent` to `return-void`.
- **`Lcom/instructure/pandautils/features/pageview/PandataManager;`:** Stubs `postEvent` to `return-void`.
- **`Lcom/instructure/pandautils/features/pageview/PageViewUploadWorker;`:** Injects `new-instance v0, Landroidx/work/u$a$c; \n invoke-direct {v0}, Landroidx/work/u$a$c;-><init>()V \n return-object v0` into `doWork()`, returning `Result.success()` immediately without querying the local database or uploading queued records.

#### Layer 5: Offline Analytics, Crashlytics, Rating & Logging
- **`Lcom/instructure/pandautils/features/offline/offlineanalytics/OfflineAnalyticsManager;`:** Stubs `openCourse` and `recordOfflineTime` to `return-void`.
- **`Lcom/google/firebase/crashlytics/FirebaseCrashlytics;`:** Stubs `log`, `recordException`, `setCrashlyticsCollectionEnabled`, `setCustomKey`, `setUserId`, `sendUnsentReports`, and `deleteUnsentReports` to `return-void`.
- **`Lcom/instructure/student/fragment/StudentHelpDialogFragmentBehavior;`:** Stubs `rateTheApp` to `return-void`.
- **`Lcom/instructure/canvasapi2/utils/Logger;`:** Stubs `canLogUserDetails` to return `false`.

### 3. Preconditions & Verification

1. Targets Canvas Student `8.10.0` signed with SHA-256 certificate `abfe1362d84c5234c174c2c03d405a480405e361162f7b28dad6bec825ba02b3`.
2. Every method lookup resolves through `mutableClassDefByOrNull` and verifies implementation presence before injecting instructions.
3. Verify build with `./gradlew :patches:buildAndroid clean --no-daemon`.
4. Verify metadata generation with `./gradlew generatePatchesList`.
