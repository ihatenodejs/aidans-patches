package app.aidan.patches.sezzle.compatibility

import app.aidan.patches.sezzle.shared.Constants.COMPATIBILITY_SEZZLE
import app.morphe.patcher.patch.PatchException
import app.morphe.patcher.patch.resourcePatch
import org.w3c.dom.Element

private const val ANDROID_NAMESPACE = "http://schemas.android.com/apk/res/android"
private const val PAGE_SIZE_16_KB = 16_384L

@Suppress("unused")
val enable16KbPageSizeCompatibilityPatch = resourcePatch(
    name = "Enable 16 KB Page Size Compatibility",
    description = "Enables 16 KB page size compatibility mode, forces native library extraction, and aligns native libraries to 16 KB boundaries to suppress the 'APK alignment check failed' warning dialog on Android 15+ devices and emulators.",
    default = true
) {
    compatibleWith(COMPATIBILITY_SEZZLE)

    execute {
        ensure16KbPageAlignment()

        document("AndroidManifest.xml").use { document ->
            val application = document.getElementsByTagName("application").item(0) as? Element
                ?: throw PatchException("Application element not found in AndroidManifest.xml")
            application.setAttributeNS(ANDROID_NAMESPACE, "android:pageSizeCompat", "enabled")
            application.setAttributeNS(ANDROID_NAMESPACE, "android:extractNativeLibs", "true")
        }
    }
}

private fun ensure16KbPageAlignment() {
    try {
        val apkUtilsClass = Class.forName("app.morphe.patcher.apk.ApkUtils")
        val zFileOptionsField = apkUtilsClass.getDeclaredField("zFileOptions").apply { isAccessible = true }
        val zFileOptions = zFileOptionsField.get(null)
            ?: throw PatchException("Morphe APK writer options are unavailable")

        val alignmentRulesClass = Class.forName("com.android.tools.build.apkzlib.zip.AlignmentRules")
        val alignmentRuleClass = Class.forName("com.android.tools.build.apkzlib.zip.AlignmentRule")
        val constantForSuffixMethod = alignmentRulesClass.getMethod(
            "constantForSuffix",
            String::class.java,
            Int::class.javaPrimitiveType
        )
        val constantMethod = alignmentRulesClass.getMethod(
            "constant",
            Int::class.javaPrimitiveType
        )
        val composeMethod = alignmentRulesClass.getMethod(
            "compose",
            java.lang.reflect.Array.newInstance(alignmentRuleClass, 0).javaClass
        )

        val soRule = constantForSuffixMethod.invoke(null, ".so", PAGE_SIZE_16_KB.toInt())
        val defaultRule = constantMethod.invoke(null, 4)
        val rulesArray = java.lang.reflect.Array.newInstance(alignmentRuleClass, 2)
        java.lang.reflect.Array.set(rulesArray, 0, soRule)
        java.lang.reflect.Array.set(rulesArray, 1, defaultRule)
        val composedRule = composeMethod.invoke(null, rulesArray)

        val setAlignmentRuleMethod = zFileOptions.javaClass.getMethod("setAlignmentRule", alignmentRuleClass)
        setAlignmentRuleMethod.invoke(zFileOptions, composedRule)
    } catch (exception: PatchException) {
        throw exception
    } catch (exception: Throwable) {
        throw PatchException("Failed to configure 16 KB native-library alignment: ${exception.message}")
    }
}
