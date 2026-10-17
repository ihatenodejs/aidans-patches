package app.aidan.patches.adobescan.customization

import app.aidan.patches.adobescan.shared.COMPATIBILITY_ADOBE_SCAN
import app.morphe.patcher.extensions.InstructionExtensions.replaceInstruction
import app.morphe.patcher.patch.BytecodePatchContext
import app.morphe.patcher.patch.PatchException
import app.morphe.patcher.patch.booleanOption
import app.morphe.patcher.patch.bytecodePatch
import com.android.tools.smali.dexlib2.Opcode
import com.android.tools.smali.dexlib2.iface.instruction.FiveRegisterInstruction
import com.android.tools.smali.dexlib2.iface.instruction.NarrowLiteralInstruction
import com.android.tools.smali.dexlib2.iface.instruction.OneRegisterInstruction
import com.android.tools.smali.dexlib2.iface.instruction.ReferenceInstruction
import com.android.tools.smali.dexlib2.iface.reference.MethodReference
import com.android.tools.smali.dexlib2.iface.reference.StringReference

private const val TEMPLATE_CHIP = "Lcom/adobe/dcmscan/rename/d;"
private const val SHARE_UTIL = "Lcom/adobe/scan/android/util/b;"

private const val RES_ADOBE_SCAN_FILENAME = 0x7f142073
private const val RES_SHARE_VIA_EMAIL_PROMOTION_MESSAGE = 0x7f142844
private const val EXTRA_TEXT = "android.intent.extra.TEXT"

@Suppress("unused")
val removeAttributionPatch = bytecodePatch(
    name = "Remove Attribution",
    description = "Names new scans 'Scan on [date]' instead of 'Adobe Scan [date]' and removes the attribution text from shared files.",
    default = true
) {
    category("Customization")
    compatibleWith(COMPATIBILITY_ADOBE_SCAN)

    val removeScanAttribution = booleanOption(
        key = "removeScanAttribution",
        default = true,
        title = "Remove Scan Name Attribution",
        description = "Names new scans 'Scan on [date]' instead of 'Adobe Scan [date]'."
    )

    val removeShareAttribution = booleanOption(
        key = "removeShareAttribution",
        default = true,
        title = "Remove Share Attribution Text",
        description = "Removes the promotional 'Created and shared using Adobe Scan.' text from shared files."
    )

    execute {
        if (removeScanAttribution.value != false) {
            patchScanDefaultName()
        }
        if (removeShareAttribution.value != false) {
            patchShareAttribution()
        }
    }
}

/**
 * Patches TemplateChip.b to evaluate the default document scan type to "Scan on"
 * rather than "Adobe Scan", so composite scan names format as "Scan on [date]".
 */
private fun BytecodePatchContext.patchScanDefaultName() {
    val chipClass = mutableClassDefByOrNull(TEMPLATE_CHIP)
        ?: throw PatchException("TemplateChip class $TEMPLATE_CHIP not found")

    val bMethod = chipClass.methods.firstOrNull {
        it.name == "b" &&
            it.parameterTypes.map(CharSequence::toString) == listOf("Landroid/content/Context;", "Z") &&
            it.returnType == "Ljava/lang/String;" &&
            it.implementation != null
    } ?: throw PatchException("Method $TEMPLATE_CHIP.b not found")

    val instructions = bMethod.implementation?.instructions?.toList()
        ?: throw PatchException("Method $TEMPLATE_CHIP.b has no implementation")

    val adobeScanResIndex = instructions.indexOfFirst { inst ->
        inst is NarrowLiteralInstruction && inst.narrowLiteral == RES_ADOBE_SCAN_FILENAME
    }
    if (adobeScanResIndex < 0) {
        throw PatchException("Could not find RES_ADOBE_SCAN_FILENAME (0x${RES_ADOBE_SCAN_FILENAME.toString(16)}) in $TEMPLATE_CHIP.b")
    }

    // Locate the Context.getString call following the resource load
    val getStringIndex = (adobeScanResIndex + 1 until instructions.size).firstOrNull { idx ->
        val inst = instructions[idx]
        val ref = (inst as? ReferenceInstruction)?.reference as? MethodReference
        inst.opcode == Opcode.INVOKE_VIRTUAL &&
            ref?.name == "getString" &&
            ref.definingClass == "Landroid/content/Context;"
    } ?: throw PatchException("Could not find Context.getString call following RES_ADOBE_SCAN_FILENAME in $TEMPLATE_CHIP.b")

    val moveResultIndex = getStringIndex + 1
    val moveResultInst = instructions.getOrNull(moveResultIndex)
    if (moveResultInst == null || moveResultInst.opcode != Opcode.MOVE_RESULT_OBJECT) {
        throw PatchException("Expected move-result-object at index $moveResultIndex in $TEMPLATE_CHIP.b")
    }

    val destReg = (moveResultInst as OneRegisterInstruction).registerA
    bMethod.replaceInstruction(getStringIndex, "const-string v$destReg, \"Scan on\"")
    bMethod.replaceInstruction(moveResultIndex, "nop")
}

/**
 * Removes "Created and shared using Adobe Scan." from all sharing pathways in util.b:
 * - r: OS chooser / Send a copy / Send compressed (strips EXTRA_TEXT putExtra)
 * - q: Email attachment share (strips EXTRA_TEXT putExtra across primary and query intents)
 * - j: Multi-file formatted text builder (neutralizes promotion text appending)
 */
