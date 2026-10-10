package app.aidan.patches.geocaching.features

import app.aidan.patches.geocaching.shared.COMPATIBILITY_GEOCACHING
import app.aidan.patches.geocaching.shared.patchAllL3cChecksToValue
import app.morphe.patcher.extensions.InstructionExtensions.addInstructions
import app.morphe.patcher.extensions.InstructionExtensions.replaceInstruction
import app.morphe.patcher.patch.BytecodePatchContext
import app.morphe.patcher.patch.PatchException
import app.morphe.patcher.patch.bytecodePatch
import com.android.tools.smali.dexlib2.Opcode
import com.android.tools.smali.dexlib2.iface.instruction.NarrowLiteralInstruction
import com.android.tools.smali.dexlib2.iface.instruction.OneRegisterInstruction

val unlockTemplatesPatch = bytecodePatch(
    name = "Unlock Templates",
    description = "Unlocks geocache log templates, allowing creating, editing, and applying custom log templates without Geocaching Premium.",
    default = true
) {
    category("Features")
    compatibleWith(COMPATIBILITY_GEOCACHING)

    execute {
        patchTemplatesSettingsGate()
        removeTemplatesPremiumBadge()
        patchTemplatesApiToLocalStore()
    }
}

/**
 * Bypasses the Premium upsell check in SettingsFragment.x when clicking Templates,
 * navigating directly to toTemplatesListFragment.
 */
private fun BytecodePatchContext.patchTemplatesSettingsGate() {
    val settingsClass = mutableClassDefBy("Lcom/groundspeak/geocaching/intro/fragments/settings/SettingsFragment;")
    val xMethod = settingsClass.methods.firstOrNull {
        it.name == "x" && it.returnType == "V" && it.implementation != null
    } ?: throw PatchException("SettingsFragment.x not found")

    patchAllL3cChecksToValue(xMethod, "SettingsFragment.x", true)
}

/**
 * Removes the "Premium" badge next to "Templates" in Settings by setting
 * the isPremium boolean in b0.<init> to false.
 */
private fun BytecodePatchContext.removeTemplatesPremiumBadge() {
    val b0Class = mutableClassDefBy("Lcom/groundspeak/geocaching/intro/fragments/settings/b0;")
    val initMethod = b0Class.methods.firstOrNull {
        it.name == "<init>" && it.implementation != null
    } ?: throw PatchException("b0.<init> not found")

    val impl = initMethod.implementation ?: throw PatchException("Missing implementation in b0.<init>")
    val instructions = impl.instructions.toList()

    val constIdx = instructions.indexOfFirst {
        it.opcode == Opcode.CONST_4 && (it as? NarrowLiteralInstruction)?.narrowLiteral == 1
    }
    if (constIdx < 0) {
        throw PatchException("Could not find isPremium literal in b0.<init>")
    }
    val reg = (instructions[constIdx] as OneRegisterInstruction).registerA
    initMethod.replaceInstruction(constIdx, "const/4 v$reg, 0x0")
}

/**
 * Redirects TemplatesApi network calls to local store equivalents so that
 * creating, updating, deleting, and fetching templates operate purely on the local
 * Room database without failing against Groundspeak's server on Basic accounts.
 */
private fun BytecodePatchContext.patchTemplatesApiToLocalStore() {
    val templatesApiClass = mutableClassDefBy("Lcom/groundspeak/geocaching/intro/network/api/templates/a;")

    // 1. Method a: create template -> return Success(cq6(0, title, text))
    val aMethod = templatesApiClass.methods.firstOrNull {
        it.name == "a" && it.implementation != null && it.parameterTypes.size == 3
    } ?: throw PatchException("TemplatesApi.a not found")
    aMethod.addInstructions(
        0,
        """
        new-instance v0, Lcq6;
        const/4 v1, 0x0
        invoke-direct {v0, v1, p1, p2}, Lcq6;-><init>(ILjava/lang/String;Ljava/lang/String;)V
        new-instance v1, Lxk9;
        invoke-direct {v1, v0}, Lxk9;-><init>(Ljava/lang/Object;)V
        return-object v1
        """.trimIndent()
    )

    // 2. Method b: delete template -> return Success(Unit)
    val bMethod = templatesApiClass.methods.firstOrNull {
        it.name == "b" && it.implementation != null && it.parameterTypes.size == 2
    } ?: throw PatchException("TemplatesApi.b not found")
    bMethod.addInstructions(
        0,
        """
        new-instance v0, Lxk9;
        sget-object v1, Lszb;->a:Lszb;
        invoke-direct {v0, v1}, Lxk9;-><init>(Ljava/lang/Object;)V
        return-object v0
        """.trimIndent()
    )

    // 3. Method c: getAll templates -> return Failure("local") to force local DB fallback
    val cMethod = templatesApiClass.methods.firstOrNull {
        it.name == "c" && it.implementation != null && it.parameterTypes.size == 1
    } ?: throw PatchException("TemplatesApi.c not found")
    cMethod.addInstructions(
        0,
        """
        new-instance v0, Lwk9;
        const-string v1, "local"
        invoke-direct {v0, v1}, Lwk9;-><init>(Ljava/lang/Object;)V
        return-object v0
        """.trimIndent()
    )

    // 4. Method d: update template -> return Success(Unit)
    val dMethod = templatesApiClass.methods.firstOrNull {
        it.name == "d" && it.implementation != null && it.parameterTypes.size == 4
    } ?: throw PatchException("TemplatesApi.d not found")
    dMethod.addInstructions(
        0,
        """
        new-instance v0, Lxk9;
        sget-object v1, Lszb;->a:Lszb;
        invoke-direct {v0, v1}, Lxk9;-><init>(Ljava/lang/Object;)V
        return-object v0
        """.trimIndent()
    )
}
