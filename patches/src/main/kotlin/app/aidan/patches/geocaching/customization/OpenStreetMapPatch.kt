package app.aidan.patches.geocaching.customization

import app.aidan.patches.geocaching.shared.COMPATIBILITY_GEOCACHING
import app.morphe.patcher.extensions.InstructionExtensions.addInstructions
import app.morphe.patcher.patch.BytecodePatchContext
import app.morphe.patcher.patch.PatchException
import app.morphe.patcher.patch.bytecodePatch
import app.morphe.patcher.patch.resourcePatch
import com.android.tools.smali.dexlib2.Opcode
import com.android.tools.smali.dexlib2.iface.instruction.OneRegisterInstruction
import com.android.tools.smali.dexlib2.iface.instruction.ReferenceInstruction
import com.android.tools.smali.dexlib2.iface.instruction.TwoRegisterInstruction
import com.android.tools.smali.dexlib2.iface.reference.FieldReference
import com.android.tools.smali.dexlib2.iface.reference.MethodReference
import com.android.tools.smali.dexlib2.iface.reference.StringReference
import org.w3c.dom.Element
private const val MAP_DECIDER_FRAGMENT = "Lcom/groundspeak/geocaching/intro/mainmap/map/MapDeciderFragment;"
private const val NAVIGATION_VIEW_MODEL = "Lcom/groundspeak/geocaching/intro/navigationmap/NavigationViewModel;"
private const val MAP_TYPE = "Lcom/groundspeak/geocaching/intro/map/type/MapType;"
private const val MAP_TYPE_SELECTION_FRAGMENT = "Lcom/groundspeak/geocaching/intro/map/type/MapTypeSelectionFragment;"
private const val ORIGINAL_STYLE_URL = "https://maptiles.geocaching.com/vector/style.json"
private const val FTUE_SUGGESTION_FLOW_STATE = "Li14;"
private const val FTUE_NAMESPACE_STRING = "FtueSuggestionFlowState.NAMESPACE"
private const val ONBOARDING_MAP_ACTIVITY = "Lcom/groundspeak/geocaching/intro/onboarding/OnboardingMapActivity;"
private const val MAIN_ACTIVITY = "Lcom/groundspeak/geocaching/intro/main/MainActivity;"
private const val OSM_STYLE_BRIDGE = "Lapp/aidan/extension/geocaching/OsmStyleBridge;"
private const val GOOGLE_MAP_FRAGMENT = "Lcom/groundspeak/geocaching/intro/mainmap/map/GoogleMapFragment;"
private const val MAP_LIBRE_FRAGMENT = "Lcom/groundspeak/geocaching/intro/mainmap/map/MapLibreFragment;"
private const val USER_MAP_PREFS = "Lk4c;"

private object OsmPreviewResourceHolder

/**
 * Resource companion patch that replaces proprietary Google Map type preview webp images
 * with rendered OpenStreetMap style thumbnails in a busy location.
 */
@Suppress("unused")
val openStreetMapPreviewAssetsPatch = resourcePatch(
    name = "OpenStreetMap Preview Assets",
    description = "Replaces proprietary Google Map type preview webp images with rendered OpenStreetMap style thumbnails in a busy location.",
    default = true
) {
    category("Customization")
    compatibleWith(COMPATIBILITY_GEOCACHING)

    execute {
        val previews = mapOf(
            "map_preview_trails.webp" to "geocaching/map_previews/map_preview_trails.webp",
            "map_preview_street.webp" to "geocaching/map_previews/map_preview_street.webp",
            "map_preview_satellite.webp" to "geocaching/map_previews/map_preview_satellite.webp",
            "map_preview_terrain.webp" to "geocaching/map_previews/map_preview_terrain.webp",
            "map_preview_hybrid.webp" to "geocaching/map_previews/map_preview_hybrid.webp"
        )

        val targetDirs = listOf(
            "res/drawable-xxhdpi",
            "res/drawable-xxhdpi-v4"
        )

        for ((fileName, resourcePath) in previews) {
            val resourceStream = OsmPreviewResourceHolder::class.java.classLoader.getResourceAsStream(resourcePath)
                ?: OsmPreviewResourceHolder::class.java.getResourceAsStream("/$resourcePath")
                ?: throw PatchException("Bundled preview asset $resourcePath not found in patch resources")

            val bytes = resourceStream.use { it.readBytes() }

            var written = false
            for (dir in targetDirs) {
                val targetFile = get("$dir/$fileName")
                if (targetFile.exists()) {
                    targetFile.outputStream().use { it.write(bytes) }
                    written = true
                }
            }
            if (!written) {
                val targetFile = get("res/drawable-xxhdpi/$fileName")
                targetFile.outputStream().use { it.write(bytes) }
            }
        }
    }
}