private fun BytecodePatchContext.patchShareAttribution() {
    val utilClass = mutableClassDefByOrNull(SHARE_UTIL)
        ?: throw PatchException("Share util class $SHARE_UTIL not found")

    // 1. Method r: OS Share sheet ("Send a copy", "Send compressed")
    val rMethod = utilClass.methods.firstOrNull {
        it.name == "r" &&
            it.parameterTypes.map(CharSequence::toString) == listOf(
                "Landroid/app/Activity;",
                "Ljava/util/List;",
                "Lbr/j\$e;",
                "Ljava/util/HashMap;"
            ) &&
            it.returnType == "Lk90/m;" &&
            it.implementation != null
    } ?: throw PatchException("Method $SHARE_UTIL.r not found")

    val rInstructions = rMethod.implementation?.instructions?.toList()
        ?: throw PatchException("Method $SHARE_UTIL.r has no implementation")

    val rExtraTextIndex = rInstructions.indexOfFirst { inst ->
        val ref = (inst as? ReferenceInstruction)?.reference as? StringReference
        inst.opcode == Opcode.CONST_STRING && ref?.string == EXTRA_TEXT
    }
    if (rExtraTextIndex < 0) {
        throw PatchException("Could not find EXTRA_TEXT in $SHARE_UTIL.r")
    }

    val rPutExtraIndex = (rExtraTextIndex + 1 until rInstructions.size).firstOrNull { idx ->
        val inst = rInstructions[idx]
        val ref = (inst as? ReferenceInstruction)?.reference as? MethodReference
        inst.opcode == Opcode.INVOKE_VIRTUAL &&
            ref?.name == "putExtra" &&
            ref.definingClass == "Landroid/content/Intent;"
    } ?: throw PatchException("Could not find putExtra following EXTRA_TEXT in $SHARE_UTIL.r")

    rMethod.replaceInstruction(rPutExtraIndex, "nop")

    // 2. Method q: Email attachment share
    val qMethod = utilClass.methods.firstOrNull {
        it.name == "q" &&
            it.parameterTypes.map(CharSequence::toString) == listOf(
                "Landroid/app/Activity;",
                "Ljava/util/List;",
                "Lbr/j\$e;",
                "Ljava/util/HashMap;"
            ) &&
            it.returnType == "Lk90/m;" &&
            it.implementation != null
    } ?: throw PatchException("Method $SHARE_UTIL.q not found")

    val qInstructions = qMethod.implementation?.instructions?.toList()
        ?: throw PatchException("Method $SHARE_UTIL.q has no implementation")

    val qExtraTextIndex = qInstructions.indexOfFirst { inst ->
        val ref = (inst as? ReferenceInstruction)?.reference as? StringReference
        inst.opcode == Opcode.CONST_STRING && ref?.string == EXTRA_TEXT
    }
    if (qExtraTextIndex < 0) {
        throw PatchException("Could not find EXTRA_TEXT in $SHARE_UTIL.q")
    }

    val textRegister = (qInstructions[qExtraTextIndex] as OneRegisterInstruction).registerA
    val qPutExtraIndices = (qExtraTextIndex + 1 until qInstructions.size).filter { idx ->
        val inst = qInstructions[idx]
        val ref = (inst as? ReferenceInstruction)?.reference as? MethodReference
        inst.opcode == Opcode.INVOKE_VIRTUAL &&
            inst is FiveRegisterInstruction &&
            inst.registerD == textRegister &&
            ref?.name == "putExtra" &&
            ref.definingClass == "Landroid/content/Intent;"
    }
    if (qPutExtraIndices.isEmpty()) {
        throw PatchException("Could not find putExtra calls for EXTRA_TEXT in $SHARE_UTIL.q")
    }
    for (idx in qPutExtraIndices) {
        qMethod.replaceInstruction(idx, "nop")
    }

    // 3. Method j: Formatted link/text generator
    val jMethod = utilClass.methods.firstOrNull {
        it.name == "j" &&
            it.parameterTypes.map(CharSequence::toString) == listOf("Ljava/util/HashMap;", "Z") &&
            it.returnType == "Ljava/lang/String;" &&
            it.implementation != null
    } ?: throw PatchException("Method $SHARE_UTIL.j not found")

    val jInstructions = jMethod.implementation?.instructions?.toList()
        ?: throw PatchException("Method $SHARE_UTIL.j has no implementation")

    val promoResIndex = jInstructions.indexOfFirst { inst ->
        inst is NarrowLiteralInstruction && inst.narrowLiteral == RES_SHARE_VIA_EMAIL_PROMOTION_MESSAGE
    }
    if (promoResIndex >= 0) {
        val promoAppendIndex = (promoResIndex + 1 until jInstructions.size).firstOrNull { idx ->
            val inst = jInstructions[idx]
            val ref = (inst as? ReferenceInstruction)?.reference as? MethodReference
            inst.opcode == Opcode.INVOKE_VIRTUAL &&
                ref?.name == "append" &&
                ref.definingClass == "Ljava/lang/StringBuilder;"
        }
        if (promoAppendIndex != null) {
            jMethod.replaceInstruction(promoAppendIndex, "nop")
        }

        val newlineAppendIndex = (0 until promoResIndex).reversed().firstOrNull { idx ->
            val inst = jInstructions[idx]
            val ref = (inst as? ReferenceInstruction)?.reference as? MethodReference
            inst.opcode == Opcode.INVOKE_VIRTUAL &&
                ref?.name == "append" &&
                ref.definingClass == "Ljava/lang/StringBuilder;"
        }
        if (newlineAppendIndex != null) {
            jMethod.replaceInstruction(newlineAppendIndex, "nop")
        }
    }
}
