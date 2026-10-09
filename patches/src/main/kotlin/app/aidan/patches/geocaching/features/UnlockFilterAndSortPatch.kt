package app.aidan.patches.geocaching.features

import app.aidan.patches.geocaching.shared.COMPATIBILITY_GEOCACHING
import app.aidan.patches.geocaching.shared.patchAllL3cChecksToValue
import app.morphe.patcher.extensions.InstructionExtensions.addInstructions
import app.morphe.patcher.patch.BytecodePatchContext
import app.morphe.patcher.patch.PatchException
import app.morphe.patcher.patch.bytecodePatch

val unlockFilterAndSortPatch = bytecodePatch(
    name = "Unlock Cache Filter and Sorting Tools",
    description = "Unlocks advanced cache search filters and sorting options without prompting for Geocaching Premium.",
    default = true
) {
    category("Features")
    compatibleWith(COMPATIBILITY_GEOCACHING)

    execute {
        patchMapFilterButtons()
        patchFilterViewModel()
        patchFilterScreenDialog()
        patchSortingTools()
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