/**
 * Resource companion patch that renames Map types to Map styles and updates labels and descriptions
 * in strings.xml to match OpenStreetMap basemap presets.
 */
@Suppress("unused")
val openStreetMapResourcePatch = resourcePatch(
    name = "OpenStreetMap Resource Strings",
    description = "Updates map type names and descriptions in strings.xml to reflect OpenStreetMap styles.",
    default = true
) {
    category("Customization")
    compatibleWith(COMPATIBILITY_GEOCACHING)

    execute {
        document("res/values/strings.xml").use { doc ->
            val stringNodes = doc.getElementsByTagName("string")
            for (i in 0 until stringNodes.length) {
                val element = stringNodes.item(i) as? Element ?: continue
                when (element.getAttribute("name")) {
                    "map_type_trails" -> element.textContent = "Bright"
                    "map_description_trails" -> element.textContent = "Colorful OpenMapTiles basemap"
                    "map_type_street" -> element.textContent = "Liberty"
                    "map_description_street" -> element.textContent = "Clean, detailed OpenStreetMap style"
                    "map_type_satellite" -> element.textContent = "Positron"
                    "map_description_satellite" -> element.textContent = "Light and minimal basemap"
                    "map_type_terrain" -> element.textContent = "Dark"
                    "map_description_terrain" -> element.textContent = "Dark mode OpenStreetMap vector tiles"
                    "map_type_hybrid" -> element.textContent = "Custom URL"
                    "map_description_hybrid" -> element.textContent = "Custom MapLibre style JSON endpoint"
                    "premium_upsell_item_map_types" -> element.textContent = "Map styles"
                }
            }
        }
    }
}

@Suppress("unused")
val openStreetMapPatch = bytecodePatch(
    name = "OpenStreetMap Drop-in Replacement",
    description = "Replaces Google Maps with OpenStreetMap (MapLibre vector engine) and adds in-app OpenStreetMap style switching and custom URL configuration to Map settings.",
    default = true
) {
    category("Customization")
    compatibleWith(COMPATIBILITY_GEOCACHING)
    dependsOn(openStreetMapResourcePatch)
    dependsOn(openStreetMapPreviewAssetsPatch)
    extendWith("extensions/extension.mpe")

    execute {
        // 1. Force MapDeciderFragment to always select the MapLibre destination
        patchMapDeciderFragment()

        // 2. Force NavigationViewModel.h() to return true so NavigationMapActivity uses MapLibre
        patchNavigationViewModel()

        // 3. Dynamically route MapLibre style JSON requests to OsmStyleBridge.getActiveStyleUrl()
        patchDynamicStyleUrls()

        // 4. Neutralize FTUE onboarding suggestion flow so MainActivity doesn't redirect to OnboardingMapActivity
        patchFtueSuggestionFlowState()

        // 5. Redirect OnboardingMapActivity to MainActivity as a defensive fallback
        patchOnboardingMapActivity()

        // 6. Neutralize MainActivity suggestion flow redirect so it never bounces to OnboardingMapActivity
        patchMainActivitySuggestionBypass()

        // 7. Rewire MapTypeSelectionFragment to handle style selection via OsmStyleBridge
        patchMapTypeSelectionFragment()

        // 8. Prevent MapLibreFragment listener from redirecting to GoogleMap on non-Trails selection
        patchMapLibreFragmentResultListener()

        // 9. Prevent MapLibreFragment.onResume from redirecting to Google Maps when non-Trails style is active
        patchMapLibreFragmentOnResume()

        // 10. Force GoogleMapFragment.onResume to redirect to MapLibre unconditionally
        patchGoogleMapFragmentOnResume()

        // 11. Remove green Premium badge from Bright style (MapType.M / TRAILS)
        patchMapTypeFree()

        // 12. Enable Better Events toggle on the map screen (BETTER_EVENTS_MAP flag)
        patchBetterEventsMapFlag()
    }
}

