# Canvas Student Patch Specifications

## Overview

This document specifies patches for **Canvas Student** (`com.instructure.candroid`).

## Patch: Remove Tracking and Analytics

- **Name:** Remove Tracking and Analytics
- **Target Package:** `com.instructure.candroid`
- **Supported Versions:** `8.10.0`
- **Default State:** `true` (Enabled by default)
- **Type:** Dalvik Bytecode Patch (`bytecodePatch`)
- **Dependencies:** None

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
- **`Lcom/instructure/canvasapi2/utils/pageview/PandataManager;`:** Stubs `uploadPageViewEvents` to `return-void`.
- **`Lcom/instructure/pandautils/analytics/pageview/PageViewUtils;`:** Stubs `saveSingleEvent` and `stopEvent` to `return-void`.
- **`Lcom/instructure/pandautils/analytics/pageview/PageViewAnnotationProcessor;`:** Stubs `startEvent` and `stopEvent` to `return-void`.
- **`Lcom/instructure/pandautils/analytics/pageview/PageViewUploadWorker;`:** Injects `new-instance v0, Landroidx/work/u$a$c; \n invoke-direct {v0}, Landroidx/work/u$a$c;-><init>()V \n return-object v0` into `doWork()`, returning `Result.success()` immediately without querying the local database or uploading queued records.

#### Layer 5: Offline Analytics, Crashlytics, Rating & Logging

- **`Lcom/instructure/pandautils/analytics/OfflineAnalyticsManager;`:** Stubs `offlineModeStarted`, `reportCourseOpenedInOfflineMode`, and `offlineModeEnded` to `return-void`.
- **`Lcom/google/firebase/crashlytics/FirebaseCrashlytics;`:** Stubs `log`, `recordException`, `setCrashlyticsCollectionEnabled`, `setCustomKey`, `setCustomKeys`, `setUserId`, `sendUnsentReports`, and `deleteUnsentReports` to `return-void`.
- **`Lcom/instructure/student/mobius/settings/help/StudentHelpDialogFragmentBehavior;`:** Stubs `rateTheApp` to `return-void`.
- **`Lcom/instructure/canvasapi2/utils/Logger;`:** Stubs `canLogUserDetails` to return `false`.

### 3. Preconditions & Verification

1. Targets Canvas Student `8.10.0` signed with SHA-256 certificate `abfe1362d84c5234c174c2c03d405a480405e361162f7b28dad6bec825ba02b3`.
2. Every method lookup resolves through `mutableClassDefByOrNull` and verifies implementation presence before injecting instructions.
3. Verify build with `./gradlew :patches:buildAndroid clean --no-daemon`.
4. Verify metadata generation with `./gradlew generatePatchesList`.

## Patch: Change App Name

- **Name:** Change App Name
- **Target Package:** `com.instructure.candroid`
- **Supported Versions:** `8.10.0`
- **Default State:** `false` (opt-in)
- **Type:** Resource Patch (`resourcePatch`)
- **Options:** `appName` (default `Canvas`)

### Behavior

Sets `android:label` to the supplied literal on the manifest `<application>` and on the single launcher `<activity>`. The launcher label changes without touching localized string resources or in-app text. Blank names and manifests without exactly one `<application>` or a MAIN/LAUNCHER activity fail the patch.

## Patch: Clone App

- **Name:** Clone App
- **Target Package:** `com.instructure.candroid`
- **Supported Versions:** `8.10.0`
- **Default State:** `false` (opt-in)
- **Type:** Resource Patch (`resourcePatch`)
- **Options:** `packageName` (default `com.instructure.candroid.morphe`)

### Behavior

Changes the manifest package so the build installs beside the original app. The patch requires the input package to be `com.instructure.candroid`, resolves relative component names, rewrites original-package custom permissions, provider authorities, and the instrumentation target, then sets the new package in the resource finalizer. Morphe's resource encoder rewrites package-qualified resource references when the manifest package changes.

`apk-lab check` and `apk-lab deploy` reject an output whose package ID differs from the input by design. Verify this patch by invoking the pinned Morphe CLI directly, then inspect, sign, and install the output. The clone has separate app data. Re-signed or cloned App Links, push notifications, OAuth redirects, and package/certificate-bound services may not work. Nutrient/PSPDFKit licensing may depend on the application ID, so document viewing is not claimed to work until it is tested with a real account and document. Malformed package names, the original package name, missing manifest anchors, and authority collisions fail instead of producing a misleading clone.

## Patch: Change App Icon

- **Name:** Change App Icon
- **Target Package:** `com.instructure.candroid`
- **Supported Versions:** `8.10.0`
- **Default State:** `false` (opt-in)
- **Type:** Resource Patch (`resourcePatch`)
- **Options:** `foregroundColor` (default `#E62429`), `backgroundColor` (default `#FFFFFF`), `customForeground` (optional PNG)

### Behavior

Recolors the `#E62429` paths in `res/drawable/ic_launcher_foreground.xml`, sets the background color on both decoded adaptive icons, `res/mipmap/ic_launcher.xml` and `res/mipmap-anydpi/ic_launcher.xml`, through a dedicated color resource, and optionally adds a square PNG as `res/drawable-nodpi/aidan_canvas_launcher_foreground.png`. The custom image replaces only the adaptive foreground; the original monochrome vector stays in place for themed icons.

The image is an adaptive foreground, so launchers mask it to their icon shape. Use a 432×432 PNG with artwork inside the central safe area. Colors may be `#RRGGBB` or opaque `#AARRGGBB` (the `FF` alpha is stripped); non-opaque alpha is rejected; images must be readable square PNGs no larger than 4096 pixels per side. Missing red vector paths, missing adaptive-icon anchors, conflicts with the new drawable name, and invalid images fail explicitly. Morphe CLI accepts `-O customForeground=<path>`; whether a given Manager version renders a file picker for this option is not established.
