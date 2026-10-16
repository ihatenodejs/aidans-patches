# NHL Android Application Architecture

## 1. Overview

- **Application Name:** NHL
- **Package Name:** `com.nhl.gc1112.free`
- **Target Version:** `17.0.0` (Morphe Compatibility: `17.0.0`, `minSdk` 29)
- **Analyzed Version Code:** `17000026`
- **Target SDK:** 37
- **Minimum SDK:** 29 (Android 10)
- **Distribution Format:** Android App Bundle / Split APKM (`ApkFileType.APKM`)
- **Supported Native ABIs:** `arm64-v8a`
- **Signer SHA-256:** `2309209340875f6f8a008ea25dbd98d8a03e482704d43211933d2b9a91923aff`
- **Technology Stack:** Jetpack Compose, Kotlin Coroutines, Koin, Retrofit / OkHttp, Brightcove Video SDK, Blaze SDK (Stories & Moments), Ticketmaster SDK, OneTrust Headless SDK, UrbanAirship (Airship), Google Mobile Ads / Google Ad Manager (GAM), Google Interactive Media Ads (IMA), Firebase Analytics / Crashlytics / Sessions.

The official NHL application is built using modern Android Jetpack libraries, Compose UI, and multi-DEX Dalvik bytecode heavily obfuscated and flattened with R8/ProGuard. Several prominent third-party media and ad SDKs (Blaze, Brightcove, Ticketmaster, OneTrust, Airship) are integrated to provide game video streaming, stories, ticket management, and telemetry.

---

## 2. Technology Stack & Key Subsystems

### UI & Architecture
- **Framework:** Jetpack Compose alongside AndroidX AppCompat components.
- **Entry Points:** `com.nhl.mobile.android.core.application.NhlApplication` and `com.nhl.mobile.android.main.MainActivity` with numerous team-specific activity aliases.
- **Dependency Injection:** Koin and lazy resolution providers.

### Media & Video Streaming
- **Brightcove Player SDK:** Powers game highlights, live video feeds, and radio audio services (`NHLRadioService`, `BrightcoveMediaSessionService`).
- **Interactive Media Ads (IMA):** Integrated through Brightcove Google IMA component (`com.brightcove.ima.GoogleIMAComponent`), handling pre-roll and mid-roll video ad pods.
- **SpringServe:** Video ad server integration via `com.brightcove.ima.springserve.SpringServeHelper`.

### Stories & Moments (Blaze SDK)
- **Blaze SDK (`com.blaze.blazesdk`):** Powers stories, moments, and vertical video experiences across the application.
- **Blaze Ad Integrations:**
  - Google Ad Manager (GAM) banner ads via `com.blaze.gam.banner.DefaultBlazeGAMBannerAdsHandler`.
  - Custom native story ads via `com.blaze.gam.custom_native.DefaultBlazeGoogleCustomNativeAdsHandler` and `BlazeCustomNativeAdsManager`.
  - Blaze IMA video ad module via `com.blaze.ima.BlazeIMA` and `ImaHandler`.

### Ticketing & Identity
- **Ticketmaster SDK:** Embedded authentication and ticket delivery (`com.ticketmaster.tickets.ticketssdk.TicketsSDKSingleton`).

### Consent & Privacy Control
- **OneTrust Headless SDK:** `com.onetrust.otpublishers.headless.Public.OTPublishersHeadlessSDK` integrated through `Lgce;` to manage IAB TCF 2.2 consent strings and purpose grants (`IAB2V2_1`, `sm2.Functional`, `sm2.Performance`).
- **Consented SDKs (`Lqoj;`):** Enum categorizing third-party modules subject to user consent (`GoogleAds`, `Brightcove`, `UrbanAirshipCore`, `UrbanAirshipFcm`, `BlazePushSdk`, `TicketmasterTickets`, `TicketmasterAuthentication`, `OneTrust`).

### Analytics, Advertising & Telemetry Systems
- **Google Mobile Ads (GMS Ads):** `AdManagerAdView`, `AdService`, `MobileAdsInitProvider`, and `MobileAds`.
- **Google Play Advertising ID (AAID):** Minified `AdvertisingIdClient` (`Ljj;` returning `Lij;`) collecting hardware advertising identifiers.
- **Firebase Analytics / AppMeasurement:** Global analytics dispatcher via `Ljxd;->x()Lcom/google/firebase/analytics/FirebaseAnalytics;` backed by Google Play Services `Lg3q;` task execution.
- **UrbanAirship Telemetry:** Install referrer capture (`InstallReceiver`), AAID collection (`AdIdModuleFactory`), and analytics database syncing.
- **Blaze SDK Telemetry:** Event bulk dispatcher (`com.blaze.blazesdk.analytics.AnalyticsApi`).
- **Brightcove Video Analytics:** Video playback metric events dispatched via `com.brightcove.player.analytics.AnalyticsClient`.
- **Firebase Crashlytics & Sessions:** Real-time crash diagnostics (`Lbn6;`) and session lifecycle monitoring (`SessionLifecycleService`).

---

## 3. Remote Configuration & Feature Gates

The NHL application relies on remote configuration payloads (bundled with an initial baseline in `assets/config.json`) to control seasonal navigation, marketing surfaces, and feature visibility.

### Shipped Configuration Keys & Serialization Mappings