/**
 * In MapDeciderFragment, ensures the navigation condition always evaluates to true,
 * directing the NavController to toMapLibreMapFragment (0x7f0a0579).
 */
private fun BytecodePatchContext.patchMapDeciderFragment() {
    val deciderClass = mutableClassDefByOrNull(MAP_DECIDER_FRAGMENT)
        ?: throw PatchException("Class $MAP_DECIDER_FRAGMENT not found")

    val onResumeMethod = deciderClass.methods.firstOrNull { it.name == "onResume" && it.implementation != null }
        ?: throw PatchException("Method onResume not found in $MAP_DECIDER_FRAGMENT")

    val instructions = onResumeMethod.implementation?.instructions?.toList() ?: emptyList()
    for ((index, instruction) in instructions.withIndex()) {
        if (instruction is ReferenceInstruction) {
            val stringRef = (instruction.reference as? MethodReference)
            if (stringRef?.name == "contains" && stringRef.definingClass == "Ljava/util/List;") {
                val moveResult = instructions.getOrNull(index + 1)
                if (moveResult is OneRegisterInstruction) {
                    val reg = moveResult.registerA
                    onResumeMethod.addInstructions(
                        index + 2,
                        """
                            const/4 v$reg, 0x1
                        """.trimIndent()
                    )
                    break
                }
            }
        }
    }
}

/**
 * In NavigationViewModel, forces h()Z to always return true, so NavigationMapActivity displays MapLibre.
 */
private fun BytecodePatchContext.patchNavigationViewModel() {
    val navVmClass = mutableClassDefByOrNull(NAVIGATION_VIEW_MODEL)
        ?: throw PatchException("Class $NAVIGATION_VIEW_MODEL not found")

    val hMethod = navVmClass.methods.firstOrNull { it.name == "h" && it.returnType == "Z" && it.implementation != null }
        ?: throw PatchException("Method h()Z not found in $NAVIGATION_VIEW_MODEL")

    hMethod.addInstructions(
        0,
        """
            const/4 v0, 0x1
            return v0
        """.trimIndent()
    )
}

/**
 * Replaces occurrences of the proprietary Groundspeak maptiles URL with dynamic resolution
 * via OsmStyleBridge.getActiveStyleUrl().
 */
private fun BytecodePatchContext.patchDynamicStyleUrls() {
    val classesWithStyle = getAllClassesWithString(ORIGINAL_STYLE_URL)
    for (classDef in classesWithStyle) {
        val mutableClass = mutableClassDefBy(classDef)
        for (method in mutableClass.methods) {
            val instructions = method.implementation?.instructions?.toList() ?: continue
            for ((index, instruction) in instructions.withIndex()) {
                if (instruction is ReferenceInstruction) {
                    val stringRef = instruction.reference as? StringReference
                    if (stringRef?.string == ORIGINAL_STYLE_URL) {
                        val reg = (instruction as OneRegisterInstruction).registerA
                        method.addInstructions(
                            index + 1,
                            """
                                invoke-static {}, $OSM_STYLE_BRIDGE->getActiveStyleUrl()Ljava/lang/String;
                                move-result-object v$reg
                            """.trimIndent()
                        )
                    }
                }
            }
        }
    }
}

/**
 * In FtueSuggestionFlowState (i14), forces d()Z and e()Z to always return false.
 * This prevents MainActivity from intercepting cold start and redirecting to OnboardingMapActivity.
 */
