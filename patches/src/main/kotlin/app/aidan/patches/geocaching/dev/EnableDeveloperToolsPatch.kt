package app.aidan.patches.geocaching.dev

import app.aidan.patches.geocaching.shared.COMPATIBILITY_GEOCACHING
import app.aidan.patches.geocaching.shared.patchAllL3cChecksToValue
import app.morphe.patcher.extensions.InstructionExtensions.addInstructions
import app.morphe.patcher.patch.BytecodePatchContext
import app.morphe.patcher.patch.PatchException
import app.morphe.patcher.patch.booleanOption
import app.morphe.patcher.patch.bytecodePatch
import com.android.tools.smali.dexlib2.AccessFlags
import com.android.tools.smali.dexlib2.Opcode
import com.android.tools.smali.dexlib2.iface.instruction.NarrowLiteralInstruction
import com.android.tools.smali.dexlib2.iface.instruction.OneRegisterInstruction
import com.android.tools.smali.dexlib2.iface.instruction.ReferenceInstruction

val enableDeveloperToolsPatch = bytecodePatch(
    name = "Enable Developer Tools and Feature Flags",
    description = "Exposes the internal Debug Menu in Settings and provides selectors for LaunchDarkly feature flags, Remote Config controls, and remaining membership gates.",
    default = false
) {
    category("Developer")
    compatibleWith(COMPATIBILITY_GEOCACHING)

    // Internal Debug & Diagnostic Surfaces
    val exposeDebugMenuInSettings = booleanOption(
        key = "exposeDebugMenuInSettings",
        title = "Expose Debug Menu in Settings",
        description = "Restores the internal Debug Menu item in Settings (LaunchDarkly overrides, Remote Config overrides, AutoLogger, Style/Component/Tooltip previews, and DebugSharedPrefs toggles).",
        default = true
    )

    val enableAchievementDebugTap = booleanOption(
        key = "enableAchievementDebugTap",
        title = "Enable Achievement Debug Tap",
        description = "Enables the internal Debug.TEST_AWARD tap handler in AchievementActivity to test confetti and cycle achievement states.",
        default = false
    )

    // Remaining Client Membership Gates
    val unlockTrailsMapTypeGate = booleanOption(
        key = "unlockTrailsMapTypeGate",
        title = "Unlock Trails Map Types",
        description = "Unlocks Trails and Wheelchair Trails in MapTypeSelectionFragment when OpenStreetMap Drop-in Replacement is not enabled.",
        default = true
    )

    val unlockNewsletterSettingsGate = booleanOption(
        key = "unlockNewsletterSettingsGate",
        title = "Unlock Newsletter Language Settings",
        description = "Unlocks newsletter language selection and hides the Basic-account upgrade banner in NewsletterSettingsFragment.",
        default = true
    )

    val unlockHideCachePremiumOption = booleanOption(
        key = "unlockHideCachePremiumOption",
        title = "Unlock Hide Cache Premium Controls",
        description = "Enables Premium-only controls in HideCacheViewModel and ConversationViewModel GC code validation.",
        default = true
    )

    // LaunchDarkly Feature Flag Selectors
    val enableCacheDetailsRedesign = booleanOption(
        key = "enableCacheDetailsRedesign",
        title = "Enable Cache Details Redesign",
        description = "Enables the redesigned Geocache Details layout (cache-details-rework).",
        default = true
    )

    val showTotalLogCount = booleanOption(
        key = "showTotalLogCount",
        title = "Show Total Log Count",
        description = "Shows the total log count in geocache activity and details (show-total-log-count).",
        default = true
    )

    val enableShareablesInApp = booleanOption(
        key = "enableShareablesInApp",
        title = "Enable Shareables",
        description = "Enables in-app Shareables, filter, and FTUE (show-shareables-in-app, mobile-shareables-filter, mobile-shareables-ftue).",
        default = false
    )

    val enableCollaborativeLists = booleanOption(
        key = "enableCollaborativeLists",
        title = "Enable Collaborative Lists",
        description = "Enables the collaborative lists feature in Listhub (collaborative-lists).",
        default = false
    )

    val enableLodestones = booleanOption(
        key = "enableLodestones",
        title = "Enable Lodestones",
        description = "Enables Lodestone features across the app (show-lodestones-mobile).",
        default = false
    )

    val enableEditLogDate = booleanOption(
        key = "enableEditLogDate",
        title = "Enable Edit Log Date",
        description = "Enables editing geocache log dates and date selector (edit-geocache-log, show-log-date-selector).",
        default = false
    )

    val enableBetterEventsFilter = booleanOption(
        key = "enableBetterEventsFilter",
        title = "Enable Better Events Filter",
        description = "Enables Better Events filter, map, and AR attribute (better-events-filter, better-events-map, show-ar-attribute-in-filter).",
        default = false
    )

    val enableSelectCacheTypes = booleanOption(
        key = "enableSelectCacheTypes",
        title = "Enable Select Cache Types",
        description = "Enables select cache types and filter settings saving (select-cache-types, save-filter-settings).",
        default = false
    )

    val enableDiagnosticFlags = booleanOption(
        key = "enableDiagnosticFlags",
        title = "Enable Diagnostic Flags",
        description = "Enables diagnostic info, FCM token display, ANR data prompt, API regions, and map timeout.",
        default = false
    )

    // Remote Config Override Selectors
    val forceMapTrayEnabled = booleanOption(
        key = "forceMapTrayEnabled",
        title = "Force Map Tray Enabled",
        description = "Forces the map tray feature enabled (RemoteConfig.MAP_TRAY).",
        default = false
    )

    val forceHideLastLogCount = booleanOption(
        key = "forceHideLastLogCount",
        title = "Force Hide Last Log DNF",
        description = "Forces hiding the last log DNF count (RemoteConfig.HIDE_LAST_LOG_DNF).",
        default = false
    )

    val forceShowDowntimeBanner = booleanOption(
        key = "forceShowDowntimeBanner",
        title = "Force Show Downtime Banner",
        description = "Forces the downtime maintenance banner visible (RemoteConfig.SHOW_DOWNTIME_BANNER).",
        default = false
    )

    val forceDisableMapAutoUpdate = booleanOption(
        key = "forceDisableMapAutoUpdate",
        title = "Force Disable Map Auto Update",
        description = "Disables automatic map updating when panning/zooming (RemoteConfig.MAP_AUTO_UPDATE_DISABLED).",
        default = false
    )

    val forceSendDiagnosticLogs = booleanOption(
        key = "forceSendDiagnosticLogs",
        title = "Force Send Diagnostic Logs",
        description = "Forces sending diagnostic logs with bug reports (RemoteConfig.SEND_DIAGNOSTIC_INFO_LOGS).",
        default = false
    )

    val forceLinearCalloutsEnabled = booleanOption(
        key = "forceLinearCalloutsEnabled",
        title = "Force Linear Callouts Enabled",
        description = "Forces linear callout views enabled (RemoteConfig.LINEAR_CALLOUTS_ENABLED).",
        default = false
    )

    execute {
        if (exposeDebugMenuInSettings.value == true) {
            patchSettingsDebugMenu()
        }
        if (enableAchievementDebugTap.value == true) {
            patchAchievementDebugTap()
        }
        if (unlockTrailsMapTypeGate.value == true) {
            patchTrailsMapTypeGate()
        }
        if (unlockNewsletterSettingsGate.value == true) {
            patchNewsletterSettingsGate()
        }
        if (unlockHideCachePremiumOption.value == true) {
            patchHideCachePremiumOption()
        }

        val ldFlags = mutableListOf<String>()
        if (enableCacheDetailsRedesign.value == true) {
            ldFlags.add("cache-details-rework")
        }
        if (showTotalLogCount.value == true) {
            ldFlags.add("show-total-log-count")
        }
        if (enableShareablesInApp.value == true) {
            ldFlags.add("show-shareables-in-app")
            ldFlags.add("mobile-shareables-filter")
            ldFlags.add("mobile-shareables-ftue")
            patchShareablesBillingGate()
        }
        if (enableCollaborativeLists.value == true) {
            ldFlags.add("collaborative-lists")
        }
        if (enableLodestones.value == true) {
            ldFlags.add("show-lodestones-mobile")
        }
        if (enableEditLogDate.value == true) {
            ldFlags.add("edit-geocache-log")
            ldFlags.add("show-log-date-selector")
        }
        if (enableBetterEventsFilter.value == true) {
            ldFlags.add("better-events-filter")
            ldFlags.add("better-events-map")
            ldFlags.add("show-ar-attribute-in-filter")
        }
        if (enableSelectCacheTypes.value == true) {
            ldFlags.add("select-cache-types")
            ldFlags.add("save-filter-settings")
        }
        if (enableDiagnosticFlags.value == true) {
            ldFlags.add("show-fcm-token-mobile")
            ldFlags.add("include-diagnostic-info")
            ldFlags.add("prompt-user-for-anr-data")
            ldFlags.add("use-api-for-regions")
            ldFlags.add("map-ready-timeout")
            ldFlags.add("stats-ftue")
            ldFlags.add("terms-of-use-dialog")
        }
        if (ldFlags.isNotEmpty()) {
            patchLaunchDarklyFlags(ldFlags)
        }

        val rcFlags = mutableListOf<String>()
        if (forceMapTrayEnabled.value == true) {
            rcFlags.add("RemoteConfig.MAP_TRAY")
        }
        if (forceHideLastLogCount.value == true) {
            rcFlags.add("RemoteConfig.HIDE_LAST_LOG_DNF")
        }
        if (forceShowDowntimeBanner.value == true) {
            rcFlags.add("RemoteConfig.SHOW_DOWNTIME_BANNER")
        }
        if (forceDisableMapAutoUpdate.value == true) {
            rcFlags.add("RemoteConfig.MAP_AUTO_UPDATE_DISABLED")
        }
        if (forceSendDiagnosticLogs.value == true) {
            rcFlags.add("RemoteConfig.SEND_DIAGNOSTIC_INFO_LOGS")
        }
        if (forceLinearCalloutsEnabled.value == true) {
            rcFlags.add("RemoteConfig.LINEAR_CALLOUTS_ENABLED")
        }
        if (rcFlags.isNotEmpty()) {
            patchRemoteConfigFlags(rcFlags)
        }
    }
}

