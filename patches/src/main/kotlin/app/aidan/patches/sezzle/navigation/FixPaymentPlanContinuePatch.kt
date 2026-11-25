package app.aidan.patches.sezzle.navigation

import app.aidan.patches.sezzle.shared.Constants.COMPATIBILITY_SEZZLE
import app.aidan.patches.shared.HermesBundleEditor
import app.aidan.patches.shared.matchesAt
import app.morphe.patcher.patch.PatchException
import app.morphe.patcher.patch.rawResourcePatch

/**
 * Makes the payment options sheet dismissible with Continue.
 *
 * `PaymentPlanPopup` wires its Continue button to a submit closure that
 * attempts to finalize the loan before the checkout agreements are accepted.
 * The request settles without advancing the UI, leaving the modal open with no
 * way to reach the underlying checkout summary and confirmation button. The
 * plan itself is already committed by `handlePlanChange`, so this patch
 * re-points the Continue press at the sheet's existing close handler.
 */
private val EXPECTED_CONTINUE_ON_PRESS_BYTES = byteArrayOf(
    0x34, 0x02, 0x00, // GetParentEnvironment r2, 0
    0x3b, 0x02, 0x02, 0x25, // LoadFromEnvironment r2, r2, 37 (handleSubmit)
    0x96.toByte(), 0x00, // LoadConstFalse r0
    0x93.toByte(), 0x01, // LoadConstUndefined r1
    0x6e, 0x02, 0x02, 0x01, 0x00, // Call2 r2, r2, r1, r0
    0x76, 0x02 // Ret r2
)

private val FIXED_CONTINUE_ON_PRESS_BYTES = EXPECTED_CONTINUE_ON_PRESS_BYTES.copyOf().also {
    it[6] = 0x24 // LoadFromEnvironment r2, r2, 36 (handleClose)
}

@Suppress("unused")
val fixPaymentPlanContinuePatch = rawResourcePatch(
    name = "Fix Payment Plan Continue",
    description = "Closes the payment options sheet when continuing so the checkout summary and loan confirmation stay reachable.",
    default = true
) {
    category("Interface")
    compatibleWith(COMPATIBILITY_SEZZLE)

    execute {
        val bundleFile = get("assets/index.android.bundle")
        if (!bundleFile.exists()) {
            throw PatchException("assets/index.android.bundle not found")
        }

        val editor = HermesBundleEditor(bundleFile.readBytes())
        editor.findFunctionOffsetByName("PaymentPlanPopup")
            ?: throw PatchException("Failed to find function PaymentPlanPopup in Hermes bundle")

        val bytes = editor.toByteArray()
        val matches = mutableListOf<Int>()
        var index = 0
        while (index <= bytes.size - EXPECTED_CONTINUE_ON_PRESS_BYTES.size) {
            if (bytes.matchesAt(index, EXPECTED_CONTINUE_ON_PRESS_BYTES)) {
                matches.add(index)
            }
            index++
        }

        when (matches.size) {
            1 -> editor.patchBytes(matches.single(), FIXED_CONTINUE_ON_PRESS_BYTES)
            0 -> {
                val alreadyPatched = bytes.indexOf(FIXED_CONTINUE_ON_PRESS_BYTES)
                if (alreadyPatched < 0) {
                    throw PatchException(
                        "Failed to find the PaymentPlanPopup Continue handler bytecode"
                    )
                }
            }
            else -> throw PatchException(
                "Expected one PaymentPlanPopup Continue handler, found ${matches.size}"
            )
        }

        editor.updateFooterHash()
        bundleFile.writeBytes(editor.toByteArray())
    }
}

private fun ByteArray.indexOf(expected: ByteArray): Int {
    for (offset in 0..size - expected.size) {
        if (matchesAt(offset, expected)) return offset
    }
    return -1
}