private fun BytecodePatchContext.patchFtueSuggestionFlowState() {
    val ftueClass = mutableClassDefByOrNull(FTUE_SUGGESTION_FLOW_STATE)
        ?: getAllClassesWithString(FTUE_NAMESPACE_STRING)
            .map { mutableClassDefBy(it) }
            .firstOrNull { cls -> cls.methods.any { it.name == "d" && it.returnType == "Z" } }
        ?: throw PatchException("Class $FTUE_SUGGESTION_FLOW_STATE not found")

    val dMethod = ftueClass.methods.firstOrNull { it.name == "d" && it.returnType == "Z" && it.implementation != null }
        ?: throw PatchException("Method d()Z not found in $FTUE_SUGGESTION_FLOW_STATE")
    dMethod.addInstructions(
        0,
        """
            const/4 v0, 0x0
            return v0
        """.trimIndent()
    )

    val eMethod = ftueClass.methods.firstOrNull { it.name == "e" && it.returnType == "Z" && it.implementation != null }
        ?: throw PatchException("Method e()Z not found in $FTUE_SUGGESTION_FLOW_STATE")
    eMethod.addInstructions(
        0,
        """
            const/4 v0, 0x0
            return v0
        """.trimIndent()
    )
}

/**
 * In OnboardingMapActivity, redirects onCreate to MainActivity with SKIP_INITIAL_SUGGESTION_FLOW = true,
 * and neutralizes onDestroy so it never crashes on uninitialized lateinit properties.
 */
private fun BytecodePatchContext.patchOnboardingMapActivity() {
    val activityClass = mutableClassDefByOrNull(ONBOARDING_MAP_ACTIVITY) ?: return

    activityClass.methods.firstOrNull {
        it.name == "onDestroy" && it.returnType == "V" && it.implementation != null
    }?.addInstructions(0, "return-void")

    val onCreateMethod = activityClass.methods.firstOrNull {
        it.name == "onCreate" && it.returnType == "V" && it.implementation != null
    } ?: return

    val instructions = onCreateMethod.implementation?.instructions?.toList() ?: return
    val superIndex = instructions.indexOfFirst {
        it.opcode.name.startsWith("invoke-super")
    }
    val insertIndex = if (superIndex != -1) superIndex + 1 else 0

    onCreateMethod.addInstructions(
        insertIndex,
        """
            move-object/from16 v0, p0
            new-instance v1, Landroid/content/Intent;
            const-class v2, $MAIN_ACTIVITY
            invoke-direct {v1, v0, v2}, Landroid/content/Intent;-><init>(Landroid/content/Context;Ljava/lang/Class;)V
            const-string v2, "MainActivity.SKIP_INITIAL_SUGGESTION_FLOW"
            const/4 v3, 0x1
            invoke-virtual {v1, v2, v3}, Landroid/content/Intent;->putExtra(Ljava/lang/String;Z)Landroid/content/Intent;
            invoke-virtual {v0, v1}, Landroid/app/Activity;->startActivity(Landroid/content/Intent;)V
            invoke-virtual {v0}, Landroid/app/Activity;->finish()V
            return-void
        """.trimIndent()
    )
}

/**
 * In MainActivity.G(MainActivityVM$b, MainActivity$NavDestination)V, intercepts the suggestion flow
 * by forcing any OnboardingMapActivity ($c) or PagingEducationActivity (b) navigation state to $a
 * (main map destination), ensuring MainActivity never launches OnboardingMapActivity.
 */
