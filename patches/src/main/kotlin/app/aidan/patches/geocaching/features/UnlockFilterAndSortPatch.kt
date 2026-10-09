package app.aidan.patches.geocaching.features

import app.aidan.patches.geocaching.shared.COMPATIBILITY_GEOCACHING
import app.aidan.patches.geocaching.shared.patchAllL3cChecksToValue
import app.aidan.patches.geocaching.shared.patchIncludeOwnedDisabledCachesToNull
import app.aidan.patches.geocaching.shared.patchSanitizeFilterModel
import app.aidan.patches.geocaching.shared.patchCz9OmitAllCacheTypes
import app.morphe.patcher.extensions.InstructionExtensions.addInstructions
import app.morphe.patcher.patch.BytecodePatchContext
import app.morphe.patcher.patch.PatchException
import com.android.tools.smali.dexlib2.Opcode
import com.android.tools.smali.dexlib2.iface.instruction.FiveRegisterInstruction
import com.android.tools.smali.dexlib2.iface.instruction.RegisterRangeInstruction
import com.android.tools.smali.dexlib2.iface.instruction.ReferenceInstruction
import com.android.tools.smali.dexlib2.iface.reference.MethodReference
import app.morphe.patcher.patch.bytecodePatch
val unlockFilterAndSortPatch = bytecodePatch(
    name = "Unlock Cache Filter and Sorting Tools",
    description = "Unlocks advanced cache search filters and sorting options without prompting for Geocaching Premium.",
    default = true
) {
    category("Features")
    compatibleWith(COMPATIBILITY_GEOCACHING)
    extendWith("extensions/extension.mpe")

    execute {
        patchMapFilterButtons()
        patchFilterViewModel()
        patchFilterScreenDialog()
        patchSortingTools()
        patchFilterPreferences()
        patchMapRenderingFilter()
        patchSharedMapViewModel()
        patchMapLibreFragmentOnResume()
    }
}

/**
 * Bypasses the Premium upsell when clicking the map screen top-right filter button,
 * opening the filter navigation graph directly.
 */
private fun BytecodePatchContext.patchMapFilterButtons() {
    // 1. GoogleMapFragment menu selection handler (e.smali)
    val googleMapMenuClass = mutableClassDefByOrNull("Lcom/groundspeak/geocaching/intro/mainmap/map/e;")
        ?: throw PatchException("GoogleMapFragment menu handler (e) not found")

    val googleMapMenuMethod = googleMapMenuClass.methods.firstOrNull {
        it.name == "c" && it.implementation != null && it.parameterTypes == listOf("Landroid/view/MenuItem;")
    } ?: throw PatchException("GoogleMapFragment menu handler c(MenuItem) not found")

    patchAllL3cChecksToValue(googleMapMenuMethod, "GoogleMapFragment menu handler (e.c)", value = true)

    // 2. MapLibreFragment menu selection handler (k.smali)
    val mapLibreMenuClass = mutableClassDefByOrNull("Lcom/groundspeak/geocaching/intro/mainmap/map/k;")
        ?: throw PatchException("MapLibreFragment menu handler (k) not found")

    val mapLibreMenuMethod = mapLibreMenuClass.methods.firstOrNull {
        it.name == "c" && it.implementation != null && it.parameterTypes == listOf("Landroid/view/MenuItem;")
    } ?: throw PatchException("MapLibreFragment menu handler c(MenuItem) not found")

    patchAllL3cChecksToValue(mapLibreMenuMethod, "MapLibreFragment menu handler (k.c)", value = true)
}

/**
 * Unlocks all 12 filter categories in FilterViewModel (cache types, sizes, difficulty,
 * terrain, attributes, favorite points, placed dates, found, and owned filters).
 */
private fun BytecodePatchContext.patchFilterViewModel() {
    val filterVmClass = mutableClassDefByOrNull("Lcom/groundspeak/geocaching/intro/geocachefilter/FilterViewModel;")
        ?: throw PatchException("FilterViewModel not found")

    val eMethod = filterVmClass.methods.firstOrNull {
        it.name == "e" && it.implementation != null && it.returnType == "Ljava/util/ArrayList;"
    } ?: throw PatchException("FilterViewModel.e not found")

    patchAllL3cChecksToValue(eMethod, "FilterViewModel.e", value = true)

    // PopulatePrefs coroutine flow
    val populatePrefsClass = mutableClassDefByOrNull("Lcom/groundspeak/geocaching/intro/geocachefilter/FilterViewModel\$populatePrefs\$1;")
    if (populatePrefsClass != null) {
        val invokeSuspendMethod = populatePrefsClass.methods.firstOrNull {
            it.name == "invokeSuspend" && it.implementation != null
        }
        if (invokeSuspendMethod != null) {
            patchAllL3cChecksToValue(invokeSuspendMethod, "FilterViewModel\$populatePrefs\$1.invokeSuspend", value = true)
        }
    }
}

