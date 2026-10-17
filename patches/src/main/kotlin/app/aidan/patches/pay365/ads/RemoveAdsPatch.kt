package app.aidan.patches.pay365.ads

import app.aidan.patches.pay365.shared.COMPATIBILITY_365_PAY
import app.aidan.patches.shared.disableOptionalVoidMethods
import app.morphe.patcher.extensions.InstructionExtensions.addInstructions
import app.morphe.patcher.patch.BytecodePatchContext
import app.morphe.patcher.patch.booleanOption
import app.morphe.patcher.patch.bytecodePatch
import com.android.tools.smali.dexlib2.Opcode

private const val ZEROED_ADVERTISING_ID = "00000000-0000-0000-0000-000000000000"

@Suppress("unused")
val removeAdsPatch = bytecodePatch(
    name = "Remove Ads",
    description = "Removes in-app promotional message campaigns and popups (OneSignal In-App Messages), neutralizes in-app review and rating prompts, and zeroes Google Play and Huawei Advertising IDs.",
    default = true
) {
    category("Ads")
    compatibleWith(COMPATIBILITY_365_PAY)

    val removeInAppMessages = booleanOption(
        key = "removeInAppMessages",
        default = true,
        title = "Remove In-App Messages",
        description = "Suppresses OneSignal promotional campaigns and marketing message displays."
    )

    val removeReviewPrompts = booleanOption(
        key = "removeReviewPrompts",
        default = true,
        title = "Remove Review Prompts",
        description = "Neutralizes Google Play in-app review and rating prompts."
    )

    val zeroAdvertisingId = booleanOption(
        key = "zeroAdvertisingId",
        default = true,
        title = "Zero Advertising ID",
        description = "Spoofs Google Play and Huawei Advertising ID (AAID) with zeroes and forces limit ad tracking enabled."
    )

    execute {
        if (removeReviewPrompts.value != false) {
            resolvePromiseFalse(
                "Lcom/ibits/react_native_in_app_review/AppReviewModule;",
                "show",
                "showInAppCommentHMS"
            )
        }

        if (removeInAppMessages.value != false) {
            suppressInAppMessages()
        }

        if (zeroAdvertisingId.value != false) {
            spoofAdvertisingId()
        }
    }
}

private fun BytecodePatchContext.suppressInAppMessages() {
    val displayer = mutableClassDefByOrNull("Lcom/onesignal/inAppMessages/internal/display/impl/InAppDisplayer;")
    if (displayer != null) {
        for (method in displayer.methods) {
            if ((method.name == "displayMessage" || method.name == "displayPreviewMessage") && method.implementation != null) {
                method.addInstructions(
                    0,
                    """
                    const/4 v0, 0x0
                    invoke-static {v0}, Ljava/lang/Boolean;->valueOf(Z)Ljava/lang/Boolean;
                    move-result-object v0
                    return-object v0
                    """.trimIndent()
                )
            }
        }
    }

    val manager = mutableClassDefByOrNull("Lcom/onesignal/inAppMessages/internal/InAppMessagesManager;")
    if (manager != null) {
        for (method in manager.methods) {
            if (method.name == "getPaused" && method.returnType == "Z" && method.implementation != null) {
                method.addInstructions(
                    0,
                    """
                    const/4 v0, 0x1
                    return v0
                    """.trimIndent()
                )
            }
            if (method.name == "start" && method.returnType == "V" && method.implementation != null) {
                val firstInst = method.implementation?.instructions?.firstOrNull()
                if (firstInst?.opcode != Opcode.RETURN_VOID) {
                    method.addInstructions(0, "return-void")
                }
            }
        }
    }

    disableOptionalVoidMethods(
        "Lcom/onesignal/rnonesignalandroid/RNOneSignal;",
        "addInAppMessageClickListener",
        "addInAppMessagesLifecycleListener"
    )
}

private fun BytecodePatchContext.resolvePromiseFalse(
    classDescriptor: String,
    vararg methodNames: String
) {
    val mutableClass = mutableClassDefByOrNull(classDescriptor) ?: return
    val methodSet = methodNames.toSet()
    for (method in mutableClass.methods) {
        if (method.name in methodSet && method.returnType == "V" && method.implementation != null) {
            val params = method.parameterTypes
            val promiseIndex = params.indexOf("Lcom/facebook/react/bridge/Promise;")
            if (promiseIndex != -1) {
                val promiseReg = "p${promiseIndex + 1}"
                method.addInstructions(
                    0,
                    """
                    sget-object v0, Ljava/lang/Boolean;->FALSE:Ljava/lang/Boolean;
                    invoke-interface {$promiseReg, v0}, Lcom/facebook/react/bridge/Promise;->resolve(Ljava/lang/Object;)V
                    return-void
                    """.trimIndent()
                )
            }
        }
    }
}

