package app.aidan.patches.flowfree.levels

import app.aidan.patches.flowfree.shared.COMPATIBILITY_FLOW_FREE
import app.aidan.patches.flowfree.shared.LibgameFunction
import app.aidan.patches.flowfree.shared.RETURN_TRUE
import app.aidan.patches.flowfree.shared.arm64
import app.aidan.patches.flowfree.shared.patchLibgameFunctions
import app.morphe.patcher.patch.rawResourcePatch

/**
 * `-[LevelPack isPurchased]` normally forwards to the persisted `LevelPackState`, with a
 * shortcut when the subscription purchase flag is set. Replacing its prologue makes every
 * pack report as purchased, which is the same condition the pack menu uses to unlock premium
 * packs. `isIndividuallyPurchased` and `isUnlocked` cover experimental/server-activated packs
 * and any future call site that does not consult `isPurchased` first.
 */
private val LEVEL_PACK_PURCHASED = LibgameFunction(
    name = "LevelPack.isPurchased",
    offset = 0x51bec8,
    expected = arm64(0x08, 0x07, 0x00, 0x90, 0x08, 0x05, 0x40, 0xf9, 0x08, 0x01, 0x40, 0xf9, 0x08, 0x01, 0x40, 0xb9),
    replacement = RETURN_TRUE
)
private val LEVEL_PACK_INDIVIDUALLY_PURCHASED = LibgameFunction(
    name = "LevelPack.isIndividuallyPurchased",
    offset = 0x51bf64,
    expected = arm64(0x08, 0x07, 0x00, 0x90, 0x08, 0x05, 0x40, 0xf9, 0x08, 0x01, 0x40, 0xf9, 0x08, 0x01, 0x40, 0xb9),
    replacement = RETURN_TRUE
)
private val LEVEL_PACK_UNLOCKED = LibgameFunction(
    name = "LevelPack.isUnlocked",
    offset = 0x51b600,
    expected = arm64(0xff, 0xc3, 0x01, 0xd1, 0xfd, 0x7b, 0x04, 0xa9, 0xf6, 0x57, 0x05, 0xa9, 0xf4, 0x4f, 0x06, 0xa9),
    replacement = RETURN_TRUE
)
private val LEVEL_PACK_PLAYABLE = LibgameFunction(
    name = "LevelPack.isPlayable",
    offset = 0x51b84c,
    expected = arm64(0xff, 0x83, 0x01, 0xd1, 0xfd, 0x7b, 0x03, 0xa9, 0xf5, 0x23, 0x00, 0xf9, 0xf4, 0x4f, 0x05, 0xa9),
    replacement = RETURN_TRUE
)
private val LEVEL_PLAYABLE = LibgameFunction(
    name = "Level.isPlayable",
    offset = 0x5075e4,
    expected = arm64(0x88, 0x07, 0x00, 0xf0, 0x08, 0x09, 0x47, 0xf9, 0x08, 0x01, 0x40, 0xf9, 0x08, 0x01, 0x40, 0xb9),
    replacement = RETURN_TRUE
)

/**
 * `-[SelectPackMenu onSelected:]` sets `_selectedPack` and then runs pack-type, prerequisite,
 * advanced-pack-warning, purchase, cloud-sync, and Play connect prompts before either navigating
 * or returning. Since every pack is unlocked by this patch, those prompts are noise that also
 * leaves the pack list behind a click catcher. Branching directly to the success tail call
 * (`navigateToSelectedPack`) keeps the selection state but skips the pointless prompt paths.
 */
private val SELECT_PACK_NAVIGATE = LibgameFunction(
    name = "SelectPackMenu.onSelected: direct navigation",
    offset = 0x571c2c,
    expected = arm64(0xe8, 0x02, 0x40, 0xf9),
    replacement = arm64(0xde, 0x00, 0x00, 0x14)
)

@Suppress("unused")
val unlockAllLevelsPatch = rawResourcePatch(
    name = "Unlock All Levels",
    description = "Unlocks every level pack and every level, including premium and individually purchased packs, without changing saved progress or purchase state.",
    default = true
) {
    category("Features")
    compatibleWith(COMPATIBILITY_FLOW_FREE)

    execute {
        patchLibgameFunctions(
            LEVEL_PACK_PURCHASED,
            LEVEL_PACK_INDIVIDUALLY_PURCHASED,
            LEVEL_PACK_UNLOCKED,
            LEVEL_PACK_PLAYABLE,
            LEVEL_PLAYABLE,
            SELECT_PACK_NAVIGATE
        )
    }
}
