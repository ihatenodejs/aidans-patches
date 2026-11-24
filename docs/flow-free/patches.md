# Flow Free Patch Specifications

## Overview

This document details the patches available for **Flow Free** (`com.bigduckgames.flow`).

| Patch Name                                | Type                    | Default | Description                                                                                                                       |
| ----------------------------------------- | ----------------------- | ------- | --------------------------------------------------------------------------------------------------------------------------------- |
| [Remove Ads](#patch-remove-ads)           | `compositeFeaturePatch` | `true`  | Blocks banner, interstitial, and rewarded video ads, stops the ad mediation SDKs, and hides the Remove Ads purchase entry points. |
| [Remove Tracking](#patch-remove-tracking) | `compositeFeaturePatch` | `true`  | Disables Firebase Analytics and Crashlytics collection and neutralizes the app's analytics and crash-reporting bridges.           |

---

## Patch: Remove Ads

- **Name:** Remove Ads
- **Target Package:** `com.bigduckgames.flow`
- **Supported Versions:** `6.0`
- **Default State:** `true` (Enabled by default)
- **Type:** Composite Feature Patch (`compositeFeaturePatch`) with internal raw-resource layer
- **Dependencies:** None

### 1. Motivation & Purpose

Flow Free displays AppLovin MAX banner ads, interstitials between levels, and rewarded videos for hints. The native game starts every ad flow through the Java `AdManager` facade, while the mediated SDKs also self-initialize through content providers before the application starts. This patch blocks both paths.

### 2. Technical Implementation

#### `com.noodlecake.flow.AdManager`

The following static void methods return immediately at instruction offset 0:

| Method                   | Effect                                                              |
| ------------------------ | ------------------------------------------------------------------- |
| `alMaxStartAfterConsent` | Skips consent flow, AppLovin MAX initialization, and ad preloading. |
| `loadInterstitial`       | Skips interstitial preloading.                                      |
| `loadRewardedVideo`      | Skips rewarded video preloading.                                    |
| `showAds`                | Skips banner activation and the `isNativeShowingAds` flag.          |
| `showBanner`             | Skips banner display and placement updates.                         |
| `showInterstitial`       | Skips interstitial display.                                         |
| `showRewardedVideo`      | Skips rewarded video display.                                       |
| `updateBannerPlacement`  | Skips banner placement changes.                                     |

`hasInterstitial` and `hasRewardedVideo` return `false` so the game does not offer ad-gated interactions or reserve banner space.

#### Advertising SDK Content Providers

| Provider                                                                | Neutralized method               |
| ----------------------------------------------------------------------- | -------------------------------- |
| `com.applovin.sdk.AppLovinInitProvider`                                 | `onCreate` returns `false`       |
| `com.google.android.gms.ads.MobileAdsInitProvider`                      | `attachInfo` returns immediately |
| `com.facebook.ads.AudienceNetworkContentProvider`                       | `onCreate` returns `false`       |
| `io.bidmachine.BidMachineInitProvider`                                  | `onCreate` returns `false`       |
| `com.vungle.ads.VungleProvider`                                         | `onCreate` returns `false`       |
| `com.mbridge.msdk.config.component.status.MBComponentLifecycleProvider` | `onCreate` returns `false`       |

Blocking provider startup prevents each ad runtime and its telemetry from initializing when the Remove Tracking patch is not selected.

#### Native Remove Ads UI Suppression

`lib/arm64-v8a/libgame.so` stores the native UI gate at file offset `0x4b5310`. `Flow.removeAllAds` normally forwards to `Flow.hasPurchasedNonConsumables`; the patch replaces its prologue with `mov w0, #1; ret` after validating the original `stp x29, x30, [sp, #-0x20]!` and `str x19, [sp, #0x10]` bytes.

Returning true hides the Remove Ads rows built by:

- `SettingsMenu` `getScrollLayerWithAutoscrollOffset:`
- `MainMenu` `refresh`
- `SelectPackMenu` `getScrollLayer:autoscrollOffset:`
- `TimeTrialPage` `getScrollLayer:autoscrollOffset:`

The internal raw-resource layer also sets these `assets/config_default.txt` flags to `0` to suppress upsell dialogs:

- `RemoveAdsModal`
- `LevelCompleteRemoveAds`
- `RemoveAdsBacking`

`assets/config_debug.txt` gets `LevelCompleteRemoveAds=0` for debug-config builds. Missing flags fail the patch instead of leaving an upsell active.

The "The Complete Package!" store card still lists "Remove all ads" because that product also unlocks every premium level pack.

### 3. Invariants

- Every patched method and native byte sequence must exist; the patch fails instead of producing a partial ad removal.
- `AdManager` methods are patched before the provider layer, so a future entry point added to the facade still requires review.
- `Flow.removeAllAds` is patched for UI visibility only; it does not change purchase state or ad delivery.
- The patch does not remove billing or the in-app "Remove Ads" product; it only disables ad delivery and hides its purchase rows.

---

## Patch: Remove Tracking

- **Name:** Remove Tracking
- **Target Package:** `com.bigduckgames.flow`
- **Supported Versions:** `6.0`
- **Default State:** `true` (Enabled by default)
- **Type:** Composite Feature Patch (`compositeFeaturePatch`) with an internal manifest resource layer
- **Dependencies:** None

### 1. Motivation & Purpose

Flow Free records gameplay events, user properties, crash logs, and non-fatal exceptions through Firebase Analytics and Firebase Crashlytics. Native code calls the Java bridges directly, so both the SDK collection switches and the app bridges must be disabled.

### 2. Technical Implementation

#### Manifest Flags

The patch sets the following `<application>` meta-data flags to `false`, adding the element when absent:

| Flag                                                  | Effect                                                                          |
| ----------------------------------------------------- | ------------------------------------------------------------------------------- |
| `firebase_analytics_collection_enabled`               | Disables automatic and explicit Analytics event upload and session measurement. |
| `firebase_crashlytics_collection_enabled`             | Disables Crashlytics report and non-fatal exception upload.                     |
| `google_analytics_automatic_screen_reporting_enabled` | Disables automatic screen-view events.                                          |

#### Bytecode Neutralization

| Class                                      | Methods                                                                                   |
| ------------------------------------------ | ----------------------------------------------------------------------------------------- |
| `com.bigduckgames.flow.AnalyticsManager`   | `logEvent`, `logEventStart`, `logEventEnd`, `logFirebaseEvent`, `setFirebaseUserProperty` |
| `com.bigduckgames.flow.CrashReportManager` | `log`, `recordException`, `recordExceptionWithLogs`, `setCustomKey`                       |

Each implemented void overload returns immediately at instruction offset 0. These managers are the final Java destination for both activity-driven and native-driven tracking, including `FirebaseHelper` calls.

### 3. Invariants

- Every named method must exist and be implemented; the patch fails instead of silently leaving an analytics path active.
- Firebase Remote Config and Cloud Messaging initialization are intentionally left intact because they are functional app features, not user tracking.
- Advertising SDK telemetry is covered by the Remove Ads patch; combining both patches removes ad and analytics telemetry.

---

## Verification & Preconditions

1. Target application must be **Flow Free** (`com.bigduckgames.flow`), version `6.0`, version code `60001`, signed by SHA-256 certificate `36cd9da86367d4e25529427681598ffef8f08416c0f2b7ceedcd856b9c6620b6`.
2. Build verification: `./gradlew :patches:buildAndroid --console=plain`.
3. Compatibility verification: `apk-lab check <apkm> --mpp <bundle> --package com.bigduckgames.flow --all`. The Play APKM contains `lib/arm64-v8a/libgame.so`; the standalone APKPure APK has no native libraries and is not a supported artifact for Remove Ads.
4. Device verification: launch the patched game, open Settings, and confirm there is no Remove Ads row or home-screen Remove Ads button, and no banner, interstitial, or rewarded video activity starts.
