package app.aidan.patches.flowfree.hints

import app.aidan.patches.flowfree.shared.COMPATIBILITY_FLOW_FREE
import app.aidan.patches.flowfree.shared.LibgameFunction
import app.aidan.patches.flowfree.shared.RETURN_HINT_BALANCE
import app.aidan.patches.flowfree.shared.RETURN_TRUE
import app.aidan.patches.flowfree.shared.arm64
import app.aidan.patches.flowfree.shared.patchLibgameFunctions
import app.morphe.patcher.patch.rawResourcePatch

/**
 * `HintManager` stores two balances: the normal hint balance and a temporary balance used by
 * time-trial-style modes. Every label, button enablement check, and dialog reads one of these
 * getters, so forcing all three to 999 keeps the hint UI consistent. `useHint` is replaced with
 * `return true` because it only validates and decrements the stored balance; the caller performs
 * the actual hint reveal, and the forced balance getters keep the displayed count at 999.
 */
private val HINT_BALANCE = LibgameFunction(
    name = "HintManager.hintBalance",
    offset = 0x4f32f0,
    expected = arm64(0x28, 0x08, 0x00, 0xf0, 0x08, 0x95, 0x45, 0xf9, 0x08, 0x01, 0x40, 0xf9, 0x08, 0x01, 0x40, 0xb9),
    replacement = RETURN_HINT_BALANCE
)
private val TEMP_HINT_BALANCE = LibgameFunction(
    name = "HintManager.tempHintBalance",
    offset = 0x4f3308,
    expected = arm64(0x28, 0x08, 0x00, 0xf0, 0x08, 0x91, 0x45, 0xf9, 0x08, 0x01, 0x40, 0xf9, 0x08, 0x01, 0x40, 0xb9),
    replacement = RETURN_HINT_BALANCE
)
private val ACTIVE_HINT_BALANCE = LibgameFunction(
    name = "HintManager.getActiveHintBalance",
    offset = 0x4eace4,
    expected = arm64(0xff, 0xc3, 0x00, 0xd1, 0xfd, 0x7b, 0x01, 0xa9, 0xf4, 0x4f, 0x02, 0xa9, 0xfd, 0x43, 0x00, 0x91),
    replacement = RETURN_HINT_BALANCE
)
private val USE_HINT = LibgameFunction(
    name = "HintManager.useHint",
    offset = 0x4ec214,
    expected = arm64(0xff, 0x03, 0x03, 0xd1, 0xfd, 0x7b, 0x08, 0xa9, 0xf8, 0x5f, 0x09, 0xa9, 0xf6, 0x57, 0x0a, 0xa9),
    replacement = RETURN_TRUE
)

@Suppress("unused")
val unlimitedHintsPatch = rawResourcePatch(
    name = "Unlimited Hints",
    description = "Makes the hint balance display 999 and allows hints to be used without consuming hints or watching rewarded videos.",
    default = true
) {
    category("Features")
    compatibleWith(COMPATIBILITY_FLOW_FREE)

    execute {
        patchLibgameFunctions(
            HINT_BALANCE,
            TEMP_HINT_BALANCE,
            ACTIVE_HINT_BALANCE,
            USE_HINT
        )
    }
}