/**
 * Restores the internal Debug Menu item in SettingsFragment.v Compose group 0x3d133189.
 */
private fun BytecodePatchContext.patchSettingsDebugMenu() {
    val lClass = mutableClassDefBy("Lcom/groundspeak/geocaching/intro/fragments/settings/l;")
    val initMethod = lClass.methods.firstOrNull {
        it.name == "<init>"
    } ?: throw PatchException("l.<init> not found")
    initMethod.accessFlags = (initMethod.accessFlags and AccessFlags.PRIVATE.value.inv()) or AccessFlags.PUBLIC.value

    val settingsClass = mutableClassDefBy("Lcom/groundspeak/geocaching/intro/fragments/settings/SettingsFragment;")
    val vMethod = settingsClass.methods.firstOrNull {
        it.name == "v" && it.returnType == "V" && it.implementation != null
    } ?: throw PatchException("SettingsFragment.v not found")

    val impl = vMethod.implementation ?: throw PatchException("Missing implementation in SettingsFragment.v")
    val instructions = impl.instructions.toList()

    val constGroupIdx = instructions.indexOfFirst {
        it.opcode == Opcode.CONST && (it as? NarrowLiteralInstruction)?.narrowLiteral == 0x3d133189
    }
    if (constGroupIdx < 0) {
        throw PatchException("Could not find debug Compose group 0x3d133189 in SettingsFragment.v")
    }

    val endGroupIdx = instructions.subList(constGroupIdx, instructions.size).indexOfFirst {
        it.opcode == Opcode.INVOKE_VIRTUAL &&
            (it as? ReferenceInstruction)?.reference?.toString()?.contains("->t(Z)V") == true
    }
    if (endGroupIdx < 0) {
        throw PatchException("Could not find group end invoke-virtual in SettingsFragment.v")
    }
    val insertIdx = constGroupIdx + endGroupIdx

    val smali = """
        new-instance v3, Lcom/groundspeak/geocaching/intro/fragments/settings/l;
        invoke-direct {v3}, Lcom/groundspeak/geocaching/intro/fragments/settings/l;-><init>()V
        invoke-static {v3}, Lg81;->c(Ljava/lang/Object;)Ljava/util/List;
        move-result-object v3
        invoke-static {v3, v5, v9, v7, v6}, Le8a;->b(Ljava/util/List;Lcom/groundspeak/geocaching/intro/fragments/settings/SettingsViewModel;Li24;Ltl1;I)V
    """.trimIndent()

    vMethod.addInstructions(insertIdx, smali)
}

