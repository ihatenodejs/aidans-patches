package app.finance.patches.sezzle.navigation

import app.finance.patches.sezzle.shared.Constants.COMPATIBILITY_SEZZLE
import app.finance.patches.sezzle.shared.HermesBundleEditor
import app.morphe.patcher.patch.PatchException
import app.morphe.patcher.patch.rawResourcePatch

@Suppress("unused")
val renameShopToHomePatch = rawResourcePatch(
    name = "Replace Shop with Home",
    description = "Replaces the Shop bottom navigation tab with Home and mounts Spending Power and Payment Streak below the search bar.",
    default = true
) {
    compatibleWith(COMPATIBILITY_SEZZLE)

    execute {
        val bundleFile = get("assets/index.android.bundle")
        if (!bundleFile.exists()) {
            throw PatchException("assets/index.android.bundle not found")
        }

        val editor = HermesBundleEditor(bundleFile.readBytes())

        // Rename tab title: replace string operand "navigation.Shop" with "navigation.Home"
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
        for (i in protectedStackOffset until protectedStackOffset + 15000) {
            // LoadConstString opcode is 0x90, format: [0x90, reg, id_low, id_high]
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

        // Replace the commercial feed element with PaymentStreakBanner. StoreRoot normally
        // constructs props for ScrollListView, so provide Spending Power from the live
        // sezzleUp.credit_limit object instead of retaining the feed props.
        val storeRootOffset = editor.findFunctionOffsetByName("StoreRoot")
            ?: throw PatchException("StoreRoot function not found")

        val feedPropsOffset = storeRootOffset + 16_215
        val expectedFeedProps = byteArrayOf(
            0x02, 0x55, 0xe3.toByte(), 0x48, 0x00, 0x00, 0x7f, 0x9a.toByte(), 0x09,
            0x00, 0x52, 0x55, 0x65, 0x00, 0x52, 0x55, 0x64, 0x01, 0x52,
            0x55, 0x63, 0x02, 0x52, 0x55, 0x62, 0x03, 0xb0.toByte(), 0x07,
            0x61, 0x3b, 0x60, 0x10, 0x45, 0x52, 0x55, 0x60, 0x04, 0x52,
            0x55, 0x5f, 0x05, 0x52, 0x55, 0x5e, 0x06, 0x52, 0x55, 0x5d,
            0x07, 0x52, 0x55, 0x5c, 0x08, 0x52, 0x55, 0x5b, 0x09, 0x52,
            0x55, 0x5a, 0x0b, 0x52, 0x55, 0x59, 0x0c, 0x52, 0x55, 0x58,
            0x0d
        )
        val paymentStreakProps = byteArrayOf(
            0x04, 0x55, // NewObject r85
            0x3b, 0x58, 0x1e, 0x08, // LoadFromEnvironment r88, r30, 8
            0x45, 0x58, 0x58, 0x68, 0xad.toByte(), 0x90.toByte(),
            // GetById r88, r88, cache104, stringId37037 (credit_limit)
            0x4a, 0x55, 0x58, 0x00, 0x53, 0xef.toByte(),
            // PutByIdLoose r85, r88, cache0, stringId61267 (sezzleUpCreditLimit)
            // 17 × Mov r0, r0 padding instructions preserve the 69-byte matched span.
            0x10, 0x00, 0x00, 0x10, 0x00, 0x00, 0x10, 0x00, 0x00,
            0x10, 0x00, 0x00, 0x10, 0x00, 0x00, 0x10, 0x00, 0x00,
            0x10, 0x00, 0x00, 0x10, 0x00, 0x00, 0x10, 0x00, 0x00,
            0x10, 0x00, 0x00, 0x10, 0x00, 0x00, 0x10, 0x00, 0x00,
            0x10, 0x00, 0x00, 0x10, 0x00, 0x00, 0x10, 0x00, 0x00,
            0x10, 0x00, 0x00, 0x10, 0x00, 0x00
        )
        if (!editor.matchesBytes(feedPropsOffset, expectedFeedProps)) {
            throw PatchException("Unexpected StoreRoot feed props instructions")
        }
        editor.patchBytesIfMatches(feedPropsOffset, expectedFeedProps, paymentStreakProps)


        val depPattern = byteArrayOf(
            0xe6.toByte(), 0x05, 0x00, 0x00, // dep[15] = 1510
            0x56, 0x0d, 0x00, 0x00, // dep[16] = 3414
            0x66, 0x2e, 0x00, 0x00, // dep[17] = 11878 (ScrollListView)
            0x69, 0x2e, 0x00, 0x00 // dep[18] = 11881
        )
        var depOffset = -1
        for (i in 0..(bytes.size - depPattern.size)) {
            if (depPattern.indices.all { j -> bytes[i + j] == depPattern[j] }) {
                depOffset = i
                break
            }
        }
        if (depOffset == -1) {
            throw PatchException("Could not find StoreRoot dependency pattern for ScrollListView")
        }
        bytes[depOffset + 8] = 0x29 // dep[17] = 8489 (PaymentStreakBanner)
        bytes[depOffset + 9] = 0x21
        bytes[depOffset + 10] = 0x00
        bytes[depOffset + 11] = 0x00
        editor.updateFooterHash()
        bundleFile.writeBytes(editor.toByteArray())
    }
}
