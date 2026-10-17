package app.aidan.patches.nhl.tracking

import app.aidan.patches.nhl.shared.COMPATIBILITY_NHL
import app.aidan.patches.shared.disableOptionalVoidMethods
import app.aidan.patches.shared.disableRequiredVoidMethods
import app.aidan.patches.shared.returnOptionalBoolean
import app.aidan.patches.shared.returnOptionalString
import app.aidan.patches.shared.returnRequiredBoolean
import app.aidan.patches.shared.returnRequiredNullObject
import app.morphe.patcher.extensions.InstructionExtensions.addInstructions
import app.morphe.patcher.extensions.InstructionExtensions.removeInstruction
import app.morphe.patcher.patch.BytecodePatchContext
import app.morphe.patcher.patch.PatchException
import app.morphe.patcher.patch.booleanOption
import app.morphe.patcher.patch.bytecodePatch
import com.android.tools.smali.dexlib2.Opcode
import com.android.tools.smali.dexlib2.iface.instruction.NarrowLiteralInstruction
import com.android.tools.smali.dexlib2.iface.instruction.ReferenceInstruction
import com.android.tools.smali.dexlib2.iface.reference.MethodReference

private const val ZEROED_ADVERTISING_ID = "00000000-0000-0000-0000-000000000000"