/**
 * Enables the internal Debug.TEST_AWARD tap handler in AchievementActivity and back-press callback r6.
 */
private fun BytecodePatchContext.patchAchievementDebugTap() {
    val achClass = mutableClassDefBy("Lcom/groundspeak/geocaching/intro/activities/AchievementActivity;")
    val targetMethods = achClass.methods.filter { method ->
        method.implementation?.instructions?.any {
            it.opcode == Opcode.CONST_STRING &&
                (it as? ReferenceInstruction)?.reference?.toString() == "Debug.TEST_AWARD"
        } == true
    }
    for (method in targetMethods) {
        val impl = method.implementation ?: continue
        val instructions = impl.instructions.toList()
        val strIdx = instructions.indexOfFirst {
            it.opcode == Opcode.CONST_STRING &&
                (it as? ReferenceInstruction)?.reference?.toString() == "Debug.TEST_AWARD"
        }
        if (strIdx >= 0) {
            val moveResultIdx = instructions.subList(strIdx, instructions.size).indexOfFirst {
                it.opcode == Opcode.MOVE_RESULT
            }
            if (moveResultIdx >= 0) {
                val actualIdx = strIdx + moveResultIdx
                val reg = (instructions[actualIdx] as OneRegisterInstruction).registerA
                method.addInstructions(actualIdx + 1, "const/4 v$reg, 0x1")
            }
        }
    }

    val r6Class = mutableClassDefBy("Lr6;")
    val r6Methods = r6Class.methods.filter { method ->
        method.implementation?.instructions?.any {
            it.opcode == Opcode.CONST_STRING &&
                (it as? ReferenceInstruction)?.reference?.toString() == "Debug.TEST_AWARD"
        } == true
    }
    for (method in r6Methods) {
        val impl = method.implementation ?: continue
        val instructions = impl.instructions.toList()
        val strIdx = instructions.indexOfFirst {
            it.opcode == Opcode.CONST_STRING &&
                (it as? ReferenceInstruction)?.reference?.toString() == "Debug.TEST_AWARD"
        }
        if (strIdx >= 0) {
            val moveResultIdx = instructions.subList(strIdx, instructions.size).indexOfFirst {
                it.opcode == Opcode.MOVE_RESULT
            }
            if (moveResultIdx >= 0) {
                val actualIdx = strIdx + moveResultIdx
                val reg = (instructions[actualIdx] as OneRegisterInstruction).registerA
                method.addInstructions(actualIdx + 1, "const/4 v$reg, 0x1")
            }
        }
    }
}

