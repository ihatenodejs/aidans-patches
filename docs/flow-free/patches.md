# Flow Free Patch Specifications

## Overview

This document details the patches available for **Flow Free** (`com.bigduckgames.flow`).

| Patch Name                                      | Type                    | Default | Description                                                                                                                       |
| ----------------------------------------------- | ----------------------- | ------- | --------------------------------------------------------------------------------------------------------------------------------- |
| [Remove Ads](#patch-remove-ads)                 | `compositeFeaturePatch` | `true`  | Blocks banner, interstitial, and rewarded video ads, stops the ad mediation SDKs, and hides the Remove Ads purchase entry points. |
| [Remove Tracking](#patch-remove-tracking)       | `compositeFeaturePatch` | `true`  | Disables Firebase Analytics and Crashlytics collection and neutralizes the app's analytics and crash-reporting bridges.           |
| [Remove Google Play](#patch-remove-google-play) | `compositeFeaturePatch` | `true`  | Disables Google Play Games sign-in, cloud-sync prompts, achievements, review requests, and Play Store launches.                   |
| [Unlock All Levels](#patch-unlock-all-levels)   | `rawResourcePatch`      | `true`  | Unlocks every level pack and every level, including premium and individually purchased packs.                                     |
| [Unlimited Hints](#patch-unlimited-hints)       | `rawResourcePatch`      | `true`  | Displays a 999 hint balance and allows hints to be used without consuming hints or watching rewarded videos.                      |
| [Unlock All Themes](#patch-unlock-all-themes)   | `rawResourcePatch`      | `true`  | Unlocks every premium theme and the Complete Package theme bundle.                                                                |

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

## Patch: Remove Google Play

- **Name:** Remove Google Play
- **Target Package:** `com.bigduckgames.flow`
- **Supported Versions:** `6.0`
- **Default State:** `true` (Enabled by default)
- **Type:** Composite Feature Patch (`compositeFeaturePatch`) with an internal raw-resource layer
- **Dependencies:** None

### 1. Motivation & Purpose

Flow Free connects to Google Play Games for sign-in, cloud saves, achievements, and review prompts, and it launches the Play Store for update and cross-promo screens. Those integrations interrupt offline play and leave the pack list behind sign-in or click-catcher dialogs. The patch disables the Play Games entry points while leaving Firebase and local gameplay intact.

### 2. Technical Implementation

#### Java Activity and Achievement Bridge

| Class                                                   | Methods                                                                                                                                                                                                             |
| ------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `com.noodlecake.noodlenews.NoodlecakeGameActivity`      | `shouldGoogleAPIConnect`, `isGoogleAPIConnected`, `isSignedIn`, `gotoQuickReviewScreen` return `false`; `loginToGoogleAPI`, `handlePlayGamesConnection`, `showAchievements`, `gotoReviewScreen` return immediately. |
| `com.noodlecake.flow.utils.GooglePlayAchievementHelper` | `submitAchievement` and `showAchievements` return immediately.                                                                                                                                                      |

`shouldGoogleAPIConnect` is forced false instead of relying on `isGoogleAPIConnected`, because the original method returns the negation of that value and would otherwise keep prompting.

#### Native Prompts and Intents

The internal raw-resource layer replaces the following arm64 `libgame.so` methods:

| Native method                                                                         | Effect                                                 |
| ------------------------------------------------------------------------------------- | ------------------------------------------------------ |
| `-[Flow hasGameCenterAuth]`                                                           | Reports no Play Games authentication.                  |
| `-[Flow isGameCenterLoginPending]`                                                    | Reports no pending sign-in.                            |
| `-[Flow requestReview]`, `shouldRequestReviewOnBoot`, `shouldRequestReviewAfterLevel` | Suppresses in-app review prompts.                      |
| `-[Flow launchReviewApp]`                                                             | Suppresses Play Store review launches.                 |
| `-[Flow launchAppStoreId:campaign:]`, `launchAppStoreId:orUrl:campaign:`              | Suppresses Play Store update and cross-promo launches. |
| `-[SelectPackMenu showGooglePlayConnectPrompt]`                                       | Suppresses the Play Games sign-in prompt.              |
| `-[SelectPackMenu showCloudSyncEnablePrompt]`                                         | Suppresses the cloud-save opt-in prompt.               |
| `-[SelectPackMenu showUpdateRequiredPrompt]`                                          | Suppresses the update-required prompt.                 |
| `-[SelectPackMenu onGoogleApiConnected:]`, `onGameCenterAuth:`                        | Ignores late Play Games callbacks.                     |
| `-[SelectPackMenu areAchievementsSupported]`, `areAchievementsReady`                  | Hides the achievements button.                         |

### 3. Invariants

- Every Java method and native prologue must exist; the patch fails instead of leaving a Play Games path active.
- The patch does not disable Firebase, local saves, or level progress.
- Play Store billing remains available; the patch only removes Play Games, review, update, and cross-promo surfaces.

---

## Patch: Unlock All Levels

- **Name:** Unlock All Levels
- **Target Package:** `com.bigduckgames.flow`
- **Supported Versions:** `6.0`
- **Default State:** `true` (Enabled by default)
- **Type:** Raw Resource Patch (`rawResourcePatch`)
- **Dependencies:** None

### 1. Motivation & Purpose

Flow Free sells level packs individually and through The Complete Package, and levels inside a pack unlock sequentially. The patch makes every pack and level available without changing purchase state or saved progress.

### 2. Technical Implementation

The patch rewrites arm64 `libgame.so` function prologues to `mov w0, #1; ret`:

| Native method                          | Effect                                                                                                                                                                    |
| -------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `-[LevelPack isPurchased]`             | Every pack reports as purchased.                                                                                                                                          |
| `-[LevelPack isIndividuallyPurchased]` | Every pack reports as individually purchased.                                                                                                                             |
| `-[LevelPack isUnlocked]`              | Every pack reports as unlocked, including gated pack types.                                                                                                               |
| `-[LevelPack isPlayable]`              | Every pack can be entered.                                                                                                                                                |
| `-[Level isPlayable]`                  | Every level inside a pack can be selected.                                                                                                                                |
| `-[SelectPackMenu onSelected:]`        | Branches directly to the success tail call after setting the selected pack, skipping prerequisite, advanced-pack-warning, purchase, cloud-sync, and Play sign-in prompts. |

`LevelPack.isPurchased` and `Level.isPlayable` are the primary gates; the additional getters cover experimental, server-activated, and future pack categories that do not consult them first. The `onSelected:` branch is required because the original method returns without navigating when any of those prompts is expected, leaving the pack list behind a click catcher even though every pack is unlocked.

Because every pack is already available, the patch also removes the now-pointless **Restore Purchases** row from Settings > Help and from the Store. The Settings row is skipped at its native row-construction branch; the Store row is collapsed by blanking its localization key and restore icon.

### 3. Invariants

- Each function is identified by its file offset and validated against its original 16-byte prologue before any bytes are written.
- The patch is idempotent: an already-patched prologue is left unchanged.
- Purchase state, receipts, and saved level progress are not modified; the game only observes the unlock getters as true.

---

## Patch: Unlimited Hints

- **Name:** Unlimited Hints
- **Target Package:** `com.bigduckgames.flow`
- **Supported Versions:** `6.0`
- **Default State:** `true` (Enabled by default)
- **Type:** Raw Resource Patch (`rawResourcePatch`)
- **Dependencies:** None

### 1. Motivation & Purpose

Hints are consumed from a stored balance and can be replenished with in-app purchases or rewarded videos. The patch keeps the displayed balance at 999 and bypasses hint consumption.

### 2. Technical Implementation

| Native method                         | Replacement         | Effect                                               |
| ------------------------------------- | ------------------- | ---------------------------------------------------- |
| `-[HintManager hintBalance]`          | `mov w0, #999; ret` | Normal hint balance labels read 999.                 |
| `-[HintManager tempHintBalance]`      | `mov w0, #999; ret` | Temporary-mode balance labels read 999.              |
| `-[HintManager getActiveHintBalance]` | `mov w0, #999; ret` | Hint button enablement checks always pass.           |
| `-[HintManager useHint]`              | `mov w0, #1; ret`   | Hint use succeeds without decrementing stored hints. |

`useHint` only validates and decrements the balance; the caller performs the actual hint reveal, so returning true keeps reveal behavior intact.

The patch also removes the **Restore Purchases** rows from Settings > Help and the Store, and removes the **Get More Hints** row from the free-play level selector's Extras section. The Get More Hints row is collapsed by blanking its label and icon, leaving Share Progress as the final Extras row.

### 3. Invariants

- Each function is identified by its file offset and validated against its original 16-byte prologue.
- The patch is idempotent and fails instead of writing when a prologue does not match.
- Stored hint counts are not modified; the forced getters only change what the game observes.

---

## Patch: Unlock All Themes

- **Name:** Unlock All Themes
- **Target Package:** `com.bigduckgames.flow`
- **Supported Versions:** `6.0`
- **Default State:** `true` (Enabled by default)
- **Type:** Raw Resource Patch (`rawResourcePatch`)
- **Dependencies:** None

### 1. Motivation & Purpose

Premium themes are sold individually or as an all-themes bundle. The patch unlocks every theme in the theme store and board customizer without changing purchase state.

### 2. Technical Implementation

| Native method                        | Effect                                                  |
| ------------------------------------ | ------------------------------------------------------- |
| `-[Theme isUnlocked]`                | Every theme reports as unlocked.                        |
| `-[Theme isPurchased]`               | Every theme reports as purchased.                       |
| `-[Theme isIndividuallyPurchased]`   | Every theme reports as individually purchased.          |
| `-[ThemeManager allThemesPurchased]` | The manager reports the all-themes bundle as purchased. |

The methods are replaced with `mov w0, #1; ret`. `ThemeManager.hasPurchasedThemes` and `hasThemesForPurchase` then observe the forced getters, so the store no longer needs the all-themes purchase row.

### 3. Invariants

- Each function is identified by its file offset and validated against its original 16-byte prologue.
- The patch is idempotent and fails instead of writing when a prologue does not match.
- Theme purchase receipts and saved state are not modified.

---

## Verification & Preconditions

1. Target application must be **Flow Free** (`com.bigduckgames.flow`), version `6.0`, version code `60001`, signed by SHA-256 certificate `36cd9da86367d4e25529427681598ffef8f08416c0f2b7ceedcd856b9c6620b6`.
2. Build verification: `./gradlew :patches:buildAndroid --console=plain`.
3. Compatibility verification: `apk-lab check <apkm> --mpp <bundle> --package com.bigduckgames.flow --all`. The Play APKM contains `lib/arm64-v8a/libgame.so`; the standalone APKPure APK has no native libraries and is not a supported artifact for Remove Ads.
4. Device verification: launch the patched game, open Settings, and confirm there is no Remove Ads row or home-screen Remove Ads button, and no banner, interstitial, or rewarded video activity starts.