@Suppress("unused")
val removeTrackingPatch = bytecodePatch(
    name = "Remove Tracking",
    description = "Removes the OneTrust CMP tracking consent popup, zeros Google Play Advertising ID (AAID), blocks host application analytics, disables Firebase Analytics, AppMeasurement, Firebase Performance Monitoring, Crashlytics, and Sessions telemetry, neutralizes UrbanAirship and Blaze tracking and logging, suppresses Brightcove video metrics reporting, and blocks Ticketmaster telemetry.",
    default = true
) {
    category("Privacy")
    compatibleWith(COMPATIBILITY_NHL)

    val removeBrokenPrivacySettings = booleanOption(
        key = "removeBrokenPrivacySettings",
        title = "Remove Broken Privacy Settings",
        description = "Removes the broken Privacy Settings entry from the Settings screen.",
        default = true
    )

    execute {
        if (removeBrokenPrivacySettings.value == true) {
            removeBrokenPrivacySettingsEntry()
        }

        // 1. OneTrust Consent Management Platform (CMP) & Tracking Banner
        disableRequiredVoidMethods(
            "Lcom/onetrust/otpublishers/headless/Public/OTPublishersHeadlessSDK;",
            "setupUI",
            "showBannerUI",
            "callSetupUI",
            "callShowConsentPreferencesUI",
            "callShowPreferenceCenterUI",
            "showPreferenceCenterUI",
            "showConsentPurposesUI",
            "showConsentUI",
            "startOtSdkNetworkCall",
            "startSDK",
            "saveConsent",
            "saveAllowAll",
            "saveRejectAll"
        )
        returnRequiredBoolean("Lcom/onetrust/otpublishers/headless/Public/OTPublishersHeadlessSDK;", "shouldShowBanner", false)
        returnInt("Lcom/onetrust/otpublishers/headless/Public/OTPublishersHeadlessSDK;", "isBannerShown", 0)
        returnInt("Lcom/onetrust/otpublishers/headless/Public/OTPublishersHeadlessSDK;", "getConsentStatusForGroupId", 0)
        returnInt("Lcom/onetrust/otpublishers/headless/Public/OTPublishersHeadlessSDK;", "getConsentStatusForSDKId", 0)
        returnInt("Lcom/onetrust/otpublishers/headless/Public/OTPublishersHeadlessSDK;", "getPurposeConsentLocal", 0)
        returnInt("Lcom/onetrust/otpublishers/headless/Public/OTPublishersHeadlessSDK;", "getPurposeLegitInterestLocal", 0)
        returnInt("Lcom/onetrust/otpublishers/headless/Public/OTPublishersHeadlessSDK;", "getUCPurposeConsent", 0)

        // 2. Google Play Advertising ID (AAID) spoofing
        spoofAdvertisingId()
        spoofMinifiedAdvertisingId("Ljj;", "Lij;")

        // 3. Host App Internal Analytics Engine
        disableRequiredVoidMethods("Lnv;", "a", "b")

        // 4. Firebase Analytics & AppMeasurement
        disableRequiredVoidMethods("Lcom/google/android/gms/measurement/AppMeasurement;", "logEventInternal")
        disableRequiredVoidMethods("Lcom/google/android/gms/measurement/api/AppMeasurementSdk;", "logEvent")
        disableRequiredVoidMethods("Lcom/google/firebase/analytics/FirebaseAnalytics;", "b", "c", "setCurrentScreen")
        disableRequiredVoidMethods("Lg3q;", "c")
        disableRequiredVoidMethods("Lhhd;", "logEvent", "setUserProperty")
        returnEmptyList("Lcom/google/firebase/analytics/connector/internal/AnalyticsConnectorRegistrar;", "getComponents")

        // 5. Firebase Performance Monitoring
        returnEmptyList("Lcom/google/firebase/perf/FirebasePerfRegistrar;", "getComponents")
        returnEmptyList("Lcom/google/firebase/perf/FirebasePerfKtxRegistrar;", "getComponents")
        disableRequiredVoidMethods("Lmak;", "b", "c", "d")
        disableRequiredVoidMethods("Lcom/google/firebase/perf/metrics/AppStartTrace;", "onActivityCreated", "onActivityResumed", "onActivityStarted")
        disableRequiredVoidMethods("Lcom/google/firebase/perf/session/SessionManager;", "initializeGaugeCollection", "updatePerfSession")

        // 6. UrbanAirship Telemetry & Analytics
        disableRequiredVoidMethods("Lcom/urbanairship/analytics/InstallReceiver;", "onReceive")
        disableRequiredVoidMethods("Lpo;", "a")
        disableRequiredVoidMethods("Lrt;", "e", "h")
        disableRequiredVoidMethods("Lqp;", "g", "h")

        // 7. Blaze SDK Analytics & Logging
        returnRequiredNullObjectIncludingDefault("Lcom/blaze/blazesdk/analytics/AnalyticsApi;", "sendTrackEvents")
        returnRequiredNullObjectIncludingDefault("Lcom/blaze/blazesdk/analytics/AnalyticsApi;", "sendDoNotTrackEvents")
        returnRequiredNullObjectIncludingDefault("Lcom/blaze/blazesdk/logger/apis/BlazeLoggerApi;", "sendLogs", optional = true)

        // 8. Brightcove Video Analytics & Interactivity Metrics
        patchBrightcoveAnalytics()
        disableRequiredVoidMethods("Lcom/brightcove/player/analytics/DefaultAnalyticsHandler;", "onAnalyticsEvent")

        // 9. Ticketmaster Analytics
        returnRequiredBoolean("Lcom/ticketmaster/authenticationsdk/Analytics;", "getEnabled", false)
        disableRequiredVoidMethods("Lcom/ticketmaster/tickets/eventanalytic/AnalyticsQueue;", "putEvent", "send")
        disableRequiredVoidMethods("Lcom/ticketmaster/tickets/eventanalytic/TmxAnalyticsSender;", "send")

        // 10. Firebase Crashlytics & Sessions Telemetry
        disableRequiredVoidMethods("Lbn6;", "b", "c")
        returnEmptyList("Lcom/google/firebase/sessions/FirebaseSessionsRegistrar;", "getComponents")
        disableOptionalVoidMethods("Lcom/google/firebase/sessions/SessionLifecycleService;", "onCreate")
        returnRequiredNullObject("Lcom/google/firebase/sessions/SessionLifecycleService;", "onBind")
        returnInt("Lcom/google/firebase/sessions/SessionLifecycleService;", "onStartCommand", 2, optional = true)

        // 11. Google DataTransport Telemetry Uploader
        disableRequiredVoidMethods("Lomk;", "a")
    }
}

/**
 * Removes the Privacy Settings row from the Settings Preferences section.
 */
