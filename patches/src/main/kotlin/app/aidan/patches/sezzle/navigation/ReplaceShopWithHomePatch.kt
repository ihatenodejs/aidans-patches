package app.aidan.patches.sezzle.navigation

import app.aidan.patches.sezzle.shared.Constants.COMPATIBILITY_SEZZLE
import app.aidan.patches.shared.HermesBundleEditor
import app.morphe.patcher.patch.PatchException
import app.morphe.patcher.patch.rawResourcePatch

private const val STORE_ROOT_LAYOUT_OFFSET = 0x451A
private const val STORE_ROOT_GLASS_BRANCH_OFFSET = 0x44D1
private const val STORE_ROOT_LAYOUT_SIZE = 144

private val EXPECTED_STORE_ROOT_GLASS_BRANCH_BYTES = byteArrayOf(
    0xb1.toByte(), 0xd9.toByte(), 0x00, 0x00, 0x00, 0x62
)
private val STORE_ROOT_GLASS_BRANCH_DISABLED_BYTES = ByteArray(6) { 0x7e }

private val EXPECTED_STORE_ROOT_DEPENDENCIES = byteArrayOf(
    0x30, 0x0e, 0x00, 0x00, // dep[15] = 3632
    0xae.toByte(), 0x05, 0x00, 0x00, // dep[16] = 1454
    0x2f, 0x0e, 0x00, 0x00, // dep[17] = 3631
    0x4a, 0x35, 0x00, 0x00, // dep[18] = 13642
    0x4d, 0x35, 0x00, 0x00 // dep[19] = 13645
)
private val STORE_ROOT_DEPENDENCIES_REPLACEMENT = byteArrayOf(
    0x30, 0x0e, 0x00, 0x00, // dep[15] = 3632
    0xae.toByte(), 0x05, 0x00, 0x00, // dep[16] = 1454
    0x63, 0x30, 0x00, 0x00, // dep[17] = 12387 (OrdersListHeader)
    0x39, 0x27, 0x00, 0x00, // dep[18] = 10041 (PaymentStreakBanner)
    0x10, 0x32, 0x00, 0x00 // dep[19] = 12816 (ShopTabShortcutsRow)
)

private val EXPECTED_STORE_ROOT_LAYOUT_PREFIX = byteArrayOf(
    0x08, 0x64, 0x02, 0x00, // NewArray r100, 2
    0x5a, 0x64, 0x62, 0x00 // DefineOwnInDenseArray r100, r98, 0
)

private val CUSTOM_HOME_LAYOUT = byteArrayOf(
    // [0] Existing StoreRoot header
    0x08, 0x64, 0x04, 0x00, // NewArray r100, 4
    0x5a, 0x64, 0x62, 0x00, // DefineOwnInDenseArray r100, r98, 0

    // [1] OrdersListHeader (env[16])
    0x44, 0x85.toByte(), 0x1b, 0xff.toByte(), 0x9e.toByte(), // GetByIdShort r133, r27, 255, jsx
    0x04, 0x78, // NewObject r120
    0x3b, 0x62, 0x1a, 0x10, // LoadFromEnvironment r98, r26, 16
    0x44, 0x81.toByte(), 0x62, 0x1d, 0x6d, // GetByIdShort r129, r98, 29, default
    0x6f, 0x62, 0x85.toByte(), 0x09, 0x81.toByte(), 0x78, // Call3 r98, r133, r9, r129, r120
    0x5a, 0x64, 0x62, 0x01, // DefineOwnInDenseArray r100, r98, 1

    // [2] PaymentStreakBanner (env[17])
    0x04, 0x78, // NewObject r120
    0x3b, 0x62, 0x1a, 0x11, // LoadFromEnvironment r98, r26, 17
    0x44, 0x81.toByte(), 0x62, 0x1d, 0x6d, // GetByIdShort r129, r98, 29, default
    0x6f, 0x62, 0x85.toByte(), 0x09, 0x81.toByte(), 0x78, // Call3 r98, r133, r9, r129, r120
    0x5a, 0x64, 0x62, 0x02, // DefineOwnInDenseArray r100, r98, 2

    // [3] ShopTabShortcutsRow (env[18])
    0x04, 0x78, // NewObject r120
    0x3b, 0x62, 0x1a, 0x12, // LoadFromEnvironment r98, r26, 18
    0x44, 0x81.toByte(), 0x62, 0x1d, 0x6d, // GetByIdShort r129, r98, 29, default
    0x6f, 0x62, 0x85.toByte(), 0x09, 0x81.toByte(), 0x78, // Call3 r98, r133, r9, r129, r120
    0x5a, 0x64, 0x62, 0x03, // DefineOwnInDenseArray r100, r98, 3

    // Return the composed children through jsxs(Fragment, {children: array})
    0x02, 0x78, 0xd0.toByte(), 0x25, 0x00, 0x00, 0x72, 0xfd.toByte(), 0x03, 0x00, // NewObjectWithBufferLong r120, 9680, 261490
    0x52, 0x78, 0x64, 0x00, // PutOwnBySlotIdx r120, r100, 0
    0x6f, 0x63, 0x65, 0x09, 0x63, 0x78, // Call3 r99, r101, r9, r99, r120
    0xaf.toByte(), 0x5e, 0x01, 0x00, 0x00 // JmpLong -> +0x46d8
).padTo(STORE_ROOT_LAYOUT_SIZE)

