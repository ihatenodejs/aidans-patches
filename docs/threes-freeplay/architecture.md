# Threes Freeplay Architecture & Reverse Engineering Specification

## 1. Overview

| Attribute                   | Specification                                                      |
| --------------------------- | ------------------------------------------------------------------ |
| **Application Name**        | Threes Freeplay                                                    |
| **Package Name**            | `vo.threes.free`                                                   |
| **Supported Version**       | `3.1.196608` (version code `53`, `minSdk` 22)                      |
| **Distribution Format**     | APKM (`ApkFileType.APKM`, base plus `arm64-v8a` split)             |
| **Package Signing SHA-256** | `5fe1156a5860a423f26c4cfc91fb97d0d0bedde050c2d4cbd6493e4fb11f80a1` |
| **Target Architecture**     | `arm64-v8a` (Unity IL2CPP native runtime)                          |
| **Primary Icon Color**      | `#EF657F`                                                          |

Threes Freeplay is the free edition of Threes, built with Unity 2022.3.62f2. Game logic, menus, free-play accounting, and the advertising bridge are compiled from C# into `lib/arm64-v8a/libil2cpp.so` using IL2CPP. Android host code remains in the base split and is limited to the Unity player plus a small `vo.threes.util` bridge.

## 2. Technology Stack & Container Layout

The Play artifact is an APKM with two splits:

- `base.apk`: manifest, DEX, Unity serialized assets, and `assets/bin/Data/Managed/Metadata/global-metadata.dat` (IL2CPP metadata version 31).
- `config.arm64_v8a.apk`: `lib/arm64-v8a/libil2cpp.so`, `libunity.so`, `libmain.so`, and `libunitycoherencenative.so`.

`libil2cpp.so` uses an Android ELF layout where the `.text` and custom `il2cpp` sections are 0x4000 bytes below their virtual addresses. File offsets in patch code and below are therefore `virtual address - 0x4000` for game methods.

The APKPure single APK contains the same package, version, and signature but omits the native split, so it cannot execute the game. Patches target the APKM form.

## 3. Free-Play Accounting

`FreeLogic` (Assembly-CSharp) owns the free-session limit:

| Method                   | Behavior                                                                                     |
| ------------------------ | -------------------------------------------------------------------------------------------- |
| `IsFree`                 | Returns `true` when the store type is `3` (free build).                                      |
| `GetPlayCount`           | Reads the stored free-play balance from `PlayerPrefs` (key name owned by the native binary). |
| `HasPlays`               | Returns `GetPlayCount() > 0`.                                                                |
| `GetStartingPlayCount`   | Returns the configured initial balance (`PlayerPrefs`, default `13`).                        |
| `GetPlaysPerWatch`       | Returns the plays granted per rewarded video (`PlayerPrefs`, default `3`).                   |
| `ModPlays`               | Adds a delta to the stored balance, clamped to zero.                                         |
| `ShowAd` / `AdPopupMenu` | Presents the out-of-plays menu and its rewarded-video action.                                |

All play gates compare `GetPlayCount()` against zero directly; `HasPlays` is a semantic helper without direct callers in this build. `FreeMenu.RefreshCount` shows the balance and enables the play button and rewarded-video offer from the same count.

`FreeMenu.OnPress` handles the "watch video" button. It calls `FreeLogic.IsOnline`, then `AdMan.HasAds`; only when a rewarded ad is ready does it call `AdMan.TryShowAd`. There are no other `AdMan.TryShowAd` call sites, so the rewarded video is the only way the original binary continues a run after the balance reaches zero.

## 4. Advertising Stack

The game uses Unity LevelPlay mediation with Unity Ads and IronSource adapters:

- **C# assembly:** `Unity.LevelPlay.dll` (LevelPlay API, `UnityLevelPlayRewardedAd`).
- **Android SDKs in DEX:** `com.unity3d.ads`, `com.unity3d.services`, `com.ironsource`, `com.unity3d.ironsourceads`.
- **Game bridge:** `AdMan` in Assembly-CSharp.

`AdMan.Start` calls `FreeLogic.IsFree` and returns early for non-free builds. For the free build it initializes LevelPlay, creates a rewarded ad with `AdMan` as listener, registers interstitial callbacks, and loads rewarded inventory. `AdMan.HasAds` returns whether the rewarded ad reports `IsAdReady`. `TryShowAd` shows the rewarded ad when ready, otherwise reloads. The only ad-rendering entry point reachable from game UI is the rewarded-video action in `FreeMenu`.

Unity Ads and LevelPlay initialization is entirely driven by `AdMan.Start`; no advertising provider auto-initializes from the manifest.

## 5. Tracking Stack

Two layers implement Google Analytics tracking:

- **Native wrapper:** `GoogleAnalyticsHelper` (Assembly-CSharp-firstpass) exposes `Settings`, `LogPage`, and `LogEvent` to the game.
- **Android bridge:** `vo.threes.util.GA` creates a `com.google.android.gms.analytics.Tracker` from the `global_tracker` XML resource and sends `HitBuilders.EventBuilder` events. `GA.Checkpoint` is the event entry point; `GA.GetTracker` lazily initializes and caches the tracker.

No Firebase Analytics, Crashlytics, Adjust, AppsFlyer, or first-party telemetry SDK is present in this version. Play Games leaderboards and achievements are user-visible services, not telemetry.

## 6. Verification Anchors

- `AdMan` type definition index 2150, method index 16045 in `global-metadata.dat`.
- `AdMan.Start` virtual address `0x82ef64` (file offset `0x82af64`).
- `FreeLogic.GetPlayCount` virtual address `0x8406e4` (file offset `0x83c6e4`).
- `FreeLogic.HasPlays` virtual address `0x8407a4` (file offset `0x83c7a4`).
- `FreeMenu.OnPress` calls `AdMan.HasAds` and then `AdMan.TryShowAd` when the rewarded ad is ready.
