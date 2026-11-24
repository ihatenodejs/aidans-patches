# Flow Free Architecture & Reverse Engineering Specification

## 1. Overview

| Attribute                   | Specification                                                                    |
| --------------------------- | -------------------------------------------------------------------------------- |
| **Application Name**        | Flow Free                                                                        |
| **Package Name**            | `com.bigduckgames.flow`                                                          |
| **Supported Version**       | `6.0` (Morphe Compatibility: `6.0`, `minSdk` 24)                                 |
| **Analyzed Version Code**   | `60001`                                                                          |
| **Minimum SDK**             | `24` (Android 7.0)                                                               |
| **Distribution Formats**    | APKM (Google Play, `arm64-v8a` split); the standalone APK lacks native libraries |
| **Package Signing SHA-256** | `36cd9da86367d4e25529427681598ffef8f08416c0f2b7ceedcd856b9c6620b6`               |
| **Target Architecture**     | `arm64-v8a`                                                                      |
| **Primary Icon Color**      | `#1E88E5`                                                                        |

Flow Free is a Cocos2d-x game ported from an iOS codebase. Java is a thin host layer for the Cocos activity, native bridges, billing, notifications, and ad mediation; gameplay, rendering, persistence, and ad/analytics trigger policy live in native code.

## 2. Technology Stack & Runtime Architecture

### 2.1 Native Runtime

- **Game logic:** `lib/arm64-v8a/libgame.so` contains the game and its Objective-C-style runtime objects.
- **Engine:** `lib/arm64-v8a/libcocos2d.so` hosts the scene graph, rendering, and input loop.
- **iOS compatibility layer:** `libfoundation.so`, `libobjc.so`, `libuikit.so`, `libcoregraphics.so`, `libquartzcore.so`, `libstorekit.so`, `libdispatch.so`, and `libnoodle.so` provide GNUstep/UIKit/StoreKit compatibility shims used by the original iOS game code.
- **Entry points:** `com.bigduckgames.flow.flow` extends `com.noodlecake.noodlenews.NoodlecakeGameActivity`, which supplies the Cocos2d-x activity, view, and handlers.

Native code reaches Java through JNI for:

- `com/noodlecake/flow/AdManager` (ads),
- `com/noodlecake/flow/utils/FirebaseHelper` (analytics and crash reporting),
- `com/noodlecake/flow/utils/ConfigHelper`, `DateHelper`, `GooglePlayAchievementHelper`,
- `com/noodlecake/iap/PurchaseWrapper`,
- `com/bigduckgames/flow/FlowNotificationManager`,
- `com/noodlecake/flow/flow`.

### 2.2 Advertising Architecture

The app uses **AppLovin MAX** mediation with Amazon Publisher Services, BidMachine, Digital Turbine/Fyber, Facebook Audience Network, Google Mobile Ads, Mintegral (MBridge), Moloco, Unity Ads, and Vungle adapters compiled into the APK.

The Java facade is:

- `com.noodlecake.flow.AdManager`: static entry points invoked by native code (`showAds`, `showBanner`, `showInterstitial`, `showRewardedVideo`, `hideAds`, `hideBanner`, `loadInterstitial`, `loadRewardedVideo`, `hasInterstitial`, `hasRewardedVideo`, `updateBannerPlacement`, `alMaxStartAfterConsent`).
- `com.noodlecake.flow.BannerAdManager`: `MaxAdView` lifecycle and banner placement.
- `com.noodlecake.flow.InterstitialAdManager`: `MaxInterstitialAd` load/show lifecycle.
- `com.noodlecake.flow.RewardedAdManager`: `MaxRewardedAd` load/show/reward lifecycle.
- `com.noodlecake.flow.AdRevenueListener`: MAX revenue callback.

The advertising SDKs also register `ContentProvider` components that initialize before `Application.onCreate`:

| Provider                                                                | SDK                       |
| ----------------------------------------------------------------------- | ------------------------- |
| `com.applovin.sdk.AppLovinInitProvider`                                 | AppLovin MAX              |
| `com.google.android.gms.ads.MobileAdsInitProvider`                      | Google Mobile Ads         |
| `com.facebook.ads.AudienceNetworkContentProvider`                       | Facebook Audience Network |
| `io.bidmachine.BidMachineInitProvider`                                  | BidMachine                |
| `com.vungle.ads.VungleProvider`                                         | Vungle                    |
| `com.mbridge.msdk.config.component.status.MBComponentLifecycleProvider` | Mintegral                 |

In-app purchase UI is driven by the native `Flow.removeAllAds` method in `libgame.so`, which returns `Flow.hasPurchasedNonConsumables`. `SettingsMenu`, `MainMenu`, `SelectPackMenu`, and `TimeTrialPage` skip their Remove Ads rows when it returns true. Additional upsells are controlled by the `RemoveAdsModal`, `LevelCompleteRemoveAds`, and `RemoveAdsBacking` keys in `assets/config_default.txt`.

