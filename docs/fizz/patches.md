# Fizz Patch Specifications

## Overview

This document specifies binary bytecode patches developed for **Fizz** (`com.ashtoncofer.Buzz`).

---

## Patch: Remove Tracking and Analytics

- **Name:** Remove Tracking and Analytics
- **Target Package:** `com.ashtoncofer.Buzz`
- **Supported Versions:** `1.53.0`
- **Default State:** `true` (Enabled by default)
- **Type:** Dalvik Bytecode Patch (`bytecodePatch`)
- **Dependencies:** None

### 1. Motivation & Purpose

Fizz incorporates multiple concurrent telemetry pipelines:
1. **PairIP / Google Play Integrity Protection**: Prevents execution of modified APKs by verifying signatures and licensing upon process startup in `attachBaseContext`.
2. **First-Party Client Event Tracking**: Streams fine-grained feed interactions, post views, votes, comment actions, and profile visits to `/app/track-client-events`.
3. **Mixpanel Analytics**: Captures funnel events, demographic properties, and distinct device installation identifiers.
4. **Airbridge Attribution**: Collects install attribution, deep-link campaigns, and device aliases.
5. **Adjust Attribution**: Gathers campaign referrals, session telemetry, and third-party data-sharing signals.
6. **Advertising Identifier (AAID)**: Collects Google Advertising ID and uploads it to Fizz backend servers for ad targeting and user re-identification.
7. **Sentry Telemetry**: Collects diagnostic session logs, error traces, HTTP breadcrumbs, and ANR events.
8. **DM Screenshot Alerts**: Notifies chat counterparts whenever a screenshot is taken inside direct message threads.

This patch neutralizes these surveillance, attribution, and anti-tamper mechanisms while preserving feed browsing, posting, voting, commenting, and direct messaging functionality.

---

### 2. Options Breakdown

- **`silentScreenshots` (Boolean, Default: `true`)**:
  - *Title:* Silent Screenshots
  - *Description:* Suppresses screenshot notification dispatches to chat counterparts in direct message conversations.
  - When enabled, allows taking screenshots of chats without triggering `/chat/notify-screenshot` alerts to the other user.
- **`disableCrashReporting` (Boolean, Default: `true`)**:
  - *Title:* Disable Crash Reporting
  - *Description:* Neutralizes Sentry crash reporting, performance tracing, and operational session telemetry.
  - When enabled, disables `SentryAndroid.init` and marks the Sentry reporting layer as permanently disabled.

---

### 3. Technical Implementation & Injection Points

#### Layer 1: PairIP / Google Play Integrity Protection Bypass
- **Target:** `Lcom/pairip/licensecheck/LicenseClient;`
- **Method:** `checkLicense(Landroid/content/Context;)V`
- **Injection:** Injects `return-void` at index 0.
- **Effect:** Immediately neutralizes runtime licensing and signature validation checks during startup.

#### Layer 2: First-Party Analytics Event Logger & Batch Dispatcher
- **Target 1:** `Lra/da;` (Event queue and flush manager)
  - `a(Lra/n9;Z)V`: Injects `return-void` (stops event enqueuing).
  - `e()V`: Injects `return-void`.
  - `f()V`: Injects `return-void`.
  - `d(ZLol/c;)Ljava/lang/Object;`: Injects `sget-object v0, Lil/z;->a:Lil/z \n return-object v0` (stops periodic flush loop).
  - `g(Lfb/c;Lol/c;)Ljava/lang/Object;`: Injects `sget-object v0, Lil/z;->a:Lil/z \n return-object v0` (stops batch network delivery).
- **Target 2:** `Ljc/i0;` (Analytics network client)
  - `a(Ljava/util/List;Lol/c;)Ljava/lang/Object;`: Injects `const/4 v0, 0x0 \n return-object v0` (neutralizes `POST /app/track-client-events`).
- **Target 3:** `Lec/j;` (Operational reporter)
  - Methods `a`, `b`, `c`, `d`, `e`: Stubbed to `return-void`.

#### Layer 3: Mixpanel Analytics SDK
- **Target 1:** `Ldk/u;` (Mixpanel API client)
  - Void methods `d` (flush) and `l` (reset): Stubbed to `return-void`.
  - `i(Ljava/lang/String;Z)V` (identify): Injects `return-void`.
  - `k(Lorg/json/JSONObject;)V` (super properties): Injects `return-void`.
  - `m(Lorg/json/JSONObject;Ljava/lang/String;Z)V` (track): Injects `return-void`.
  - `b(Ljava/lang/String;Lorg/json/JSONObject;Ljava/lang/Long;)Ldk/a;`: Injects `const/4 v0, 0x0 \n return-object v0`.
  - `h()Z` (hasOptedOutTracking): Injects `const/4 v0, 0x1 \n return v0` (signals user opted out).
- **Target 2:** `Lra/va;` (AnalyticsIdentity manager)
  - `c()V`: Injects `return-void` (prevents device identity generation and refresh).

#### Layer 4: Airbridge Attribution & Mobile Measurement SDK
- **Target 1:** `Lsa/n;` (App Airbridge wrapper)
  - `f(Landroid/app/Application;Lqa/a0;)V`: Injects `return-void` (neutralizes SDK init in `FizzApplication.onCreate`).
  - `b(Ljava/lang/String;)V`: Injects `return-void`.
  - `c(Lsa/d0;)V`: Injects `return-void`.
  - `d()V`: Injects `return-void`.
  - `e(Ljava/lang/String;)V`: Injects `return-void`.
  - `a(Landroid/content/Intent;Lge/e;)Z`: Injects `const/4 v0, 0x0 \n return v0`.
