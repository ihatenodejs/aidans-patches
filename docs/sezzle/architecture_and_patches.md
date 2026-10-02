# Sezzle Android App Reverse Engineering & Patching Guide

## Overview

- **App Name:** Sezzle: Buy Now, Pay Later
- **Package Name:** `com.sezzle.sezzlemobile`
- **Analyzed Version:** `5.3.9` (VersionCode: `1889`)
- **Technology Stack:** React Native with Fabric (New Architecture) compiled to **Hermes Bytecode (version 98)**, hosted in native Android (`MainActivity` extending `ReactActivity`).
- **Over-The-Air (OTA) Updates:** Microsoft CodePush (`CodePush.getJSBundleFile()`).

### OTA Update Lock & Integrity Checks
- `MainApplication` supplies `CodePush.getJSBundleFile()` to `DefaultReactHost`; a downloaded CodePush bundle therefore takes precedence over `assets/index.android.bundle`.
- The `Suppress Updates and Integrity Checks` patch replaces that return value with `null` at the host call site. The React host then loads the patched embedded bundle while retaining the CodePush native package for compatibility with base-bundle imports.
- The patch also stubs out Dalvik root/hook detection libraries (`RootBeer` and `JailMonkeyModule`), forces Redux update selectors (`selectShouldForceUpdate`) and sagas (`shouldForceUpdateAppSaga`) to inert returns, unmounts `UpdateAppModal`, neutralizes store URLs, and suppresses in-app rating prompts.
---

## App Architecture

### 1. Native Layer (`base.apk`)
- **Entry Points:**
  - `com.sezzle.sezzlemobile.MainActivity`: Subclasses `com.facebook.react.ReactActivity`. Initializes `RNBootSplash`, `RNScreensFragmentFactory`, and Firebase.
  - `com.sezzle.sezzlemobile.MainApplication`: Subclasses `android.app.Application` and implements `com.facebook.react.ReactApplication`.
- **React Native Host:**
  - Configures `DefaultReactHost` with `PackageList(mainApplication).getPackages()`.
  - JS Bundle: `assets/index.android.bundle` (fallback) or CodePush local updates cache.
- **Native Advertising & Rewards SDKs:**
  - **AppLovin MAX:** `com.applovin.reactnative.AppLovinMAXPackage`, `com.applovin.adview.AppLovinFullscreenActivity`, `com.applovin.adview.AppLovinAdView`.
  - **Google Mobile Ads (AdMob):** `io.invertase.googlemobileads.ReactNativeGoogleMobileAdsPackage`, `com.google.android.gms.ads.AdActivity`.
  - **Rokt Marketing / Ads:** `com.rokt.roktsdk.*`, `RoktLayoutView`.
  - **Adjoe (Playtime Rewards):** `io.adjoe.sdk.reactnative.RNPlaytimeSdkPackage`, `AdjoeActivity`.
  - **InBrain Surveys (Reward Surveys):** `com.inbrain.rn.InBrainSurveysPackage`, `SurveysActivity`.
  - **Braze:** `com.braze.reactbridge.BrazeReactBridgePackage`, `ContentCardsActivity` (in-app marketing / promo content cards).

### 2. JavaScript / Hermes Bytecode Layer (`assets/index.android.bundle`)
- **Hermes Bytecode Version:** 98 (`0x62`), Magic: `c6 1f bc 03 c1 03 19 1f`.
- **String Storage:** 153,256 strings across 6,687,552 bytes in the string table.
- **Navigation Framework:** React Navigation bottom tabs (`@react-navigation/bottom-tabs`) rendered inside `ProtectedStack` (`func #27734`).

---

## Target Features & Patch Implementation Analysis

### Patch 1: Remove Ads and Tracking
#### Findings:
Advertising and tracking in Sezzle are sourced across multiple native SDKs and React Native bridges:
1. **AppLovin MAX:**
   - Banners: `AppLovinMAX.AdView` / `AppLovinMAXAdView`
   - Interstitials, Rewarded, App Open: `AppLovinMAXModuleImpl`
   - Core SDK initialization: `AppLovinInitProvider`, `AppLovinSdk`
2. **Google Mobile Ads (AdMob / GAM):**
   - Banners: `ReactNativeGoogleMobileAdsBannerAdViewManager`
   - Interstitials & Rewarded: `ReactNativeGoogleMobileAdsFullScreenAdModule`
   - Initialization & Debugger: `MobileAdsInitProvider`, `MobileAds`, `ReactNativeGoogleMobileAdsModule`
