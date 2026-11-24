package app.aidan.patches.flowfree.shared

import app.morphe.patcher.patch.ApkFileType
import app.morphe.patcher.patch.AppTarget
import app.morphe.patcher.patch.Compatibility

const val FLOW_FREE_PACKAGE_NAME = "com.bigduckgames.flow"

val COMPATIBILITY_FLOW_FREE = Compatibility(
    name = "Flow Free",
    packageName = FLOW_FREE_PACKAGE_NAME,
    apkFileType = ApkFileType.APKM,
    appIconColor = 0x1E88E5,
    signatures = setOf("36cd9da86367d4e25529427681598ffef8f08416c0f2b7ceedcd856b9c6620b6"),
    targets = listOf(AppTarget(version = "6.0", minSdk = 24))
)
