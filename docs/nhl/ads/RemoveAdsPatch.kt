package app.aidan.patches.nhl.ads

import app.aidan.patches.nhl.shared.COMPATIBILITY_NHL
import app.morphe.patcher.extensions.InstructionExtensions.addInstructions
import app.morphe.patcher.patch.BytecodePatchContext
import app.morphe.patcher.patch.PatchException
import app.morphe.patcher.patch.booleanOption
import app.morphe.patcher.patch.bytecodePatch

@Suppress("unused")
val removeAdsPatch = bytecodePatch(
    name = "Remove Ads",
    description = "Removes Google Ad Manager banner and native advertisements, disables Blaze story/moment sponsored placements, blocks Google Interactive Media Ads (IMA) video ad requests, and neutralizes Brightcove IMA mid-roll and pre-roll ad playback.",
    default = true
) {
    category("Ads")
    compatibleWith(COMPATIBILITY_NHL)

    val removePromotionsCarousel = booleanOption(
        key = "removePromotionsCarousel",
        title = "Remove Promotions Carousel",
        description = "Removes the promotions carousel from the Home screen.",
        default = true
    )

    execute {
        // 0. Promotions Carousel Neutralization
        if (removePromotionsCarousel.value == true) {
            removePromotionsCarousel()
        }

        // 1. Google Mobile Ads (GMS) SDK initialization & ContentProvider
        disableVoidMethods("Lcom/google/android/gms/ads/MobileAds;", "a")
        disableVoidMethods("Lcom/google/android/gms/ads/MobileAdsInitProvider;", "attachInfo")

        // 2. BaseAdView (kh1) loadAd neutralization
        disableVoidMethods("Lkh1;", "b")

        // 3. Blaze GAM Banner & Native Ad Handlers
        disableVoidMethods(
            "Lcom/blaze/gam/BlazeGAM;",
            "enableBannerAds",
            "enableCustomNativeAds",
            "initializeMobileAdsSdk",
            "setBlazeAdBannerHandler"
        )
        patchBlazeBannerCreation()
        returnNullObject("Lcom/blaze/gam/custom_native/DefaultBlazeGoogleCustomNativeAdsHandler;", "provideAd")
        disableVoidMethods("Lcom/blaze/gam/custom_native/DefaultBlazeGoogleCustomNativeAdsHandler;", "onAdEvent")
        disableVoidMethods("Lcom/blaze/gam/custom_native/BlazeCustomNativeAdsManager;", "requestAd")

        // 4. Blaze IMA Video Ads
        disableVoidMethods("Lcom/blaze/ima/BlazeIMA;", "enableAds")

        // 5. Brightcove Google IMA Video Ad Component & Event Listeners
        disableVoidMethods(
            "Lcom/brightcove/ima/GoogleIMAComponent;",
            "internalInitializeAdsRequests",
            "reloadAdsRequest",
            "initializeAdsRequests"
        )
        disableVoidMethods("Lcom/brightcove/ima/GoogleIMAComponent\$OnPlayListener;", "processEvent")
        disableVoidMethods("Lcom/brightcove/ima/GoogleIMAComponent\$OnCuePointListener;", "processEvent")
        returnBoolean("Lcom/brightcove/ima/BaseIMAComponent;", "shouldSkipAdBreak", true)
        returnBoolean("Lcom/brightcove/ima/BaseIMAComponent;", "isPlayingAd", false)
    }
}

/**
 * Removes promotional carousels by returning null from Lawi.b / Lawi.e when
 * the component title, id, or slug contains "promotion".
 */
