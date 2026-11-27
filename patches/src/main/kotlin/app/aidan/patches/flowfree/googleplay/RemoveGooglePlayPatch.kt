package app.aidan.patches.flowfree.googleplay

import app.aidan.patches.flowfree.shared.COMPATIBILITY_FLOW_FREE
import app.aidan.patches.flowfree.shared.LibgameFunction
import app.aidan.patches.flowfree.shared.RETURN_FALSE
import app.aidan.patches.flowfree.shared.RETURN_VOID
import app.aidan.patches.flowfree.shared.arm64
import app.aidan.patches.flowfree.shared.patchLibgameFunctions
import app.aidan.patches.shared.compositeFeaturePatch
import app.aidan.patches.shared.disableRequiredVoidMethodsByName
import app.aidan.patches.shared.returnRequiredBoolean

private const val GAME_ACTIVITY = "Lcom/noodlecake/noodlenews/NoodlecakeGameActivity;"
private const val PLAY_ACHIEVEMENT_HELPER = "Lcom/noodlecake/flow/utils/GooglePlayAchievementHelper;"

private val FLOW_HAS_AUTH = LibgameFunction(
    name = "Flow.hasGameCenterAuth",
    offset = 0x4c2cd8,
    expected = arm64(0xc8, 0x09, 0x00, 0x90, 0x08, 0x09, 0x44, 0xf9, 0x08, 0x01, 0x40, 0xf9, 0x08, 0x01, 0x40, 0xb9),
    replacement = RETURN_FALSE
)
private val FLOW_LOGIN_PENDING = LibgameFunction(
    name = "Flow.isGameCenterLoginPending",
    offset = 0x4bf028,
    expected = arm64(0xc8, 0x09, 0x00, 0xf0, 0x08, 0x05, 0x44, 0xf9, 0x08, 0x01, 0x40, 0xf9, 0x08, 0x01, 0x40, 0xb9),
    replacement = RETURN_FALSE
)
private val FLOW_REQUEST_REVIEW = LibgameFunction(
    name = "Flow.requestReview",
    offset = 0x4c0254,
    expected = arm64(0xff, 0x03, 0x02, 0xd1, 0xfd, 0x7b, 0x04, 0xa9, 0xf7, 0x2b, 0x00, 0xf9, 0xf6, 0x57, 0x06, 0xa9),
    replacement = RETURN_VOID
)
private val FLOW_REVIEW_ON_BOOT = LibgameFunction(
    name = "Flow.shouldRequestReviewOnBoot",
    offset = 0x4bfef8,
    expected = arm64(0xff, 0x03, 0x01, 0xd1, 0xfd, 0x7b, 0x02, 0xa9, 0xf4, 0x4f, 0x03, 0xa9, 0xfd, 0x83, 0x00, 0x91),
    replacement = RETURN_FALSE
)
private val FLOW_REVIEW_AFTER_LEVEL = LibgameFunction(
    name = "Flow.shouldRequestReviewAfterLevel",
    offset = 0x4c0044,
    expected = arm64(0xff, 0xc3, 0x01, 0xd1, 0xfd, 0x7b, 0x04, 0xa9, 0xf6, 0x57, 0x05, 0xa9, 0xf4, 0x4f, 0x06, 0xa9),
    replacement = RETURN_FALSE
)
private val FLOW_LAUNCH_REVIEW = LibgameFunction(
    name = "Flow.launchReviewApp",
    offset = 0x4b38cc,
    expected = arm64(0xff, 0x03, 0x02, 0xd1, 0xfd, 0x7b, 0x04, 0xa9, 0xf8, 0x5f, 0x05, 0xa9, 0xf6, 0x57, 0x06, 0xa9),
    replacement = RETURN_VOID
)
private val FLOW_LAUNCH_STORE = LibgameFunction(
    name = "Flow.launchAppStoreId:campaign:",
    offset = 0x4b336c,
    expected = arm64(0xfd, 0x7b, 0xbd, 0xa9, 0xf5, 0x0b, 0x00, 0xf9, 0xf4, 0x4f, 0x02, 0xa9, 0xfd, 0x03, 0x00, 0x91),
    replacement = RETURN_VOID
)
private val FLOW_LAUNCH_STORE_OR_URL = LibgameFunction(
    name = "Flow.launchAppStoreId:orUrl:campaign:",
    offset = 0x4b33c8,
    expected = arm64(0xff, 0x43, 0x01, 0xd1, 0xfd, 0x7b, 0x01, 0xa9, 0xf7, 0x13, 0x00, 0xf9, 0xf6, 0x57, 0x03, 0xa9),
    replacement = RETURN_VOID
)
private val SHOW_PLAY_CONNECT = LibgameFunction(
    name = "SelectPackMenu.showGooglePlayConnectPrompt",
    offset = 0x571320,
    expected = arm64(0xff, 0x43, 0x02, 0xd1, 0xfd, 0x7b, 0x05, 0xa9, 0xf7, 0x33, 0x00, 0xf9, 0xf6, 0x57, 0x07, 0xa9),
    replacement = RETURN_VOID
)
private val SHOW_CLOUD_SYNC = LibgameFunction(
    name = "SelectPackMenu.showCloudSyncEnablePrompt",
    offset = 0x5710f8,
    expected = arm64(0xff, 0x43, 0x02, 0xd1, 0xfd, 0x7b, 0x05, 0xa9, 0xf7, 0x33, 0x00, 0xf9, 0xf6, 0x57, 0x07, 0xa9),
    replacement = RETURN_VOID
)
private val SHOW_UPDATE_REQUIRED = LibgameFunction(
    name = "SelectPackMenu.showUpdateRequiredPrompt",
    offset = 0x5718e4,
    expected = arm64(0xff, 0x43, 0x02, 0xd1, 0xfd, 0x7b, 0x05, 0xa9, 0xf7, 0x33, 0x00, 0xf9, 0xf6, 0x57, 0x07, 0xa9),
    replacement = RETURN_VOID
)
private val ON_GOOGLE_API_CONNECTED = LibgameFunction(
    name = "SelectPackMenu.onGoogleApiConnected:",
    offset = 0x573d7c,
    expected = arm64(0x48, 0x04, 0x00, 0x90, 0x08, 0xa1, 0x44, 0xf9, 0x08, 0x01, 0x40, 0xf9, 0x08, 0x01, 0x40, 0xb9),
    replacement = RETURN_VOID
)
private val ON_GAME_CENTER_AUTH = LibgameFunction(
    name = "SelectPackMenu.onGameCenterAuth:",
    offset = 0x573e20,
    expected = arm64(0xff, 0x83, 0x01, 0xd1, 0xfd, 0x7b, 0x03, 0xa9, 0xf6, 0x57, 0x04, 0xa9, 0xf4, 0x4f, 0x05, 0xa9),
    replacement = RETURN_VOID
)
private val ACHIEVEMENTS_SUPPORTED = LibgameFunction(
    name = "SelectPackMenu.areAchievementsSupported",
    offset = 0x573b70,
    expected = arm64(0x20, 0x00, 0x80, 0x52, 0xc0, 0x03, 0x5f, 0xd6),
    replacement = RETURN_FALSE
)
private val ACHIEVEMENTS_READY = LibgameFunction(
    name = "SelectPackMenu.areAchievementsReady",
    offset = 0x573b78,
    expected = arm64(0x20, 0x00, 0x80, 0x52, 0xc0, 0x03, 0x5f, 0xd6),
    replacement = RETURN_FALSE
)