3. **Rokt Embedded / Modal Offers:**
   - Native modules & views: `MPRoktModule`, `RoktLayoutViewManager`, `RoktInternalImplementation`, `Rokt`
4. **Adjoe (Playtime Rewards / Ads):**
   - Native modules & views: `Playtime`, `RNPlaytimeSdkModule`
5. **InBrain Surveys (Reward Surveys / Ads):**
   - Native modules: `InBrainSurveysModule`
6. **Tracking & Analytics SDKs:**
   - **AppsFlyer:** `AppsFlyerLib`, `RNAppsFlyerModule`, `PCAppsFlyerModule`
   - **FullStory:** `FS`, `FullStoryModule`
   - **Braze:** `Braze`, `BrazeReactBridgeImpl`, `BrazeBannerManager`
   - **mParticle:** `MParticle`, `MParticleModule`
   - **Firebase Analytics & Performance:** `FirebaseAnalytics`, `ReactNativeFirebaseAnalyticsModule`, `FirebasePerformance`
   - **Facebook SDK:** `FacebookInitProvider`, `FBAppEventsLoggerModule`
   - **AppCenter Analytics:** `Analytics`, `AppCenterReactNativeAnalyticsModule`
   - **Advertising ID (AAID):** Zeroed out via `AdvertisingIdClient.Info` (`getId` -> `00000000-0000-0000-0000-000000000000`, `isLimitAdTrackingEnabled` -> `true`)

#### Patch Strategy:
- **Native Bytecode Hook (`bytecodePatch`):**
  - Neutralizes entry point methods across all ad and tracking SDKs at the Dalvik layer by prepending immediate returns (`return-void`, `return false`, or dummy constants).
  - Keeps the React Native native module interfaces intact so JavaScript never encounters null/undefined references or property errors.
---

