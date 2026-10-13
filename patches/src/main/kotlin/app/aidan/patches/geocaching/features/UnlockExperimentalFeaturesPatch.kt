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
import com.android.tools.smali.dexlib2.iface.instruction.ReferenceInstruction
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

    val enableUnpersistedExperimentalFeatures = booleanOption(
        key = "enableUnpersistedExperimentalFeatures",
        title = "Enable Hidden Experimental Toggles",
        description = "Persists and enables the Collaborative Lists, Has Parking, and Trackables Only toggles in Experimental Features.",
        default = true
    )

    val bypassExperimentalServerSync = booleanOption(
        key = "bypassExperimentalServerSync",
        title = "Bypass Experimental Server Sync",
        description = "Keeps experimental feature toggles local without POSTing to or being overwritten by /mobile/v1/user/settings/mobileexperiments.",
        default = true
    )

    execute {
        patchExperimentalSettingsGate()
        removeExperimentalPremiumBadge()
        if (bypassExperimentalServerSync.value == true) {
            neutralizeExperimentalServerSync()
        }
        if (unlockAllBetaFeatures.value == true) {
            unlockBetaFeatureFlags()
        }
        if (enableUnpersistedExperimentalFeatures.value == true) {
            patchUnpersistedExperimentalFeatures()
        }
    }
}

/**
 * Neutralizes remote server synchronization for experimental features in ExperimentalRepo.
 * By default, saving a feature attempts to POST to /mobile/v1/user/settings/mobileexperiments,
 * which returns HTTP 403 Forbidden for non-beta/Basic accounts. Furthermore, whenever the app
 * resumes or navigates to the map, ExperimentalRepo.syncFeatures queries the backend and silently
 * overwrites all local experimental preferences back to false.
 *
 * Neutralizing both methods preserves user toggles strictly within local SharedPreferences.
 */
private fun BytecodePatchContext.neutralizeExperimentalServerSync() {
    val expRepoClass = mutableClassDefBy("Lcom/groundspeak/geocaching/intro/experimentalfeatures/e;")
    val saveMethod = expRepoClass.methods.firstOrNull {
        it.name == "a" && it.returnType == "Ljava/lang/Object;" && it.implementation != null
    } ?: throw PatchException("ExperimentalRepo.a (saveFeatureToServer) not found")

    val syncMethod = expRepoClass.methods.firstOrNull {
        it.name == "b" && it.returnType == "Ljava/lang/Object;" && it.implementation != null
    } ?: throw PatchException("ExperimentalRepo.b (syncFeatures) not found")

    val smali = """
        sget-object v0, Lszb;->a:Lszb;
        return-object v0
    """.trimIndent()

    saveMethod.addInstructions(0, smali)
    syncMethod.addInstructions(0, smali)
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
    val ldClass = mutableClassDefBy("Ll75;")
    val iMethod = ldClass.methods.firstOrNull {
        it.name == "E" && it.returnType == "Z" && it.implementation != null
    } ?: throw PatchException("LaunchDarkly l75.E not found")

    val smali = """
        sget-object v0, Lcom/groundspeak/geocaching/intro/analytics/launchdarkly/LaunchDarklyFlag;->K:Lcom/groundspeak/geocaching/intro/analytics/launchdarkly/LaunchDarklyFlag;
        if-eq p0, v0, :cond_is_true
        sget-object v0, Lcom/groundspeak/geocaching/intro/analytics/launchdarkly/LaunchDarklyFlag;->T:Lcom/groundspeak/geocaching/intro/analytics/launchdarkly/LaunchDarklyFlag;
        if-eq p0, v0, :cond_is_true
        sget-object v0, Lcom/groundspeak/geocaching/intro/analytics/launchdarkly/LaunchDarklyFlag;->b0:Lcom/groundspeak/geocaching/intro/analytics/launchdarkly/LaunchDarklyFlag;
        if-eq p0, v0, :cond_is_true
        goto :cond_not_beta
        :cond_is_true
        const/4 v0, 0x1
        return v0
        :cond_not_beta
    """.trimIndent()

    iMethod.addInstructions(0, smali)
}

/**
 * Injects the hidden experimental features (Collaborative Lists, Has Parking, Trackables Only)
 * into the ExperimentalFragment menu, overrides Lyc3.a to read their values from SharedPreferences,
 * and adds persistent storage in ExpFeaturesViewModel.saveFeaturePreference.
 */
