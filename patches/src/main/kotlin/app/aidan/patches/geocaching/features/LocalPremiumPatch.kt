package app.aidan.patches.geocaching.features

import app.aidan.patches.geocaching.shared.COMPATIBILITY_GEOCACHING
import app.aidan.patches.geocaching.shared.patchAllL3cChecksToValue
import app.morphe.patcher.extensions.InstructionExtensions.addInstructions
import app.morphe.patcher.extensions.InstructionExtensions.replaceInstruction
import app.morphe.patcher.patch.BytecodePatchContext
import app.morphe.patcher.patch.PatchException
import app.morphe.patcher.patch.bytecodePatch
import com.android.tools.smali.dexlib2.Opcode
import com.android.tools.smali.dexlib2.builder.instruction.BuilderInstruction30t
import com.android.tools.smali.dexlib2.iface.instruction.NarrowLiteralInstruction
import com.android.tools.smali.dexlib2.iface.instruction.OneRegisterInstruction
import com.android.tools.smali.dexlib2.iface.instruction.ReferenceInstruction
import com.android.tools.smali.dexlib2.iface.reference.FieldReference
import com.android.tools.smali.dexlib2.iface.reference.MethodReference
import com.android.tools.smali.dexlib2.iface.reference.StringReference

val localPremiumPatch = bytecodePatch(
    name = "Local Premium",
    description = "Enables local Premium membership status across profile and account screens, and removes upgrade promotions, banners, and icons.",
    default = true
) {
    category("Interface")
    compatibleWith(COMPATIBILITY_GEOCACHING)

    execute {
        patchProfileMembershipLabel()
        patchMapUpgradeIcon()
        patchProfileUpgradeCard()
        patchSettingsFragment()
        patchAccountFragment()
    }
}

/**
 * Changes the membership title in the Profile header from the server-provided Basic label to
 * "Premium member".
 */
private fun BytecodePatchContext.patchMembershipStrings() {
    val hb5Class = mutableClassDefByOrNull("Lhb5;") ?: return
    val iMethod = hb5Class.methods.firstOrNull {
        it.name == "i" && it.returnType == "I" && it.parameterTypes == listOf("I", "I") && it.implementation != null
    } ?: return

    val impl = iMethod.implementation ?: return
    val instructions = impl.instructions.toList()

    for ((index, instruction) in instructions.withIndex()) {
        if ((instruction as? NarrowLiteralInstruction)?.narrowLiteral == 0x7f14068e) {
            val reg = (instruction as OneRegisterInstruction).registerA
            val after = instructions.getOrNull(index + 1)
            val alreadyPatched = after?.opcode == Opcode.CONST &&
                (after as? OneRegisterInstruction)?.registerA == reg &&
                (after as? NarrowLiteralInstruction)?.narrowLiteral == 0x7f140690
            if (!alreadyPatched) {
                iMethod.addInstructions(index + 1, "const v$reg, 0x7f140690")
            }
        }
    }
}

private fun BytecodePatchContext.patchL3cIsPremium() {
    val l3cClass = mutableClassDefByOrNull("Ll3c;") ?: return
    val dMethod = l3cClass.methods.firstOrNull {
        it.name == "d" && it.returnType == "Z" && it.parameterTypes.isEmpty() && it.implementation != null
    } ?: return

    val firstInst = dMethod.implementation?.instructions?.firstOrNull()
    val isAlreadyPatched = firstInst?.opcode == Opcode.CONST_4 &&
        (firstInst as? OneRegisterInstruction)?.registerA == 0 &&
        (firstInst as? NarrowLiteralInstruction)?.narrowLiteral == 1
    if (!isAlreadyPatched) {
        dMethod.addInstructions(0, "const/4 v0, 0x1\nreturn v0")
    }
}

private fun BytecodePatchContext.patchProfileModelMembershipType() {
    val profileClass = mutableClassDefByOrNull("Lcom/groundspeak/geocaching/intro/network/api/profile/OwnProfileResponse\$Profile;") ?: return
    val iMethod = profileClass.methods.firstOrNull {
        it.name == "i" && it.returnType == "I" && it.parameterTypes.isEmpty() && it.implementation != null
    } ?: return

    val firstInst = iMethod.implementation?.instructions?.firstOrNull()
    val isAlreadyPatched = firstInst?.opcode == Opcode.CONST_4 &&
        (firstInst as? OneRegisterInstruction)?.registerA == 0 &&
        (firstInst as? NarrowLiteralInstruction)?.narrowLiteral == 3
    if (!isAlreadyPatched) {
        iMethod.addInstructions(0, "const/4 v0, 0x3\nreturn v0")
    }
}