Baseline snapshot in `assets/config.json`:
- `draft.isMiniTrackerActive`: Boolean flag (default `false`) gating the Draft Mini Tracker on home/scores surfaces.
- `sections.gameZone.showOnHome`: Boolean flag (default `false`) gating the GameZone web hub card on the Home surface.
- `tabBarConfiguration`: String enum (default `"seasonPreview"`; variants include `"Draft"`, `"FourNations"`, `"InternationalHockey"`, `"OffSeason"`, `"Playoffs"`, `"PreSeason"`, `"Regular"`).
- `sections.flags.isPrebidEnabled`: Boolean flag (default `false`) gating Magnite/Prebid advertising in `Llxk;->g(...)V`.

Bytecode mappings and consumers:
- **Draft Configuration:** Deserialized via `Lmi5;` into `Loi5;` where field `a:Z` represents `isMiniTrackerActive`.
- **GameZone Configuration:** Deserialized via `Lhv7$a;` into `Lhv7;` where field `a:Ljava/lang/Boolean;` represents `showOnHome` and field `b:Ljava/lang/String;` contains the target URL (`https://s3.foreverflow.click/api/gameZone/index.html?...`).
- **Central Consumer:** `Lcdr;->U(Ljg0;)Lg24;` is the sole consumer mapping both deserialized config responses into active UI state.
- **Dependencies & Constraints:** While client-side bytecode renders the `DraftMiniTracker` and GameZone WebView host surfaces, active tracker content depends on live NHL draft feeds, and GameZone relies on server authentication and host web availability.

### Developer Settings & Diagnostic Subsystem

The application includes an internal diagnostic and developer configuration suite gated by the static boolean method `Llpk;->a()Z`. In standard release builds, `Llpk;->a()Z` delegates to `!Lftd;->a`, which resolves to `true` (production mode). When forced to return `false` (developer/non-prod mode), the following subsystems activate:

1. **Settings Screen Developer Section:**
   - Rendered by composable `Ldq8;k(...)` / `Lmrd;` and driven by `DeveloperSettingsViewModel` (`Lbnh;`).
   - **API Environment Switcher (`Lw26;`):** Allows selecting between:
     - `PROD` (`https://api-mobile.nhle.com/`)
     - `STAGING` (`https://api-mobile-stage.nhle.com/`)
     - `STAGING_PLAYOFFS` (`https://api-mobile-stage2.nhle.com/`)
     - `TEST` (`https://api-mobile-test.nhle.com/`)
     - `MOCK` (`https://apinhl.foreverflow.click/`)
     - `DYNAMIC_MOCK` (`http://localhost:8080/`)
     Persisted in the `debug_settings` DataStore through `Lvs4;` and read by `Lzr4;`.
   - **Config Environment Switcher (`Lhr3;`):** Toggles between `PROD`, `STAGING`, and `TEST`.
   - **Country Code Override (`Ljb4;`):** Overrides detected geo-location with `US`, `CANADA`, `SE`, `CZ`, `FI`, `DE`, `SK`, `ES`, or `RoW`.
   - **Dynamic Mock URL Entry:** Text inputs for custom mock host URLs.
   - **Analytics Debugger Toggle:** Persisted via DataStore key `analytics_debugger_enabled`.
   - **String Key Overlay Toggle:** Persisted via DataStore key `string_key_overlay_enabled` in `Llc8;p`; displays raw string localization resource keys directly over UI text labels.
   - **Mock Localization Toggle:** Persisted via DataStore key `mock_localization_enabled`.
   - **Team Icon Tricode Override:** Sets `pending_icon_tricode` for UI styling.
   - **Developer Action Triggers:**
     - *Invalidate Cache:* Clears OkHttp network disk caches and image caches with an immediate Android Toast confirmation.
     - *Ad Inspector:* Launches the Google Mobile Ads SDK debug inspector (`Lvze;invoke`).
     - *OneTrust Clear SDK Data:* Resets stored consent and privacy data.
2. **Developer Deep Link Navigation:**
   - NavHost builder (`Lxs8;invoke`) registers the deep link route `setAPI?testingURL={testingURL}&name={name}` (`nhl://setAPI?testingURL={testingURL}&name={name}` in `Lheh;invoke`), routing parameters directly to `Lzj3$a;`.
3. **Test Ad Unit Gating:**
   - `Luat;a` enables test ad unit configurations when developer mode is active.

### Rejected False Leads & Non-Features

- **Prebid Flag (`sections.flags.isPrebidEnabled`):** Maps through `Lzp6;`/`Lxp6;` into `Llxk;->g(...)V`. Gating this on forces third-party Magnite header-bidding ad requests, conflicting with the Remove Ads patch; it is not a user feature.
- **Tab Bar Seasonal Configuration (`tabBarConfiguration`):** Selecting seasonal navigation configurations (`Draft`, `Playoffs`, etc.) changes global tab hierarchies without providing required seasonal feed data, leading to empty or broken views.
- **Google Mobile Ads Internal Debug Strings:** Mentions of debug menus in `Le4p;->W1(...)` correspond to internal Google Mobile Ads SDK dialogs rather than NHL application features.
- **Airship & Ticketmaster Staging Controls:** Feature flags in UrbanAirship or staging environment switches in Ticketmaster SDK (`FastlyStaging`, `TeamStaging`) are third-party SDK internals that disrupt production authentication, ticketing, and push token registration.
- **React Native / Hermes Bundles:** Investigation confirmed the application is native Kotlin with Jetpack Compose; no React Native, Hermes bytecode, or JavaScript bundles exist in the artifact.