/**
 * Neutralizes any remaining upsell dialog clicks in FilterScreen.
 */
private fun BytecodePatchContext.patchFilterScreenDialog() {
    val dialogListenerClass = mutableClassDefByOrNull("Ljk3;") ?: return
    val onClickMethod = dialogListenerClass.methods.firstOrNull {
        it.name == "onClick" && it.implementation != null && it.parameterTypes == listOf("Landroid/content/DialogInterface;", "I")
    } ?: return

    onClickMethod.addInstructions(
        0,
        """
        invoke-interface {p1}, Landroid/content/DialogInterface;->dismiss()V
        return-void
        """.trimIndent()
    )
}

/**
 * Unlocks list view and GeoTour sorting options, displaying the sortByDialog without
 * prompting for Geocaching Premium.
 */
private fun BytecodePatchContext.patchSortingTools() {
    // 1. GeocacheListFragment sorting listener (kb4.smali)
    val listSortClass = mutableClassDefByOrNull("Lkb4;")
        ?: throw PatchException("GeocacheListFragment sort listener (kb4) not found")

    val invokeMethod = listSortClass.methods.firstOrNull {
        it.name == "invoke" && it.implementation != null && it.parameterTypes.isEmpty()
    } ?: throw PatchException("GeocacheListFragment sort listener invoke() not found")

    patchAllL3cChecksToValue(invokeMethod, "GeocacheListFragment sort listener (kb4.invoke)", value = true)

    // 2. GeoTour sorting presenter (hf4.smali)
    val geoTourSortClass = mutableClassDefByOrNull("Lhf4;")
    if (geoTourSortClass != null) {
        val pMethod = geoTourSortClass.methods.firstOrNull {
            it.name == "p" && it.implementation != null && it.returnType == "Lszb;"
        }
        if (pMethod != null) {
            patchAllL3cChecksToValue(pMethod, "GeoTour sort presenter (hf4.p)", value = true)
        }
    }
}

/**
 * Unlocks FilterPreferences (dk3) and ak3 so that saved user filter criteria (cache types, sizes,
 * difficulty, terrain, attributes, favorite points, found, owned, etc.) are actually loaded into
 * FilterModel (ck3) and applied to the map, while nulling out `includeOwnedDisabledCaches` to avoid
 * HTTP 403 Forbidden on the server.
 */
private fun BytecodePatchContext.patchFilterPreferences() {
    // 1. FilterPreferences (dk3)
    val filterPrefsClass = mutableClassDefByOrNull("Ldk3;")
        ?: throw PatchException("FilterPreferences (dk3) not found")

    // Force l3c.d() in dk3.h to true so saved preferences are loaded instead of empty defaults
    val hMethod = filterPrefsClass.methods.firstOrNull {
        it.name == "h" && it.returnType == "Lck3;" && it.parameterTypes.isEmpty() && it.implementation != null
    } ?: throw PatchException("FilterPreferences.h not found")
    patchAllL3cChecksToValue(hMethod, "FilterPreferences.h", value = true)

    // Null out includeOwnedDisabledCaches so map search avoids HTTP 403 Forbidden
    patchIncludeOwnedDisabledCachesToNull(hMethod)
    patchSanitizeFilterModel(hMethod)

    // Force l3c.d() in dk3.b to true so saved attributes are loaded instead of EmptyList
    val bMethod = filterPrefsClass.methods.firstOrNull {
        it.name == "b" && it.returnType == "Ljava/util/List;" && it.parameterTypes.isEmpty() && it.implementation != null
    } ?: throw PatchException("FilterPreferences.b not found")
    patchAllL3cChecksToValue(bMethod, "FilterPreferences.b", value = true)

    // 2. ak3 (difficulty & terrain filter parser)
    val ak3Class = mutableClassDefByOrNull("Lak3;")
    if (ak3Class != null) {
        val invokeMethod = ak3Class.methods.firstOrNull {
            it.name == "invoke" && it.implementation != null
        }
        if (invokeMethod != null) {
            patchAllL3cChecksToValue(invokeMethod, "ak3.invoke", value = true)
        }
    }

    // 3. Search parameter builder (cz9.a) - omit cacheTypes if all cache types are selected
    val cz9Class = mutableClassDefByOrNull("Lcz9;")
    if (cz9Class != null) {
        val aMethod = cz9Class.methods.firstOrNull {
            it.name == "a" && it.returnType == "Lce4;" && it.implementation != null
        }
        if (aMethod != null) {
            patchCz9OmitAllCacheTypes(aMethod)
        }
    }
}