private fun BytecodePatchContext.patchL3cMembershipType() {
    val l3cClass = mutableClassDefByOrNull("Ll3c;") ?: return
    val sMethod = l3cClass.methods.firstOrNull {
        it.name == "s" && it.returnType == "V" && it.parameterTypes == listOf("I") && it.implementation != null
    } ?: return

    val impl = sMethod.implementation ?: return
    val p1Reg = impl.registerCount - 1
    val firstInst = impl.instructions.firstOrNull()
    val isAlreadyPatched = firstInst?.opcode == Opcode.CONST_4 &&
        (firstInst as? OneRegisterInstruction)?.registerA == p1Reg &&
        (firstInst as? NarrowLiteralInstruction)?.narrowLiteral == 3
    if (!isAlreadyPatched) {
        sMethod.addInstructions(0, "const/4 v$p1Reg, 0x3")
    }
}

/**
 * Changes the membership title in the Profile header from the server-provided Basic label to
 * "Premium member".
 */
private fun BytecodePatchContext.patchProfileMembershipLabel() {
    patchMembershipStrings()
    patchL3cIsPremium()
    patchL3cMembershipType()
    patchProfileModelMembershipType()
    val profileMapper = mutableClassDefByOrNull("Lcom/groundspeak/geocaching/intro/profile/o;") ?: return
    val mapProfileMethod = profileMapper.methods.firstOrNull {
        it.name == "a" &&
            it.parameterTypes.singleOrNull() == "Lcom/groundspeak/geocaching/intro/network/api/profile/OwnProfileResponse;" &&
            it.returnType == "Lz09;" &&
            it.implementation != null
    } ?: return

    val instructions = mapProfileMethod.implementation!!.instructions.toList()

    // 1. Force hb5.i result register (membership string resource ID) to 0x7f140690 (R.string.member_type_premium)
    for (i in 0 until instructions.size - 1) {
        val curr = instructions[i]
        val next = instructions[i + 1]
        val isHb5I = curr.opcode == Opcode.INVOKE_STATIC &&
            (curr as? ReferenceInstruction)?.reference?.let { ref ->
                (ref as? MethodReference)?.let { m ->
                    m.name == "i" && m.definingClass == "Lhb5;" && m.returnType == "I"
                } == true
            } == true
        if (isHb5I && next.opcode == Opcode.MOVE_RESULT) {
            val reg = (next as OneRegisterInstruction).registerA
            val after = instructions.getOrNull(i + 2)
            val alreadyPatched = after?.opcode == Opcode.CONST &&
                (after as? OneRegisterInstruction)?.registerA == reg &&
                (after as? NarrowLiteralInstruction)?.narrowLiteral == 0x7f140690
            if (!alreadyPatched) {
                mapProfileMethod.addInstructions(i + 2, "const v$reg, 0x7f140690")
            }
        }
    }

    // 2. Also ensure OwnProfileResponse.Profile.k() returns "Premium member"
    val kIndex = instructions.indexOfFirst { instruction ->
        instruction.opcode == Opcode.INVOKE_VIRTUAL &&
            (instruction as? ReferenceInstruction)?.reference?.let { reference ->
                (reference as? MethodReference)?.let { method ->
                    method.definingClass == "Lcom/groundspeak/geocaching/intro/network/api/profile/OwnProfileResponse\$Profile;" &&
                        method.name == "k" &&
                        method.returnType == "Ljava/lang/String;"
                } == true
            } == true
    }
    if (kIndex >= 0 && instructions.getOrNull(kIndex + 1)?.opcode == Opcode.MOVE_RESULT_OBJECT) {
        val reg = (instructions[kIndex + 1] as OneRegisterInstruction).registerA
        val after = instructions.getOrNull(kIndex + 2)
        val alreadyPatched = after?.opcode == Opcode.CONST_STRING &&
            (after as? ReferenceInstruction)?.reference?.let { ref ->
                (ref as? StringReference)?.string == "Premium member"
            } == true
        if (!alreadyPatched) {
            mapProfileMethod.addInstructions(kIndex + 2, "const-string v$reg, \"Premium member\"")
        }
    }
}

