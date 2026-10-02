package app.aidan.patches.sidelineswap.customization

import app.aidan.patches.sidelineswap.shared.COMPATIBILITY_SIDELINESWAP
import app.morphe.patcher.patch.resourcePatch
import app.morphe.patcher.patch.stringOption
import org.w3c.dom.Element

val changeBrandColorPatch = resourcePatch(
    name = "Change Brand Color",
    description = "Customizes the primary accent brand color across SidelineSwap buttons, navigation highlights, badges, and accents.",
    default = false
) {
    compatibleWith(COMPATIBILITY_SIDELINESWAP)

    val primaryColor = stringOption(
        key = "primaryColor",
        default = "#1E88E5",
        title = "Primary Brand Color",
        description = "Hex color string (e.g. #1E88E5 for Material Blue, #9C27B0 for Purple, #FF5722 for Orange, #000000 for Monochrome)."
    )

    val primaryDarkColor = stringOption(
        key = "primaryDarkColor",
        default = "#1565C0",
        title = "Primary Dark Color",
        description = "Hex color string for status bars and dark accents (defaults to #1565C0)."
    )

    execute {
        val primary = primaryColor.value ?: "#1E88E5"
        val primaryDark = primaryDarkColor.value ?: "#1565C0"

        val colorFiles = listOf("res/values/colors.xml", "res/values-night/colors.xml")

        for (filePath in colorFiles) {
            runCatching {
                document(filePath).use { document ->
                    val colorNodes = document.getElementsByTagName("color")
                    for (i in 0 until colorNodes.length) {
                        val element = colorNodes.item(i) as? Element ?: continue
                        val name = element.getAttribute("name")
                        when (name) {
                            "colorPrimary",
                            "badge_color",
                            "green_badge",
                            "ic_launcher_background" -> {
                                element.textContent = primary
                            }
                            "colorPrimaryDark" -> {
                                element.textContent = primaryDark
                            }
                        }
                    }
                }
            }
        }
    }
}