private fun BytecodePatchContext.patchMainActivitySuggestionBypass() {
    val mainActivityClass = mutableClassDefByOrNull(MAIN_ACTIVITY) ?: return
    val gMethod = mainActivityClass.methods.firstOrNull {
        it.name == "G" &&
            it.implementation != null &&
            it.parameterTypes.map(CharSequence::toString).firstOrNull() == "Lcom/groundspeak/geocaching/intro/main/MainActivityVM\$b;"
    } ?: return

    val bypassSmali = """
        move-object/from16 v1, p1
        instance-of v0, v1, Lcom/groundspeak/geocaching/intro/main/MainActivityVM${'$'}b${'$'}c;
        if-eqz v0, :cond_skip_override_c
        sget-object v1, Lcom/groundspeak/geocaching/intro/main/MainActivityVM${'$'}b${'$'}a;->a:Lcom/groundspeak/geocaching/intro/main/MainActivityVM${'$'}b${'$'}a;
        move-object/16 p1, v1
        :cond_skip_override_c
        instance-of v0, v1, Lcom/groundspeak/geocaching/intro/main/b;
        if-eqz v0, :cond_skip_override_b
        sget-object v1, Lcom/groundspeak/geocaching/intro/main/MainActivityVM${'$'}b${'$'}a;->a:Lcom/groundspeak/geocaching/intro/main/MainActivityVM${'$'}b${'$'}a;
        move-object/16 p1, v1
        :cond_skip_override_b
    """.trimIndent()

    gMethod.addInstructions(0, bypassSmali)
}

/**
 * Rewires MapTypeSelectionFragment.z to invoke OsmStyleBridge.onStyleSelected(this, mapTypeId)
 * whenever a map style is selected, bypassing Premium upsells and updating the style immediately.
 */
private fun BytecodePatchContext.patchMapTypeSelectionFragment() {
    val fragmentClass = mutableClassDefByOrNull(MAP_TYPE_SELECTION_FRAGMENT)
        ?: throw PatchException("Class $MAP_TYPE_SELECTION_FRAGMENT not found")

    val zMethod = fragmentClass.methods.firstOrNull { it.name == "z" && it.implementation != null }
        ?: throw PatchException("Method z not found in $MAP_TYPE_SELECTION_FRAGMENT")

    val impl = zMethod.implementation ?: throw PatchException("Missing implementation in $MAP_TYPE_SELECTION_FRAGMENT.z")
    val instructions = impl.instructions.toList()

    // Find instance-of check for l07 (MapType click)
    val l07Index = instructions.indexOfFirst { inst ->
        inst.opcode == Opcode.INSTANCE_OF && (inst as? ReferenceInstruction)?.reference?.toString()?.contains("l07") == true
    }
    if (l07Index >= 0) {
        val targetBlock = """
            check-cast p1, Ll07;
            iget-object v0, p1, Ll07;->a:$MAP_TYPE
            iget v0, v0, $MAP_TYPE->A:I
            invoke-static {p0, v0}, $OSM_STYLE_BRIDGE->onStyleSelected(Ljava/lang/Object;I)V
            return-void
        """.trimIndent()
        zMethod.addInstructions(l07Index + 2, targetBlock)
    }
}

/**
 * Patches MapLibreFragment's fragment result listener (hy6) to prevent it from navigating
 * back to Google Maps when any map style is selected.
 */
private fun BytecodePatchContext.patchMapLibreFragmentResultListener() {
    val hy6Class = mutableClassDefByOrNull("Lhy6;") ?: return
    val invokeMethod = hy6Class.methods.firstOrNull { it.name == "invoke" && it.implementation != null } ?: return

    val instructions = invokeMethod.implementation?.instructions?.toList() ?: emptyList()

    val containsIndex = instructions.indexOfFirst { inst ->
        inst.opcode == Opcode.INVOKE_INTERFACE &&
            (inst as? ReferenceInstruction)?.reference?.let { (it as? MethodReference)?.name == "contains" } == true
    }

    if (containsIndex >= 0) {
        val moveResult = instructions.getOrNull(containsIndex + 1)
        if (moveResult is OneRegisterInstruction) {
            val reg = moveResult.registerA
            invokeMethod.addInstructions(
                containsIndex + 2,
                """
                    const/4 v$reg, 0x1
                """.trimIndent()
            )
        }
    }
}

/**
 * Enables Better Events on the map screen (LaunchDarkly BETTER_EVENTS_MAP = O)
 * so the Event Dates option appears on the Map settings sheet.
 */
