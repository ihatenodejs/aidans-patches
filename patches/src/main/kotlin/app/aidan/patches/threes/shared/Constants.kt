package app.aidan.patches.threes.shared

import app.morphe.patcher.patch.ApkFileType
import app.morphe.patcher.patch.AppTarget
import app.morphe.patcher.patch.Compatibility

const val THREES_PACKAGE_NAME = "vo.threes.free"

val COMPATIBILITY_THREES = Compatibility(
    name = "Threes Freeplay",
    packageName = THREES_PACKAGE_NAME,
    apkFileType = ApkFileType.APKM,
    appIconColor = 0xEF657F,
    signatures = setOf(
        "5fe1156a5860a423f26c4cfc91fb97d0d0bedde050c2d4cbd6493e4fb11f80a1"
    ),
    targets = listOf(
        AppTarget(version = "3.1.196608", minSdk = 22)
    )
)
