package app.aidan.patches.geocaching.shared

import app.morphe.patcher.extensions.InstructionExtensions.addInstructions
import app.morphe.patcher.extensions.InstructionExtensions.replaceInstruction
import app.morphe.patcher.patch.PatchException
import app.morphe.patcher.util.proxy.mutableTypes.MutableMethod
import com.android.tools.smali.dexlib2.Opcode
import com.android.tools.smali.dexlib2.iface.instruction.OneRegisterInstruction
import com.android.tools.smali.dexlib2.iface.instruction.ReferenceInstruction
import com.android.tools.smali.dexlib2.iface.reference.FieldReference
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

/**
 * In FilterPreferences.h (dk3.h), replaces the hardcoded `sget-object v15, Ljava/lang/Boolean;->TRUE:Ljava/lang/Boolean;`
 * argument for `includeOwnedDisabledCaches` with `const/4 v15, 0x0` (null).
 *
 * This prevents the Geocaching server from rejecting map searches with HTTP 403 Forbidden,
 * while allowing all user-configured filters (cache types, sizes, difficulty, terrain, attributes, etc.)
 * to be sent and applied cleanly.
 */
fun patchIncludeOwnedDisabledCachesToNull(method: MutableMethod) {
    val impl = method.implementation ?: return
    val instructions = impl.instructions.toList()
    for ((index, instruction) in instructions.withIndex()) {
        if (instruction.opcode == Opcode.SGET_OBJECT) {
            val ref = (instruction as? ReferenceInstruction)?.reference as? FieldReference
            if (ref?.definingClass == "Ljava/lang/Boolean;" && ref.name == "TRUE" && ref.type == "Ljava/lang/Boolean;") {
                val reg = (instruction as OneRegisterInstruction).registerA
                method.replaceInstruction(index, "const/4 v$reg, 0x0")
                return
            }
        }
    }
}

/**
 * In FilterPreferences.h (dk3.h), injects GeocacheFilterBridge.sanitizeFilterModel(ck3)
 * right before returning the ck3 object.
 *
 * This sanitizes inactive toggle filters (hideMyFinds, hideMyCaches, emptyGridsOnly, etc.)
 * from Boolean.FALSE to null, and minFavorites from 0 to null, so that the server does not
 * reject basic account map search requests with HTTP 403 Forbidden.
 */
fun patchSanitizeFilterModel(method: MutableMethod) {
    val impl = method.implementation ?: return
    val instructions = impl.instructions.toList()

    val alreadyPatched = instructions.any { inst ->
        (inst as? ReferenceInstruction)?.reference?.toString()?.contains("sanitizeFilterModel") == true
    }
    if (alreadyPatched) return

    val returnIdx = instructions.indexOfFirst { inst ->
        inst.opcode == Opcode.RETURN_OBJECT &&
            (
                instructions.getOrNull(instructions.indexOf(inst) - 1)?.opcode == Opcode.INVOKE_DIRECT_RANGE ||
                    instructions.getOrNull(instructions.indexOf(inst) - 1)?.opcode == Opcode.INVOKE_DIRECT
                )
    }
    if (returnIdx != -1) {
        val returnInst = instructions[returnIdx] as OneRegisterInstruction
        val reg = returnInst.registerA
        method.addInstructions(
            returnIdx,
            """
            invoke-static {v$reg}, Lapp/aidan/extension/geocaching/GeocacheFilterBridge;->sanitizeFilterModel(Ljava/lang/Object;)Ljava/lang/Object;
            move-result-object v$reg
            check-cast v$reg, Lck3;
            """.trimIndent()
        )
    }
}

/**
 * In cz9.a, if all cache types in ck3.a are selected (or none), sets `cacheTypes` to null
 * so the `cacheTypes` query parameter is omitted from `/mobile/v2/map/search`.
 *
 * This prevents the Geocaching server from rejecting searches with HTTP 403 Forbidden when
 * a basic account searches with all cache types enabled (which includes non-traditional/premium types).
 */
fun patchCz9OmitAllCacheTypes(method: MutableMethod) {
    val impl = method.implementation ?: return
    val instructions = impl.instructions.toList()

    val alreadyPatched = instructions.any { inst ->
        (inst as? ReferenceInstruction)?.reference?.toString()?.contains("GeocacheFilterBridge") == true
    }
    if (alreadyPatched) return

    for (i in 0 until instructions.size - 1) {
        val curr = instructions[i]
        val next = instructions[i + 1]
        val isGoto = curr.opcode.name.startsWith("goto")
        val isMoveToV5 = next.opcode.name.startsWith("move-object") &&
            (next as? OneRegisterInstruction)?.registerA == 5
        if (isGoto && isMoveToV5) {
            val prev = instructions.getOrNull(i - 1)
            val isMoveNull = prev?.opcode?.name?.startsWith("move-object") == true &&
                (prev as? OneRegisterInstruction)?.registerA == 5
            if (isMoveNull) {
                method.replaceInstruction(i + 1, "move-object/from16 v1, p1")
                method.addInstructions(
                    i + 2,
                    """
                    invoke-static {v1, v0}, Lapp/aidan/extension/geocaching/GeocacheFilterBridge;->sanitizeCacheTypes(Ljava/util/List;Ljava/lang/Object;)Ljava/util/List;
                    move-result-object v5
                    """.trimIndent()
                )
                break
            }
        }
    }

    val refreshed = method.implementation?.instructions?.toList() ?: return
    val initIdx = refreshed.indexOfFirst { inst ->
        inst.opcode == Opcode.INVOKE_DIRECT_RANGE &&
            (inst as? ReferenceInstruction)?.reference?.let { ref ->
                (ref as? MethodReference)?.let { m ->
                    m.definingClass == "Lce4;" && m.name == "<init>"
                }
            } == true
    }
    if (initIdx != -1) {
        method.addInstructions(
            initIdx,
            """
            const/4 v6, 0x0
            const/4 v7, 0x0
            const/4 v8, 0x0
            const/4 v9, 0x0
            const/4 v10, 0x0
            const/4 v11, 0x0
            const/4 v12, 0x0
            const/4 v13, 0x0
            const/4 v14, 0x0
            const/4 v15, 0x0
            const/16 v16, 0x0
            """.trimIndent()
        )
    }
}
