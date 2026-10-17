package app.aidan.patches.pay365.dev

import app.aidan.patches.pay365.shared.COMPATIBILITY_365_PAY
import app.aidan.patches.shared.HermesBundleEditor
import app.morphe.patcher.patch.PatchException
import app.morphe.patcher.patch.booleanOption
import app.morphe.patcher.patch.rawResourcePatch

private const val BUNDLE_PATH = "assets/index.android.bundle"

private const val LOGO_PRESS_FUNCTION_NAME = "_0x16a90c"
private const val WELCOME_SCREEN_CONTEXT_OFFSET = 0x98
private const val LOGO_TAP_COUNT_OFFSET = 0xa3

private const val INIT_STATE_FUNCTION_NAME = "_0x3844af"
private const val NULL_INVARIANT_OFFSET = 0x6d
private const val IS_DEBUG_OFFSET = 0x183
private const val DEMO_MODE_OFFSET = 0x1c0

private val WELCOME_SCREEN_CONTEXT_BYTES = byteArrayOf(0x73, 0x01, 0x96.toByte(), 0x6b)
private val EXPECTED_LOGO_TAP_COUNT_BYTES = byteArrayOf(0x6e, 0x01, 0x05, 0xb0.toByte(), 0x18, 0x05)
private val REPLACEMENT_LOGO_TAP_COUNT_BYTES = byteArrayOf(0x6e, 0x01, 0x01, 0xb0.toByte(), 0x18, 0x05)

private val LOAD_CONST_NULL_R9_BYTES = byteArrayOf(0x77, 0x09)
private val EXPECTED_IS_DEBUG_BYTES = byteArrayOf(0x0b, 0x0a, 0x0a)
private val REPLACEMENT_IS_DEBUG_BYTES = byteArrayOf(0x0b, 0x0a, 0x09)
private val EXPECTED_DEMO_MODE_BYTES = byteArrayOf(0x49, 0x0a, 0x0b, 0x0a)
private val REPLACEMENT_DEMO_MODE_BYTES = byteArrayOf(0x0e, 0x0a, 0x09, 0x09)

@Suppress("unused")
val unlockDeveloperToolsPatch = rawResourcePatch(
    name = "Unlock Developer Tools",
    description = "Exposes the internal developer and diagnostics screen (Environment Settings) on the Welcome Screen, and allows booting directly with Debug Messages or Demo Mode enabled.",
    default = false
) {
    category("Developer")
    compatibleWith(COMPATIBILITY_365_PAY)

    val unlockDeveloperSettings = booleanOption(
        key = "unlockDeveloperSettings",
        default = true,
        title = "Unlock Developer Settings",
        description = "Reduces the hidden Welcome Screen logo tap gesture from five taps to a single tap to open the Environment Settings menu."
    )

    val startWithDebugMessages = booleanOption(
        key = "startWithDebugMessages",
        default = false,
        title = "Start with Debug Messages",
        description = "Initializes application state with Debug Messages enabled upon startup."
    )

    val startInDemoMode = booleanOption(
        key = "startInDemoMode",
        default = false,
        title = "Start in Demo Mode",
        description = "Initializes application state with Demo Mode enabled upon startup."
    )

    execute {
        val shouldUnlockDev = unlockDeveloperSettings.value != false
        val shouldStartDebug = startWithDebugMessages.value != false
        val shouldStartDemo = startInDemoMode.value != false

        if (!shouldUnlockDev && !shouldStartDebug && !shouldStartDemo) {
            return@execute
        }

        val bundleFile = get(BUNDLE_PATH)
        if (!bundleFile.exists()) {
            throw PatchException("$BUNDLE_PATH not found")
        }

        val editor = HermesBundleEditor(bundleFile.readBytes())
        if (editor.version != 96) {
            throw PatchException("Unsupported Hermes bytecode version: ${editor.version}, expected 96")
        }

        if (shouldUnlockDev) {
            val logoPressOffsets = editor.findFunctionOffsetsByName(LOGO_PRESS_FUNCTION_NAME)
            if (logoPressOffsets.size != 1) {
                throw PatchException(
                    "Expected exactly one function named $LOGO_PRESS_FUNCTION_NAME, found ${logoPressOffsets.size}"
                )
            }
            val logoPressOffset = logoPressOffsets.single()

            val welcomeContextOffset = logoPressOffset + WELCOME_SCREEN_CONTEXT_OFFSET
            if (!editor.matchesBytes(welcomeContextOffset, WELCOME_SCREEN_CONTEXT_BYTES)) {
                throw PatchException(
                    "WelcomeScreen context check failed at offset 0x${Integer.toHexString(welcomeContextOffset)}"
                )
            }

            val logoTapOffset = logoPressOffset + LOGO_TAP_COUNT_OFFSET
            if (editor.matchesBytes(logoTapOffset, EXPECTED_LOGO_TAP_COUNT_BYTES)) {
                editor.patchBytes(logoTapOffset, REPLACEMENT_LOGO_TAP_COUNT_BYTES)
            } else if (!editor.matchesBytes(logoTapOffset, REPLACEMENT_LOGO_TAP_COUNT_BYTES)) {
                throw PatchException(
                    "Unexpected logo tap count check at offset 0x${Integer.toHexString(logoTapOffset)}"
                )
            }
        }

        if (shouldStartDebug || shouldStartDemo) {
            val initStateOffsets = editor.findFunctionOffsetsByName(INIT_STATE_FUNCTION_NAME)
            if (initStateOffsets.size != 1) {
                throw PatchException(
                    "Expected exactly one function named $INIT_STATE_FUNCTION_NAME, found ${initStateOffsets.size}"
                )
            }
            val initStateOffset = initStateOffsets.single()

            val nullInvariantOffset = initStateOffset + NULL_INVARIANT_OFFSET
            if (!editor.matchesBytes(nullInvariantOffset, LOAD_CONST_NULL_R9_BYTES)) {
                throw PatchException(
                    "Null register invariant check failed at offset 0x${Integer.toHexString(nullInvariantOffset)}"
                )
            }

            if (shouldStartDebug) {
                val isDebugOffset = initStateOffset + IS_DEBUG_OFFSET
                if (editor.matchesBytes(isDebugOffset, EXPECTED_IS_DEBUG_BYTES)) {
                    editor.patchBytes(isDebugOffset, REPLACEMENT_IS_DEBUG_BYTES)
                } else if (!editor.matchesBytes(isDebugOffset, REPLACEMENT_IS_DEBUG_BYTES)) {
                    throw PatchException(
                        "Unexpected isDebug initialization bytecode at offset 0x${Integer.toHexString(isDebugOffset)}"
                    )
                }
            }

            if (shouldStartDemo) {
                val demoModeOffset = initStateOffset + DEMO_MODE_OFFSET
                if (editor.matchesBytes(demoModeOffset, EXPECTED_DEMO_MODE_BYTES)) {
                    editor.patchBytes(demoModeOffset, REPLACEMENT_DEMO_MODE_BYTES)
                } else if (!editor.matchesBytes(demoModeOffset, REPLACEMENT_DEMO_MODE_BYTES)) {
                    throw PatchException(
                        "Unexpected demo mode initialization bytecode at offset 0x${Integer.toHexString(demoModeOffset)}"
                    )
                }
            }
        }

        editor.updateFooterHash()
        bundleFile.writeBytes(editor.toByteArray())
    }
}
