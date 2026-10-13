package app.aidan.patches.sidelineswap.features

import app.aidan.patches.shared.compositeFeaturePatch
import app.aidan.patches.sidelineswap.shared.COMPATIBILITY_SIDELINESWAP
import app.morphe.patcher.extensions.InstructionExtensions.addInstructions
import app.morphe.patcher.patch.BytecodePatchContext
import app.morphe.patcher.patch.PatchException
import com.android.tools.smali.dexlib2.Opcode
import com.android.tools.smali.dexlib2.iface.instruction.ReferenceInstruction
import org.w3c.dom.Element

private const val ANDROID_NS = "http://schemas.android.com/apk/res/android"
private const val ACCOUNT_DETAILS_LAYOUT = "res/layout/fragment_account_details.xml"
private const val BADGE_RED_LAYOUT = "res/layout/badge_red.xml"

private const val MAIN_ACTIVITY = "Lcom/sidelineswap/android/MainActivity;"
private const val DEEP_LINK_INVITE_TARGET = "Lcom/sidelineswap/android/MainActivity\$processDeepLink\$target\$7;"
private const val ACCOUNT_DETAILS_FRAGMENT = "Lcom/sidelineswap/android/account/AccountDetailsFragment;"

/**
 * Composite patch that removes the Invite Friends option and eliminates the phantom
 * orange dot on the Account navbar icon caused by a non-functional badge setup.
 */
@Suppress("unused")
val removeInviteFriendsPatch = compositeFeaturePatch(
    name = "Remove Invite Friends",
    description = "Removes the Invite Friends option from the Account screen and eliminates the non-functional orange dot notification badge from the Account navigation icon.",
    default = true
) {
    category("Interface")
    compatibleWith(COMPATIBILITY_SIDELINESWAP)

    resources {
        document(ACCOUNT_DETAILS_LAYOUT).use { doc ->
            val allElements = doc.getElementsByTagName("*")
            var foundInviteFriends = false
            var foundDivider = false

            for (i in 0 until allElements.length) {
                val elem = allElements.item(i) as? Element ?: continue
                val id = elem.getAttributeNS(ANDROID_NS, "id")
                    .ifEmpty { elem.getAttribute("android:id") }
                    .trim()

                if (id == "@id/inviteFriends" || id == "@+id/inviteFriends" || id.endsWith("inviteFriends")) {
                    elem.setAttribute("android:visibility", "gone")
                    foundInviteFriends = true

                    // Find the trailing divider View directly following inviteFriends
                    var next = elem.nextSibling
                    while (next != null && next !is Element) {
                        next = next.nextSibling
                    }
                    if (next is Element && next.tagName == "View") {
                        next.setAttribute("android:visibility", "gone")
                        foundDivider = true
                    }
                    break
                }
            }

            if (!foundInviteFriends) {
                throw PatchException("[$ACCOUNT_DETAILS_LAYOUT] Could not find @id/inviteFriends element")
            }
            if (!foundDivider) {
                throw PatchException("[$ACCOUNT_DETAILS_LAYOUT] Could not find divider View following @id/inviteFriends")
            }
        }

        runCatching {
            document(BADGE_RED_LAYOUT).use { doc ->
                val images = doc.getElementsByTagName("ImageView")
                for (i in 0 until images.length) {
                    val elem = images.item(i) as? Element ?: continue
                    elem.setAttribute("android:visibility", "gone")
                }
            }
        }

        runCatching {
            document("AndroidManifest.xml").use { doc ->
                val manifest = doc.documentElement
                if (manifest.hasAttribute("android:requiredSplitTypes")) {
                    manifest.removeAttribute("android:requiredSplitTypes")
                }
                if (manifest.hasAttributeNS(ANDROID_NS, "requiredSplitTypes")) {
                    manifest.removeAttributeNS(ANDROID_NS, "requiredSplitTypes")
                }
                if (manifest.hasAttribute("android:splitTypes")) {
                    manifest.removeAttribute("android:splitTypes")
                }
                if (manifest.hasAttributeNS(ANDROID_NS, "splitTypes")) {
                    manifest.removeAttributeNS(ANDROID_NS, "splitTypes")
                }

                val metaDataNodes = doc.getElementsByTagName("meta-data")
                val toRemove = mutableListOf<Element>()
                for (i in 0 until metaDataNodes.length) {
                    val elem = metaDataNodes.item(i) as? Element ?: continue
                    val name = elem.getAttributeNS(ANDROID_NS, "name")
                        .ifEmpty { elem.getAttribute("android:name") }
                    if (name == "com.android.vending.splits.required" || name == "com.android.vending.splits") {
                        toRemove.add(elem)
                    }
                }
                for (elem in toRemove) {
                    elem.parentNode?.removeChild(elem)
                }
            }
        }
    }

    bytecode {
        patchMainActivityAccountBadge()
        patchDeepLinkTarget()
        patchAccountDetailsFragment()
    }
}

/**
 * Neutralizes setupAccountBadge in MainActivity so badge_red is never inflated,
 * avoiding the ClassCastException and eliminating the non-functional orange dot.
 */
private fun BytecodePatchContext.patchMainActivityAccountBadge() {
    val mainActivityClass = classDefByOrNull(MAIN_ACTIVITY)
        ?: throw PatchException("Class $MAIN_ACTIVITY not found")
    val mutableClass = mutableClassDefBy(mainActivityClass)
    val setupAccountBadgeMethod = mutableClass.methods.firstOrNull {
        it.name == "setupAccountBadge" && it.returnType == "V" && it.implementation != null
    } ?: throw PatchException("Method setupAccountBadge not found in $MAIN_ACTIVITY")

    setupAccountBadgeMethod.addInstructions(0, "return-void")
}

/**
 * Neutralizes the deep link target navigating to Invite Friends if present.
 */
private fun BytecodePatchContext.patchDeepLinkTarget() {
    val deepLinkClass = classDefByOrNull(DEEP_LINK_INVITE_TARGET) ?: return
    val mutableClass = mutableClassDefBy(deepLinkClass)
    val invokeMethod = mutableClass.methods.firstOrNull {
        it.name == "invoke" && it.returnType == "V" && it.implementation != null
    } ?: return

    invokeMethod.addInstructions(0, "return-void")
}

/**
 * Neutralizes navigation in the Invite Friends click handler within AccountDetailsFragment.
 */
private fun BytecodePatchContext.patchAccountDetailsFragment() {
    val fragmentClass = classDefByOrNull(ACCOUNT_DETAILS_FRAGMENT)
        ?: throw PatchException("Class $ACCOUNT_DETAILS_FRAGMENT not found")
    val mutableClass = mutableClassDefBy(fragmentClass)

    for (method in mutableClass.methods.filter { it.implementation != null }) {
        val instructions = method.implementation?.instructions?.toList().orEmpty()
        val callsInviteFriends = instructions.any {
            it.opcode == Opcode.INVOKE_VIRTUAL &&
                (it as? ReferenceInstruction)?.reference?.toString()?.contains("inviteFriends") == true
        }
        if (callsInviteFriends && method.returnType == "V") {
            method.addInstructions(0, "return-void")
        }
    }
}
