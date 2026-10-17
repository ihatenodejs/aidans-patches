<div align="center">

<h1>
  <img src="https://avatars.githubusercontent.com/u/178358868?v=4"
       width="40"
       height="40"
       align="absmiddle"
       style="border-radius:50%;">
  &nbsp;
  <i>Aidan's Morphe Patches</i>
</h1>

My [Morphe](https://morphe.software) patches

[![License: WTFPL](https://img.shields.io/badge/License-WTFPL-white.svg)](http://www.wtfpl.net/about/)
![Kotlin](https://img.shields.io/badge/Kotlin-%237F52FF.svg?logo=kotlin&logoColor=white)
![Java](https://img.shields.io/badge/Java-%23ED8B00.svg?logo=openjdk&logoColor=white)
[![AI Generated](https://img.shields.io/badge/AI%20Generated-orange?logo=googlegemini&logoColor=white)](https://img.shields.io/badge/AI%20Generated-orange?logo=googlegemini&logoColor=white)

[![APK Lab Tests](https://github.com/ihatenodejs/aidans-patches/actions/workflows/apk-lab-tests.yml/badge.svg)](https://github.com/ihatenodejs/aidans-patches/actions/workflows/apk-lab-tests.yml)
[![Tests Passing](https://worker.patch.p0ntus.com/badges/tests.svg)](https://patch.p0ntus.com/status)

[![Add to Morphe badge](https://github.com/ihatenodejs/aidans-patches/blob/main/assets/add-to-morphe.png?raw=true)](https://morphe.software/add-source?github=ihatenodejs/aidans-patches)
<a href="https://www.buymeacoffee.com/ihatenodejs">
  <img src="https://cdn.buymeacoffee.com/buttons/v2/default-yellow.png" alt="Buy Me a Coffee" height="100">
</a>

<h2>DISCLAIMERS</h2>

> These patches are provided for educational, research, and personal-use purposes and are not affiliated with, endorsed by, or supported by Morphe or any third-party application, service, financial institution, brokerage, exchange, or other entity affected by them.

> Use of these patches is entirely at your own risk. Patches may modify the behavior, appearance, functionality, security characteristics, or data displayed by third-party applications. They may stop working at any time or cause unintended behavior.

> **Do not rely on these patches for financial, investment, trading, banking, accounting, or other financial decisions.** The author makes no representation regarding the accuracy, completeness, reliability, availability, or correctness of any information displayed or affected by these patches.

> TO THE MAXIMUM EXTENT PERMITTED BY APPLICABLE LAW, THE SOFTWARE AND PATCHES ARE PROVIDED **“AS IS” AND “AS AVAILABLE,” WITHOUT WARRANTY OF ANY KIND**, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE, NONINFRINGEMENT, ACCURACY, RELIABILITY, OR SECURITY.

> TO THE MAXIMUM EXTENT PERMITTED BY APPLICABLE LAW, IN NO EVENT SHALL THE AUTHOR OR COPYRIGHT HOLDER BE LIABLE FOR ANY CLAIM, LOSS, DAMAGE, OR OTHER LIABILITY, WHETHER IN CONTRACT, TORT, NEGLIGENCE, OR OTHERWISE, ARISING FROM OR RELATED TO THE SOFTWARE, INCLUDING WITHOUT LIMITATION **FINANCIAL LOSSES, TRADING LOSSES, LOSS OF FUNDS, LOSS OF DATA, ACCOUNT RESTRICTIONS OR TERMINATION, FAILED OR UNINTENDED TRANSACTIONS, OR OTHER DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES.**

</div>

## 📱 Supported Applications & Patches

<!-- PATCHES_START EXPANDED -->
> **[v1.8.0](https://github.com/ihatenodejs/aidans-patches/releases/tag/v1.8.0)**&nbsp;&nbsp;•&nbsp;&nbsp;`main`&nbsp;&nbsp;•&nbsp;&nbsp;66 patches total
<details open>
<summary>📦 SidelineSwap&nbsp;&nbsp;•&nbsp;&nbsp;4 patches&nbsp;&nbsp;<a href="https://patch.p0ntus.com/status"><img src="https://worker.patch.p0ntus.com/badges/tests/com.sidelineswap.android.svg" alt="Tests Passing" align="absmiddle"></a></summary>
<br>

**🎯 Supported versions:**

| 1.52.0 |
| :---: |

| 💊&nbsp;Patch | 📜&nbsp;Description | ⚙️&nbsp;Options |
|----------|----------------|-----------|
| [AMOLED Theme](#amoled-theme) | Forces SidelineSwap into a pure-black AMOLED theme with dark system bars, black app surfaces, readable light text and icons, and force dark mode on embedded WebViews. |  |
| [Block Tracking and Telemetry](#block-tracking-and-telemetry) | Neutralizes first-party analytics (SidelineSwap backend), behavioral tracking (Amplitude, Firebase Analytics, Facebook App Events, Iterable), diagnostic telemetry (Firebase Crashlytics, Timber logging tree), payment gateway telemetry (Braintree FPTI), and zeros the Google Play Advertising ID (AAID). |  |
| [Change Brand Color](#change-brand-color) | Customizes the primary accent brand color across SidelineSwap buttons, navigation highlights, badges, and accents. | • Primary Brand Color<br>• Primary Dark Color |
| [Remove Invite Friends](#remove-invite-friends) | Removes the Invite Friends option from the Account screen and eliminates the non-functional orange dot notification badge from the Account navigation icon. |  |

</details>

<details open>
<summary>📦 AfterShip&nbsp;&nbsp;•&nbsp;&nbsp;11 patches&nbsp;&nbsp;<a href="https://patch.p0ntus.com/status"><img src="https://worker.patch.p0ntus.com/badges/tests/com.aftership.AfterShip.svg" alt="Tests Passing" align="absmiddle"></a></summary>
<br>

**🎯 Supported versions:**

| 5.25.8 |
| :---: |

| 💊&nbsp;Patch | 📜&nbsp;Description | ⚙️&nbsp;Options |
|----------|----------------|-----------|
| [Add Copy Tracking Number Option](#add-copy-tracking-number-option) | Adds an option to copy tracking numbers in the multi-shipment selection menu. |  |
| [Bypass Native Signature Check](#bypass-native-signature-check) | Neutralizes APK signature verification in libandroidsig-lib.so so API requests succeed when signed with custom keys. |  |
| [Custom Google Maps API Key](#custom-google-maps-api-key) | Replaces the embedded Google Maps API key with a personal Google Cloud API key so native Google Maps renders on re-signed builds. Note: To use native Google Maps, disable the OpenStreetMap Drop-in Replacement patch. | • Google Maps API Key |
| [Full AMOLED Theme](#full-amoled-theme) | Themes AfterShip in pure AMOLED black by removing dark gray backgrounds from the bottom navigation bar, account items, cards, and windows. |  |
| [Hide Broken Tracking Map](#hide-broken-tracking-map) | Suppresses the unauthenticated blank white Google Maps view when neither a custom Google Maps API key nor OpenStreetMap is used. |  |
| [OpenStreetMap Drop-in Replacement](#openstreetmap-drop-in-replacement) | Replaces the broken Google Maps view with a free, self-contained OpenStreetMap (Leaflet) engine that renders routes, checkpoints, and dark/light styled tiles without requiring an API key. | • Add Zoom Buttons |
| [Remove Ads and Tracking](#remove-ads-and-tracking) | Neutralizes in-app advertisements (Disco Network SDK shopping/cashback ads and list placements), removes the 'Leave us a 5-star review' in-app rating prompt dialogs, zeros the Google Play Advertising ID (AAID), disables first-party behavioral and impression analytics (StatisticsCenter, AbsListImpEventHelper, AutoUploadManager), and blocks diagnostic telemetry (Firebase Analytics, Crashlytics, Logan logging). |  |
| [Remove AfterShip Account Page Links](#remove-aftership-account-page-links) | Removes the About the app, Share the app, and Feedback links from the Account screen. |  |
| [Remove Feedback](#remove-feedback) | Removes prompting for feedback on shipments. |  |
| [Remove Login](#remove-login) | Forces guest mode always on first install, removes login buttons and carousels, strips account/login controls from the Account tab, and suppresses login prompts. |  |
| [Remove Shipment Sync](#remove-shipment-sync) | Removes email shipment synchronization features, including prompts, banners, dialogs, empty state sync cards, and account settings. |  |

</details>

<details open>
<summary>📦 Blackjack&nbsp;&nbsp;•&nbsp;&nbsp;6 patches&nbsp;&nbsp;<a href="https://patch.p0ntus.com/status"><img src="https://worker.patch.p0ntus.com/badges/tests/com.tripledot.blackjack.svg" alt="Tests Passing" align="absmiddle"></a></summary>
<br>

**🎯 Supported versions:**

| 2.22.09 |
| :---: |

| 💊&nbsp;Patch | 📜&nbsp;Description | ⚙️&nbsp;Options |
|----------|----------------|-----------|
| [Add Custom Chip Store](#add-custom-chip-store) | Replaces the unavailable store with a dialog to view and set your exact chip balance. |  |
| [Remove Ads](#remove-ads) | Removes banner, interstitial, and rewarded advertising and removes ad-based chip offers. |  |
| [Remove Internet Permissions](#remove-internet-permissions) | Removes Internet permissions from AndroidManifest.xml to prevent network access. | • Remove Broken Screens |
| [Remove Notifications](#remove-notifications) | Removes notification permissions from AndroidManifest.xml to eliminate push notifications entirely. |  |
| [Remove Tracking and Analytics](#remove-tracking-and-analytics) | Neutralizes active advertising telemetry, analytics, attribution, and crash reporting. |  |
| [Skip to Next Level](#skip-to-next-level) | Allows tapping the next level indicator on the top bar to show a confirmation dialog and skip to the next level. REQUIRES Add Custom Chip Store to be enabled. |  |

</details>

<details open>
<summary>📦 Sezzle&nbsp;&nbsp;•&nbsp;&nbsp;16 patches&nbsp;&nbsp;<a href="https://patch.p0ntus.com/status"><img src="https://worker.patch.p0ntus.com/badges/tests/com.sezzle.sezzlemobile.svg" alt="Tests Passing" align="absmiddle"></a></summary>
<br>

**🎯 Supported versions:**

| 5.3.9 |
| :---: |

| 💊&nbsp;Patch | 📜&nbsp;Description | ⚙️&nbsp;Options |
|----------|----------------|-----------|
| [Clean Authentication](#clean-authentication) | Shows Google sign-in only and removes the unavailable phone sign-in controls. | • Inject patch warning |
| [Configure Shortcuts](#configure-shortcuts) | Customizes items displayed in the Your Shortcuts carousel. | • Hide Refer a Friend<br>• Hide Giveaway<br>• Hide Offers<br>• Hide Rewards<br>• Hide Sezzle Mobile<br>• Hide Amazon Deals<br>• Hide Pay Later Anywhere |
| [Enable App Debugging](#enable-app-debugging) | Marks the app debuggable so patch developers can use ADB run-as after reinstalling. |  |
| [Hide Sezzle Mobile](#hide-sezzle-mobile) | Hides Sezzle Mobile offers and account entry points. |  |
| [Patch Consent Screen](#patch-consent-screen) | Requires consent to a patched-app warning before opening Sezzle authentication. |  |
| [Remove Ads and Tracking](#remove-ads-and-tracking) | Removes all ads (AppLovin MAX, Google Mobile Ads, Rokt, Thanks network, Playtime, InBrain Surveys) and disables analytics and tracking SDKs (AppsFlyer, FullStory, Braze, Firebase Analytics, mParticle, Facebook SDK, AppCenter). |  |
| [Remove Promos & Giveaways](#remove-promos-giveaways) | Blocks in-app deal popups, giveaway screens, Knot card-linking dialogs, and marketing banners across the app. | • Block Merchant Deal Popovers<br>• Block Knot Account Linking Promos<br>• Block Wallet Marketing<br>• Block Playtime Marketing<br>• Block Referrals & Social<br>• Block Trivia<br>• Block Notification Prompts |
| [Remove Rewards](#remove-rewards) | Removes Rewards navigation while keeping Account's Sezzle Points item and routing the Home shortcut to the same page. |  |
| [Remove Sezzle Subscriptions](#remove-sezzle-subscriptions) | Removes references to Sezzle Anywhere and Sezzle Premium subscriptions in Account benefits, Wallet, and Orders help. |  |
| [Replace AI Discover with Products](#replace-ai-discover-with-products) | Replaces the AI Discover navigation tab with Sezzle's original non-AI Products tab and removes the Sezzle AI callout in search. Includes an option to remove the Products tab completely. | • Remove Products Tab |
| [Replace Shop with Home](#replace-shop-with-home) | Replaces the Shop bottom navigation tab with Home, a custom screen to replace the overly-commercial Shop screen. |  |
| [Suppress In-App Updates and Rating Prompts](#suppress-in-app-updates-and-rating-prompts) | Neutralizes Hermes Redux update sagas, UpdateAppModal dialogs, Play Store URL redirects, trustFall tamper detection, and in-app rating prompts. | • Suppress Force Updates<br>• Bypass Hermes Tamper Checks<br>• Suppress Store Rating Prompts |
| [Suppress Updates and Integrity Checks](#suppress-updates-and-integrity-checks) | Disables Microsoft CodePush OTA updates and neutralizes Dalvik root and tamper detection SDKs (RootBeer and JailMonkey). | • Disable CodePush OTA<br>• Bypass Root & Tamper Detection |
| [Unlock Custom App Icons](#unlock-custom-app-icons) | Enables custom launcher app icons (Arctic, Peach, Glass, Rainbow, Sand, Classic) without requiring a Sezzle Premium subscription. |  |
| [Unlock Developer Settings](#unlock-developer-settings) | Makes the internal Development Settings menu visible to every signed-in account. |  |
| [Unlock Receipt Scanner](#unlock-receipt-scanner) | Makes the receipt scanner available from Development Settings and forces its V2 flow to render. REQUIRES Unlock Developer Settings to be enabled. |  |

</details>

<details open>
<summary>📦 365 Pay&nbsp;&nbsp;•&nbsp;&nbsp;4 patches&nbsp;&nbsp;<a href="https://patch.p0ntus.com/status"><img src="https://worker.patch.p0ntus.com/badges/tests/com.avinside.svg" alt="Tests Passing" align="absmiddle"></a></summary>
<br>

**🎯 Supported versions:**

| 10.2605.0 |
| :---: |

| 💊&nbsp;Patch | 📜&nbsp;Description | ⚙️&nbsp;Options |
|----------|----------------|-----------|
| [Display Cutout Adaptation](#display-cutout-adaptation) | Dynamically adjusts header height and top padding on devices with camera cutouts or tall status bars so location selectors and action icons clear the camera hole. |  |
| [Remove Ads](#remove-ads) | Removes in-app promotional message campaigns and popups (OneSignal In-App Messages), neutralizes in-app review and rating prompts, and zeroes Google Play and Huawei Advertising IDs. | • Remove In-App Messages<br>• Remove Review Prompts<br>• Zero Advertising ID |
| [Remove Tracking](#remove-tracking) | Neutralizes diagnostic telemetry, event logging, and user tracking (Firebase Analytics, Crashlytics, Instana APM, Branch Metrics, and OneSignal session and outcome tracking). | • Disable Firebase Analytics<br>• Disable Firebase Crashlytics<br>• Disable Instana APM<br>• Disable Branch Metrics<br>• Disable OneSignal Telemetry |
| [Unlock Developer Tools](#unlock-developer-tools) | Exposes the internal developer and diagnostics screen (Environment Settings) on the Welcome Screen, and allows booting directly with Debug Messages or Demo Mode enabled. | • Unlock Developer Settings<br>• Start with Debug Messages<br>• Start in Demo Mode |

</details>

<details open>
<summary>📦 Fizz&nbsp;&nbsp;•&nbsp;&nbsp;4 patches&nbsp;&nbsp;<a href="https://patch.p0ntus.com/status"><img src="https://worker.patch.p0ntus.com/badges/tests/com.ashtoncofer.Buzz.svg" alt="Tests Passing" align="absmiddle"></a></summary>
<br>

**🎯 Supported versions:**

| 1.54.0 |
| :---: |

| 💊&nbsp;Patch | 📜&nbsp;Description | ⚙️&nbsp;Options |
|----------|----------------|-----------|
| [Enable Developer Settings](#enable-developer-settings) | Adds an in-app developer mod menu accessible via a top-bar header button, with controls for Mobile Studio. | • Mobile Studio |
| [Remove Ads](#remove-ads) | Removes sponsored feed advertisements and optional marketplace listing advertisements from the feed. | • Remove Marketplace Ads |
| [Remove Tracking and Analytics](#remove-tracking-and-analytics) | Neutralizes first-party client event tracking, Mixpanel analytics, Airbridge and Adjust attribution SDKs, Google Advertising ID (AAID) collection, and bypasses PairIP Play Integrity verification, with options for silent DM screenshots and Sentry telemetry removal. | • Silent Screenshots<br>• Disable Crash Reporting |
| [Replace Emoji Font with iOS](#replace-emoji-font-with-ios) | Replaces Android system emoji with iOS Apple Color Emoji across Compose UI, posts, comments, and direct messages (effective on Android 10+ / API 29+). |  |

</details>

<details open>
<summary>📦 Geocaching&nbsp;&nbsp;•&nbsp;&nbsp;9 patches&nbsp;&nbsp;<a href="https://patch.p0ntus.com/status"><img src="https://worker.patch.p0ntus.com/badges/tests/com.groundspeak.geocaching.intro.svg" alt="Tests Passing" align="absmiddle"></a></summary>
<br>

**🎯 Supported versions:**

| 10.21.0 |
| :---: |

| 💊&nbsp;Patch | 📜&nbsp;Description | ⚙️&nbsp;Options |
|----------|----------------|-----------|
| [Enable Developer Tools and Feature Flags](#enable-developer-tools-and-feature-flags) | Exposes the internal Debug Menu in Settings and provides selectors for LaunchDarkly feature flags, Remote Config controls, and remaining membership gates. | • Expose Debug Menu in Settings<br>• Enable Achievement Debug Tap<br>• Unlock Trails Map Types<br>• Unlock Newsletter Language Settings<br>• Unlock Hide Cache Premium Controls<br>• Enable Cache Details Redesign<br>• Show Total Log Count<br>• Enable Shareables<br>• Enable Collaborative Lists<br>• Enable Lodestones<br>• Enable Edit Log Date<br>• Enable Better Events Filter<br>• Enable Select Cache Types<br>• Enable Diagnostic Flags<br>• Force Map Tray Enabled<br>• Force Hide Last Log DNF<br>• Force Show Downtime Banner<br>• Force Disable Map Auto Update<br>• Force Send Diagnostic Logs<br>• Force Linear Callouts Enabled |
| [Local Premium](#local-premium) | Enables local Premium membership status across profile and account screens, removes upgrade promotions, banners, icons, and server-dependent profile statistics, and enriches public cache details (description, hints, owner) when server blocks basic accounts. | • Show Local Premium Warnings |
| [OpenStreetMap Drop-in Replacement](#openstreetmap-drop-in-replacement) | Replaces Google Maps with OpenStreetMap (MapLibre vector engine) and adds in-app OpenStreetMap style switching and custom URL configuration to Map settings. |  |
| [Remove Lists](#remove-lists) | Removes the Lists option from the bottom navigation bar. |  |
| [Remove Shop](#remove-shop) | Removes the Shop Geocaching promotional section and link from the Profile screen. |  |
| [Remove Tracking and Analytics](#remove-tracking-and-analytics) | Neutralizes first-party analytics (AnalyticsRepo), Google Analytics / Firebase (Analytics, Crashlytics, Performance, In-App Messaging), Facebook App Events, Iterable marketing telemetry, Usercentrics consent collection, and zeros the Google Play Advertising ID (AAID). | • Disable Firebase Crashlytics<br>• Disable Facebook Telemetry<br>• Disable Iterable Marketing Telemetry<br>• Disable Usercentrics Consent Telemetry |
| [Unlock Cache Filter and Sorting Tools](#unlock-cache-filter-and-sorting-tools) | Unlocks advanced cache search filters and sorting options without prompting for Geocaching Premium. |  |
| [Unlock Experimental Features](#unlock-experimental-features) | Unlocks beta and experimental features in Settings without Geocaching Premium. | • Unlock All Beta Projects<br>• Enable Hidden Experimental Toggles<br>• Bypass Experimental Server Sync |
| [Unlock Templates](#unlock-templates) | Unlocks geocache log templates, allowing creating, editing, and applying custom log templates without Geocaching Premium. |  |

</details>

<details open>
<summary>📦 NHL&nbsp;&nbsp;•&nbsp;&nbsp;3 patches&nbsp;&nbsp;<a href="https://patch.p0ntus.com/status"><img src="https://worker.patch.p0ntus.com/badges/tests/com.nhl.gc1112.free.svg" alt="Tests Passing" align="absmiddle"></a></summary>
<br>

**🎯 Supported versions:**

| 17.0.0 |
| :---: |

| 💊&nbsp;Patch | 📜&nbsp;Description | ⚙️&nbsp;Options |
|----------|----------------|-----------|
| [Remove Ads](#remove-ads) | Removes Google Ad Manager banner and native advertisements, disables Blaze story/moment sponsored placements, blocks Google Interactive Media Ads (IMA) video ad requests, and neutralizes Brightcove IMA mid-roll and pre-roll ad playback. | • Remove Promotions Carousel |
| [Remove Tracking](#remove-tracking) | Removes the OneTrust CMP tracking consent popup, zeros Google Play Advertising ID (AAID), blocks host application analytics, disables Firebase Analytics, AppMeasurement, Firebase Performance Monitoring, Crashlytics, and Sessions telemetry, neutralizes UrbanAirship and Blaze tracking and logging, suppresses Brightcove video metrics reporting, and blocks Ticketmaster telemetry. | • Remove Broken Privacy Settings |
| [Unlock Hidden Features](#unlock-hidden-features) | Unlocks the internal Developer Settings menu, Draft Mini Tracker, and NHL GameZone on the Home surface, with independent options for each feature. | • Enable Developer Tools & Settings<br>• Enable Draft Mini Tracker<br>• Show GameZone on Home |

</details>

<details open>
<summary>📦 Adobe Scan&nbsp;&nbsp;•&nbsp;&nbsp;6 patches&nbsp;&nbsp;<a href="https://patch.p0ntus.com/status"><img src="https://worker.patch.p0ntus.com/badges/tests/com.adobe.scan.android.svg" alt="Tests Passing" align="absmiddle"></a></summary>
<br>

**🎯 Supported versions:**

| 26.09.25 |
| :---: |

| 💊&nbsp;Patch | 📜&nbsp;Description | ⚙️&nbsp;Options |
|----------|----------------|-----------|
| [Remove Ads and Tracking](#remove-ads-and-tracking) | Disables Adobe, Branch, Facebook, Creative SDK, and Crashlytics telemetry; removes in-app ads and review prompts; blocks install-referrer collection; and zeroes the Google Play Advertising ID. | • Remove Settings |
| [Remove Attribution](#remove-attribution) | Names new scans 'Scan on [date]' instead of 'Adobe Scan [date]' and removes the attribution text from shared files. | • Remove Scan Name Attribution<br>• Remove Share Attribution Text |
| [Remove Login](#remove-login) | Starts Adobe Scan in its existing account-free local workspace and removes account sign-in gates; Adobe cloud features are unavailable. | • Clean Up Auth Components<br>• Remove Link Sharing<br>• Remove Edit Text<br>• Remove Move<br>• Remove Save as Word Doc<br>• Remove Compress PDF |
| [Remove Useless/Promotional Items](#remove-useless-promotional-items) | Removes promotional, feedback, and support items from Settings and file menus. | • About Adobe Scan<br>• Help<br>• Rate App<br>• Online Support Forum<br>• Share This App<br>• Fill & Sign<br>• Open in Adobe Acrobat |
| [Unlock Premium](#unlock-premium) | Enables locally executable premium OCR, editing, compression, and page-organization tools without cloud or GenAI access. | • Remove Broken Features<br>• Enable Cloud Tools (Experimental) |
| [Use System Font](#use-system-font) | Overrides Adobe Clean fonts across XML layouts, dialogs, and Jetpack Compose screens with the device's system font. |  |

</details>

<details open>
<summary>📦 Canvas Student&nbsp;&nbsp;•&nbsp;&nbsp;1 patch&nbsp;&nbsp;<a href="https://patch.p0ntus.com/status"><img src="https://worker.patch.p0ntus.com/badges/tests/com.instructure.candroid.svg" alt="Tests Passing" align="absmiddle"></a></summary>
<br>

**🎯 Supported versions:**

| 8.10.0 |
| :---: |

| 💊&nbsp;Patch | 📜&nbsp;Description | ⚙️&nbsp;Options |
|----------|----------------|-----------|
| [Remove Tracking and Analytics](#remove-tracking-and-analytics) | Neutralizes behavioral tracking (Pendo SDK session recordings, guides, and click tracking), student surveillance telemetry (Pandata pageview recording, time-spent counters, and background upload worker), first-party app analytics (ScreenView processors, offline analytics, token logging), crash reporting (Firebase Crashlytics), and in-app rating prompts. |  |

</details>

<details open>
<summary>📦 Navigate360 Student&nbsp;&nbsp;•&nbsp;&nbsp;2 patches&nbsp;&nbsp;<a href="https://patch.p0ntus.com/status"><img src="https://worker.patch.p0ntus.com/badges/tests/com.eab.se.svg" alt="Tests Passing" align="absmiddle"></a></summary>
<br>

**🎯 Supported versions:**

| 26.19.22 |
| :---: |

| 💊&nbsp;Patch | 📜&nbsp;Description | ⚙️&nbsp;Options |
|----------|----------------|-----------|
| [Remove Tracking and Telemetry](#remove-tracking-and-telemetry) | Neutralizes Gainsight PX behavioral analytics (session tracking, screen views, custom events, user identification) and disables the Cordova Gainsight native plugin. |  |
| [Remove Web Telemetry](#remove-web-telemetry) | Removes Sentry error/performance reporting, CSP telemetry endpoints, and Gainsight web bootstrap scripts from the embedded Cordova web bundle. |  |

</details>

<!-- PATCHES_END -->
## 🛠️ Building

### Prerequisites

- JDK 21+ (Eclipse Temurin baseline)
- Android SDK (set `sdk.dir` in `local.properties` or export `ANDROID_HOME`)

### Build Command

```bash
./gradlew buildAndroid
```

The compiled patch bundle will be generated at `patches/build/libs/patches-*.mpp`.

To update `patches-list.json`:
```bash
./gradlew generatePatchesList
```
### Analysis & Compatibility Testing (`apk-lab`)

Reverse engineering workflows, multi-split artifact inspection, ARM64 assembly, IL2CPP metadata querying, and isolated patch compatibility checks use `tools/apk-lab`:

```bash
uv run --project tools/apk-lab apk-lab doctor
uv run --project tools/apk-lab apk-lab check path/to/app.apkm --mpp patches/build/libs/patches-1.0.0.mpp --package com.example.app --all
```

See [`docs/apk-lab.md`](docs/apk-lab.md) for complete CLI documentation and workflows.


## 📲 Applying Patches

Using [Morphe Desktop](https://github.com/MorpheApp/morphe-desktop):

```bash
java -jar morphe-desktop.jar patch \
  --patches patches/build/libs/patches-1.0.0.mpp \
  --out sezzle-patched.apk \
  base.apk
```

## 📊 Statistics

![Total Downloads](https://worker.patch.p0ntus.com/charts/total-downloads.svg)
![Release Downloads](https://worker.patch.p0ntus.com/charts/release-downloads.svg)

## 📜 License

Licensed under the [WTFPL](LICENSE) with warranty addendum.
