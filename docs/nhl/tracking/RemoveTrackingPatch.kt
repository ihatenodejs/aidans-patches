package app.aidan.patches.nhl.tracking

import app.aidan.patches.nhl.shared.COMPATIBILITY_NHL
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
        disableVoidMethods(
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
        returnBoolean("Lcom/onetrust/otpublishers/headless/Public/OTPublishersHeadlessSDK;", "shouldShowBanner", false)
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
        disableVoidMethods("Lnv;", "a", "b")

        // 4. Firebase Analytics & AppMeasurement
        disableVoidMethods("Lcom/google/android/gms/measurement/AppMeasurement;", "logEventInternal")
        disableVoidMethods("Lcom/google/android/gms/measurement/api/AppMeasurementSdk;", "logEvent")
        disableVoidMethods("Lcom/google/firebase/analytics/FirebaseAnalytics;", "b", "c", "setCurrentScreen")
        disableVoidMethods("Lg3q;", "c")
        disableVoidMethods("Lhhd;", "logEvent", "setUserProperty")
        returnEmptyList("Lcom/google/firebase/analytics/connector/internal/AnalyticsConnectorRegistrar;", "getComponents")

        // 5. Firebase Performance Monitoring
        returnEmptyList("Lcom/google/firebase/perf/FirebasePerfRegistrar;", "getComponents")
        returnEmptyList("Lcom/google/firebase/perf/FirebasePerfKtxRegistrar;", "getComponents")
        disableVoidMethods("Lmak;", "b", "c", "d")
        disableVoidMethods("Lcom/google/firebase/perf/metrics/AppStartTrace;", "onActivityCreated", "onActivityResumed", "onActivityStarted")
        disableVoidMethods("Lcom/google/firebase/perf/session/SessionManager;", "initializeGaugeCollection", "updatePerfSession")

        // 6. UrbanAirship Telemetry & Analytics
        disableVoidMethods("Lcom/urbanairship/analytics/InstallReceiver;", "onReceive")
        disableVoidMethods("Lpo;", "a")
        disableVoidMethods("Lrt;", "e", "h")
        disableVoidMethods("Lqp;", "g", "h")

        // 7. Blaze SDK Analytics & Logging
        returnNullObject("Lcom/blaze/blazesdk/analytics/AnalyticsApi;", "sendTrackEvents")
        returnNullObject("Lcom/blaze/blazesdk/analytics/AnalyticsApi;", "sendDoNotTrackEvents")
        returnNullObject("Lcom/blaze/blazesdk/logger/apis/BlazeLoggerApi;", "sendLogs")

        // 8. Brightcove Video Analytics & Interactivity Metrics
        patchBrightcoveAnalytics()
        disableVoidMethods("Lcom/brightcove/player/analytics/DefaultAnalyticsHandler;", "onAnalyticsEvent")

        // 9. Ticketmaster Analytics
        returnBoolean("Lcom/ticketmaster/authenticationsdk/Analytics;", "getEnabled", false)
        disableVoidMethods("Lcom/ticketmaster/tickets/eventanalytic/AnalyticsQueue;", "putEvent", "send")
        disableVoidMethods("Lcom/ticketmaster/tickets/eventanalytic/TmxAnalyticsSender;", "send")

        // 10. Firebase Crashlytics & Sessions Telemetry
        disableVoidMethods("Lbn6;", "b", "c")
        returnEmptyList("Lcom/google/firebase/sessions/FirebaseSessionsRegistrar;", "getComponents")
        disableVoidMethods("Lcom/google/firebase/sessions/SessionLifecycleService;", "onCreate")
        returnNullObject("Lcom/google/firebase/sessions/SessionLifecycleService;", "onBind")
        returnInt("Lcom/google/firebase/sessions/SessionLifecycleService;", "onStartCommand", 2)

        // 11. Google DataTransport Telemetry Uploader
        disableVoidMethods("Lomk;", "a")
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
            return
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

    returnConstString(
        "Lcom/google/android/gms/ads/identifier/AdvertisingIdClient${'$'}Info;",
        "getId",
        ZEROED_ADVERTISING_ID
    )
    returnBoolean(
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
    val mutableClass = mutableClassDefByOrNull(classDescriptor) ?: return

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
        }
    }
}

