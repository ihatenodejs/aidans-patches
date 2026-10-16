package app.aidan.patches.sidelineswap.tracking

import app.aidan.patches.shared.disableOptionalVoidMethods
import app.aidan.patches.shared.returnOptionalBoolean
import app.aidan.patches.shared.returnOptionalNullObject
import app.aidan.patches.shared.returnOptionalString
import app.aidan.patches.sidelineswap.shared.COMPATIBILITY_SIDELINESWAP
import app.morphe.patcher.extensions.InstructionExtensions.addInstructions
import app.morphe.patcher.patch.BytecodePatchContext
import app.morphe.patcher.patch.bytecodePatch
import com.android.tools.smali.dexlib2.AccessFlags

val blockTrackingAndTelemetryPatch = bytecodePatch(
    name = "Block Tracking and Telemetry",
    description = "Neutralizes first-party analytics (SidelineSwap backend), behavioral tracking (Amplitude, Firebase Analytics, Facebook App Events, Iterable), diagnostic telemetry (Firebase Crashlytics, Timber logging tree), payment gateway telemetry (Braintree FPTI), and zeros the Google Play Advertising ID (AAID).",
    default = true
) {
    category("Privacy")
    compatibleWith(COMPATIBILITY_SIDELINESWAP)

    execute {
        // Layer 1: Central Dispatcher Neutralization
        // Cuts off all event broadcasting across the application.
        disableOptionalVoidMethods("Lcom/sidelineswap/android/analytics/AnalyticsLogger;", "delegateToClients")
        returnThis("Lcom/sidelineswap/android/analytics/AnalyticsLogger;", "addClient")

        // Layer 2: First-Party Backend Telemetry Neutralization
        disableOptionalVoidMethods(
            "Lcom/sidelineswap/android/analytics/SidelineSwapClient;",
            "logEvent",
            "access\$logEvent"
        )
        returnOptionalNullObject("Lcom/sidelineswap/android/repo/LogRepo;", "trackEvent")

        // Layer 3: Amplitude Analytics Neutralization
        disableOptionalVoidMethods(
            "Lcom/sidelineswap/android/analytics/AmplitudeClient;",
            "logEvent",
            "access\$logEvent"
        )
        disableOptionalVoidMethods("Lp073j1/d;", "d")

        // Layer 4: Firebase Analytics & Measurement Neutralization
        disableOptionalVoidMethods(
            "Lcom/sidelineswap/android/analytics/FirebaseClient;",
            "logEvent",
            "access\$logEvent"
        )
        disableOptionalVoidMethods(
            "Lcom/google/firebase/analytics/FirebaseAnalytics;",
            "logEvent",
            "setAnalyticsCollectionEnabled",
            "setUserProperty",
            "setDefaultEventParameters"
        )
        disableOptionalVoidMethods(
            "Lcom/google/firebase/perf/FirebasePerformance;",
            "setPerformanceCollectionEnabled"
        )

        // Layer 5: Firebase Crashlytics & Timber Tree Neutralization
        disableOptionalVoidMethods("Lcom/sidelineswap/android/log/CrashlyticsTree;", "log")
        disableOptionalVoidMethods(
            "Lcom/google/firebase/crashlytics/FirebaseCrashlytics;",
            "log",
            "recordException",
            "setCrashlyticsCollectionEnabled"
        )

        // Layer 6: Facebook SDK & App Events Neutralization
        // Stub Facebook validation and event logging methods
        disableOptionalVoidMethods(
            "Lp173v2/y;",
            "e",
            "f",
            "g",
            "h",
            "i",
            "j",
            "k",
            "l"
        )
        disableOptionalVoidMethods(
            "Lcom/sidelineswap/android/analytics/FacebookClient;",
            "completedCheckout",
            "initiatedCheckout",
            "joined",
            "newMadeOffer",
            "newMadePurchase",
            "visitedItem",
            "visitedResults"
        )
        disableOptionalVoidMethods("Lp057h2/h;", "a")

        // Layer 7: Iterable Telemetry Neutralization
        disableOptionalVoidMethods("Lcom/sidelineswap/android/analytics/IterableClient;", "visitedLocker")
        disableOptionalVoidMethods("Lp159t5/C1123h;", "d", "e", "f", "g")

        // Layer 8: Braintree FPTI Gateway Telemetry Neutralization
        returnWorkerSuccess("Lcom/braintreepayments/api/AnalyticsUploadWorker;", "g")
        returnWorkerSuccess("Lcom/braintreepayments/api/AnalyticsWriteToDbWorker;", "g")
        disableOptionalVoidMethods("Lcom/braintreepayments/api/L;", "b", "c")

        // Layer 9: Google Play Advertising ID Zeroing & Opt-Out
        spoofMinifiedAdvertisingId("LY2/a;")
        returnOptionalString(
            "Lcom/google/android/gms/ads/identifier/AdvertisingIdClient\$Info;",
            "getId",
            "00000000-0000-0000-0000-000000000000"
        )
        returnOptionalBoolean(
            "Lcom/google/android/gms/ads/identifier/AdvertisingIdClient\$Info;",
            "isLimitAdTrackingEnabled",
            true
        )
    }
}