private fun BytecodePatchContext.removeBrokenPrivacySettingsEntry() {
    val settingsPreferencesClass = mutableClassDefByOrNull("Lbg8;")
        ?: throw PatchException("Missing required class Lbg8; (Settings Preferences builder)")

    val dMethod = settingsPreferencesClass.methods.singleOrNull { method ->
        method.name == "d" &&
            method.parameterTypes == listOf("Z", "Ljava/lang/String;", "Lkotlin/jvm/functions/Function1;", "Lpn3;", "I") &&
            method.returnType == "V"
    } ?: throw PatchException("Missing required method Lbg8.d(ZLjava/lang/String;Lkotlin/jvm/functions/Function1;Lpn3;I)V")

    val implementation = dMethod.implementation
        ?: throw PatchException("Lbg8.d has no implementation")

    val instructions = implementation.instructions.toList()
    val constIndex = instructions.indexOfFirst { instruction ->
        instruction.opcode == Opcode.CONST &&
            (instruction as? NarrowLiteralInstruction)?.narrowLiteral == -0x3635e702
    }
    if (constIndex == -1) {
        throw PatchException("Could not find Privacy Settings composable key anchor in Lbg8.d")
    }

    val relativeAddIndex = instructions.subList(constIndex, instructions.size).indexOfFirst { instruction ->
        val ref = (instruction as? ReferenceInstruction)?.reference as? MethodReference
        instruction.opcode == Opcode.INVOKE_INTERFACE &&
            ref?.definingClass == "Ljava/util/List;" &&
            ref.name == "add"
    }
    if (relativeAddIndex == -1) {
        val buildIndex = instructions.subList(constIndex, instructions.size).indexOfFirst { instruction ->
            val ref = (instruction as? ReferenceInstruction)?.reference as? MethodReference
            instruction.opcode == Opcode.INVOKE_STATIC &&
                ref?.definingClass == "Lkotlin/collections/CollectionsKt;" &&
                ref.name == "build"
        }
        if (buildIndex != -1) {
            throw PatchException("Found CollectionsKt.build but could not find List.add invocation for Privacy Settings in Lbg8.d")
        }
        throw PatchException("Could not find List.add invocation for Privacy Settings in Lbg8.d")
    }

    val targetIndex = constIndex + relativeAddIndex
    dMethod.removeInstruction(targetIndex)
}

/**
 * Spoofs unminified AdvertisingIdClient if present.
 */
private fun BytecodePatchContext.spoofAdvertisingId() {
    val aaidClass = mutableClassDefByOrNull("Lcom/google/android/gms/ads/identifier/AdvertisingIdClient;")
    if (aaidClass != null) {
        for (method in aaidClass.methods) {
            if (method.name == "getAdvertisingIdInfo" && method.implementation != null) {
                method.addInstructions(
                    0,
                    """
                    new-instance v0, Lcom/google/android/gms/ads/identifier/AdvertisingIdClient${'$'}Info;
                    const-string v1, "$ZEROED_ADVERTISING_ID"
                    const/4 v2, 0x1
                    invoke-direct {v0, v1, v2}, Lcom/google/android/gms/ads/identifier/AdvertisingIdClient${'$'}Info;-><init>(Ljava/lang/String;Z)V
                    return-object v0
                    """.trimIndent()
                )
            }
        }
    }

    returnOptionalString(
        "Lcom/google/android/gms/ads/identifier/AdvertisingIdClient${'$'}Info;",
        "getId",
        ZEROED_ADVERTISING_ID
    )
    returnOptionalBoolean(
        "Lcom/google/android/gms/ads/identifier/AdvertisingIdClient${'$'}Info;",
        "isLimitAdTrackingEnabled",
        true
    )
}

/**
 * Spoofs minified AdvertisingIdClient methods returning the minified Info class.
 */
private fun BytecodePatchContext.spoofMinifiedAdvertisingId(
    classDescriptor: String,
    infoClassDescriptor: String
) {
    val mutableClass = mutableClassDefByOrNull(classDescriptor)
        ?: throw PatchException("Missing required class $classDescriptor (minified AdvertisingIdClient)")

    var patched = false
    for (method in mutableClass.methods) {
        if ((method.name == "a" || method.name == "f") &&
            method.returnType == infoClassDescriptor &&
            method.implementation != null
        ) {
            method.addInstructions(
                0,
                """
                new-instance v0, $infoClassDescriptor
                const-string v1, "$ZEROED_ADVERTISING_ID"
                const/4 v2, 0x1
                invoke-direct {v0, v1, v2}, $infoClassDescriptor-><init>(Ljava/lang/String;Z)V
                return-object v0
                """.trimIndent()
            )
            patched = true
        }
    }

    if (!patched) {
        throw PatchException("Missing required minified AdvertisingIdClient method in $classDescriptor")
    }
}

