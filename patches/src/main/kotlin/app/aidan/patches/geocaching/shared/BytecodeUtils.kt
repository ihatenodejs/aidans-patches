package app.aidan.patches.geocaching.shared

import app.morphe.patcher.extensions.InstructionExtensions.addInstructions
import app.morphe.patcher.patch.PatchException
import com.android.tools.smali.dexlib2.Opcode
import com.android.tools.smali.dexlib2.iface.instruction.OneRegisterInstruction
import app.morphe.patcher.util.proxy.mutableTypes.MutableMethod
import com.android.tools.smali.dexlib2.iface.instruction.ReferenceInstruction
import com.android.tools.smali.dexlib2.iface.reference.MethodReference

/**
 * Finds all Ll3c;->d()Z call sites in [method] and forces their move-result register
 * to [value] (0x1 for true, 0x0 for false).
 */
fun patchAllL3cChecksToValue(
    method: MutableMethod,
    contextName: String,
    value: Boolean
) {
    val impl = method.implementation ?: throw PatchException("Missing implementation in $contextName")
    val instructions = impl.instructions.toList()

    // Collect indices of all MOVE_RESULT instructions that immediately follow an invoke-virtual Ll3c;->d()Z
    val patches = mutableListOf<Pair<Int, Int>>() // pair of (insertionIndex, registerNumber)
    var foundCall = false

    for (i in 0 until instructions.size - 1) {
        val current = instructions[i]
        val next = instructions[i + 1]

        val isL3cInvoke = current.opcode == Opcode.INVOKE_VIRTUAL &&
            (current as? ReferenceInstruction)?.reference?.let { ref ->
                (ref as? MethodReference)?.let { m ->
                    m.name == "d" && m.definingClass == "Ll3c;" && m.returnType == "Z"
                } == true
            } == true

        if (isL3cInvoke && next.opcode == Opcode.MOVE_RESULT) {
            foundCall = true
            val reg = (next as OneRegisterInstruction).registerA
            val after = instructions.getOrNull(i + 2)
            val expectedVal = if (value) 1 else 0
            val alreadyPatched = after?.opcode == Opcode.CONST_4 &&
                (after as? OneRegisterInstruction)?.registerA == reg &&
                (after as? com.android.tools.smali.dexlib2.iface.instruction.NarrowLiteralInstruction)?.narrowLiteral == expectedVal
            if (!alreadyPatched) {
                patches.add(Pair(i + 2, reg))
            }
        }
    }

    if (!foundCall) {
        throw PatchException("No Ll3c;->d()Z calls found in $contextName")
    }

    // Apply insertions from highest index to lowest so instruction indices remain valid
    val constVal = if (value) "0x1" else "0x0"
    for ((insertIdx, reg) in patches.asReversed()) {
        method.addInstructions(insertIdx, "const/4 v$reg, $constVal")
    }
}
