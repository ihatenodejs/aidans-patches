package app.aidan.patches.geocaching.features

import app.aidan.patches.geocaching.shared.COMPATIBILITY_GEOCACHING
import app.aidan.patches.geocaching.shared.patchAllL3cChecksToValue
import app.aidan.patches.geocaching.shared.patchIncludeOwnedDisabledCachesToNull
import app.aidan.patches.geocaching.shared.patchSanitizeFilterModel
import app.aidan.patches.shared.compositeFeaturePatch
import app.morphe.patcher.extensions.InstructionExtensions.addInstructions
import app.morphe.patcher.extensions.InstructionExtensions.replaceInstruction
import app.morphe.patcher.patch.BytecodePatchContext
import app.morphe.patcher.patch.PatchException
import app.morphe.patcher.patch.ResourcePatchContext
import app.morphe.patcher.patch.booleanOption
import com.android.tools.smali.dexlib2.Opcode
import com.android.tools.smali.dexlib2.builder.instruction.BuilderInstruction30t
import com.android.tools.smali.dexlib2.iface.instruction.NarrowLiteralInstruction
import com.android.tools.smali.dexlib2.iface.instruction.OneRegisterInstruction
import com.android.tools.smali.dexlib2.iface.instruction.ReferenceInstruction
import com.android.tools.smali.dexlib2.iface.reference.FieldReference
import com.android.tools.smali.dexlib2.iface.reference.MethodReference
import com.android.tools.smali.dexlib2.iface.reference.StringReference
import org.w3c.dom.Element

