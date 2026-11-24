package app.aidan.patches.flowfree.ads

import app.aidan.patches.flowfree.shared.COMPATIBILITY_FLOW_FREE
import app.aidan.patches.shared.compositeFeaturePatch
import app.aidan.patches.shared.disableRequiredVoidMethods
import app.aidan.patches.shared.disableRequiredVoidMethodsByName
import app.aidan.patches.shared.returnRequiredBoolean
import app.morphe.patcher.patch.PatchException
import app.morphe.patcher.patch.ResourcePatchContext

private const val AD_MANAGER = "Lcom/noodlecake/flow/AdManager;"

/**
 * `Flow.removeAllAds` is the native UI gate for every in-app Remove Ads purchase entry point.
 * Its prologue is replaced with `mov w0, #1; ret`, so Settings, MainMenu, SelectPackMenu, and
 * TimeTrialPage skip their Remove Ads rows instead of offering a purchase that no longer has
 * any effect. Offset is a file offset in the arm64 `libgame.so`.
 */
private const val NATIVE_REMOVE_ALL_ADS_OFFSET = 0x4b5310
private val NATIVE_REMOVE_ALL_ADS_EXPECTED = byteArrayOf(
    0xfd.toByte(), 0x7b, 0xbe.toByte(), 0xa9.toByte(),
    0xf3.toByte(), 0x0b, 0x00, 0xf9.toByte()
)
private val NATIVE_RETURN_TRUE = byteArrayOf(
    0x20, 0x00, 0x80.toByte(), 0x52,
    0xc0.toByte(), 0x03, 0x5f, 0xd6.toByte()
)

/**
 * Advertising SDK content providers run before the application and initialize the mediation
 * runtimes independently of [AD_MANAGER]. Each entry maps a provider to the method that starts
 * its SDK: providers initialize from `onCreate`, except Google Mobile Ads, which overrides
 * `attachInfo`.
 */
private val AD_PROVIDERS = mapOf(
    "Lcom/applovin/sdk/AppLovinInitProvider;" to "onCreate",
    "Lcom/facebook/ads/AudienceNetworkContentProvider;" to "onCreate",
    "Lio/bidmachine/BidMachineInitProvider;" to "onCreate",
    "Lcom/vungle/ads/VungleProvider;" to "onCreate",
    "Lcom/mbridge/msdk/config/component/status/MBComponentLifecycleProvider;" to "onCreate"
)

@Suppress("unused")
val removeAdsPatch = compositeFeaturePatch(
    name = "Remove Ads",
    description = "Blocks banner, interstitial, and rewarded video ads, prevents the AppLovin MAX, Google Mobile Ads, Amazon, Facebook Audience Network, Vungle, BidMachine, and Mintegral advertising runtimes from initializing, and hides the in-app Remove Ads purchase prompts.",
    default = true
) {
    category("Ads")
    compatibleWith(COMPATIBILITY_FLOW_FREE)

    execute {
        // The native game reaches ads only through this Java facade. Blocking the entry points
        // also leaves the static banner-visible flag false so the game does not reserve ad space.
        disableRequiredVoidMethodsByName(
            AD_MANAGER,
            "alMaxStartAfterConsent",
            "loadInterstitial",
            "loadRewardedVideo",
            "showAds",
            "showBanner",
            "showInterstitial",
            "showRewardedVideo",
            "updateBannerPlacement"
        )
        returnRequiredBoolean(AD_MANAGER, "hasInterstitial", false)
        returnRequiredBoolean(AD_MANAGER, "hasRewardedVideo", false)

        AD_PROVIDERS.forEach { (provider, startupMethod) ->
            returnRequiredBoolean(provider, startupMethod, false)
        }
        disableRequiredVoidMethods("Lcom/google/android/gms/ads/MobileAdsInitProvider;", "attachInfo")
    }
    rawResources {
        val library = get("lib/arm64-v8a/libgame.so")
        if (!library.exists()) throw PatchException("Missing arm64 libgame.so required for Remove Ads UI suppression")
        val bytes = library.readBytes()
        val offset = NATIVE_REMOVE_ALL_ADS_OFFSET

        if (bytes.size < offset + NATIVE_RETURN_TRUE.size) {
            throw PatchException("Flow.removeAllAds is outside libgame.so")
        }
        val alreadyPatched = NATIVE_RETURN_TRUE.indices.all { bytes[offset + it] == NATIVE_RETURN_TRUE[it] }
        if (!alreadyPatched) {
            if (!NATIVE_REMOVE_ALL_ADS_EXPECTED.indices.all { bytes[offset + it] == NATIVE_REMOVE_ALL_ADS_EXPECTED[it] }) {
                throw PatchException("Flow.removeAllAds byte sequence mismatch at 0x${offset.toString(16)}")
            }
            System.arraycopy(NATIVE_RETURN_TRUE, 0, bytes, offset, NATIVE_RETURN_TRUE.size)
            library.writeBytes(bytes)
        }

        setZeroConfigFlag("assets/config_default.txt", "RemoveAdsModal")
        setZeroConfigFlag("assets/config_default.txt", "LevelCompleteRemoveAds")
        setZeroConfigFlag("assets/config_default.txt", "RemoveAdsBacking")
        setZeroConfigFlag("assets/config_debug.txt", "LevelCompleteRemoveAds")
    }
}

/**
 * Sets an integer game config flag to 0, preserving the surrounding JSON formatting. The flag
 * must exist so a renamed or removed key fails the patch instead of silently leaving an upsell.
 */
private fun ResourcePatchContext.setZeroConfigFlag(path: String, key: String) {
    val file = get(path)
    if (!file.exists()) throw PatchException("Missing $path for Remove Ads UI suppression")
    val content = file.readText()
    val flag = Regex("(\"$key\"\\s*:\\s*)-?\\d+")
    if (!flag.containsMatchIn(content)) throw PatchException("Missing $key flag in $path")
    val updated = flag.replace(content) { "${it.groupValues[1]}0" }
    if (updated != content) file.writeText(updated)
}
