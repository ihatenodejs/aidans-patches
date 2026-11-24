package app.aidan.patches.threes.ads

import app.aidan.patches.threes.shared.COMPATIBILITY_THREES
import app.morphe.patcher.patch.PatchException
import app.morphe.patcher.patch.rawResourcePatch

private val ARM64_RETURN = byteArrayOf(0xc0.toByte(), 0x03, 0x5f, 0xd6.toByte())
private val RETURN_TRUE =
    byteArrayOf(0x20, 0x00, 0x80.toByte(), 0x52, 0xc0.toByte(), 0x03, 0x5f, 0xd6.toByte())
private val RETURN_UNLIMITED_PLAYS =
    byteArrayOf(0xe0.toByte(), 0xe1.toByte(), 0x84.toByte(), 0x52, 0xc0.toByte(), 0x03, 0x5f, 0xd6.toByte())

@Suppress("unused")
val removeAdsPatch = rawResourcePatch(
    name = "Remove Ads",
    description = "Removes rewarded-video advertising and grants unlimited free plays so the game never requires watching an ad to continue.",
    default = true
) {
    category("Ads")
    compatibleWith(COMPATIBILITY_THREES)

    execute {
        val library = get("lib/arm64-v8a/libil2cpp.so")
        if (!library.exists()) throw PatchException("Missing arm64 IL2CPP library")
        val bytes = library.readBytes()

        /**
         * Writes [replacement] at byte [offset] after verifying [expected], or does nothing when
         * the replacement is already present. [target] identifies the patch in errors.
         *
         * @throws PatchException if the replacement extends past the library or the expected bytes differ.
         * @throws IndexOutOfBoundsException if an unchecked negative offset or expected range is invalid.
         */
        fun patch(offset: Int, expected: ByteArray, replacement: ByteArray, target: String) {
            if (bytes.size < offset + replacement.size) throw PatchException("$target is outside libil2cpp.so")
            if (replacement.indices.all { bytes[offset + it] == replacement[it] }) return
            if (!expected.indices.all { bytes[offset + it] == expected[it] }) {
                throw PatchException("$target byte sequence mismatch at 0x${offset.toString(16)}")
            }
            System.arraycopy(replacement, 0, bytes, offset, replacement.size)
        }

        // FreeLogic.GetPlayCount: every free-play gate checks the returned count; a fixed high
        // count keeps all of them satisfied without touching the stored preference.
        patch(
            0x83c6e4,
            byteArrayOf(0xfe.toByte(), 0x57, 0xbe.toByte(), 0xa9.toByte(), 0xf4.toByte(), 0x4f, 0x01, 0xa9.toByte()),
            RETURN_UNLIMITED_PLAYS,
            "FreeLogic.GetPlayCount"
        )
        // FreeLogic.HasPlays: also force the semantic helper to true for any future call site.
        patch(
            0x83c7a4,
            byteArrayOf(0xfe.toByte(), 0x0f, 0x1f, 0xf8.toByte(), 0xcf.toByte(), 0xff.toByte(), 0xff.toByte(), 0x97.toByte()),
            RETURN_TRUE,
            "FreeLogic.HasPlays"
        )
        // AdMan.Start: never initialize LevelPlay/Unity Ads or request any ad inventory.
        patch(
            0x82af64,
            byteArrayOf(0xfe.toByte(), 0x0f, 0x1c, 0xf8.toByte()),
            ARM64_RETURN,
            "AdMan.Start"
        )

        library.writeBytes(bytes)
    }
}