private fun ByteArray.padTo(size: Int): ByteArray {
    require(this.size <= size) { "Custom Home layout (${this.size}) exceeds $size bytes" }
    return copyOf(size).also { bytes ->
        for (index in this.size until size) bytes[index] = 0x7e
    }
}

@Suppress("unused")
val renameShopToHomePatch = rawResourcePatch(
    name = "Replace Shop with Home",
    description = "Replaces the Shop bottom navigation tab with Home, a custom screen to replace the overly-commercial Shop screen.",
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

        // 1. Rename tab title: replace string operand "navigation.Shop" with "navigation.Home" in ProtectedStack
        val protectedStackOffset = editor.findFunctionOffsetByName("ProtectedStack")
            ?: throw PatchException("ProtectedStack function not found")

        val oldTitleId = editor.findStringId("navigation.Shop")
            ?: throw PatchException("String 'navigation.Shop' not found")
        val newTitleId = editor.findStringId("navigation.Home")
            ?: throw PatchException("String 'navigation.Home' not found")

        val bytes = editor.toByteArray()

        val oldTitleBytes = byteArrayOf(
            (oldTitleId and 0xFF).toByte(),
            ((oldTitleId ushr 8) and 0xFF).toByte()
        )
        val newTitleBytes = byteArrayOf(
            (newTitleId and 0xFF).toByte(),
            ((newTitleId ushr 8) and 0xFF).toByte()
        )

        // Locate LoadConstString for navigation.Shop within ProtectedStack bytecode
        var titlePatched = false
        for (i in protectedStackOffset until minOf(protectedStackOffset + 15_000, bytes.size - 3)) {
            // LoadConstString opcode is 0x90, format: [0x90, reg, id_low, id_high].
            if ((bytes[i].toInt() and 0xFF) == 0x90 &&
                bytes[i + 2] == oldTitleBytes[0] &&
                bytes[i + 3] == oldTitleBytes[1]
            ) {
                bytes[i + 2] = newTitleBytes[0]
                bytes[i + 3] = newTitleBytes[1]
                titlePatched = true
                break
            }
        }
        if (!titlePatched) {
            throw PatchException("Could not find LoadConstString instruction for navigation.Shop in ProtectedStack")
        }

        // 2. Redirect StoreRoot dependencies to the clean Home components.
        var storeRootDepOffset = -1
        for (index in 0..(bytes.size - EXPECTED_STORE_ROOT_DEPENDENCIES.size)) {
            if (EXPECTED_STORE_ROOT_DEPENDENCIES.indices.all { offset ->
                    bytes[index + offset] == EXPECTED_STORE_ROOT_DEPENDENCIES[offset]
                }
            ) {
                storeRootDepOffset = index
                break
            }
        }
        if (storeRootDepOffset == -1) {
            throw PatchException("Could not find StoreRoot dependency pattern")
        }
        editor.patchBytesIfMatches(
            storeRootDepOffset,
            EXPECTED_STORE_ROOT_DEPENDENCIES,
            STORE_ROOT_DEPENDENCIES_REPLACEMENT
        )

        // 3. Always take the non-Liquid-Glass StoreRoot render branch.
        val storeRootOffset = editor.findFunctionOffsetByName("StoreRoot")
            ?: throw PatchException("StoreRoot function not found")
        editor.patchBytesIfMatches(
            storeRootOffset + STORE_ROOT_GLASS_BRANCH_OFFSET,
            EXPECTED_STORE_ROOT_GLASS_BRANCH_BYTES,
            STORE_ROOT_GLASS_BRANCH_DISABLED_BYTES
        )

        // 4. Replace the commercial feed composition with the clean Home layout.
        //    [0] Header, [1] OrdersListHeader, [2] PaymentStreakBanner, [3] ShopTabShortcutsRow
        val layoutOffset = storeRootOffset + STORE_ROOT_LAYOUT_OFFSET
        if (!editor.matchesBytes(layoutOffset, EXPECTED_STORE_ROOT_LAYOUT_PREFIX)) {
            throw PatchException("Unexpected StoreRoot feed layout bytecode")
        }
        editor.patchBytes(layoutOffset, CUSTOM_HOME_LAYOUT)

        editor.updateFooterHash()
        bundleFile.writeBytes(editor.toByteArray())
    }
}