/**
 * Removes the upgrade canister and pill button from the top left of the map screen
 * by ensuring SharedMapViewModel initializes and maintains r0 (upgrade button state)
 * as false, and forcing the map UI composable to bypass the upgrade button container.
 */
private fun BytecodePatchContext.patchMapUpgradeIcon() {
    val viewModelClass = mutableClassDefBy("Lcom/groundspeak/geocaching/intro/mainmap/map/SharedMapViewModel;")
    val initMethod = viewModelClass.methods.firstOrNull {
        it.name == "<init>" && it.implementation != null
    } ?: throw PatchException("SharedMapViewModel.<init> not found")
    patchAllL3cChecksToValue(initMethod, "SharedMapViewModel.<init>", true)

    val aMethod = viewModelClass.methods.firstOrNull {
        it.name == "A" && it.implementation != null && it.parameterTypes == listOf("Lcom/groundspeak/geocaching/intro/mainmap/map/MapMode;")
    } ?: throw PatchException("SharedMapViewModel.A not found")
    patchAllL3cChecksToValue(aMethod, "SharedMapViewModel.A", true)
    val mapUiClass = mutableClassDefBy("Lcom/groundspeak/geocaching/intro/mainmap/map/i;")
    val bMethod = mapUiClass.methods.firstOrNull {
        it.name == "b" && it.returnType == "V" && it.implementation != null
    } ?: throw PatchException("com.groundspeak.geocaching.intro.mainmap.map.i.b not found")

    val impl = bMethod.implementation ?: throw PatchException("Missing implementation in map i.b")
    val instructions = impl.instructions.toList()

    var patched = false
    for (i in 0 until instructions.size - 1) {
        val curr = instructions[i]
        val next = instructions[i + 1]
        val isBooleanValue = curr.opcode == Opcode.INVOKE_VIRTUAL &&
            (curr as? ReferenceInstruction)?.reference?.let { ref ->
                (ref as? MethodReference)?.let { m ->
                    m.name == "booleanValue" && m.definingClass == "Ljava/lang/Boolean;" && m.returnType == "Z"
                } == true
            } == true

        if (isBooleanValue && next.opcode == Opcode.MOVE_RESULT) {
            val reg = (next as OneRegisterInstruction).registerA
            val after = instructions.getOrNull(i + 2)
            val alreadyPatched = after?.opcode == Opcode.CONST_4 &&
                (after as? OneRegisterInstruction)?.registerA == reg
            if (!alreadyPatched) {
                bMethod.addInstructions(i + 2, "const/4 v$reg, 0x0")
            }
            patched = true
            break
        }
    }
    if (!patched) {
        throw PatchException("Could not find Boolean.booleanValue() in map i.b")
    }
}

/**
 * Removes the "Upgrade to premium" promotional card section at the bottom of the Profile screen
 * while leaving the Statistics item completely untouched.
 */
private fun BytecodePatchContext.patchProfileUpgradeCard() {
    val x29Class = mutableClassDefBy("Lx29;")
    val invokeMethod = x29Class.methods.firstOrNull {
        it.name == "invoke" && it.implementation != null && it.parameterTypes.size == 3
    } ?: throw PatchException("x29.invoke not found")
    patchAllL3cChecksToValue(invokeMethod, "x29.invoke", true)

    val profileClass = mutableClassDefBy("Lcom/groundspeak/geocaching/intro/profile/l;")
    val bMethod = profileClass.methods.firstOrNull {
        it.name == "b" && it.returnType == "V" && it.implementation != null && it.parameterTypes.size == 13
    } ?: throw PatchException("com.groundspeak.geocaching.intro.profile.l.b not found")

    val impl = bMethod.implementation ?: throw PatchException("Missing implementation in profile l.b")
    val p5Reg = impl.registerCount - bMethod.parameters.size + 5
    val instructions = impl.instructions.toList()

    val branchIndex = instructions.indexOfFirst { instruction ->
        instruction.opcode == Opcode.IF_NEZ &&
            (instruction as? OneRegisterInstruction)?.registerA == p5Reg
    }
    if (branchIndex >= 0) {
        val prev = instructions.getOrNull(branchIndex - 1)
        val alreadyPatched = prev?.opcode == Opcode.CONST_16 &&
            (prev as? OneRegisterInstruction)?.registerA == p5Reg
        if (!alreadyPatched) {
            bMethod.addInstructions(branchIndex, "const/16 v$p5Reg, 0x1")
        }
    }
}

