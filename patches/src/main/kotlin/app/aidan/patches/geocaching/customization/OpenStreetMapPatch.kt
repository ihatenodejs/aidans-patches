package app.aidan.patches.geocaching.customization

import app.aidan.patches.geocaching.shared.COMPATIBILITY_GEOCACHING
import app.morphe.patcher.extensions.InstructionExtensions.addInstructions
import app.morphe.patcher.patch.BytecodePatchContext
import app.morphe.patcher.patch.PatchException
import app.morphe.patcher.patch.bytecodePatch
import app.morphe.patcher.patch.stringOption
import com.android.tools.smali.dexlib2.iface.instruction.OneRegisterInstruction
import com.android.tools.smali.dexlib2.iface.instruction.ReferenceInstruction
import com.android.tools.smali.dexlib2.iface.reference.StringReference

private const val MAP_DECIDER_FRAGMENT = "Lcom/groundspeak/geocaching/intro/mainmap/map/MapDeciderFragment;"
private const val NAVIGATION_VIEW_MODEL = "Lcom/groundspeak/geocaching/intro/navigationmap/NavigationViewModel;"
private const val USER_MAP_PREFS = "Lk4c;"
private const val MAP_TYPE = "Lcom/groundspeak/geocaching/intro/map/type/MapType;"
private const val ORIGINAL_STYLE_URL = "https://maptiles.geocaching.com/vector/style.json"
private const val DEFAULT_OSM_STYLE_URL = "https://tiles.openfreemap.org/styles/bright"
private const val FTUE_SUGGESTION_FLOW_STATE = "Li14;"
private const val FTUE_NAMESPACE_STRING = "FtueSuggestionFlowState.NAMESPACE"
private const val ONBOARDING_MAP_ACTIVITY = "Lcom/groundspeak/geocaching/intro/onboarding/OnboardingMapActivity;"
private const val MAIN_ACTIVITY = "Lcom/groundspeak/geocaching/intro/main/MainActivity;"

@Suppress("unused")
val openStreetMapPatch = bytecodePatch(
    name = "OpenStreetMap Drop-in Replacement",
    description = "Replaces Google Maps with OpenStreetMap (MapLibre vector engine) across the main map and navigation screens, removing the Google watermark and rendering community-driven OpenStreetMap tiles.",
    default = true
) {
    category("Customization")
    compatibleWith(COMPATIBILITY_GEOCACHING)

    val styleUrlOption = stringOption(
        key = "styleUrl",
        default = DEFAULT_OSM_STYLE_URL,
        title = "OpenStreetMap Style URL",
        description = "URL of the MapLibre/OpenMapTiles style JSON defining OpenStreetMap tiles and layers. Examples: https://tiles.openfreemap.org/styles/bright, https://tiles.openfreemap.org/styles/liberty, https://tiles.openfreemap.org/styles/positron"
    )

    execute {
        val styleUrl = styleUrlOption.value?.trim().takeIf { !it.isNullOrEmpty() }
            ?: DEFAULT_OSM_STYLE_URL

        // 1. Force UserMapPrefs (k4c.e()) to return MapType.M (TRAILS), which selects MapLibre everywhere
        patchUserMapPrefs()

        // 2. Force MapDeciderFragment to always select the MapLibre destination
        patchMapDeciderFragment()

        // 3. Force NavigationViewModel.h() to return true so NavigationMapActivity uses MapLibre
        patchNavigationViewModel()

        // 4. Replace proprietary Groundspeak maptiles URL with the OpenStreetMap style URL in all classes
        patchStyleUrls(styleUrl)

        // 5. Neutralize FTUE onboarding suggestion flow so MainActivity doesn't redirect to OnboardingMapActivity
        patchFtueSuggestionFlowState()

        // 6. Redirect OnboardingMapActivity to MainActivity as a defensive fallback
        patchOnboardingMapActivity()
    }
}

/**
 * In UserMapPrefs (k4c), forces e() to always return MapType.M (TRAILS).
 * This switches the entire app's map engine to MapLibre, eliminating Google Maps and its watermark.
 */
private fun BytecodePatchContext.patchUserMapPrefs() {
    val prefsClass = mutableClassDefByOrNull(USER_MAP_PREFS)
        ?: throw PatchException("Class $USER_MAP_PREFS not found")

    val eMethod = prefsClass.methods.firstOrNull { it.name == "e" && it.returnType == MAP_TYPE && it.implementation != null }
        ?: throw PatchException("Method e() not found in $USER_MAP_PREFS")

    eMethod.addInstructions(
        0,
        """
            sget-object v0, $MAP_TYPE->M:$MAP_TYPE
            return-object v0
        """.trimIndent()
    )
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
            val stringRef = (instruction.reference as? com.android.tools.smali.dexlib2.iface.reference.MethodReference)
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
 * Replaces occurrences of the proprietary maptiles URL with the OpenStreetMap style URL.
 */
private fun BytecodePatchContext.patchStyleUrls(styleUrl: String) {
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
                                const-string v$reg, "$styleUrl"
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
 * This prevents MainActivity from intercepting cold start and redirecting to OnboardingMapActivity,
 * ensuring users directly access MainActivity with the top bar, bottom navbar, and OpenStreetMap.
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
 * In OnboardingMapActivity, redirects onCreate to MainActivity and calls finish().
 * Also neutralizes onDestroy so it never crashes if presenter or binding was uninitialized.
 * This provides a defensive fallback if OnboardingMapActivity is ever launched directly.
 */
private fun BytecodePatchContext.patchOnboardingMapActivity() {
    val activityClass = mutableClassDefByOrNull(ONBOARDING_MAP_ACTIVITY)
        ?: return

    // Neutralize onDestroy so it never crashes on uninitialized lateinit properties
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