/**
 * Makes implemented non-static overloads with [methodName] whose return type matches
 * [classDescriptor] return their receiver (`this`). Parameter register widths are taken
 * into account (long and double count as two registers). Absent classes or non-matching
 * overloads are skipped.
 */
private fun BytecodePatchContext.returnThis(
    classDescriptor: String,
    methodName: String
) {
    val mutableClass = mutableClassDefByOrNull(classDescriptor) ?: return

    for (method in mutableClass.methods) {
        if (
            method.name == methodName &&
            method.returnType == classDescriptor &&
            method.implementation != null &&
            !AccessFlags.STATIC.isSet(method.accessFlags)
        ) {
            val registerCount = method.implementation!!.registerCount
            val paramRegisterCount = method.parameterTypes.sumOf { type ->
                if (type == "J" || type == "D") 2 else 1
            }
            val thisRegister = registerCount - 1 - paramRegisterCount
            method.addInstructions(
                0,
                """
                return-object v$thisRegister
                """.trimIndent()
            )
        }
    }
}

/**
 * Makes implemented, parameterless overloads with [methodName] and return type
 * `Landroidx/work/ListenableWorker$a;` return a new WorkManager success result
 * without running their original bodies. Absent classes or non-matching overloads are skipped.
 */
private fun BytecodePatchContext.returnWorkerSuccess(
    classDescriptor: String,
    methodName: String = "g"
) {
    val mutableClass = mutableClassDefByOrNull(classDescriptor) ?: return

    for (method in mutableClass.methods) {
        if (
            method.name == methodName &&
            method.returnType == "Landroidx/work/ListenableWorker\$a;" &&
            method.parameterTypes.isEmpty() &&
            method.implementation != null
        ) {
            method.addInstructions(
                0,
                """
                new-instance v0, Landroidx/work/ListenableWorker${'$'}a${'$'}c;
                invoke-direct {v0}, Landroidx/work/ListenableWorker${'$'}a${'$'}c;-><init>()V
                return-object v0
                """.trimIndent()
            )
        }
    }
}

/**
 * Makes implemented one-parameter methods named a return an AdvertisingIdClient
 * info object with a zeroed AAID and limit-ad-tracking enabled. Absent classes or
 * matches are skipped; parameter and return types are not checked.
 */
private fun BytecodePatchContext.spoofMinifiedAdvertisingId(
    classDescriptor: String = "LY2/a;"
) {
    val mutableClass = mutableClassDefByOrNull(classDescriptor) ?: return

    for (method in mutableClass.methods) {
        if (method.name == "a" && method.implementation != null && method.parameterTypes.size == 1) {
            method.addInstructions(
                0,
                """
                new-instance v0, LY2/a${'$'}a;
                const-string v1, "00000000-0000-0000-0000-000000000000"
                const/4 v2, 0x1
                invoke-direct {v0, v1, v2}, LY2/a${'$'}a;-><init>(Ljava/lang/String;Z)V
                return-object v0
                """.trimIndent()
            )
        }
    }
}