private fun BytecodePatchContext.spoofAdvertisingId() {
    // 1. Minified AAID client: Lb6/a;
    val minifiedClient = mutableClassDefByOrNull("Lb6/a;")
    if (minifiedClient != null) {
        for (method in minifiedClient.methods) {
            if (method.name == "a" && method.parameterTypes == listOf("Landroid/content/Context;") && method.returnType == "Lb6/a\$a;" && method.implementation != null) {
                method.addInstructions(
                    0,
                    """
                    new-instance v0, Lb6/a${'$'}a;
                    const-string v1, "$ZEROED_ADVERTISING_ID"
                    const/4 v2, 0x1
                    invoke-direct {v0, v1, v2}, Lb6/a${'$'}a;-><init>(Ljava/lang/String;Z)V
                    return-object v0
                    """.trimIndent()
                )
            }
        }
    }

    val minifiedInfo = mutableClassDefByOrNull("Lb6/a\$a;")
    if (minifiedInfo != null) {
        for (method in minifiedInfo.methods) {
            if (method.name == "a" && method.parameterTypes.isEmpty() && method.returnType == "Ljava/lang/String;" && method.implementation != null) {
                method.addInstructions(
                    0,
                    """
                    const-string v0, "$ZEROED_ADVERTISING_ID"
                    return-object v0
                    """.trimIndent()
                )
            }
            if (method.name == "b" && method.parameterTypes.isEmpty() && method.returnType == "Z" && method.implementation != null) {
                method.addInstructions(
                    0,
                    """
                    const/4 v0, 0x1
                    return v0
                    """.trimIndent()
                )
            }
        }
    }

    // 2. Standard GMS AdvertisingIdClient
    val gmsClient = mutableClassDefByOrNull("Lcom/google/android/gms/ads/identifier/AdvertisingIdClient;")
    if (gmsClient != null) {
        for (method in gmsClient.methods) {
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

    val gmsInfo = mutableClassDefByOrNull("Lcom/google/android/gms/ads/identifier/AdvertisingIdClient\$Info;")
    if (gmsInfo != null) {
        for (method in gmsInfo.methods) {
            if (method.name == "getId" && method.implementation != null) {
                method.addInstructions(0, "const-string v0, \"$ZEROED_ADVERTISING_ID\"\nreturn-object v0")
            }
            if (method.name == "isLimitAdTrackingEnabled" && method.implementation != null) {
                method.addInstructions(0, "const/4 v0, 0x1\nreturn v0")
            }
        }
    }

    // 3. Huawei HMS AdvertisingIdClient
    val hmsClient = mutableClassDefByOrNull("Lcom/huawei/hms/ads/identifier/AdvertisingIdClient;")
    if (hmsClient != null) {
        for (method in hmsClient.methods) {
            if (method.name == "getAdvertisingIdInfo" && method.implementation != null) {
                method.addInstructions(
                    0,
                    """
                    new-instance v0, Lcom/huawei/hms/ads/identifier/AdvertisingIdClient${'$'}Info;
                    const-string v1, "$ZEROED_ADVERTISING_ID"
                    const/4 v2, 0x1
                    invoke-direct {v0, v1, v2}, Lcom/huawei/hms/ads/identifier/AdvertisingIdClient${'$'}Info;-><init>(Ljava/lang/String;Z)V
                    return-object v0
                    """.trimIndent()
                )
            }
        }
    }

    val hmsInfo = mutableClassDefByOrNull("Lcom/huawei/hms/ads/identifier/AdvertisingIdClient\$Info;")
    if (hmsInfo != null) {
        for (method in hmsInfo.methods) {
            if (method.name == "getId" && method.implementation != null) {
                method.addInstructions(0, "const-string v0, \"$ZEROED_ADVERTISING_ID\"\nreturn-object v0")
            }
            if (method.name == "isLimitAdTrackingEnabled" && method.implementation != null) {
                method.addInstructions(0, "const/4 v0, 0x1\nreturn v0")
            }
        }
    }
}
