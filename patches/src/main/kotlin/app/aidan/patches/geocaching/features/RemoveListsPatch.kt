package app.aidan.patches.geocaching.features

import app.aidan.patches.geocaching.shared.COMPATIBILITY_GEOCACHING
import app.morphe.patcher.extensions.InstructionExtensions.addInstructions
import app.morphe.patcher.patch.BytecodePatchContext
import app.morphe.patcher.patch.PatchException
import app.morphe.patcher.patch.bytecodePatch
import app.morphe.patcher.patch.resourcePatch
import com.android.tools.smali.dexlib2.iface.instruction.NarrowLiteralInstruction
import org.w3c.dom.Element

private const val ANDROID_NAMESPACE = "http://schemas.android.com/apk/res/android"
private const val MENU_PATH = "res/menu/bottom_nav_menu.xml"
private const val LIST_NAV_GRAPH_ID = 0x7f0a02e8

/**
 * Resource companion patch that removes the Lists item from bottom_nav_menu.xml.
 */
val removeListsResourcePatch = resourcePatch(
    name = "Remove Lists Resource",
    description = "Removes the Lists option from bottom_nav_menu.xml.",
    default = true
) {
    category("Interface")
    compatibleWith(COMPATIBILITY_GEOCACHING)

    execute {
        document(MENU_PATH).use { document ->
            val itemNodes = document.getElementsByTagName("item")
            for (i in 0 until itemNodes.length) {
                val element = itemNodes.item(i) as? Element ?: continue
                val id = element.getAttributeNS(ANDROID_NAMESPACE, "id")
                    .ifEmpty { element.getAttribute("android:id") }
                    .trim()

                if (id == "@+id/list_nav_graph" || id == "@id/list_nav_graph" || id.endsWith("list_nav_graph")) {
                    element.parentNode?.removeChild(element)
                    break
                }
            }
        }
    }
}

/**
 * Bytecode patch that neutralizes the onResume notification badge lookup for list_nav_graph,
 * preventing NullPointerException when the tab is removed from the navbar.
 */
val removeListsPatch = bytecodePatch(
    name = "Remove Lists",
    description = "Removes the Lists option from the bottom navigation bar.",
    default = true
) {
    category("Interface")
    compatibleWith(COMPATIBILITY_GEOCACHING)
    dependsOn(removeListsResourcePatch)

    execute {
        patchOnResumeListBadge()
    }
}

private fun BytecodePatchContext.patchOnResumeListBadge() {
    val coroutineClass = mutableClassDefBy(
        "Lcom/groundspeak/geocaching/intro/main/MainActivity\$onResume\$3\$1\$1;"
    )
    val invokeSuspendMethod = coroutineClass.methods.firstOrNull {
        it.name == "invokeSuspend" && it.implementation != null
    } ?: throw PatchException("MainActivity\$onResume\$3\$1\$1.invokeSuspend not found")

    val impl = invokeSuspendMethod.implementation
        ?: throw PatchException("Missing implementation in invokeSuspend")
    val instructions = impl.instructions.toList()

    val constIdx = instructions.indexOfFirst {
        (it as? NarrowLiteralInstruction)?.narrowLiteral == LIST_NAV_GRAPH_ID
    }
    if (constIdx < 0) {
        throw PatchException("Could not find list_nav_graph literal (0x7f0a02e8) in invokeSuspend")
    }

    val prevInst = if (constIdx > 0) instructions[constIdx - 1] else null
    val prevPrevInst = if (constIdx > 1) instructions[constIdx - 2] else null
    if (prevInst?.opcode == com.android.tools.smali.dexlib2.Opcode.RETURN_OBJECT &&
        prevPrevInst?.opcode == com.android.tools.smali.dexlib2.Opcode.SGET_OBJECT
    ) {
        return
    }

    invokeSuspendMethod.addInstructions(
        constIdx,
        """
        sget-object p0, Lszb;->a:Lszb;
        return-object p0
        """.trimIndent()
    )
}
