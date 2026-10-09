package app.aidan.patches.geocaching.features

import app.aidan.patches.geocaching.shared.COMPATIBILITY_GEOCACHING
import app.aidan.patches.geocaching.shared.patchAllL3cChecksToValue
import app.morphe.patcher.extensions.InstructionExtensions.addInstructions
import app.morphe.patcher.extensions.InstructionExtensions.replaceInstruction
import app.morphe.patcher.patch.BytecodePatchContext
import app.morphe.patcher.patch.PatchException
import app.morphe.patcher.patch.booleanOption
import app.morphe.patcher.patch.bytecodePatch
import com.android.tools.smali.dexlib2.Opcode
import com.android.tools.smali.dexlib2.iface.instruction.NarrowLiteralInstruction
import com.android.tools.smali.dexlib2.iface.instruction.OneRegisterInstruction

val unlockExperimentalFeaturesPatch = bytecodePatch(
    name = "Unlock Experimental Features",
    description = "Unlocks beta and experimental features in Settings without Geocaching Premium.",
    default = true
) {
    category("Features")
    compatibleWith(COMPATIBILITY_GEOCACHING)

    val unlockAllBetaFeatures = booleanOption(
        key = "unlockAllBetaFeatures",
        title = "Unlock All Beta Projects",
        description = "Unlocks all 10% beta features (Activity Feed, Cache Log Translation, Autofill, Confetti, etc.).",
        default = true
    )

    execute {
        patchExperimentalSettingsGate()
        removeExperimentalPremiumBadge()
        if (unlockAllBetaFeatures.value == true) {
            unlockBetaFeatureFlags()
        }
    }
}

/**
 * Bypasses the Premium upsell check in SettingsFragment.y when clicking Experimental Features,
 * opening toExperimentalFeaturesFrag directly.
 */
private fun BytecodePatchContext.patchExperimentalSettingsGate() {
    val settingsClass = mutableClassDefBy("Lcom/groundspeak/geocaching/intro/fragments/settings/SettingsFragment;")
    val yMethod = settingsClass.methods.firstOrNull {
        it.name == "y" && it.returnType == "V" && it.implementation != null
    } ?: throw PatchException("SettingsFragment.y not found")

    patchAllL3cChecksToValue(yMethod, "SettingsFragment.y", true)
}

/**
 * Removes the "Premium" badge next to "Experimental features" in Settings by setting
 * the isPremium boolean in p.<init> to false.
 */
private fun BytecodePatchContext.removeExperimentalPremiumBadge() {
    val pClass = mutableClassDefBy("Lcom/groundspeak/geocaching/intro/fragments/settings/p;")
    val initMethod = pClass.methods.firstOrNull {
        it.name == "<init>" && it.implementation != null
    } ?: throw PatchException("p.<init> not found")

    val impl = initMethod.implementation ?: throw PatchException("Missing implementation in p.<init>")
    val instructions = impl.instructions.toList()

    val constIdx = instructions.indexOfFirst {
        it.opcode == Opcode.CONST_4 && (it as? NarrowLiteralInstruction)?.narrowLiteral == 1
    }
    if (constIdx < 0) {
        throw PatchException("Could not find isPremium literal in p.<init>")
    }
    val reg = (instructions[constIdx] as OneRegisterInstruction).registerA
    initMethod.replaceInstruction(constIdx, "const/4 v$reg, 0x0")
}

/**
 * Enables LaunchDarkly feature flags (10% projects menu K, recent log icons T, and dark web descriptions b0)
 * in LaunchDarkly b.i, making all experimental feature projects accessible and savable.
 */
private fun BytecodePatchContext.unlockBetaFeatureFlags() {
    val ldClass = mutableClassDefBy("Lcom/groundspeak/geocaching/intro/analytics/launchdarkly/b;")
    val iMethod = ldClass.methods.firstOrNull {
        it.name == "i" && it.returnType == "Z" && it.implementation != null
    } ?: throw PatchException("LaunchDarkly b.i not found")

    val smali = """
        sget-object v0, Lcom/groundspeak/geocaching/intro/analytics/launchdarkly/LaunchDarklyFlag;->K:Lcom/groundspeak/geocaching/intro/analytics/launchdarkly/LaunchDarklyFlag;
        if-ne p0, v0, :cond_check_t
        const/4 v0, 0x1
        return v0
        :cond_check_t
        sget-object v0, Lcom/groundspeak/geocaching/intro/analytics/launchdarkly/LaunchDarklyFlag;->T:Lcom/groundspeak/geocaching/intro/analytics/launchdarkly/LaunchDarklyFlag;
        if-ne p0, v0, :cond_check_b0
        const/4 v0, 0x1
        return v0
        :cond_check_b0
        sget-object v0, Lcom/groundspeak/geocaching/intro/analytics/launchdarkly/LaunchDarklyFlag;->b0:Lcom/groundspeak/geocaching/intro/analytics/launchdarkly/LaunchDarklyFlag;
        if-ne p0, v0, :cond_continue
        const/4 v0, 0x1
        return v0
        :cond_continue
    """.trimIndent()

    iMethod.addInstructions(0, smali)
}
