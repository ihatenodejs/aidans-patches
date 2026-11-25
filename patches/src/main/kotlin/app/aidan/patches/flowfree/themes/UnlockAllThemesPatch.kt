package app.aidan.patches.flowfree.themes

import app.aidan.patches.flowfree.shared.COMPATIBILITY_FLOW_FREE
import app.aidan.patches.flowfree.shared.HIDE_RESTORE_ICON
import app.aidan.patches.flowfree.shared.HIDE_RESTORE_LABEL
import app.aidan.patches.flowfree.shared.HIDE_SETTINGS_RESTORE_ROW
import app.aidan.patches.flowfree.shared.LibgameFunction
import app.aidan.patches.flowfree.shared.RETURN_TRUE
import app.aidan.patches.flowfree.shared.arm64
import app.aidan.patches.flowfree.shared.patchLibgameFunctions
import app.morphe.patcher.patch.rawResourcePatch

/**
 * Themes are gated by `-[Theme isUnlocked]` / `-[Theme isPurchased]`, which combine the
 * per-theme persisted flags with `ThemeManager.hasPurchasedThemes`. The manager itself
 * consults `allThemesPurchased`. Forcing each of these getters true unlocks every theme and
 * makes the store's "Unlock All Themes" row unnecessary.
 */
private val THEME_UNLOCKED = LibgameFunction(
    name = "Theme.isUnlocked",
    offset = 0x5d7bd4,
    expected = arm64(0xff, 0x03, 0x01, 0xd1, 0xfd, 0x7b, 0x02, 0xa9, 0xf4, 0x4f, 0x03, 0xa9, 0xfd, 0x83, 0x00, 0x91),
    replacement = RETURN_TRUE
)
private val THEME_PURCHASED = LibgameFunction(
    name = "Theme.isPurchased",
    offset = 0x5d7cf4,
    expected = arm64(0x28, 0x01, 0x00, 0xb0, 0x08, 0x0d, 0x41, 0xf9, 0x08, 0x01, 0x40, 0xf9, 0x08, 0x01, 0x40, 0xb9),
    replacement = RETURN_TRUE
)
private val THEME_INDIVIDUALLY_PURCHASED = LibgameFunction(
    name = "Theme.isIndividuallyPurchased",
    offset = 0x5d7d90,
    expected = arm64(0x28, 0x01, 0x00, 0xb0, 0x08, 0x0d, 0x41, 0xf9, 0x08, 0x01, 0x40, 0xf9, 0x08, 0x01, 0x40, 0xb9),
    replacement = RETURN_TRUE
)
private val THEME_MANAGER_ALL_PURCHASED = LibgameFunction(
    name = "ThemeManager.allThemesPurchased",
    offset = 0x5da954,
    expected = arm64(0x08, 0x01, 0x00, 0xd0, 0x08, 0x39, 0x41, 0xf9, 0x08, 0x01, 0x40, 0xf9, 0x08, 0x01, 0x40, 0xb9),
    replacement = RETURN_TRUE
)

@Suppress("unused")
val unlockAllThemesPatch = rawResourcePatch(
    name = "Unlock All Themes",
    description = "Unlocks every premium theme and the Complete Package theme bundle in the store without changing saved progress or purchase state.",
    default = true
) {
    category("Customization")
    compatibleWith(COMPATIBILITY_FLOW_FREE)

    execute {
        patchLibgameFunctions(
            THEME_UNLOCKED,
            THEME_PURCHASED,
            THEME_INDIVIDUALLY_PURCHASED,
            THEME_MANAGER_ALL_PURCHASED,
            HIDE_SETTINGS_RESTORE_ROW,
            HIDE_RESTORE_LABEL,
            HIDE_RESTORE_ICON
        )
    }
}