/**
 * Unlocks Trails and Wheelchair Trails map types in MapTypeSelectionFragment.
 */
private fun BytecodePatchContext.patchTrailsMapTypeGate() {
    val o17Class = mutableClassDefBy("Lo17;")
    val invokeMethod = o17Class.methods.firstOrNull {
        it.name == "invoke" && it.returnType == "Ljava/lang/Object;" && it.implementation != null
    } ?: throw PatchException("o17.invoke not found")

    val impl = invokeMethod.implementation ?: throw PatchException("Missing implementation in o17.invoke")
    val instructions = impl.instructions.toList()
    val strIdx = instructions.indexOfFirst {
        it.opcode == Opcode.CONST_STRING &&
            (it as? ReferenceInstruction)?.reference?.toString() == "userIsPremium"
    }
    if (strIdx < 0) {
        throw PatchException("Could not find userIsPremium in o17.invoke")
    }
    val moveResultIdx = instructions.subList(strIdx, instructions.size).indexOfFirst {
        it.opcode == Opcode.MOVE_RESULT
    }
    if (moveResultIdx < 0) {
        throw PatchException("Could not find move-result after userIsPremium in o17.invoke")
    }
    val actualIdx = strIdx + moveResultIdx
    val reg = (instructions[actualIdx] as OneRegisterInstruction).registerA
    invokeMethod.addInstructions(actualIdx + 1, "const/4 v$reg, 0x1")
}

/**
 * Unlocks newsletter language settings in NewsletterSettingsFragment.
 */
private fun BytecodePatchContext.patchNewsletterSettingsGate() {
    val uq6Class = mutableClassDefBy("Luq6;")
    val invokeMethod = uq6Class.methods.firstOrNull {
        it.name == "invoke" && it.returnType == "Ljava/lang/Object;" && it.implementation != null
    } ?: throw PatchException("uq6.invoke not found")
    patchAllL3cChecksToValue(invokeMethod, "NewsletterSettingsFragment isUserPremium (uq6.invoke)", true)
}

/**
 * Unlocks Hide Cache controls and ConversationViewModel GC code validation.
 */
private fun BytecodePatchContext.patchHideCachePremiumOption() {
    val hideCacheClass = mutableClassDefBy("Lcom/groundspeak/geocaching/intro/profile/hidesandfinds/hidecache/HideCacheViewModel;")
    val initMethod = hideCacheClass.methods.firstOrNull {
        it.name == "<init>" && it.implementation != null
    } ?: throw PatchException("HideCacheViewModel.<init> not found")
    patchAllL3cChecksToValue(initMethod, "HideCacheViewModel.<init>", true)

    val convoClass = mutableClassDefBy("Lcom/groundspeak/geocaching/intro/messagecenter/ConversationViewModel\$validateGcCode\$2;")
    val convoMethod = convoClass.methods.firstOrNull {
        it.name == "invokeSuspend" && it.implementation != null
    } ?: throw PatchException("ConversationViewModel\$validateGcCode\$2.invokeSuspend not found")
    patchAllL3cChecksToValue(convoMethod, "ConversationViewModel\$validateGcCode\$2", true)
}