/**
 * In MapRendering.e (com.groundspeak.geocaching.intro.map.rendering.a.e), filters map items
 * against the user's active filter criteria before generating pins for the map.
 */
private fun BytecodePatchContext.patchMapRenderingFilter() {
    val mapRenderingClass = mutableClassDefByOrNull("Lcom/groundspeak/geocaching/intro/map/rendering/a;")
        ?: throw PatchException("Map rendering class (a) not found")

    val eMethod = mapRenderingClass.methods.firstOrNull {
        it.name == "e" && it.implementation != null && it.parameterTypes == listOf("Ljava/util/List;", "Ll3c;")
    } ?: throw PatchException("a.e(List, l3c) not found")

    eMethod.addInstructions(
        0,
        """
        invoke-static/range {p0 .. p0}, Lapp/aidan/extension/geocaching/GeocacheFilterBridge;->filterMapItems(Ljava/util/List;)Ljava/util/List;
        move-result-object p0
        """.trimIndent()
    )
}

/**
 * In SharedMapViewModel:
 * 1. trySearching$1: updates master list and filters map items loaded from Room DB.
 * 2. trySearching$2: updates master list and filters map items loaded after network search.
 * 3. reloadPins$1: reapplies active filters to master list on filter change.
 * 4. refreshFilteredState$1: triggers reloadPins() whenever filter state is refreshed.
 */
private fun BytecodePatchContext.patchSharedMapViewModel() {
    // 1. trySearching$1
    val trySearching1Class = mutableClassDefByOrNull("Lcom/groundspeak/geocaching/intro/mainmap/map/SharedMapViewModel\$trySearching\$1;")
    if (trySearching1Class != null) {
        val invokeSuspendMethod = trySearching1Class.methods.firstOrNull {
            it.name == "invokeSuspend" && it.implementation != null
        }
        if (invokeSuspendMethod != null) {
            val instructions = invokeSuspendMethod.implementation!!.instructions.toList()
            for ((idx, inst) in instructions.withIndex()) {
                if (inst.opcode == Opcode.INVOKE_VIRTUAL) {
                    val ref = (inst as? ReferenceInstruction)?.reference?.toString() ?: ""
                    if (ref.contains("Lkotlinx/coroutines/flow/m;->n(Ljava/lang/Object;)V")) {
                        val prev = instructions.getOrNull(idx - 1)
                        val prevRef = (prev as? ReferenceInstruction)?.reference?.toString() ?: ""
                        if (prevRef.contains("SharedMapViewModel;->p0")) {
                            val reg = (inst as? FiveRegisterInstruction)?.registerD
                                ?: (inst as? RegisterRangeInstruction)?.let { it.startRegister + 1 }
                                ?: 4
                            invokeSuspendMethod.addInstructions(
                                idx,
                                """
                                invoke-static {v$reg}, Lapp/aidan/extension/geocaching/GeocacheFilterBridge;->updateAndFilterMapItems(Ljava/util/List;)Ljava/util/List;
                                move-result-object v$reg
                                """.trimIndent()
                            )
                            break
                        }
                    }
                }
            }
        }
    }

    // 2. trySearching$2
    val trySearching2Class = mutableClassDefByOrNull("Lcom/groundspeak/geocaching/intro/mainmap/map/SharedMapViewModel\$trySearching\$2;")
    if (trySearching2Class != null) {
        val invokeSuspendMethod = trySearching2Class.methods.firstOrNull {
            it.name == "invokeSuspend" && it.implementation != null
        }
        if (invokeSuspendMethod != null) {
            val instructions = invokeSuspendMethod.implementation!!.instructions.toList()
            for ((idx, inst) in instructions.withIndex()) {
                if (inst.opcode == Opcode.INVOKE_VIRTUAL) {
                    val ref = (inst as? ReferenceInstruction)?.reference?.toString() ?: ""
                    if (ref.contains("Lkotlinx/coroutines/flow/m;->n(Ljava/lang/Object;)V")) {
                        val prev = instructions.getOrNull(idx - 1)
                        val prevRef = (prev as? ReferenceInstruction)?.reference?.toString() ?: ""
                        if (prevRef.contains("SharedMapViewModel;->p0")) {
                            val reg = (inst as? FiveRegisterInstruction)?.registerD
                                ?: (inst as? RegisterRangeInstruction)?.let { it.startRegister + 1 }
                                ?: 0
                            invokeSuspendMethod.addInstructions(
                                idx,
                                """
                                invoke-static {v$reg}, Lapp/aidan/extension/geocaching/GeocacheFilterBridge;->updateAndFilterMapItems(Ljava/util/List;)Ljava/util/List;
                                move-result-object v$reg
                                """.trimIndent()
                            )
                            break
                        }
                    }
                }
            }
        }
    }

    // 3. reloadPins$1
    val reloadPinsClass = mutableClassDefByOrNull("Lcom/groundspeak/geocaching/intro/mainmap/map/SharedMapViewModel\$reloadPins\$1;")
    if (reloadPinsClass != null) {
        val invokeSuspendMethod = reloadPinsClass.methods.firstOrNull {
            it.name == "invokeSuspend" && it.implementation != null
        }
        if (invokeSuspendMethod != null) {
            val instructions = invokeSuspendMethod.implementation!!.instructions.toList()
            val nCalls = instructions.indices.filter { idx ->
                val inst = instructions[idx]
                inst.opcode == Opcode.INVOKE_VIRTUAL &&
                    (inst as? ReferenceInstruction)?.reference?.toString()?.contains("Lkotlinx/coroutines/flow/m;->n(Ljava/lang/Object;)V") == true
            }
            if (nCalls.size >= 2) {
                val secondCallIdx = nCalls[1]
                val reg = (instructions[secondCallIdx] as? FiveRegisterInstruction)?.registerD
                    ?: (instructions[secondCallIdx] as? RegisterRangeInstruction)?.let { it.startRegister + 1 }
                    ?: 0
                invokeSuspendMethod.addInstructions(
                    secondCallIdx,
                    """
                    invoke-static {v$reg}, Lapp/aidan/extension/geocaching/GeocacheFilterBridge;->reapplyFilters(Ljava/util/List;)Ljava/util/List;
                    move-result-object v$reg
                    """.trimIndent()
                )
            }
        }
    }

    // 4. refreshFilteredState$1
    val refreshFilteredClass = mutableClassDefByOrNull("Lcom/groundspeak/geocaching/intro/mainmap/map/SharedMapViewModel\$refreshFilteredState\$1;")
    if (refreshFilteredClass != null) {
        val invokeSuspendMethod = refreshFilteredClass.methods.firstOrNull {
            it.name == "invokeSuspend" && it.implementation != null
        }
        if (invokeSuspendMethod != null) {
            val instructions = invokeSuspendMethod.implementation!!.instructions.toList()
            for ((idx, inst) in instructions.withIndex()) {
                if (inst.opcode == Opcode.INVOKE_VIRTUAL) {
                    val ref = (inst as? ReferenceInstruction)?.reference?.toString() ?: ""
                    if (ref.contains("Lkotlinx/coroutines/flow/m;->o(Ljava/lang/Object;Ljava/lang/Object;)Z")) {
                        invokeSuspendMethod.addInstructions(
                            idx + 1,
                            """
                            iget-object p1, p0, Lcom/groundspeak/geocaching/intro/mainmap/map/SharedMapViewModel${'$'}refreshFilteredState${'$'}1;->D:Lcom/groundspeak/geocaching/intro/mainmap/map/SharedMapViewModel;
                            invoke-virtual {p1}, Lcom/groundspeak/geocaching/intro/mainmap/map/SharedMapViewModel;->p()V
                            """.trimIndent()
                        )
                        break
                    }
                }
            }
        }
    }
}