- **Target 2:** `Lco/ab180/airbridge/Airbridge;`
  - Void methods: `initializeSDK`, `trackEvent`, `startTracking`, `startInAppPurchaseTracking`, `stopTracking`, `stopInAppPurchaseTracking`, `clearUser`, `clearDeviceAlias`, `clearUserAlias`, `clearUserAttributes`, `clearUserEmail`, `clearUserID`, `clearUserPhone`, `allowTrackingItem`, `blockTrackingItem`, `disableSDK`, `enableSDK`, `registerPushToken`, `removeDeviceAlias`, `removeUserAlias`, `removeUserAttribute`, `setDeviceAlias`, `setUserAlias`, `setUserAttribute`, `setUserEmail`, `setUserID`, `setUserPhone`, `setWebInterface`: Injects `return-void`.
  - Boolean methods: `isTrackingEnabled()Z`, `isInAppPurchaseTrackingEnabled()Z`, `isSDKEnabled()Z`: Injects `const/4 v0, 0x0 \n return v0`.

#### Layer 5: Adjust Attribution & Tracking SDK
- **Target 1:** `Lsa/b;` (App Adjust wrapper)
  - `f(Landroid/content/Context;)Z`: Injects `const/4 v0, 0x0 \n return v0` (signals Adjust is disabled/uninitialized).
  - Void methods `b`, `c`, `d`, `e`: Stubbed to `return-void`.
  - `a(Landroid/content/Intent;Lge/e;)Z`: Injects `const/4 v0, 0x0 \n return v0`.
- **Target 2:** `Lcom/adjust/sdk/Adjust;`
  - Void methods: `initSdk`, `trackEvent`, `trackAdRevenue`, `trackMeasurementConsent`, `trackPlayStoreSubscription`, `trackThirdPartySharing`, `setPushToken`, `setReferrer`, `onResume`, `onPause`, `disable`, `enable`, `gdprForgetMe`, `switchToOfflineMode`, `switchBackToOnlineMode`: Injects `return-void`.

#### Layer 6: Google Play Advertising ID (AAID) Neutralization
- **Target 1:** `Lcom/google/android/gms/ads/identifier/AdvertisingIdClient;`
  - `getAdvertisingIdInfo(Landroid/content/Context;)Lcom/google/android/gms/ads/identifier/AdvertisingIdClient$Info;`: Injects spoofed `Info` object with zeroes:
    ```smali
    new-instance v0, Lcom/google/android/gms/ads/identifier/AdvertisingIdClient$Info;
    const-string v1, "00000000-0000-0000-0000-000000000000"
    const/4 v2, 0x1
    invoke-direct {v0, v1, v2}, Lcom/google/android/gms/ads/identifier/AdvertisingIdClient$Info;-><init>(Ljava/lang/String;Z)V
    return-object v0
    ```
- **Target 2:** `Lcom/google/android/gms/ads/identifier/AdvertisingIdClient$Info;`
  - `getId()Ljava/lang/String;`: Injects `const-string v0, "00000000-0000-0000-0000-000000000000" \n return-object v0`.
  - `isLimitAdTrackingEnabled()Z`: Injects `const/4 v0, 0x1 \n return v0`.
- **Target 3:** `Lsa/k;` (AAID Backend Registrar)
  - `a(Lol/c;)Ljava/lang/Object;`: Injects `sget-object v0, Lil/z;->a:Lil/z \n return-object v0`.

#### Layer 7: Sentry Telemetry (Guarded by `disableCrashReporting`)
- **Target 1:** `Lec/b1;` (App Sentry coordinator)
  - `b(Lcom/fizzsocial/fizz/FizzApplication;)V`: Injects `return-void` (suppresses Sentry initialization on startup).
  - `isEnabled()Z`: Injects `const/4 v0, 0x0 \n return v0`.
- **Target 2:** `Lio/sentry/android/core/q1;` (SentryAndroid core)
  - `b(Landroid/content/Context;Lio/sentry/android/core/y;Lio/sentry/k4;)V`: Injects `return-void`.

#### Layer 8: Silent DM Screenshots (Guarded by `silentScreenshots`)
- **Target 1:** `Lrd/b2;` (Conversation screenshot notification worker)
  - `invokeSuspend(Ljava/lang/Object;)Ljava/lang/Object;`: Injects `sget-object v0, Lil/z;->a:Lil/z \n return-object v0`.
- **Target 2:** `Ljc/l2;` (Chat repository)
  - `q(Ljava/lang/String;Lol/c;)Ljava/lang/Object;`: Injects synthetic successful `Result.success(Unit)`:
    ```smali
    sget-object v0, Lil/z;->a:Lil/z;
    new-instance v1, Lcb/l;
    invoke-direct {v1, v0}, Lcb/l;-><init>(Ljava/lang/Object;)V
    return-object v1
    ```

---

### 4. Preconditions & Verification

1. **Target Specification**: Compatible with Fizz `1.53.0` (`com.ashtoncofer.Buzz`), signature SHA-256 `622850867847ccb7a1371bc42c865b1137fa51bd19987dc81b53815c9a9817bf`.
2. **Build Verification**:
   ```bash
   ./gradlew :patches:buildAndroid clean --no-daemon
   ```
3. **Metadata Generation**:
   ```bash
   ./gradlew generatePatchesList
   ```
4. **Bytecode Verification**:
   - Inspect patched APK using `jadx` or `baksmali` to confirm `checkLicense`, `ra.da`, `jc.i0`, `dk.u`, `sa.n`, `sa.b`, `AdvertisingIdClient`, `ec.b1`, and `rd.b2` contain injected instructions.