private fun BytecodePatchContext.patchBetterEventsMapFlag() {
    val le7Class = mutableClassDefByOrNull("Lle7;") ?: return
    val invokeMethod = le7Class.methods.firstOrNull { it.name == "invoke" && it.implementation != null } ?: return
    val insts = invokeMethod.implementation?.instructions?.toList() ?: return

    for ((index, inst) in insts.withIndex()) {
        if (inst.opcode == Opcode.SGET_OBJECT && (inst as? ReferenceInstruction)?.reference?.toString()?.contains("LaunchDarklyFlag;->O") == true) {
            val next = insts.getOrNull(index + 2)
            if (next is OneRegisterInstruction && next.opcode == Opcode.MOVE_RESULT) {
                val reg = next.registerA
                invokeMethod.addInstructions(index + 3, "const/4 v$reg, 0x1")
                break
            }
        }
    }
}

/**
 * In MapLibreFragment.onResume, forces the k4c.e() result to MapType.M (TRAILS),
 * preventing it from ever navigating to Google Maps (0x7f0a0567).
 */
private fun BytecodePatchContext.patchMapLibreFragmentOnResume() {
    val fragmentClass = mutableClassDefByOrNull(MAP_LIBRE_FRAGMENT) ?: return
    val onResumeMethod = fragmentClass.methods.firstOrNull { it.name == "onResume" && it.implementation != null } ?: return
    val impl = onResumeMethod.implementation ?: return
    val instructions = impl.instructions.toList()

    for ((index, instruction) in instructions.withIndex()) {
        if (instruction is ReferenceInstruction) {
            val methodRef = instruction.reference as? MethodReference
            if (methodRef?.name == "e" && methodRef.definingClass == USER_MAP_PREFS) {
                val next = instructions.getOrNull(index + 1)
                if (next is OneRegisterInstruction && next.opcode == Opcode.MOVE_RESULT_OBJECT) {
                    val reg = next.registerA
                    onResumeMethod.addInstructions(
                        index + 2,
                        """
                            sget-object v$reg, $MAP_TYPE->M:$MAP_TYPE
                        """.trimIndent()
                    )
                    break
                }
            }
        }
    }
}

/**
 * In GoogleMapFragment.onResume, forces the k4c.e() result to MapType.M (TRAILS),
 * ensuring that if GoogleMapFragment is ever opened, it immediately navigates to MapLibre (0x7f0a0579).
 */
private fun BytecodePatchContext.patchGoogleMapFragmentOnResume() {
    val fragmentClass = mutableClassDefByOrNull(GOOGLE_MAP_FRAGMENT) ?: return
    val onResumeMethod = fragmentClass.methods.firstOrNull { it.name == "onResume" && it.implementation != null } ?: return
    val impl = onResumeMethod.implementation ?: return
    val instructions = impl.instructions.toList()

    for ((index, instruction) in instructions.withIndex()) {
        if (instruction is ReferenceInstruction) {
            val methodRef = instruction.reference as? MethodReference
            if (methodRef?.name == "e" && methodRef.definingClass == USER_MAP_PREFS) {
                val next = instructions.getOrNull(index + 1)
                if (next is OneRegisterInstruction && next.opcode == Opcode.MOVE_RESULT_OBJECT) {
                    val reg = next.registerA
                    onResumeMethod.addInstructions(
                        index + 2,
                        """
                            sget-object v$reg, $MAP_TYPE->M:$MAP_TYPE
                        """.trimIndent()
                    )
                    break
                }
            }
        }
    }
}

/**
 * Removes the green Premium badge from Bright (MapType.M / TRAILS) by forcing MapType.E to false.
 */
private fun BytecodePatchContext.patchMapTypeFree() {
    val mapTypeClass = mutableClassDefByOrNull(MAP_TYPE) ?: return
    for (method in mapTypeClass.methods) {
        if (method.name == "<init>" && method.implementation != null) {
            val insts = method.implementation?.instructions?.toList() ?: continue
            for ((index, inst) in insts.withIndex()) {
                if (inst.opcode == Opcode.IPUT_BOOLEAN && (inst as? ReferenceInstruction)?.reference?.let { (it as? FieldReference)?.name == "E" } == true) {
                    val reg = (inst as TwoRegisterInstruction).registerA
                    method.addInstructions(index, "const/4 v$reg, 0x0")
                    break
                }
            }
        }
    }
}