/**
 * In MapLibreFragment.onResume, triggers SharedMapViewModel.p() (reloadPins)
 * so that when returning from FilterNavHostActivity, MapLibre reloads pins with newly applied filters.
 */
private fun BytecodePatchContext.patchMapLibreFragmentOnResume() {
    val mapLibreClass = mutableClassDefByOrNull("Lcom/groundspeak/geocaching/intro/mainmap/map/MapLibreFragment;")
        ?: throw PatchException("MapLibreFragment not found")

    val onResumeMethod = mapLibreClass.methods.firstOrNull {
        it.name == "onResume" && it.implementation != null && it.parameterTypes.isEmpty()
    } ?: throw PatchException("MapLibreFragment.onResume not found")

    val instructions = onResumeMethod.implementation!!.instructions.toList()
    val superCallIdx = instructions.indexOfFirst { inst ->
        inst.opcode == Opcode.INVOKE_SUPER &&
            (inst as? ReferenceInstruction)?.reference?.toString()?.contains("onResume") == true
    }
    if (superCallIdx != -1) {
        onResumeMethod.addInstructions(
            superCallIdx + 1,
            """
            invoke-virtual {p0}, Lcom/groundspeak/geocaching/intro/mainmap/map/MapLibreFragment;->v()Lcom/groundspeak/geocaching/intro/mainmap/map/SharedMapViewModel;
            move-result-object v0
            invoke-virtual {v0}, Lcom/groundspeak/geocaching/intro/mainmap/map/SharedMapViewModel;->p()V
            """.trimIndent()
        )
    }
}