val localPremiumPatch = compositeFeaturePatch(
    name = "Local Premium",
    description = "Enables local Premium membership status across profile and account screens, removes upgrade promotions, banners, icons, and server-dependent profile statistics, and enriches public cache details (description, hints, owner) when server blocks basic accounts.",
    default = true
) {
    category("Interface")
    compatibleWith(COMPATIBILITY_GEOCACHING)
    extendWith("extensions/extension.mpe")

    val showLocalPremiumWarnings = booleanOption(
        key = "showLocalPremiumWarnings",
        title = "Show Local Premium Warnings",
        description = "Shows an informational warning banner on Premium Member Only caches indicating that complete cache data is restricted by the server for basic accounts.",
        default = true
    )

    resources {
        if (showLocalPremiumWarnings.value != false) {
            patchCacheSummaryWarningLayout()
        }
    }

    bytecode {
        patchProfileMembershipLabel()
        patchMapUpgradeIcon()
        patchProfileUpgradeCard()
        patchProfileStatisticsItem()
        patchSettingsFragment()
        patchAccountFragment()
        patchPreventCacheDetailsError()
        patchPreventMapSearchError()
        patchPreventNavigationLoadError()
        patchDescriptionFragmentWebViewLineBreak()
        if (showLocalPremiumWarnings.value != false) {
            patchGeocacheDetailsActivityRoute()
            patchCacheSummaryWarningBinding()
        }
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
 * Removes the "Upgrade to premium" promotional card section at the bottom of the Profile screen.
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
 * Removes the Statistics item row and its trailing spacer from the Profile screen.
 *
 * In `com.groundspeak.geocaching.intro.profile.l.b`, the Statistics item is rendered via
 * `k19.p(Lu4c;->f:Lg29;, ...)`. When local Premium is active, this item appears on the Profile screen,
 * but navigating into it results in an error screen because the statistics data relies on server-side
 * processing unavailable to non-premium accounts.
 *
 * Neutralizing the `u4c.f` field access, the `k19.p` composable invocation, and the trailing spacer
 * call (`f.u`) cleanly removes the Statistics row while preserving slot table alignment and leaving
 * a single section spacer above Trackables.
 */
private fun BytecodePatchContext.patchProfileStatisticsItem() {
    val profileClass = mutableClassDefBy("Lcom/groundspeak/geocaching/intro/profile/l;")
    val bMethod = profileClass.methods.firstOrNull {
        it.name == "b" && it.returnType == "V" && it.implementation != null && it.parameterTypes.size == 13
    } ?: throw PatchException("com.groundspeak.geocaching.intro.profile.l.b not found")

    val impl = bMethod.implementation ?: throw PatchException("Missing implementation in profile l.b")
    val instructions = impl.instructions.toList()

    val statsIndex = instructions.indexOfFirst { instruction ->
        instruction.opcode == Opcode.INVOKE_STATIC &&
            (instruction as? ReferenceInstruction)?.reference?.let { ref ->
                (ref as? MethodReference)?.let { m ->
                    m.definingClass == "Lk19;" && m.name == "p"
                }
            } == true
    }
    if (statsIndex < 0) {
        throw PatchException("Could not find k19.p invocation in profile l.b")
    }

    val igetIndex = statsIndex - 1
    val igetInst = instructions.getOrNull(igetIndex)
    val isIgetStats = igetInst?.opcode == Opcode.IGET_OBJECT &&
        (igetInst as? ReferenceInstruction)?.reference?.let { ref ->
            (ref as? FieldReference)?.let { f ->
                f.name == "f" && f.type == "Lg29;"
            }
        } == true

    if (!isIgetStats) {
        throw PatchException("Instruction preceding k19.p is not Lu4c;->f:Lg29; iget-object")
    }

    val spacerIndex = statsIndex + 1
    val spacerInst = instructions.getOrNull(spacerIndex)
    val isSpacer = spacerInst?.opcode == Opcode.INVOKE_STATIC &&
        (spacerInst as? ReferenceInstruction)?.reference?.let { ref ->
            (ref as? MethodReference)?.let { m ->
                m.definingClass == "Lcom/groundspeak/geocaching/intro/ui/compose/f;" && m.name == "u"
            }
        } == true

    if (!isSpacer) {
        throw PatchException("Instruction following k19.p is not f.u spacer invoke-static")
    }

    bMethod.replaceInstruction(spacerIndex, "nop")
    bMethod.replaceInstruction(statsIndex, "nop")
    bMethod.replaceInstruction(igetIndex, "nop")
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

/**
 * Prevents non-traditional cache details from being wiped out with a "Something went wrong"
 * error screen after the map loads the cache data.
 *
 * In LegacyGeocacheRepo.loadGeocache (d.e), after converting the database's LiteGeocache
 * to a LegacyGeocache and emitting it to the UI callback, preserves the LegacyGeocache reference
 * in p1 across the fetchLegacyCacheFromServer suspension point. When the server returns 403 Forbidden
 * (because basic accounts cannot fetch full non-traditional cache details from the server), the
 * repository's existing non-null cache check suppresses the error callback.
 */
private fun BytecodePatchContext.patchPreventCacheDetailsError() {
    patchLegacyGeocacheRepoLiteCache()
    patchFetchLegacyCacheFallback()
    patchSanitizeLoadedCache()
}

private fun BytecodePatchContext.patchLegacyGeocacheRepoLiteCache() {
    val dClass = mutableClassDefByOrNull("Lcom/groundspeak/geocaching/intro/model/d;") ?: return
    val eMethod = dClass.methods.firstOrNull {
        it.name == "e" &&
            it.parameterTypes == listOf(
                "Ljava/lang/String;",
                "Lcom/groundspeak/geocaching/intro/geocachedetails/k;",
                "Lkotlin/coroutines/jvm/internal/ContinuationImpl;"
            ) &&
            it.implementation != null
    } ?: return

    val impl = eMethod.implementation ?: return
    val instructions = impl.instructions.toList()

    val invokeLIndex = instructions.indexOfFirst { inst ->
        inst.opcode == Opcode.INVOKE_STATIC &&
            (inst as? ReferenceInstruction)?.reference?.let { ref ->
                (ref as? MethodReference)?.let { m ->
                    m.definingClass == "Lie4;" && m.name == "l"
                }
            } == true
    }
    if (invokeLIndex == -1) return

    val nextInst = instructions.getOrNull(invokeLIndex + 2)
    val isAlreadyPatched = nextInst?.opcode == Opcode.MOVE_OBJECT
    if (!isAlreadyPatched) {
        eMethod.addInstructions(
            invokeLIndex + 2,
            """
                move-object p1, p3
                invoke-static {p3}, Lapp/aidan/extension/geocaching/GeocacheEnrichmentBridge;->rememberLiteGeocache(Ljava/lang/Object;)V
            """.trimIndent()
        )
    }
}

/**
 * In LegacyGeocacheRepo$fetchLegacyCacheFromServer$2.invokeSuspend, intercepts the error return point
 * when server requests return 403 Forbidden (which occurs for basic accounts on traditional caches
 * with D > 1.5 or T > 1.5, or on non-traditional caches).
 *
 * Calls GeocacheEnrichmentBridge.onFetchFailed to scrape the public webpage (where non-PMO cache
 * descriptions, hints, and owner details are public) and constructs an enriched LegacyGeocache
 * wrapped in Success (xk9), persisting it to the local cache and satisfying callers seamlessly.
 */
private fun BytecodePatchContext.patchFetchLegacyCacheFallback() {
    val lambdaClass = mutableClassDefByOrNull(
        "Lcom/groundspeak/geocaching/intro/model/LegacyGeocacheRepo\$fetchLegacyCacheFromServer\$2;"
    ) ?: return

    val invokeSuspendMethod = lambdaClass.methods.firstOrNull {
        it.name == "invokeSuspend" &&
            it.parameterTypes == listOf("Ljava/lang/Object;") &&
            it.implementation != null
    } ?: return

    val impl = invokeSuspendMethod.implementation ?: return
    val instructions = impl.instructions.toList()

    val logStringIndex = instructions.indexOfFirst { inst ->
        inst.opcode == Opcode.CONST_STRING &&
            ((inst as? ReferenceInstruction)?.reference as? StringReference)?.string ==
            "fetchLegacyCacheFromServer: error encountered while loading cache"
    }
    if (logStringIndex == -1) return

    // The instruction before logStringIndex is: const-string v0, "GeocacheRepo"
    val targetIndex = logStringIndex - 1
    val thisReg = impl.registerCount - 2

    invokeSuspendMethod.addInstructions(
        targetIndex,
        """
            move-object/16 v0, v$thisReg
            invoke-static {v0, v7}, Lapp/aidan/extension/geocaching/GeocacheEnrichmentBridge;->onFetchFailed(Ljava/lang/Object;Ljava/lang/Object;)Ljava/lang/Object;
            move-result-object v7
        """.trimIndent()
    )
}

private fun BytecodePatchContext.patchSanitizeLoadedCache() {
    val y82Class = mutableClassDefByOrNull("Ly82;") ?: return
    val pMethod = y82Class.methods.firstOrNull {
        it.name == "p" &&
            it.parameterTypes == listOf("Ljava/lang/String;") &&
            it.returnType == "Lcom/groundspeak/geocaching/intro/types/LegacyGeocache;" &&
            it.implementation != null
    } ?: return

    val impl = pMethod.implementation ?: return
    val instructions = impl.instructions.toList()
    val returnIndex = instructions.indexOfLast { it.opcode == Opcode.RETURN_OBJECT }
    if (returnIndex == -1) return

    val returnInst = instructions[returnIndex] as? OneRegisterInstruction ?: return
    val reg = returnInst.registerA

    pMethod.addInstructions(
        returnIndex,
        """
            invoke-static {v$reg}, Lapp/aidan/extension/geocaching/GeocacheEnrichmentBridge;->sanitizeLoadedCache(Ljava/lang/Object;)Ljava/lang/Object;
            move-result-object v$reg
            check-cast v$reg, Lcom/groundspeak/geocaching/intro/types/LegacyGeocache;
        """.trimIndent()
    )
}

/**
 * Prevents map searches from failing with HTTP 403 Forbidden when moving around the map.
 *
 * In FilterPreferences.h (dk3.h), `includeOwnedDisabledCaches` is hardcoded to Boolean.TRUE
 * for Premium accounts. Because basic accounts on Geocaching's server cannot search with
 * `includeOwnedDisabledCaches=true`, the server rejects the request with HTTP 403 Forbidden.
 *
 * Nulling out `includeOwnedDisabledCaches` ensures map search requests send only valid
 * parameters, preventing 403 errors while preserving all user-configured filters.
 */
private fun BytecodePatchContext.patchPreventMapSearchError() {
    val filterPrefsClass = mutableClassDefByOrNull("Ldk3;") ?: return
    val hMethod = filterPrefsClass.methods.firstOrNull {
        it.name == "h" &&
            it.parameterTypes.isEmpty() &&
            it.returnType == "Lck3;" &&
            it.implementation != null
    } ?: return
    patchIncludeOwnedDisabledCachesToNull(hMethod)
    patchSanitizeFilterModel(hMethod)
}

/**
 * In NavigationViewModel.l (loadCache), forces the second boolean parameter
 * (startWithFullCacheData) to false so that NavigationViewModel always queries
 * LegacyGeocacheRepo.b (loadCacheEnsuringWaypoints / local database) instead of
 * LegacyGeocacheRepo.f (loadGeocacheLocallyWithServerFallback).
 *
 * Basic accounts get 403 Forbidden when requesting full server details for
 * traditional and non-traditional caches alike on the navigation screen, which
 * causes NavigationViewModel to emit "Unable to load geocache" and leaves the
 * navigation destination at (0.0, 0.0). Using local cache data loads the coordinates
 * that the map already holds, preventing the error and pointing navigation directly to the cache.
 */
private fun BytecodePatchContext.patchPreventNavigationLoadError() {
    val navVmClass = mutableClassDefByOrNull("Lcom/groundspeak/geocaching/intro/navigationmap/NavigationViewModel;")
        ?: return

    val lMethod = navVmClass.methods.firstOrNull {
        it.name == "l" &&
            it.parameterTypes.size == 4 &&
            it.parameterTypes[1] == "Z" &&
            it.implementation != null
    } ?: return

    lMethod.addInstructions(
        0,
        """
            const/4 p2, 0x0
        """.trimIndent()
    )
}

private const val CD_ITEM_SUMMARY_LAYOUT = "res/layout/cd_item_summary.xml"
private const val CACHE_SUMMARY_ITEM_CLASS =
    "Lcom/groundspeak/geocaching/intro/geocachedetails/overviewitems/CacheSummaryItem;"
private const val CACHE_TITLE_RES_ID = 0x7f0a0543

/**
 * Injects a light-orange warning banner container into the cache details summary layout
 * directly between the favorites section divider (view_extra_space) and the difficulty meters
 * (dts_container). The banner is initially GONE and made visible at runtime for PMO caches.
 */
private fun ResourcePatchContext.patchCacheSummaryWarningLayout() {
    document(CD_ITEM_SUMMARY_LAYOUT).use { doc ->
        val existing = doc.getElementsByTagName("LinearLayout")
        for (i in 0 until existing.length) {
            val elem = existing.item(i) as? Element ?: continue
            val tag = elem.getAttribute("android:tag")
            val id = elem.getAttribute("android:id")
            if (tag == "pmo_warning_container" || id.contains("pmo_warning_container")) {
                return@use
            }
        }

        val allElements = doc.getElementsByTagName("*")
        var dtsContainer: Element? = null
        for (i in 0 until allElements.length) {
            val elem = allElements.item(i) as? Element ?: continue
            val id = elem.getAttribute("android:id")
            if (id == "@id/dts_container" || id.endsWith("dts_container")) {
                dtsContainer = elem
                break
            }
        }

        if (dtsContainer == null) {
            throw PatchException("[$CD_ITEM_SUMMARY_LAYOUT] Could not find @id/dts_container element")
        }

        val parent = dtsContainer.parentNode
            ?: throw PatchException("[$CD_ITEM_SUMMARY_LAYOUT] @id/dts_container has no parent")

        val warning = doc.createElement("LinearLayout")
        warning.setAttribute("android:id", "@+id/pmo_warning_container")
        warning.setAttribute("android:tag", "pmo_warning_container")
        warning.setAttribute("android:orientation", "horizontal")
        warning.setAttribute("android:gravity", "center_vertical")
        warning.setAttribute("android:visibility", "gone")
        warning.setAttribute("android:layout_width", "0dp")
        warning.setAttribute("android:layout_height", "wrap_content")
        warning.setAttribute("android:layout_marginLeft", "@dimen/large")
        warning.setAttribute("android:layout_marginRight", "@dimen/large")
        warning.setAttribute("android:layout_marginStart", "@dimen/large")
        warning.setAttribute("android:layout_marginEnd", "@dimen/large")
        warning.setAttribute("android:layout_marginTop", "@dimen/small")
        warning.setAttribute("android:layout_marginBottom", "@dimen/small")
        warning.setAttribute("android:paddingLeft", "@dimen/large")
        warning.setAttribute("android:paddingRight", "@dimen/large")
        warning.setAttribute("android:paddingStart", "@dimen/large")
        warning.setAttribute("android:paddingEnd", "@dimen/large")
        warning.setAttribute("android:paddingTop", "@dimen/medium")
        warning.setAttribute("android:paddingBottom", "@dimen/medium")
        warning.setAttribute("app:layout_constraintStart_toStartOf", "parent")
        warning.setAttribute("app:layout_constraintEnd_toEndOf", "parent")
        warning.setAttribute("app:layout_constraintTop_toBottomOf", "@id/view_extra_space")

        val icon = doc.createElement("ImageView")
        icon.setAttribute("android:id", "@+id/pmo_warning_icon")
        icon.setAttribute("android:tag", "pmo_warning_icon")
        icon.setAttribute("android:layout_width", "20.0dp")
        icon.setAttribute("android:layout_height", "20.0dp")
        icon.setAttribute("android:layout_marginRight", "@dimen/medium")
        icon.setAttribute("android:layout_marginEnd", "@dimen/medium")
        icon.setAttribute("android:src", "@drawable/warning_filled")
        icon.setAttribute("android:contentDescription", "@null")
        warning.appendChild(icon)

        val text = doc.createElement("com.google.android.material.textview.MaterialTextView")
        text.setAttribute("android:id", "@+id/pmo_warning_text")
        text.setAttribute("android:tag", "pmo_warning_text")
        text.setAttribute("android:layout_width", "0dp")
        text.setAttribute("android:layout_height", "wrap_content")
        text.setAttribute("android:layout_weight", "1.0")
        text.setAttribute("android:textAppearance", "@style/textAppearanceCaption1")
        text.setAttribute("android:textColor", "#b54b00")
        text.setAttribute("android:text", "Premium Member Only cache. Complete cache data (description, hints, logs) is unavailable for basic accounts due to a server restriction.")
        warning.appendChild(text)

        parent.insertBefore(warning, dtsContainer)
        dtsContainer.setAttribute("app:layout_constraintTop_toBottomOf", "@id/pmo_warning_container")
    }
}

/**
 * Injects a call to GeocacheEnrichmentBridge.bindCacheSummaryWarning in CacheSummaryItem.bindViewHolder
 * to evaluate the cache's PMO status, dynamically apply light-orange alert theming, and reveal the warning banner.
 */
private fun BytecodePatchContext.patchCacheSummaryWarningBinding() {
    val summaryItemClass = mutableClassDefBy(CACHE_SUMMARY_ITEM_CLASS)
    val bindMethod = summaryItemClass.methods.firstOrNull {
        it.name == "bindViewHolder" && it.implementation != null
    } ?: throw PatchException("[$CACHE_SUMMARY_ITEM_CLASS] bindViewHolder method not found")

    val impl = bindMethod.implementation
        ?: throw PatchException("Missing implementation in bindViewHolder")
    val instructions = impl.instructions.toList()

    val constIdx = instructions.indexOfFirst {
        (it as? NarrowLiteralInstruction)?.narrowLiteral == CACHE_TITLE_RES_ID
    }
    if (constIdx < 0) {
        throw PatchException("[$CACHE_SUMMARY_ITEM_CLASS] Could not find 0x7f0a0543 in bindViewHolder")
    }

    val igetIdx = constIdx - 1
    if (igetIdx < 0 || instructions[igetIdx].opcode != Opcode.IGET_OBJECT) {
        throw PatchException("[$CACHE_SUMMARY_ITEM_CLASS] Expected iget-object directly before const 0x7f0a0543")
    }

    bindMethod.addInstructions(
        igetIdx,
        """
        iget-object v0, p1, Landroidx/recyclerview/widget/o0;->itemView:Landroid/view/View;
        invoke-virtual {p0}, Lcom/groundspeak/geocaching/intro/geocachedetails/LifecycleItemAdapter${'$'}Item;->getData()Ljava/lang/Object;
        move-result-object v1
        invoke-static {v0, v1}, Lapp/aidan/extension/geocaching/GeocacheEnrichmentBridge;->bindCacheSummaryWarning(Landroid/view/View;Ljava/lang/Object;)V
        """.trimIndent()
    )
}

private const val GEOCACHE_DETAILS_ACTIVITY_COMPANION =
    "Lcom/groundspeak/geocaching/intro/geocachedetails/GeocacheDetailsActivity\$a;"
private const val DESCRIPTION_FRAGMENT_CLASS = "Lr64;"

/**
 * Ensures GeocacheDetailsActivity (the XML-backed details screen containing cd_item_summary.xml)
 * is launched instead of the experimental Jetpack Compose rewrite (GeocacheDetailsActivity2)
 * so the warning banner is displayed between the favorites divider and difficulty meters.
 */
private fun BytecodePatchContext.patchGeocacheDetailsActivityRoute() {
    val companionClass = mutableClassDefByOrNull(GEOCACHE_DETAILS_ACTIVITY_COMPANION) ?: return
    val aMethod = companionClass.methods.firstOrNull {
        it.name == "a" && it.implementation != null
    } ?: return

    val impl = aMethod.implementation ?: return
    val instructions = impl.instructions.toList()

    for (i in instructions.indices) {
        val inst = instructions[i]
        if (inst.opcode == Opcode.CONST_CLASS) {
            val typeRef = (inst as? ReferenceInstruction)?.reference as? com.android.tools.smali.dexlib2.iface.reference.TypeReference
            if (typeRef?.type == "Lcom/groundspeak/geocaching/intro/geocachedetails/GeocacheDetailsActivity2;") {
                val reg = (inst as OneRegisterInstruction).registerA
                aMethod.replaceInstruction(
                    i,
                    "const-class v$reg, Lcom/groundspeak/geocaching/intro/geocachedetails/GeocacheDetailsActivity;"
                )
                break
            }
        }
    }
}

/**
 * Ensures WebView in DescriptionFragment (r64.y) uses HTML line breaks (<br/><br/>\n\n) between
 * the short description ("Premium Member Only Cache") and the long description so that WebView
 * renders a blank line instead of collapsing the whitespace.
 */
private fun BytecodePatchContext.patchDescriptionFragmentWebViewLineBreak() {
    val r64Class = mutableClassDefByOrNull(DESCRIPTION_FRAGMENT_CLASS) ?: return
    val yMethod = r64Class.methods.firstOrNull {
        it.name == "y" && it.implementation != null
    } ?: return

    val impl = yMethod.implementation ?: return
    val instructions = impl.instructions.toList()

    for (i in instructions.indices) {
        val inst = instructions[i]
        if (inst.opcode == Opcode.CONST_STRING) {
            val strRef = (inst as? ReferenceInstruction)?.reference as? StringReference
            if (strRef?.string == "\n\n") {
                val reg = (inst as OneRegisterInstruction).registerA
                yMethod.replaceInstruction(i, "const-string v$reg, \"<br/><br/>\\n\\n\"")
                break
            }
        }
    }
}