private fun BytecodePatchContext.removePromotionsCarousel() {
    val awiClass = mutableClassDefByOrNull("Lawi;")
        ?: throw PatchException("Missing required class Lawi; (Content component mapper)")

    val bMethod = awiClass.methods.singleOrNull { method ->
        method.name == "b" &&
            method.parameterTypes == listOf("Ldf3;", "Z") &&
            method.returnType == "Lpj4;"
    } ?: throw PatchException("Missing required method Lawi.b(Ldf3;Z)Lpj4;")

    if (bMethod.implementation == null) {
        throw PatchException("Lawi.b has no implementation")
    }

    // In static method b(df3, z), registers: p0 = v19, p1 = v20.
    // Format 22c iget-object requires registers v0..v15, so copy p0 to v2.
    val promoCheckSmali = """
        if-eqz p0, :cond_check_done
        move-object/from16 v2, p0
        iget-object v0, v2, Ldf3;->b:Lfva;
        if-eqz v0, :cond_check_id
        iget-object v0, v0, Lfva;->a:Ljava/lang/String;
        if-eqz v0, :cond_check_id
        sget-object v1, Ljava/util/Locale;->ROOT:Ljava/util/Locale;
        invoke-virtual {v0, v1}, Ljava/lang/String;->toLowerCase(Ljava/util/Locale;)Ljava/lang/String;
        move-result-object v0
        const-string v1, "promotion"
        invoke-virtual {v0, v1}, Ljava/lang/String;->contains(Ljava/lang/CharSequence;)Z
        move-result v0
        if-nez v0, :cond_return_null
        :cond_check_id
        iget-object v0, v2, Ldf3;->a:Ljava/lang/String;
        if-eqz v0, :cond_check_slug
        sget-object v1, Ljava/util/Locale;->ROOT:Ljava/util/Locale;
        invoke-virtual {v0, v1}, Ljava/lang/String;->toLowerCase(Ljava/util/Locale;)Ljava/lang/String;
        move-result-object v0
        const-string v1, "promotion"
        invoke-virtual {v0, v1}, Ljava/lang/String;->contains(Ljava/lang/CharSequence;)Z
        move-result v0
        if-nez v0, :cond_return_null
        :cond_check_slug
        iget-object v0, v2, Ldf3;->f:Ljava/lang/String;
        if-eqz v0, :cond_check_section
        sget-object v1, Ljava/util/Locale;->ROOT:Ljava/util/Locale;
        invoke-virtual {v0, v1}, Ljava/lang/String;->toLowerCase(Ljava/util/Locale;)Ljava/lang/String;
        move-result-object v0
        const-string v1, "promotion"
        invoke-virtual {v0, v1}, Ljava/lang/String;->contains(Ljava/lang/CharSequence;)Z
        move-result v0
        if-nez v0, :cond_return_null
        :cond_check_section
        iget-object v0, v2, Ldf3;->e:Ljava/lang/String;
        if-eqz v0, :cond_check_done
        sget-object v1, Ljava/util/Locale;->ROOT:Ljava/util/Locale;
        invoke-virtual {v0, v1}, Ljava/lang/String;->toLowerCase(Ljava/util/Locale;)Ljava/lang/String;
        move-result-object v0
        const-string v1, "promotion"
        invoke-virtual {v0, v1}, Ljava/lang/String;->contains(Ljava/lang/CharSequence;)Z
        move-result v0
        if-nez v0, :cond_return_null
        goto :cond_check_done
        :cond_return_null
        const/4 v0, 0x0
        return-object v0
        :cond_check_done
    """.trimIndent()

    bMethod.addInstructions(0, promoCheckSmali)

    val eMethod = awiClass.methods.singleOrNull { method ->
        method.name == "e" &&
            method.parameterTypes == listOf("Ldf3;", "Z") &&
            method.returnType == "Ltj4;"
    }
    if (eMethod?.implementation != null) {
        eMethod.addInstructions(0, promoCheckSmali)
    }
}

/**
 * Replaces Blaze GAM banner view creation with an empty hidden View.
 */
private fun BytecodePatchContext.patchBlazeBannerCreation() {
    val handlerClass = mutableClassDefByOrNull("Lcom/blaze/gam/banner/DefaultBlazeGAMBannerAdsHandler;") ?: return
    for (method in handlerClass.methods) {
        if (method.name == "createAndLoadAdBannerView" && method.implementation != null) {
            method.addInstructions(
                0,
                """
                new-instance v0, Landroid/view/View;
                invoke-direct {v0, p1}, Landroid/view/View;-><init>(Landroid/content/Context;)V
                const/16 v1, 0x8
                invoke-virtual {v0, v1}, Landroid/view/View;->setVisibility(I)V
                return-object v0
                """.trimIndent()
            )
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
 * Stubs an object-returning method to immediately return null.
 */
private fun BytecodePatchContext.returnNullObject(
    classDescriptor: String,
    methodName: String
) {
    val mutableClass = mutableClassDefByOrNull(classDescriptor) ?: return

    for (method in mutableClass.methods) {
        if (method.name == methodName && method.returnType.startsWith("L") && method.implementation != null) {
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