/**
 * Unlocks billing gate in ShareablesViewModel init.
 */
private fun BytecodePatchContext.patchShareablesBillingGate() {
    val shareablesClass = mutableClassDefBy("Lcom/groundspeak/geocaching/intro/shareables/j;")
    val shareablesInit = shareablesClass.methods.firstOrNull {
        it.name == "<init>" && it.implementation != null
    } ?: throw PatchException("shareables.j.<init> not found")
    patchAllL3cChecksToValue(shareablesInit, "shareables.j.<init>", true)
}

/**
 * Evaluates LaunchDarkly feature flags to true before falling back to SharedPreferences,
 * while preserving manual in-app overrides from LaunchDarklyOverrideFragment.
 */
private fun BytecodePatchContext.patchLaunchDarklyFlags(flags: List<String>) {
    val ldClass = mutableClassDefBy("Lcom/groundspeak/geocaching/intro/analytics/launchdarkly/b;")
    val iMethod = ldClass.methods.firstOrNull {
        it.name == "i" && it.returnType == "Z" && it.implementation != null
    } ?: throw PatchException("LaunchDarkly b.i not found")

    val sb = StringBuilder()
    sb.appendLine(
        """
        invoke-static {}, Lcom/groundspeak/geocaching/intro/analytics/launchdarkly/b;->e()Landroid/content/SharedPreferences;
        move-result-object v0
        iget-object v1, p0, Lcom/groundspeak/geocaching/intro/analytics/launchdarkly/LaunchDarklyFlag;->A:Ljava/lang/String;
        invoke-interface {v0, v1}, Landroid/content/SharedPreferences;->contains(Ljava/lang/String;)Z
        move-result v0
        if-nez v0, :cond_ld_fallthrough
        """.trimIndent()
    )

    flags.forEachIndexed { index, flagKey ->
        sb.appendLine(
            """
            const-string v0, "$flagKey"
            invoke-virtual {v1, v0}, Ljava/lang/String;->equals(Ljava/lang/Object;)Z
            move-result v0
            if-eqz v0, :cond_ld_next_$index
            const/4 v0, 0x1
            return v0
            :cond_ld_next_$index
            """.trimIndent()
        )
    }

    sb.appendLine(":cond_ld_fallthrough")

    iMethod.addInstructions(0, sb.toString().trimIndent())
}

/**
 * Evaluates Remote Config feature flags to true in Leh9.a, while preserving manual
 * in-app overrides from RemoteConfigDebugFragment.
 */
private fun BytecodePatchContext.patchRemoteConfigFlags(flags: List<String>) {
    val rcClass = mutableClassDefBy("Leh9;")
    val aMethod = rcClass.methods.firstOrNull {
        it.name == "a" && it.parameterTypes.size == 3 && it.returnType == "Z" && it.implementation != null
    } ?: throw PatchException("Leh9.a(Leh9, String, boolean) not found")

    val sb = StringBuilder()
    sb.appendLine(
        """
        iget-object v0, p0, Leh9;->a:Landroid/content/Context;
        const-string v1, "RemoteConfig.REMOTE_CONFIG_DEBUG_PREFS"
        const/4 p0, 0x0
        invoke-virtual {v0, v1, p0}, Landroid/content/Context;->getSharedPreferences(Ljava/lang/String;I)Landroid/content/SharedPreferences;
        move-result-object v0
        invoke-interface {v0, p1}, Landroid/content/SharedPreferences;->contains(Ljava/lang/String;)Z
        move-result v1
        if-eqz v1, :cond_check_rc_patch
        invoke-interface {v0, p1, p0}, Landroid/content/SharedPreferences;->getBoolean(Ljava/lang/String;Z)Z
        move-result p0
        return p0
        :cond_check_rc_patch
        """.trimIndent()
    )

    flags.forEachIndexed { index, rcKey ->
        sb.appendLine(
            """
            const-string v0, "$rcKey"
            invoke-virtual {p1, v0}, Ljava/lang/String;->equals(Ljava/lang/Object;)Z
            move-result v0
            if-eqz v0, :cond_next_rc_$index
            const/4 p0, 0x1
            return p0
            :cond_next_rc_$index
            """.trimIndent()
        )
    }

    sb.appendLine("return p2")

    aMethod.addInstructions(0, sb.toString().trimIndent())
}