@Suppress("unused")
val removeGooglePlayPatch = compositeFeaturePatch(
    name = "Remove Google Play",
    description = "Disables Google Play Games sign-in, cloud-sync prompts, achievements, review requests, and Play Store update and cross-promo launches.",
    default = true
) {
    category("Privacy")
    compatibleWith(COMPATIBILITY_FLOW_FREE)

    execute {
        // Play Games sign-in and achievements are driven by the Java activity and the
        // JNI-facing achievement helper. Neutralizing both prevents the cloud-sync prompt and
        // the Play Games overlay while leaving Firebase and billing untouched.
        returnRequiredBoolean(GAME_ACTIVITY, "shouldGoogleAPIConnect", false)
        returnRequiredBoolean(GAME_ACTIVITY, "isGoogleAPIConnected", false)
        returnRequiredBoolean(GAME_ACTIVITY, "isSignedIn", false)
        returnRequiredBoolean(GAME_ACTIVITY, "gotoQuickReviewScreen", false)
        disableRequiredVoidMethodsByName(
            GAME_ACTIVITY,
            "loginToGoogleAPI",
            "handlePlayGamesConnection",
            "showAchievements",
            "gotoReviewScreen"
        )
        disableRequiredVoidMethodsByName(PLAY_ACHIEVEMENT_HELPER, "submitAchievement", "showAchievements")
    }
    rawResources {
        // Native Flow-side prompts and Play Store intents. The appstore launch methods are the
        // paths used by review prompts and cross-promo screens that leave the game for Play Store.
        patchLibgameFunctions(
            FLOW_HAS_AUTH,
            FLOW_LOGIN_PENDING,
            FLOW_REQUEST_REVIEW,
            FLOW_REVIEW_ON_BOOT,
            FLOW_REVIEW_AFTER_LEVEL,
            FLOW_LAUNCH_REVIEW,
            FLOW_LAUNCH_STORE,
            FLOW_LAUNCH_STORE_OR_URL,
            SHOW_PLAY_CONNECT,
            SHOW_CLOUD_SYNC,
            SHOW_UPDATE_REQUIRED,
            ON_GOOGLE_API_CONNECTED,
            ON_GAME_CENTER_AUTH,
            ACHIEVEMENTS_SUPPORTED,
            ACHIEVEMENTS_READY
        )
    }
}