### Patch 2: Replace "Shop" with "Home"
#### Findings:
1. **Tab Title & Icon:**
   - In `ProtectedStack` (`func #27734`):
     - Instruction 2277 calls `t("navigation.Shop")` (string index #7057).
     - String index #58406 is `"navigation.Home"`, which translates to `"Home"`.
     - Route name: `"Shop"` (string index #66990).
     - Tab Icon: `func #57080` loads `"shopping-cart"` (string index #32477). String index #3870 is `"home"`, and #54299 is `"home-outline"`.
2. **Safe Tab Title Renaming:**
   +- Modifies `LoadConstString` in `ProtectedStack` to load string index #58406 (`"navigation.Home"`), which the `t()` localization catalog maps to `"Home"`. (Directly using `"Home"` fails localization lookup and outputs `[missing "en-US.Home" translation]`).
   +- Keeps the tab icon (`shopping-cart`) and internal route name (`Shop`) intact: the tab icon map only defines 6 pre-cached icons (excluding `home`), so querying `home` yields `undefined` and crashes React Navigation.
3. **Home Content / Custom Dashboard Composition:**
   - In `StoreRoot` (`func #44055`), `storeDirectoryContainer` originally mounts `children = [HeaderView, ScrollListView]`.
   - `ScrollListView` contains all commercial feed content: "Trending This Week", "Most Popular Brands", promotional content card carousels, "Shop The Marketplace", and gift card sections.
   - The patch redirects Metro module dependencies in `StoreRoot`:
     - `dep[17]` (was 11878 `ScrollListView`) -> 10517 (`OrdersListHeader`)
     - `dep[18]` (was 11881 `StoreHeaderList`) -> 8489 (`PaymentStreakBanner`)
     - `dep[19]` (was 11892) -> 11882 (`ShopTabShortcutsRow`)
   - The patch reconstructs the `children` array into a 4-component personal finance dashboard:
     1. `HeaderView` (Search bar, profile initial, notification bell)
     2. `OrdersListHeader` ("Total you owe" balance)
     3. `PaymentStreakBanner` (Spending power and payment streak progress)
     4. `ShopTabShortcutsRow` ("Your Shortcuts" carousel)
   - **Home-Specific Clean Design Adaptations (preserving Orders screen intact):**
     - In `OrdersListHeader` (`func #41278`), checks `props.status` (which is always defined on the Orders tab and `undefined` on Home).
     - When on Home (`!props.status`):
       - Omits passing `navigation` to `TotalOwedSectionV2` (leaves `props.navigation` as `null`).
       - Early-returns the lilac "Total you owe" container (`r12`), stripping the "Upcoming Payments" and "Orders" filter buttons (`OrderFilterView`) and promotional Braze banner (`BrazeBannerWithPadding`).
     - In `TotalOwedSectionV2` (`func #41281`):
       - When `props.navigation` is falsy (Home screen):
         - Skips mounting the "Shop now" button in `totalOwedAmountRow`, displaying only the clean balance amount.
         - Skips mounting the redundant spending power badge and lightning bolt icon, as the dedicated `PaymentStreakBanner` component is mounted directly below.
       - On the Orders screen (`props.navigation` is provided):
         - Displays "Shop now", spending power badge, and all filter buttons as normal.
   - Completely unmounts all commercial advertising, store feeds, and sponsored carousels while keeping utility and account management controls on Home.
---

### Patch 3: Remove Rewards
#### Findings:
1. **Rewards Tab Configuration:**
   - Internal Route Name: `EarnTab` (String index #18836).
   - Display Title: Resolved via `useEarnTabTitle` / `resolveEarnTabTitle` (String #31871: `"navigation.EarnTab"` -> displays "Rewards").
   - Sub-screens: `EarnPage`, `AllWaysToEarn`, `AdjoeDetails`, `SurveyViewAll`, `VideoAdsReward`, `ClaimedOffers`, `Giveaway`.
2. **Master Feature Toggle & Shortcut Routing:**
   - In `ProtectedStack`, `EarnTab` is conditionally mounted using `useIsShowEarnTabEnabled`.
   - In `ShopTabShortcutsRow`, shortcut items evaluate `useCachedIsShowEarnTabEnabled`; the patch filters only Rewards shortcuts instead of globally forcing the shared `useShowAsPoints` selector to `false`, which previously hid Account > Benefits > Sezzle Points.
   - The Home `customer/points` shortcut is routed through the always-mounted P2P deep-link dispatcher, whose targeted branch opens `Account` > `SezzleSpend` > `SezzlePoints`.
   - Account > Benefits > Sezzle Points retains its original navigation to the same `SezzleSpendSezzlePointsScreen`.

### Patch 4: Hide Sezzle Mobile
#### Findings:
- `useIsSezzleMobilePlanEnabled` remains forced to return `false`, preventing entry points controlled by the named feature gate.
- Wallet renders the Phone Plan offer independently through the uniquely named `MobilePlanSection` Hermes function, which creates the `mobile-plan-section` view.
- Home and Shop tabs render "Your Shortcuts" via `ShopTabShortcutsRow` and `useShopTabShortcuts`, which filters shortcut items using an anonymous predicate closure (`func 97584`).

#### Patch Strategy:
- Guard the `useIsSezzleMobilePlanEnabled` function's four-byte prologue, then replace it with `LoadConstFalse r1; Ret r1`.
- Guard `MobilePlanSection` and replace its prologue with `LoadConstUndefined r1; Ret r1`.
- Intercept the shortcuts filter closure in `useShopTabShortcuts`, replacing its 9-byte `sezzle_mobile` evaluation block at offset `0x2d` with `LoadConstFalse r1; Ret r1`, causing the Sezzle Mobile shortcut to be unmounted from "Your Shortcuts" without breaking environment scopes or other shortcuts.
- Recalculate the Hermes bundle SHA-1 footer hash.

### Patch 5: Remove Promos & Giveaways
#### Findings:
- Promotional components such as Giveaways, Refer a Friend, Offers, Merchant Deal Popovers, Knot Account Linking Promos, Wallet Marketing, Playtime Campaigns, Live Trivia, and Notification Soft Prompts are governed by React hooks and modals in Hermes bytecode:
  - **Base Giveaways**: `useRoktGiveawaysEnabled`, `useGiveawayShopRoutingEnabled`, `useMarketingGiveaway`, `useGiveawayScreenStatus`.
  - **Merchant Deal Popovers**: `useSelectPopupOffers`, `useStoreOfferModalConfig`, `useWebviewDealsPopoverConfig`, `useIsNewOfferModalEnabled`, `useSingleMerchantDeal`, `useIsOffersV2Enabled`, `useIsOfferSurfaceEnabled`.
  - **Knot Account Linking**: `useKnotActivationModalEnabled`, `useKnotActivationModalGate`, `useKnotBannerVisibility`.
  - **Wallet Marketing**: `useWalletMarketingMerchants`, `useWalletEmptyStateMarketing`, `useOfferBoostBanner`, `useConvertPointsToSpendBanner`.
  - **Playtime Marketing**: `usePlaytimeCampaigns`.
  - **Referrals & Social**: `useIsReferralUnavailable` and shortcuts filter.
  - **Trivia**: `useTriviaGiveawayBanner`, `useTriviaLiveActivityPushToStart`.
  - **Notification Prompts**: `NotificationPermissionModalV2`, `EnableNotificationsModal`, `useSyncPushPermissionWithBraze`.

#### Patch Options & Strategy:
- **Base Giveaways (Always applied)**: Neutralizes Rokt and marketing giveaway sagas/screens (`useRoktGiveawaysEnabled`, `useGiveawayShopRoutingEnabled`, `useMarketingGiveaway`, `useGiveawayScreenStatus`).
- **`blockMerchantDealPopovers` (Default: `true`)**: Suppresses in-session merchant deal popovers, store offer modals, and webview deal overlays (`useSelectPopupOffers`, `useStoreOfferModalConfig`, `useWebviewDealsPopoverConfig`, `useIsNewOfferModalEnabled`, `useSingleMerchantDeal`, `useIsOffersV2Enabled`, `useIsOfferSurfaceEnabled`).
- **`blockKnotAccountLinking` (Default: `true`)**: Suppresses Knot card auto-sync onboarding dialogs, activation gates, and account linking banners (`useKnotActivationModalEnabled`, `useKnotActivationModalGate`, `useKnotBannerVisibility`).
- **`blockWalletMarketing` (Default: `true`)**: Removes promotional merchant cards, empty state marketing, and boost banners from wallet screens (`useWalletMarketingMerchants`, `useWalletEmptyStateMarketing`, `useOfferBoostBanner`, `useConvertPointsToSpendBanner`).
- **`blockPlaytimeMarketing` (Default: `true`)**: Neutralizes Playtime campaign listings and rewards promo carousels in Hermes UI (`usePlaytimeCampaigns`).
- **`blockReferralsAndSocial` (Default: `true`)**: Sets `useIsReferralUnavailable` to return `true` and filters `"referral"` from the shortcuts row.
- **`blockTrivia` (Default: `false`)**: Hides live trivia giveaway banners and push-to-start game prompts (`useTriviaGiveawayBanner`, `useTriviaLiveActivityPushToStart`).
- **`blockNotificationPrompts` (Default: `false`)**: Suppresses the pre-permission "Don't Miss Out!" marketing dialog (`NotificationPermissionModalV2` / `EnableNotificationsModal`) and bypasses soft push-opt-in gates (`useSyncPushPermissionWithBraze`).
- Recalculates the Hermes bundle SHA-1 footer hash.
---
### Patch 6: Enable 16 KB Page Size Compatibility
#### Findings:
On Android 15+ devices and emulators running a 16 KB page size kernel, apps with uncompressed native libraries (`.so`) that lack 16 KB ZIP alignment or have 4 KB ELF segments trigger an Android OS compatibility dialog:
`"This app isn't 16 KB compatible. APK alignment check failed. This app will be run using page size compatible mode."`

AOSP's `PageSizeMismatchDialog` checks `PackageManager.getPageSizeCompatWarningMessage(packageName)`. If the application declares `android:pageSizeCompat="enabled"` in `AndroidManifest.xml`, `getPageSizeCompatWarningMessage` returns `null` and the OS suppresses the dialog while running eligible native code in 16 KB app compat mode.

#### Patch Strategy:
- **Morphe APK Writer Re-alignment:** Uses reflection to configure Morphe Patcher's internal `apkzlib` `ZFileOptions` alignment rules, enforcing 16,384-byte (`PAGE_SIZE_16_KB`) alignment for all `.so` archive entries in the repackaged APK.
- **Manifest Optimization (`resourcePatch`):** Injects `android:pageSizeCompat="enabled"` and `android:extractNativeLibs="true"` into the `<application>` tag of `AndroidManifest.xml`. This opts into OS-level page size compatibility mode, silences the warning dialog, and directs the package manager to extract native libraries upon installation so unaligned in-ZIP mmap failures are prevented.
---


## Multi-App Repository Architecture (`aidans-patches`)

Following the `morphe-patches-template` and production `morphe-patches` layout:

```
aidans-patches/
├── build.gradle.kts
├── settings.gradle.kts
├── gradle.properties
├── patches/
│   ├── build.gradle.kts
│   └── src/main/kotlin/
│       └── app/morphe/patches/
│           ├── shared/
│           │   └── Constants.kt
│           ├── sezzle/
│           │   ├── ads/
│           │   │   └── HideBannerAdsPatch.kt
│           │   ├── navigation/
│           │   │   ├── RenameShopToHomePatch.kt
│           │   │   └── ConfigureShortcutsPatch.kt
│           │   └── shared/
│           │       └── Constants.kt
│           └── <other_apps>/
│               └── ...
```