### 2.3 Tracking Architecture

- **`com.bigduckgames.flow.AnalyticsManager`** records Firebase Analytics events (`logEvent`, `logEventStart`, `logEventEnd`), forwards parameters to `FirebaseAnalytics`, and sets Firebase user properties.
- **`com.bigduckgames.flow.CrashReportManager`** records Crashlytics logs, non-fatal exceptions, and custom keys.
- **`com.noodlecake.flow.utils.FirebaseHelper`** is the JNI-facing bridge for native events, user properties, Crashlytics logs, and non-fatal exceptions. It calls `AnalyticsManager` and `CrashReportManager`.
- **Firebase Analytics** and **Firebase Crashlytics** are initialized through `com.google.firebase.provider.FirebaseInitProvider` and registered Firebase components.
- The merged manifest enables analytics collection with `firebase_analytics_collection_enabled=true`; Crashlytics collection is enabled by default. Automatic screen reporting is already disabled.

Native tracking selectors such as `trackDailyLevelCompleted`, `trackPurchase:isRestore:beforePurchaseFlags:`, `logCrashlytics:`, and `crashlyticsRecordException:domain:code:` ultimately call `FirebaseHelper` or the app managers above.

## 3. Patch Targets Summary

1. **Remove Ads (`compositeFeaturePatch`):** Neutralizes the `AdManager` static entry points called by native code, returns `false` from ad-availability checks, prevents the six advertising SDK content providers from initializing, suppresses the native Remove Ads purchase rows, and disables remove-ads upsell config flags.
2. **Remove Tracking (`compositeFeaturePatch`):** Sets Firebase Analytics and Crashlytics collection manifest flags to `false` and neutralizes `AnalyticsManager` and `CrashReportManager` event, user-property, and crash-reporting methods.
3. **Remove Google Play (`compositeFeaturePatch`):** Disables the Java Play Games sign-in, achievement, and review entry points and replaces the native Play connect, cloud-sync, update, review, and Play Store launch methods.
4. **Unlock All Levels (`rawResourcePatch`):** Replaces the arm64 `LevelPack` and `Level` unlock getters in `libgame.so` with `return true` and branches `SelectPackMenu.onSelected:` directly to navigation so every pack and level is selectable.
5. **Unlimited Hints (`rawResourcePatch`):** Replaces `HintManager` balance getters with `return 999` and makes `useHint` succeed without decrementing the stored balance.
6. **Unlock All Themes (`rawResourcePatch`):** Replaces the `Theme` and `ThemeManager` purchase/unlock getters with `return true`.

## 4. Invariants

- Ads are only requested through `AdManager`; blocking its static entry points prevents banner, interstitial, and rewarded video traffic without editing native game logic.
- Advertising SDK initialization must be blocked both at `AdManager` and at the content-provider layer, because providers initialize before `Application.onCreate`.
- Remove Ads purchase rows are gated by native `Flow.removeAllAds`; patching its prologue to return true removes the rows without changing actual purchase state.
- The store package card still lists "Remove all ads" as one benefit of The Complete Package because that bundle also unlocks every level pack.
- Native analytics and crash reporting reach Firebase only through `AnalyticsManager` and `CrashReportManager`; disabling those methods also disables `FirebaseHelper`'s JNI-facing paths.
- Firebase Remote Config and Cloud Messaging are functional features and remain enabled.
- Premium pack access is gated by `LevelPack.isUnlocked`, which consults `isPurchased`, purchase flags, and pack categories; the unlock patch replaces the getters at their native function prologues rather than changing saved state.
- Level availability within a pack is gated by `Level.isPlayable`, which `LevelPack.updateSetsWithUnlocks:` normally sets from progress.
- Theme availability is gated by `Theme.isUnlocked`/`isPurchased`, with `ThemeManager.allThemesPurchased` acting as the bundle-wide override.
- Hint availability is gated by `HintManager`'s stored balance getters; `useHint` only consumes and persists that balance, while the caller reveals the hint.
- Play Games sign-in is gated by `NoodlecakeGameActivity.shouldGoogleAPIConnect` and `isGoogleAPIConnected`; native prompts use `SelectPackMenu` and `Flow` methods that must all be neutralized, because a no-op prompt can otherwise leave a click catcher over the pack list.
- `Flow` launches the Play Store for reviews and cross-promos through `launchAppStoreId:campaign:` and `launchAppStoreId:orUrl:campaign:`; blocking those methods leaves URL-based support and privacy launches intact.
- Every native patch validates the original prologue, is idempotent, and fails on a version mismatch instead of writing a partial edit.
