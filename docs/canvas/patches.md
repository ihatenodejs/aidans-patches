# Canvas Student Patch Specifications

## Overview

This document specifies patches for **Canvas Student** (`com.instructure.candroid`).

| Patch Name | Type | Default | Description |
|---|---|---|---|
| [Fix 16 KB Page Compatibility](#fix-16-kb-page-compatibility) | `rawResourcePatch` | `true` | Repairs native-library ZIP alignment and removes Canvas's incompatible GNU RELRO declarations. |
| [Remove Tracking and Analytics](#remove-tracking-and-analytics) | `bytecodePatch` | `true` | Removes Pendo behavioral tracking, Pandata student activity surveillance, first-party analytics, Firebase Crashlytics reporting, and Play Store rating redirects. Depends on the 16 KB compatibility fix. |

## Fix 16 KB Page Compatibility

Canvas's arm64 libraries are ZIP-aligned after packaging but three GNU RELRO declarations still fail Android's required `(VirtAddr + MemSiz) % 0x4000 == 0` check: `libandroidx.graphics.path.so`, `libdatastore_shared_counter.so`, and `libpspdfkit.so`. Android displays the compatibility dialog even when ZIP alignment succeeds.

The patch configures Morphe to 16 KB-align all uncompressed `.so` entries and replaces only those three incompatible `PT_GNU_RELRO` program headers with `PT_NULL`. This preserves each library's bytes, load segments, and relocation data while opting those libraries out of RELRO memory protection; their original prebuilt binaries cannot be safely relaid out without source and a compatible NDK rebuild. The patch fails if a target library or its expected incompatible RELRO header is absent.

## Remove Tracking and Analytics

### Motivation

Canvas Student records detailed learning activity beyond functional Canvas API requests. The application initializes Pendo with a hash of the student's UUID and account UUID, tracks app events and screen activity, logs Canvas URLs and time spent in courses/groups to a local database, and uploads those records to Instructure through a background worker. It also enables Crashlytics crash reporting and links its Help screen to a Play Store rating flow.

The patch preserves course access, login, synchronization, document viewing, submission, messaging, and notification behavior. It intercepts only telemetry/reporting paths and returns normal success results where WorkManager requires one.

### Technical Strategy

#### 1. Pendo SDK

`Pendo.setup`, `startSession`, `track`, guide controls, screen-content notifications, visitor/account data methods, and `endSession` are reduced to no-ops. `sendClickAnalytic` returns `false`, and all account, visitor, and device ID accessors return `null`. Canvas's normal Pendo activity lifecycle continues, but the direct SDK stubs prevent sessions, behavioral events, identity data, and visual guide controls from operating.

#### 2. Pendo Startup and Consent Paths

`AnalyticsConsentHandler` cannot start or stop Pendo sessions. `SetupPendoTrackingUseCase.execute` continues to load Canvas user, account, feature-flag, and Pendo-token state so splash initialization and post-auth routing complete; the direct Pendo SDK stubs prevent that orchestration from starting Pendo tracking.

#### 3. First-Party Event Dispatch

The patch no-ops `canvasapi2.utils.Analytics` event/property methods, returns `false` from `isSessionActive`, suppresses all `student.util.Analytics` tracking helpers, and prevents `AppManager.logTokenAnalytics` from categorizing and reporting persistent versus refresh token use.

#### 4. Screen and Pageview Data

`ScreenViewAnnotationProcessor` cannot generate navigation events. `PandataManager` can obtain the startup token required by Canvas's authenticated initialization, but cannot post event batches. `PageViewUtils.startEvent` remains intact for normal fragment lifecycle behavior, while `saveSingleEvent` and `stopEvent` cannot persist or complete pageview events. `PageViewAnnotationProcessor` cannot record pageview events.

`PageViewUploadWorker.doWork` returns `androidx.work.u$a$c` (`WorkManager Result.success()`) before reading or sending the local queue. A success result prevents failed-worker retry loops after telemetry is disabled.

#### 5. Offline, Crash, and Rating Telemetry

`OfflineAnalyticsManager` cannot report offline session duration or course opening events. Firebase Crashlytics logging, exception recording, collection controls, user IDs, custom keys, and unsent-report operations are no-ops. The Help screen's `rateTheApp` method is no-op, and `Logger.canLogUserDetails` always returns `false`.

### Patch-Time Preconditions

The patch addresses Canvas Student `8.10.0` signed by SHA-256 certificate `abfe1362d84c5234c174c2c03d405a480405e361162f7b28dad6bec825ba02b3`. Every helper resolves target classes through `mutableClassDefByOrNull` and only modifies methods with compatible return types and implementations, so omitted or altered optional library methods do not corrupt the APK.
