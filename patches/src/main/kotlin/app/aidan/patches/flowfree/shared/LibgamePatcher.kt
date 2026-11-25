package app.aidan.patches.flowfree.shared

import app.morphe.patcher.patch.PatchException
import app.morphe.patcher.patch.ResourcePatchContext

private const val LIBGAME = "lib/arm64-v8a/libgame.so"

/** `mov w0, #1; ret` for boolean getters. */
internal val RETURN_TRUE = byteArrayOf(
    0x20, 0x00, 0x80.toByte(), 0x52,
    0xc0.toByte(), 0x03, 0x5f, 0xd6.toByte()
)

/** `mov w0, wzr; ret` for boolean getters. */
internal val RETURN_FALSE = byteArrayOf(
    0xe0.toByte(), 0x03, 0x1f, 0x2a,
    0xc0.toByte(), 0x03, 0x5f, 0xd6.toByte()
)

/** `ret` for void methods. */
internal val RETURN_VOID = byteArrayOf(0xc0.toByte(), 0x03, 0x5f, 0xd6.toByte())

/** `mov w0, #999; ret` for hint balances. */
internal val RETURN_HINT_BALANCE = byteArrayOf(
    0xe0.toByte(), 0x7c, 0x80.toByte(), 0x52,
    0xc0.toByte(), 0x03, 0x5f, 0xd6.toByte()
)

/**
 * Skips the Settings > Help "Restore Purchases" row by branching from its label descriptor
 * load to the start of the following About row. The following row reuses the same layout
 * cursor and closes the gap.
 */
internal val HIDE_SETTINGS_RESTORE_ROW = LibgameFunction(
    name = "SettingsMenu Restore Purchases row",
    offset = 0x57f1b8,
    expected = arm64(0x42, 0x80, 0x15, 0x91),
    replacement = arm64(0xbc, 0x00, 0x00, 0x14)
)

/** Blanks the localization key used by both Restore Purchases rows. */
internal val HIDE_RESTORE_LABEL = LibgameFunction(
    name = "Restore Purchases label",
    offset = 0x402efe,
    expected = "Restore Purchases\u0000".toByteArray(),
    replacement = byteArrayOf(0)
)

/** Blanks the extra_restore.png icon name used by both Restore Purchases rows. */
internal val HIDE_RESTORE_ICON = LibgameFunction(
    name = "Restore Purchases icon",
    offset = 0x402f10,
    expected = "extra_restore.png\u0000".toByteArray(),
    replacement = byteArrayOf(0)
)

/** Blanks the free-play Extras "Get More Hints" label. */
internal val HIDE_GET_MORE_HINTS_LABEL = LibgameFunction(
    name = "Get More Hints label",
    offset = 0x4041af,
    expected = "Get More Hints\u0000".toByteArray(),
    replacement = byteArrayOf(0)
)

/** Points the Get More Hints row at an empty icon slot so only the label row keeps its layout. */
internal val HIDE_GET_MORE_HINTS_ICON = LibgameFunction(
    name = "Get More Hints icon",
    offset = 0x56c95c,
    expected = arm64(0x42, 0x20, 0x12, 0x91),
    replacement = arm64(0x42, 0xc0, 0x11, 0x91)
)

/**
 * A native function in `libgame.so` identified by its file offset, the original prologue bytes,
 * and the replacement instructions. Offsets are file offsets and therefore equal virtual
 * addresses inside `.text`.
 */
internal class LibgameFunction(
    val name: String,
    val offset: Int,
    val expected: ByteArray,
    val replacement: ByteArray
)

/**
 * Replaces the prologue of each [function] in `libgame.so`, leaving the already-patched state
 * idempotent. Every function is validated before any bytes are written so a version mismatch
 * fails the patch instead of producing a partially modified library.
 */
internal fun ResourcePatchContext.patchLibgameFunctions(vararg function: LibgameFunction) {
    val library = get(LIBGAME)
    if (!library.exists()) throw PatchException("Missing arm64 libgame.so required for Flow Free native unlock patches")

    val bytes = library.readBytes()
    function.forEach { target ->
        if (target.expected.size < target.replacement.size) {
            throw PatchException("${target.name} expected prologue is shorter than its replacement")
        }
        if (bytes.size < target.offset + target.expected.size) {
            throw PatchException("${target.name} is outside libgame.so")
        }
        if (target.replacement.indices.all { bytes[target.offset + it] == target.replacement[it] }) return@forEach
        if (!target.expected.indices.all { bytes[target.offset + it] == target.expected[it] }) {
            throw PatchException("${target.name} byte sequence mismatch at 0x${target.offset.toString(16)}")
        }
        System.arraycopy(target.replacement, 0, bytes, target.offset, target.replacement.size)
    }
    library.writeBytes(bytes)
}

/** Reads a little-endian ARM64 instruction at [offset] for expected-prologue declarations. */
internal fun arm64(vararg bytes: Int) = ByteArray(bytes.size) { bytes[it].toByte() }
