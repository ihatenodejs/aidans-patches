package app.aidan.patches.geocaching.tracking

import app.aidan.patches.geocaching.shared.COMPATIBILITY_GEOCACHING
import app.aidan.patches.shared.disableOptionalVoidMethods
import app.morphe.patcher.extensions.InstructionExtensions.addInstructions
import app.morphe.patcher.patch.BytecodePatchContext
import app.morphe.patcher.patch.booleanOption
import app.morphe.patcher.patch.bytecodePatch

val removeTrackingAndAnalyticsPatch = bytecodePatch(
    name = "Remove Tracking and Analytics",
    description = "Neutralizes first-party analytics (AnalyticsRepo), Google Analytics / Firebase (Analytics, Crashlytics, Performance, In-App Messaging), Facebook App Events, Iterable marketing telemetry, Usercentrics consent collection, and zeros the Google Play Advertising ID (AAID).",
    default = true
) {
    category("Privacy")
    compatibleWith(COMPATIBILITY_GEOCACHING)

    val stripCrashlytics = booleanOption(
        key = "stripCrashlytics",
        default = true,
        title = "Disable Firebase Crashlytics",
        description = "Neutralizes Firebase Crashlytics diagnostic telemetry and crash reporting."
    )
    val stripFacebook = booleanOption(
        key = "stripFacebook",
        default = true,
        title = "Disable Facebook Telemetry",
        description = "Neutralizes Facebook App Events SDK logging and activation pings."
    )
    val stripIterable = booleanOption(
        key = "stripIterable",
        default = true,
        title = "Disable Iterable Marketing Telemetry",
        description = "Neutralizes Iterable marketing SDK event and push tracking."
    )
    val stripUsercentrics = booleanOption(
        key = "stripUsercentrics",
        default = true,
        title = "Disable Usercentrics Consent Telemetry",
        description = "Neutralizes Usercentrics consent management and tracking telemetry."
    )

    execute {
        // Layer 1: Google Play Advertising ID (AAID) Spoofing
        spoofAdvertisingId()

        // Layer 2: Centralized First-Party Analytics (AnalyticsRepo & Helpers)
        disableOptionalVoidMethods(
            "Lcom/groundspeak/geocaching/intro/analytics/firebase/AnalyticsRepo;",
            "m", "a", "b", "c", "f", "g", "h", "i", "j", "k", "l", "n", "o", "p", "q"
        )
        disableOptionalVoidMethods(
            "Lcom/groundspeak/geocaching/intro/analytics/firebase/AnalyticsRepo\$a;",
            "a"
        )
        disableOptionalVoidMethods(
            "Lvm3;",
            "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m", "n", "o", "p", "q", "r", "s", "t", "u", "v"
        )
        disableOptionalVoidMethods(
            "Lgo3;",
            "b", "f"
        )
        disableOptionalVoidMethods(
            "Lcom/groundspeak/geocaching/intro/analytics/firebase/AnalyticsWebInterface;",
            "logEvent"
        )

        // Layer 3: Firebase Analytics & Measurement Neutralization
        disableOptionalVoidMethods(
            "Lcom/google/firebase/analytics/FirebaseAnalytics;",
            "logEvent",
            "setAnalyticsCollectionEnabled",
            "setUserProperty",
            "setDefaultEventParameters",
            "setSessionTimeoutDuration",
            "setUserId"
        )
        disableOptionalVoidMethods(
            "Lcom/google/firebase/perf/FirebasePerformance;",
            "setPerformanceCollectionEnabled"
        )
        disableOptionalVoidMethods(
            "Lcom/google/firebase/inappmessaging/FirebaseInAppMessaging;",
            "setAutomaticDataCollectionEnabled",
            "setMessagesSuppressed"
        )

        // Layer 4: Firebase Crashlytics Neutralization
        if (stripCrashlytics.value != false) {
            disableOptionalVoidMethods(
                "Lcom/google/firebase/crashlytics/FirebaseCrashlytics;",
                "log",
                "recordException",
                "setUserId",
                "setCustomKey",
                "setCrashlyticsCollectionEnabled"
            )
            disableOptionalVoidMethods(
                "Lcom/groundspeak/geocaching/intro/analytics/crashlytics/a;",
                "a", "b", "c", "d", "e"
            )
        }

        // Layer 5: Facebook App Events Neutralization
        if (stripFacebook.value != false) {
            disableOptionalVoidMethods(
                "Lcom/facebook/appevents/AppEventsLogger;",
                "activateApp",
                "logEvent",
                "logPurchase",
                "flush",
                "setFlushBehavior"
            )
            disableOptionalVoidMethods(
                "Lcom/facebook/internal/FetchedAppSettingsManager;",
                "loadAppSettingsAsync"
            )
            disableOptionalVoidMethods(
                "Ldefpackage/wv;",
                "l"
            )
        }

        // Layer 6: Iterable Telemetry Neutralization
        if (stripIterable.value != false) {
            disableOptionalVoidMethods(
                "Lcom/iterable/iterableapi/d;",
                "p",
                "trackPushOpen",
                "updateEmail",
                "updateUser",
                "registerDeviceToken",
                "disableDeviceForCurrentUser"
            )
        }

        // Layer 7: Usercentrics Consent Telemetry Neutralization
        if (stripUsercentrics.value != false) {
            disableOptionalVoidMethods(
                "Lcom/groundspeak/geocaching/intro/permissions/usercentrics/a;",
                "j"
            )
        }
    }
}

private fun BytecodePatchContext.spoofAdvertisingId(
    classDescriptor: String = "Lcom/google/android/gms/ads/identifier/AdvertisingIdClient;"
) {
    val mutableClass = mutableClassDefByOrNull(classDescriptor) ?: return

    for (method in mutableClass.methods) {
        if (
            method.name == "getAdvertisingIdInfo" &&
            method.returnType == "Lcom/google/android/gms/ads/identifier/AdvertisingIdClient\$Info;" &&
            method.implementation != null
        ) {
            val firstInst = method.implementation?.instructions?.firstOrNull()
            if (firstInst?.opcode != com.android.tools.smali.dexlib2.Opcode.NEW_INSTANCE) {
                method.addInstructions(
                    0,
                    """
                    new-instance v0, Lcom/google/android/gms/ads/identifier/AdvertisingIdClient${'$'}Info;
                    const-string v1, "00000000-0000-0000-0000-000000000000"
                    const/4 v2, 0x1
                    invoke-direct {v0, v1, v2}, Lcom/google/android/gms/ads/identifier/AdvertisingIdClient${'$'}Info;-><init>(Ljava/lang/String;Z)V
                    return-object v0
                    """.trimIndent()
                )
            }
        }
    }
}