/**
 * Suppresses Brightcove video analytics publishing and network requests.
 */
private fun BytecodePatchContext.patchBrightcoveAnalytics() {
    val clientClass = mutableClassDefByOrNull("Lcom/brightcove/player/analytics/AnalyticsClient;")
        ?: throw PatchException("Missing required class Lcom/brightcove/player/analytics/AnalyticsClient;")
    var publishPatched = false
    for (method in clientClass.methods) {
        if (method.name == "publish" && method.implementation != null) {
            method.addInstructions(0, "return-object p0")
            publishPatched = true
        }
    }
    if (!publishPatched) {
        throw PatchException("Missing required method Lcom/brightcove/player/analytics/AnalyticsClient;->publish")
    }

    val analyticsClass = mutableClassDefByOrNull("Lcom/brightcove/player/analytics/Analytics;")
        ?: throw PatchException("Missing required class Lcom/brightcove/player/analytics/Analytics;")
    var sendRequestPatched = false
    for (method in analyticsClass.methods) {
        if (method.name == "sendAnalyticsRequest" && method.implementation != null) {
            method.addInstructions(0, "return-object p3")
            sendRequestPatched = true
        }
    }
    if (!sendRequestPatched) {
        throw PatchException("Missing required method Lcom/brightcove/player/analytics/Analytics;->sendAnalyticsRequest")
    }
}

/**
 * Stubs matching integer-returning methods to immediately return [value].
 */
private fun BytecodePatchContext.returnInt(
    classDescriptor: String,
    methodName: String,
    value: Int,
    optional: Boolean = false
) {
    val mutableClass = mutableClassDefByOrNull(classDescriptor)
    if (mutableClass == null) {
        if (optional) return
        throw PatchException("Missing required class $classDescriptor")
    }
    var patched = false

    for (method in mutableClass.methods) {
        if (method.name == methodName && method.returnType == "I" && method.implementation != null) {
            method.addInstructions(
                0,
                """
                const/4 v0, $value
                return v0
                """.trimIndent()
            )
            patched = true
        }
    }

    if (!optional && !patched) {
        throw PatchException("Missing required method $classDescriptor->$methodName()I")
    }
}

/**
 * Stubs matching object-returning methods, including Kotlin's `$default` overload, to return null.
 */
private fun BytecodePatchContext.returnRequiredNullObjectIncludingDefault(
    classDescriptor: String,
    methodName: String,
    optional: Boolean = false
) {
    val mutableClass = mutableClassDefByOrNull(classDescriptor)
    if (mutableClass == null) {
        if (optional) return
        throw PatchException("Missing required class $classDescriptor")
    }
    var patched = false

    for (method in mutableClass.methods) {
        val isTargetMethod = method.name == methodName || method.name == "$methodName\$default"
        if (isTargetMethod && method.returnType.startsWith("L") && method.implementation != null) {
            method.addInstructions(
                0,
                """
                const/4 v0, 0x0
                return-object v0
                """.trimIndent()
            )
            patched = true
        }
    }

    if (!optional && !patched) {
        throw PatchException("Missing required method $classDescriptor->$methodName")
    }
}

/**
 * Stubs a List-returning method to immediately return an empty list.
 */
private fun BytecodePatchContext.returnEmptyList(
    classDescriptor: String,
    methodName: String,
    optional: Boolean = false
) {
    val mutableClass = mutableClassDefByOrNull(classDescriptor)
    if (mutableClass == null) {
        if (optional) return
        throw PatchException("Missing required class $classDescriptor")
    }
    var patched = false

    for (method in mutableClass.methods) {
        if (method.name == methodName && method.returnType == "Ljava/util/List;" && method.implementation != null) {
            method.addInstructions(
                0,
                """
                invoke-static {}, Ljava/util/Collections;->emptyList()Ljava/util/List;
                move-result-object v0
                return-object v0
                """.trimIndent()
            )
            patched = true
        }
    }

    if (!optional && !patched) {
        throw PatchException("Missing required method $classDescriptor->$methodName()Ljava/util/List;")
    }
}