private fun BytecodePatchContext.patchUnpersistedExperimentalFeatures() {
    // 1. Add f$f, f$h, f$q to ExperimentalFragment.onResume()V
    val fragmentClass = mutableClassDefBy("Lcom/groundspeak/geocaching/intro/experimentalfeatures/ExperimentalFragment;")
    val onResumeMethod = fragmentClass.methods.firstOrNull {
        it.name == "onResume" && it.returnType == "V" && it.implementation != null
    } ?: throw PatchException("ExperimentalFragment.onResume not found")

    val onResumeImpl = onResumeMethod.implementation ?: throw PatchException("Missing implementation in onResume")
    val onResumeInstructions = onResumeImpl.instructions.toList()

    val targetIdx = onResumeInstructions.indexOfFirst {
        it.opcode == Opcode.SGET_OBJECT &&
            (it as? ReferenceInstruction)?.reference?.toString() ==
            "Lcom/groundspeak/geocaching/intro/experimentalfeatures/f\$a;->a:Lcom/groundspeak/geocaching/intro/experimentalfeatures/f\$a;"
    }
    if (targetIdx < 0) {
        throw PatchException("Could not find f\$a reference in ExperimentalFragment.onResume")
    }

    val addIdx = onResumeInstructions.subList(targetIdx, onResumeInstructions.size).indexOfFirst {
        it.opcode == Opcode.INVOKE_VIRTUAL
    }
    if (addIdx < 0) {
        throw PatchException("Could not find invoke-virtual after f\$a in ExperimentalFragment.onResume")
    }
    val insertIdx = targetIdx + addIdx + 1

    val addFeaturesSmali = """
        sget-object v3, Lcom/groundspeak/geocaching/intro/experimentalfeatures/f${'$'}f;->a:Lcom/groundspeak/geocaching/intro/experimentalfeatures/f${'$'}f;
        invoke-virtual {v2, v3}, Lkotlin/collections/builders/ListBuilder;->add(Ljava/lang/Object;)Z
        sget-object v3, Lcom/groundspeak/geocaching/intro/experimentalfeatures/f${'$'}h;->a:Lcom/groundspeak/geocaching/intro/experimentalfeatures/f${'$'}h;
        invoke-virtual {v2, v3}, Lkotlin/collections/builders/ListBuilder;->add(Ljava/lang/Object;)Z
        sget-object v3, Lcom/groundspeak/geocaching/intro/experimentalfeatures/f${'$'}q;->a:Lcom/groundspeak/geocaching/intro/experimentalfeatures/f${'$'}q;
        invoke-virtual {v2, v3}, Lkotlin/collections/builders/ListBuilder;->add(Ljava/lang/Object;)Z
    """.trimIndent()
    onResumeMethod.addInstructions(insertIdx, addFeaturesSmali)

    // 2. Prepend preference checks in Lyc3;->a(Lcom/groundspeak/geocaching/intro/experimentalfeatures/f;)Z
    val prefsClass = mutableClassDefBy("Lyc3;")
    val aMethod = prefsClass.methods.firstOrNull {
        it.name == "a" && it.returnType == "Z" && it.implementation != null
    } ?: throw PatchException("yc3.a not found")

    val prefsSmali = """
        sget-object v0, Lcom/groundspeak/geocaching/intro/experimentalfeatures/f${'$'}h;->a:Lcom/groundspeak/geocaching/intro/experimentalfeatures/f${'$'}h;
        invoke-virtual {p1, v0}, Ljava/lang/Object;->equals(Ljava/lang/Object;)Z
        move-result v0
        if-eqz v0, :cond_exp_check_f
        iget-object v0, p0, Lyc3;->a:Landroid/content/SharedPreferences;
        const-string v1, "ExperimentalSharedPrefs.HAS_PARKING"
        const/4 v2, 0x0
        invoke-interface {v0, v1, v2}, Landroid/content/SharedPreferences;->getBoolean(Ljava/lang/String;Z)Z
        move-result v0
        return v0
        :cond_exp_check_f
        sget-object v0, Lcom/groundspeak/geocaching/intro/experimentalfeatures/f${'$'}f;->a:Lcom/groundspeak/geocaching/intro/experimentalfeatures/f${'$'}f;
        invoke-virtual {p1, v0}, Ljava/lang/Object;->equals(Ljava/lang/Object;)Z
        move-result v0
        if-eqz v0, :cond_exp_check_q
        iget-object v0, p0, Lyc3;->a:Landroid/content/SharedPreferences;
        const-string v1, "ExperimentalSharedPrefs.COLLABORATIVE_LISTS"
        const/4 v2, 0x0
        invoke-interface {v0, v1, v2}, Landroid/content/SharedPreferences;->getBoolean(Ljava/lang/String;Z)Z
        move-result v0
        return v0
        :cond_exp_check_q
        sget-object v0, Lcom/groundspeak/geocaching/intro/experimentalfeatures/f${'$'}q;->a:Lcom/groundspeak/geocaching/intro/experimentalfeatures/f${'$'}q;
        invoke-virtual {p1, v0}, Ljava/lang/Object;->equals(Ljava/lang/Object;)Z
        move-result v0
        if-eqz v0, :cond_exp_orig
        iget-object v0, p0, Lyc3;->a:Landroid/content/SharedPreferences;
        const-string v1, "ExperimentalSharedPrefs.SHOW_TRACKABLES_ONLY"
        const/4 v2, 0x0
        invoke-interface {v0, v1, v2}, Landroid/content/SharedPreferences;->getBoolean(Ljava/lang/String;Z)Z
        move-result v0
        return v0
        :cond_exp_orig
    """.trimIndent()
    aMethod.addInstructions(0, prefsSmali)

    // 3. Save preferences in ExpFeaturesViewModel$saveFeaturePreference$1.invokeSuspend
    val vmSaveClass = mutableClassDefBy("Lcom/groundspeak/geocaching/intro/experimentalfeatures/ExpFeaturesViewModel\$saveFeaturePreference\$1;")
    val invokeSuspendMethod = vmSaveClass.methods.firstOrNull {
        it.name == "invokeSuspend" && it.implementation != null
    } ?: throw PatchException("ExpFeaturesViewModel\$saveFeaturePreference\$1.invokeSuspend not found")

    val saveImpl = invokeSuspendMethod.implementation ?: throw PatchException("Missing implementation in invokeSuspend")
    val saveInstructions = saveImpl.instructions.toList()

    val targets = listOf(
        Pair("Lcom/groundspeak/geocaching/intro/experimentalfeatures/f\$q;", "ExperimentalSharedPrefs.SHOW_TRACKABLES_ONLY"),
        Pair("Lcom/groundspeak/geocaching/intro/experimentalfeatures/f\$h;", "ExperimentalSharedPrefs.HAS_PARKING"),
        Pair("Lcom/groundspeak/geocaching/intro/experimentalfeatures/f\$f;", "ExperimentalSharedPrefs.COLLABORATIVE_LISTS")
    )

    val insertions = targets.map { (typeDesc, prefKey) ->
        val idx = saveInstructions.indexOfFirst {
            it.opcode == Opcode.INSTANCE_OF &&
                (it as? ReferenceInstruction)?.reference?.toString() == typeDesc
        }
        if (idx < 0) {
            throw PatchException("Could not find instance-of for $typeDesc in invokeSuspend")
        }
        Triple(idx, typeDesc, prefKey)
    }.sortedByDescending { it.first }

    for ((idx, _, prefKey) in insertions) {
        val label = prefKey.substringAfterLast(".")
        val saveSmali = """
            if-eqz v8, :cond_skip_save_$label
            invoke-interface {p1}, Landroid/content/SharedPreferences;->edit()Landroid/content/SharedPreferences${'$'}Editor;
            move-result-object p1
            const-string v8, "$prefKey"
            invoke-interface {p1, v8, v3}, Landroid/content/SharedPreferences${'$'}Editor;->putBoolean(Ljava/lang/String;Z)Landroid/content/SharedPreferences${'$'}Editor;
            move-result-object p1
            invoke-interface {p1}, Landroid/content/SharedPreferences${'$'}Editor;->apply()V
            const/4 v8, 0x1
            :cond_skip_save_$label
        """.trimIndent()
        invokeSuspendMethod.addInstructions(idx + 1, saveSmali)
    }
}
