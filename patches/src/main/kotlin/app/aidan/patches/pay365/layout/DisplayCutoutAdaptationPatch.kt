package app.aidan.patches.pay365.layout

import app.aidan.patches.pay365.shared.COMPATIBILITY_365_PAY
import app.morphe.patcher.extensions.InstructionExtensions.addInstructions
import app.morphe.patcher.patch.PatchException
import app.morphe.patcher.patch.bytecodePatch
import com.android.tools.smali.dexlib2.Opcode

private const val MAIN_ACTIVITY = "Lcom/avinside/MainActivity;"
private const val DISPLAY_CUTOUT_BRIDGE = "Lapp/aidan/extension/pay365/DisplayCutoutBridge;"

@Suppress("unused")
val displayCutoutAdaptationPatch = bytecodePatch(
    name = "Display Cutout Adaptation",
    description = "Dynamically adjusts header height and top padding on devices with camera cutouts or tall status bars so location selectors and action icons clear the camera hole.",
    default = true
) {
    category("General")
    compatibleWith(COMPATIBILITY_365_PAY)
    extendWith("extensions/extension.mpe")

    execute {
        val activity = mutableClassDefByOrNull(MAIN_ACTIVITY)
            ?: throw PatchException("365 Pay MainActivity not found: $MAIN_ACTIVITY")
        val onCreate = activity.methods.singleOrNull { method ->
            method.name == "onCreate" &&
                method.parameterTypes == listOf("Landroid/os/Bundle;") &&
                method.returnType == "V"
        } ?: throw PatchException("365 Pay MainActivity.onCreate(Bundle) not found")
        val implementation = onCreate.implementation
            ?: throw PatchException("365 Pay MainActivity.onCreate(Bundle) has no implementation")

        val returnIndex = implementation.instructions.indexOfLast { it.opcode == Opcode.RETURN_VOID }
        if (returnIndex < 0) {
            throw PatchException("Unexpected MainActivity.onCreate(Bundle) instruction list: no return-void found")
        }

        onCreate.addInstructions(
            returnIndex,
            "invoke-static {p0}, $DISPLAY_CUTOUT_BRIDGE->init(Landroid/app/Activity;)V"
        )
    }
}
