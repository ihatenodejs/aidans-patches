package app.aidan.patches.threes.tracking

import app.aidan.patches.shared.disableRequiredVoidMethods
import app.aidan.patches.shared.returnRequiredNullObject
import app.aidan.patches.threes.shared.COMPATIBILITY_THREES
import app.morphe.patcher.patch.bytecodePatch

@Suppress("unused")
val removeTrackingPatch = bytecodePatch(
    name = "Remove Tracking",
    description = "Neutralizes Google Analytics event logging and prevents the analytics tracker from being created.",
    default = true
) {
    category("Privacy")
    compatibleWith(COMPATIBILITY_THREES)

    execute {
        disableRequiredVoidMethods("Lvo/threes/util/GA;", "Checkpoint")
        returnRequiredNullObject("Lvo/threes/util/GA;", "GetTracker")
    }
}