/**
 * Suppresses Brightcove video analytics publishing and network requests.
 */
private fun BytecodePatchContext.patchBrightcoveAnalytics() {
    val clientClass = mutableClassDefByOrNull("Lcom/brightcove/player/analytics/AnalyticsClient;")
    if (clientClass != null) {
        for (method in clientClass.methods) {
            if (method.name == "publish" && method.implementation != null) {
                method.addInstructions(0, "return-object p0")
            }
        }
    }

    val analyticsClass = mutableClassDefByOrNull("Lcom/brightcove/player/analytics/Analytics;")
    if (analyticsClass != null) {
        for (method in analyticsClass.methods) {
            if (method.name == "sendAnalyticsRequest" && method.implementation != null) {
                method.addInstructions(0, "return-object p3")
            }
        }
    }
}

/**
 * Neutralizes all matched void-returning methods by injecting an immediate `return-void` at index 0.
 */
private fun BytecodePatchContext.disableVoidMethods(
    classDescriptor: String,
    vararg methodNames: String
) {
    val mutableClass = mutableClassDefByOrNull(classDescriptor) ?: return
    val methodSet = methodNames.toSet()

    for (method in mutableClass.methods) {
        if (method.name in methodSet && method.returnType == "V" && method.implementation != null) {
            method.addInstructions(0, "return-void")
        }
    }
}

/**
 * Stubs a method to immediately return a constant boolean value.
 */
private fun BytecodePatchContext.returnBoolean(
    classDescriptor: String,
    methodName: String,
    value: Boolean
) {
    val mutableClass = mutableClassDefByOrNull(classDescriptor) ?: return

    for (method in mutableClass.methods) {
        if (method.name == methodName && method.returnType == "Z" && method.implementation != null) {
            val constInstruction = if (value) "const/4 v0, 0x1" else "const/4 v0, 0x0"
            method.addInstructions(
                0,
                """
                $constInstruction
                return v0
                """.trimIndent()
            )
        }
    }
}

/**
 * Stubs matching integer-returning methods to immediately return [value].
 */
private fun BytecodePatchContext.returnInt(
    classDescriptor: String,
    methodName: String,
    value: Int
) {
    val mutableClass = mutableClassDefByOrNull(classDescriptor) ?: return

    for (method in mutableClass.methods) {
        if (method.name == methodName && method.returnType == "I" && method.implementation != null) {
            method.addInstructions(
                0,
                """
                const/4 v0, $value
                return v0
                """.trimIndent()
            )
        }
    }
}

/**
 * Stubs a method to immediately return a constant string.
 */
private fun BytecodePatchContext.returnConstString(
    classDescriptor: String,
    methodName: String,
    value: String
) {
    val mutableClass = mutableClassDefByOrNull(classDescriptor) ?: return

    for (method in mutableClass.methods) {
        if (method.name == methodName && method.returnType == "Ljava/lang/String;" && method.implementation != null) {
            method.addInstructions(
                0,
                """
                const-string v0, "$value"
                return-object v0
                """.trimIndent()
            )
        }
    }
}

/**
 * Stubs an object-returning method to immediately return null.
 */
private fun BytecodePatchContext.returnNullObject(
    classDescriptor: String,
    methodName: String
) {
    val mutableClass = mutableClassDefByOrNull(classDescriptor) ?: return

    for (method in mutableClass.methods) {
        if (method.name.startsWith(methodName) && method.returnType.startsWith("L") && method.implementation != null) {
            method.addInstructions(
                0,
                """
                const/4 v0, 0x0
                return-object v0
                """.trimIndent()
            )
        }
    }
}

/**
 * Stubs a List-returning method to immediately return an empty list.
 */
private fun BytecodePatchContext.returnEmptyList(
    classDescriptor: String,
    methodName: String
) {
    val mutableClass = mutableClassDefByOrNull(classDescriptor) ?: return

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
        }
    }
}
