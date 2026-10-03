# Fizz Architecture & Reverse Engineering Specification

## Overview

| Attribute | Specification |
|---|---|
| **Application Name** | Fizz |
| **Package Name** | `com.ashtoncofer.Buzz` |
| **Supported Version** | `1.53.0` (Morphe Compatibility: `1.53.0`, `minSdk` 23) |
| **Analyzed Version Code** | `394385` |
| **Target SDK** | `36` (Android 16) |
| **Minimum SDK** | `23` (Android 6.0) |
| **APK Format** | APKM (`base.apk`, split configs) |
| **Package Signing SHA-256** | `622850867847ccb7a1371bc42c865b1137fa51bd19987dc81b53815c9a9817bf` |
| **Architecture** | Native Kotlin/Java; Jetpack Compose, Dagger/Hilt, Kotlinx Coroutines, Retrofit/OkHttp, Mixpanel, Airbridge, Adjust, Sentry, Google Play Integrity / PairIP |

Fizz is a college-centric anonymous social feed and messaging platform. The application incorporates extensive telemetry across first-party analytics pipelines, attribution services, third-party analytics SDKs, session crash reporting, and Google Play Integrity Protection (PairIP).

---

## Technology Stack

- **UI & Presentation**: Modern Jetpack Compose UI with declarative design components and Compose navigation.
- **Dependency Injection**: Dagger and Hilt manage dependencies for repositories, network clients, view models, and application lifecycles.
- **Networking**: Retrofit 2 and OkHttp handle RESTful endpoints for feed browsing, comments, direct messaging, client event tracking, and attribution syncing.
- **Coroutines & Asynchronous Work**: Kotlinx Coroutines run background jobs, batch uploads, and real-time state management.
- **Serialization**: Kotlinx Serialization parses incoming and outgoing API models.
- **Tamper Protection**: Google Play Automatic Integrity Protection / PairIP client (`com.pairip.licensecheck.LicenseClient`) verifies licensing and APK signature integrity at application startup.

---

## Telemetry & Anti-Tamper Architecture

### 1. Google Play Automatic Integrity Protection (PairIP)

The application embeds PairIP (`com.pairip.licensecheck.LicenseClient`) invoked early in `Application.attachBaseContext`. If signature mismatches or tampering are detected, the process terminates before initialization of main activities or user interactions.

```mermaid
flowchart TD
    A[Application.attachBaseContext] --> B[LicenseClient.checkLicense]
    B -->|Check Fails| C[Process Exit / Tamper Terminated]
    B -->|Bypassed / Normal| D[Application.onCreate]
```

### 2. First-Party Client Event Pipeline

User interactions (post impressions, votes, profile opens, community switching, comment expansion) are captured into event models (`ra.n9`) and dispatched through `ra.da` (implementing `ra.s9`). `ra.da` queues events locally in memory, flushes them periodically, and uploads serialized JSON payloads to the `/app/track-client-events` backend endpoint via `jc.i0`.

```mermaid
flowchart LR
    UI[User / Feed Action] --> DA[ra.da.a Event Enqueue]
    DA --> FL[ra.da.d Flush Loop]
    FL --> BA[ra.da.g Batch Dispatch]
    BA --> NET[jc.i0 Retrofit Client]
    NET --> ENDPOINT[Fizz Backend /app/track-client-events]
```

### 3. Mixpanel Product Analytics

Mixpanel (`dk.u`) tracks granular user flow, super properties, funnel analytics, and distinct user identities. In conjunction with `ra.va` (which generates and persists random installation and device tracking UUIDs), events are aggregated and uploaded to Mixpanel ingestion endpoints.

```mermaid
flowchart LR
    ID[ra.va AnalyticsIdentity] --> MP[dk.u MixpanelAPI]
    EV[App Navigation Events] --> MP
    MP --> CLOUD[Mixpanel Ingestion Servers]
```

### 4. Airbridge Measurement & Attribution

Airbridge (`co.ab180.airbridge.Airbridge`, wrapped in `sa.n`) collects install attribution, deferred deep links, campaign parameters, and device hardware aliases. It hooks `Application.onCreate` via `sa.n.f` and listens for campaign intent triggers.

### 5. Adjust Mobile Attribution

Adjust (`com.adjust.sdk.Adjust`, wrapped in `sa.b`) tracks deep-link referrals, app sessions, advertising campaigns, and subscription measurement. `sa.b.f` manages initialization from application context.

### 6. Google Advertising Identifier (AAID)

The app queries Google Play Services `AdvertisingIdClient` to retrieve the device Advertising ID (AAID) and ad-tracking preferences. `sa.k.a` registers this device identifier with backend services for cross-device user matching and targeted re-engagement campaigns.

### 7. Sentry Performance & Crash Reporting

Sentry Android (`io.sentry.android.core.q1`, initialized via `ec.b1.b`) monitors unhandled exceptions, ANRs, HTTP breadcrumbs, and UI transaction performance traces. Telemetry captures breadcrumbs of navigation, network requests, and device state.

### 8. Direct Message Screenshot Surveillance

In private direct messaging conversations, `ConversationViewModel` observes system screenshot events. When a screenshot is captured, `rd.b2` (a coroutine worker) triggers `jc.l2.q` to invoke `/chat/notify-screenshot`, alerting the chat counterpart that their private messages were captured.

```mermaid
flowchart LR
    SC[Android Screenshot Event] --> VM[ConversationViewModel]
    VM --> WK[rd.b2 Coroutine Worker]
    WK --> RP[jc.l2.q ChatRepository]
    RP --> API[POST /chat/notify-screenshot]
    API --> CP[Counterpart Notified]
```

---

## Patch Targets

1. **`RemoveTrackingAndAnalyticsPatch.kt` (`bytecodePatch`)**:
   - Neutralizes PairIP signature verification (`LicenseClient.checkLicense`).
   - Stubs first-party event enqueuing and batch uploads (`ra.da`, `jc.i0`, `ec.j`).
   - Disables Mixpanel tracking methods, resets, and forces tracking opt-out (`dk.u`, `ra.va`).
   - Neutralizes Airbridge attribution methods and lifecycle initialization (`sa.n`, `Airbridge`).
   - Neutralizes Adjust attribution methods and session hooks (`sa.b`, `Adjust`).
   - Zeroes the Google Advertising ID (`00000000-0000-0000-0000-000000000000`) and limits ad tracking (`AdvertisingIdClient`, `sa.k`).
   - Configurable option `disableCrashReporting`: Disables Sentry initialization and reporting (`ec.b1`, `q1`).
   - Configurable option `silentScreenshots`: Disables screenshot detection alerts to chat counterparts (`rd.b2`, `jc.l2.q`).
