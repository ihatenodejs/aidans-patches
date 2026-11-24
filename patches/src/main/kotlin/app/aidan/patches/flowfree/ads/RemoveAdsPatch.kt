package app.aidan.patches.flowfree.ads

import app.aidan.patches.flowfree.shared.COMPATIBILITY_FLOW_FREE
import app.aidan.patches.shared.disableRequiredVoidMethods
import app.aidan.patches.shared.disableRequiredVoidMethodsByName
import app.aidan.patches.shared.returnRequiredBoolean
import app.morphe.patcher.patch.bytecodePatch

private const val AD_MANAGER = "Lcom/noodlecake/flow/AdManager;"

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
val removeAdsPatch = bytecodePatch(
    name = "Remove Ads",
    description = "Blocks banner, interstitial, and rewarded video ads and prevents the AppLovin MAX, Google Mobile Ads, Amazon, Facebook Audience Network, Vungle, BidMachine, and Mintegral advertising runtimes from initializing.",
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
}