/**
 * Removes the "Upgrade to Premium" item from the Settings screen.
 */
private fun BytecodePatchContext.patchSettingsFragment() {
    val settingsClass = mutableClassDefBy("Lcom/groundspeak/geocaching/intro/fragments/settings/SettingsFragment;")
    val vMethod = settingsClass.methods.firstOrNull {
        it.name == "v" && it.returnType == "V" && it.implementation != null && it.parameterTypes.size == 3
    } ?: throw PatchException("SettingsFragment.v not found")

    patchAllL3cChecksToValue(vMethod, "SettingsFragment.v", true)
}

/**
 * Changes "Membership type: Basic" to "Membership type: Premium" on the Account screen
 * and removes the "Upgrade to Premium" button item.
 */
private fun BytecodePatchContext.patchAccountFragment() {
    val accountClass = mutableClassDefBy("Lcom/groundspeak/geocaching/intro/fragments/AccountFragment;")
    val vMethod = accountClass.methods.firstOrNull {
        it.name == "v" && it.returnType == "V" && it.implementation != null && it.parameterTypes.size == 4
    } ?: throw PatchException("AccountFragment.v not found")

    // 1. Override "Membership type: Basic" text with "Membership type: Premium"
    val initialInstructions = vMethod.implementation?.instructions?.toList() ?: emptyList()
    for (i in 0 until initialInstructions.size - 1) {
        val curr = initialInstructions[i]
        val next = initialInstructions[i + 1]
        val isL65U = curr.opcode == Opcode.INVOKE_STATIC &&
            (curr as? ReferenceInstruction)?.reference?.let { ref ->
                (ref as? MethodReference)?.let { m ->
                    m.name == "U" && m.definingClass == "Ll65;" && m.returnType == "Ljava/lang/String;"
                } == true
            } == true
        if (isL65U && next.opcode == Opcode.MOVE_RESULT_OBJECT) {
            val prev = if (i > 0) initialInstructions[i - 1] else null
            val isBasicString = (prev as? NarrowLiteralInstruction)?.narrowLiteral == 0x7f140692
            if (isBasicString) {
                val reg = (next as OneRegisterInstruction).registerA
                val after = initialInstructions.getOrNull(i + 2)
                val alreadyReplaced = after?.opcode == Opcode.CONST_STRING &&
                    (after as? ReferenceInstruction)?.reference?.let { ref ->
                        (ref as? StringReference)?.string == "Membership type: Premium"
                    } == true
                if (!alreadyReplaced) {
                    vMethod.addInstructions(i + 2, "const-string v$reg, \"Membership type: Premium\"")
                }
                break
            }
        }
    }

    // 2. Refresh instructions list after insertion so indices are exact
    val instructions = vMethod.implementation?.instructions?.toList() ?: emptyList()
    val sgetIndex = instructions.indexOfFirst { instruction ->
        instruction.opcode == Opcode.SGET_OBJECT &&
            (instruction as? ReferenceInstruction)?.reference?.let { ref ->
                (ref as? FieldReference)?.let { f ->
                    f.name == "g" && (f.definingClass.contains("e0") || f.type.contains("e0"))
                } == true
            } == true
    }
    if (sgetIndex >= 0) {
        val targetIndex = (sgetIndex until instructions.size).firstOrNull { i ->
            val inst = instructions[i]
            inst.opcode == Opcode.INVOKE_VIRTUAL &&
                (inst as? ReferenceInstruction)?.reference?.let { ref ->
                    (ref as? MethodReference)?.let { m ->
                        m.name == "t" && m.definingClass == "Lb44;"
                    } == true
                } == true
        } ?: throw PatchException("Could not find b44.t target after e0.g in AccountFragment.v")

        val targetLabel = vMethod.implementation!!.newLabelForIndex(targetIndex)
        vMethod.implementation!!.replaceInstruction(sgetIndex, BuilderInstruction30t(Opcode.GOTO_32, targetLabel))
    }
}
