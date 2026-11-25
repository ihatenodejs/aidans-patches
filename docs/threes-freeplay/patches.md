# Threes Freeplay Patch Specifications

## Overview

This document details the patches available for **Threes Freeplay** (`vo.threes.free`).

| Patch Name                                | Type               | Default | Description                                                                                   |
| ----------------------------------------- | ------------------ | ------- | --------------------------------------------------------------------------------------------- |
| [Remove Ads](#patch-remove-ads)           | `rawResourcePatch` | `true`  | Removes rewarded-video advertising and grants unlimited free plays so no ad is ever required. |
| [Remove Tracking](#patch-remove-tracking) | `bytecodePatch`    | `true`  | Neutralizes Google Analytics event logging and prevents tracker creation.                     |

---

## Patch: Remove Ads

- **Name:** Remove Ads
- **Target Package:** `vo.threes.free`
- **Supported Versions:** `3.1.196608`
- **Default State:** `true` (Enabled by default)
- **Type:** Raw Binary / Asset Patch (`rawResourcePatch`)
- **Dependencies:** None

### 1. Motivation & Purpose

The free edition gives a limited number of sessions and then requires watching a rewarded video to continue. The rewarded flow is implemented through `FreeLogic`, `FreeMenu`, and the LevelPlay-backed `AdMan` bridge. The patch removes the ad requirement and prevents the advertising SDK from initializing.

### 2. Technical Implementation & Binary Modifications

#### Target File: `lib/arm64-v8a/libil2cpp.so`

| Target Function          | File Offset | Original Bytes            | Replacement                                      | Effect                                                                                                           |
| ------------------------ | ----------- | ------------------------- | ------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------- |
| `FreeLogic.GetPlayCount` | `0x83c6e4`  | `fe 57 be a9 f4 4f 01 a9` | `e0 e1 84 52 c0 03 5f d6` (`mov w0, #9999; ret`) | Every free-play gate sees a positive balance, so the game never reaches the out-of-plays flow.                   |
| `FreeLogic.HasPlays`     | `0x83c7a4`  | `fe 0f 1f f8 cf ff ff 97` | `20 00 80 52 c0 03 5f d6` (`mov w0, #1; ret`)    | Forces the semantic helper to `true` for any indirect call site.                                                 |
| `AdMan.Start`            | `0x82af64`  | `fe 0f 1c f8`             | `c0 03 5f d6` (`ret`)                            | Skips LevelPlay initialization, rewarded-ad creation, inventory loading, and interstitial callback registration. |

The stored preference is not modified. Because `GetPlayCount` returns a fixed `9999`, the free-play balance in the menu never reaches zero and the rewarded-video offer is not required. `ModPlays` may still update the stored preference, but no code path consults it for gating after the patch.

No advertising provider initializes from the manifest, so no remaining ad inventory is requested or rendered.

---

## Patch: Remove Tracking

- **Name:** Remove Tracking
- **Target Package:** `vo.threes.free`
- **Supported Versions:** `3.1.196608`
- **Default State:** `true` (Enabled by default)
- **Type:** Dalvik Bytecode Patch (`bytecodePatch`)
- **Dependencies:** None

### 1. Motivation & Purpose

The app sends Google Analytics events through `vo.threes.util.GA`. This patch neutralizes the event entry point and prevents the lazy tracker initialization, without touching Play Games leaderboards or achievements.

### 2. Technical Implementation & Bytecode Modifications

#### Target Classes

- `Lvo/threes/util/GA;`
  - `Checkpoint(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;I)V` is rewritten to `return-void` at method entry.
  - `GetTracker(Landroid/app/Activity;)Lcom/google/android/gms/analytics/Tracker;` is rewritten to return `null` at method entry.

With `GetTracker` returning `null`, `Checkpoint` takes its existing early-return warning path even if reached through a different caller, and `Takeoff` cannot create a tracker. No analytics hits are built or sent.

No Firebase Analytics, Crashlytics, Adjust, AppsFlyer, or comparable telemetry SDK is present in this version. Advertising telemetry is stopped independently when the Remove Ads patch skips `AdMan.Start`.

---

## Verification

- `apk-lab check <apkm> --mpp <bundle> --package vo.threes.free --all` must pass all patch configurations.
- Device smoke: install the patched APKM, clear or zero the stored free-play count, start a game, and confirm no rewarded or interstitial ad appears and the game continues.
- Device smoke: confirm Play Games sign-in, leaderboards, and achievements still work.
