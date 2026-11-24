package app.aidan.patches.flowfree.tracking

import app.aidan.patches.flowfree.shared.COMPATIBILITY_FLOW_FREE
import app.aidan.patches.shared.compositeFeaturePatch
import app.aidan.patches.shared.disableRequiredVoidMethodsByName
import app.morphe.patcher.patch.PatchException
import org.w3c.dom.Document
import org.w3c.dom.Element

private const val ANDROID_NAMESPACE = "http://schemas.android.com/apk/res/android"
private const val ANDROID_NAME = "android:name"
private const val ANDROID_VALUE = "android:value"

private const val ANALYTICS_MANAGER = "Lcom/bigduckgames/flow/AnalyticsManager;"
private const val CRASH_REPORT_MANAGER = "Lcom/bigduckgames/flow/CrashReportManager;"

private const val FIREBASE_ANALYTICS_COLLECTION = "firebase_analytics_collection_enabled"
private const val FIREBASE_CRASHLYTICS_COLLECTION = "firebase_crashlytics_collection_enabled"
private const val AUTOMATIC_SCREEN_REPORTING = "google_analytics_automatic_screen_reporting_enabled"

@Suppress("unused")
val removeTrackingPatch = compositeFeaturePatch(
    name = "Remove Tracking",
    description = "Disables Firebase Analytics and Crashlytics collection and neutralizes Flow Free's event, user-property, and crash-reporting bridges.",
    default = true
) {
    category("Privacy")
    compatibleWith(COMPATIBILITY_FLOW_FREE)

    resources {
        document("AndroidManifest.xml").use { document ->
            val applications = document.getElementsByTagName("application")
            if (applications.length != 1) {
                throw PatchException("Expected exactly one <application> element, found ${applications.length}.")
            }
            val application = applications.item(0) as? Element
                ?: throw PatchException("Expected <application> to be an element.")

            setMetaDataValue(document, application, FIREBASE_ANALYTICS_COLLECTION, "false")
            setMetaDataValue(document, application, FIREBASE_CRASHLYTICS_COLLECTION, "false")
            setMetaDataValue(document, application, AUTOMATIC_SCREEN_REPORTING, "false")
        }
    }
    execute {
        // Flow Free's native code and Java activities funnel analytics through these managers.
        // Firebase's automatic collection is disabled separately by the manifest flags above.
        disableRequiredVoidMethodsByName(
            ANALYTICS_MANAGER,
            "logEvent",
            "logEventStart",
            "logEventEnd",
            "logFirebaseEvent",
            "setFirebaseUserProperty"
        )
        disableRequiredVoidMethodsByName(
            CRASH_REPORT_MANAGER,
            "log",
            "recordException",
            "recordExceptionWithLogs",
            "setCustomKey"
        )
    }
}

/**
 * Sets an `<application>` meta-data flag to [value], adding the element when the app does not
 * already declare it. Fail-fast behavior applies to the application lookup, not to the flag:
 * the Firebase SDK reads a missing flag as enabled.
 */
private fun setMetaDataValue(document: Document, application: Element, name: String, value: String) {
    val children = application.childNodes
    for (index in 0 until children.length) {
        val child = children.item(index) as? Element ?: continue
        if (child.tagName == "meta-data" && child.getAttribute(ANDROID_NAME) == name) {
            child.setAttribute(ANDROID_VALUE, value)
            return
        }
    }

    val metaData = document.createElement("meta-data")
    metaData.setAttributeNS(ANDROID_NAMESPACE, ANDROID_NAME, name)
    metaData.setAttributeNS(ANDROID_NAMESPACE, ANDROID_VALUE, value)
    application.appendChild(metaData)
}
