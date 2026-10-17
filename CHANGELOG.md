## [1.8.0](https://github.com/ihatenodejs/aidans-patches/compare/v1.7.0...v1.8.0) (2026-10-17)

### 🐛 Bug Fixes

* **apk-lab:** prevented overwriting input artifact during deploy ([0275070](https://github.com/ihatenodejs/aidans-patches/commit/0275070f7b758809063d82ef062bd61ef99d63dc))
* **apk-lab:** validate move-result variants and invoke producers ([823def9](https://github.com/ihatenodejs/aidans-patches/commit/823def9784b437dd180ce4342d6c796704eb482e))
* **ci:** revert 5412f299237e486590dab6e8dda1e89ba6419150 ([dbffd82](https://github.com/ihatenodejs/aidans-patches/commit/dbffd8207faeaeb6a061d539a3361d5a41bf29f2))
* **geocaching:** sanitized loaded caches and fixed fallback enrichment ([de0a05b](https://github.com/ihatenodejs/aidans-patches/commit/de0a05b936420b8d5c908fb03f19ac2197120d45))
* **hermes:** corrected large function header offset decoding in bundle editor ([64bbd82](https://github.com/ihatenodejs/aidans-patches/commit/64bbd827e9e54e3bb8fb0488cabbeda5b9da0bb5))
* **nhl:** corrected developer settings gate to return false ([b555a5f](https://github.com/ihatenodejs/aidans-patches/commit/b555a5f4cfef68921be08d7de3e7c401b9ff77ec))
* **nhl:** preserve synthetic default matching for Blaze analytics ([b6bd2b2](https://github.com/ihatenodejs/aidans-patches/commit/b6bd2b2b6ddb99d6a7ef9e5c284fba32ea1eb536))
* **nhl:** validated patch targets and patched all constructor returns ([5c3b57b](https://github.com/ihatenodejs/aidans-patches/commit/5c3b57b13ed725c04f44d76f32000aa05c7cec01))
* **worker:** use utf-16 code-unit order for monitored apps catalog ([964e01f](https://github.com/ihatenodejs/aidans-patches/commit/964e01f00da67982c7c259ced8a797f3449c09cc))

### ✨ New Features

* **365pay:** add support for 365 pay ([52cd8a5](https://github.com/ihatenodejs/aidans-patches/commit/52cd8a5805cd84f1aab098a48ad774ff55c3dec8))
* **365pay:** added display cutout adaptation patch ([be08c95](https://github.com/ihatenodejs/aidans-patches/commit/be08c954996edfaf0c310f99d4b5f6a0b6b0402a))
* **365pay:** expanded telemetry removal for crashlytics branch and onesignal ([1b11aee](https://github.com/ihatenodejs/aidans-patches/commit/1b11aeec83f2b15eff069d97f93ba5084b7889a8))
* **adobe-scan:** added compress pdf removal and card action cleanup ([ea76005](https://github.com/ihatenodejs/aidans-patches/commit/ea760053cb44a96d20de4e55736f0edf6027cfc9))
* **adobe-scan:** added remove attribution patch ([726f4f4](https://github.com/ihatenodejs/aidans-patches/commit/726f4f4ae2526707581faf8f285eb63feb57c282))
* **apk-lab:** add crash-trace command to harvest device exceptions ([d99b76d](https://github.com/ihatenodejs/aidans-patches/commit/d99b76d38c6eacae0a117eb0083652366f0bb708))
* **apk-lab:** add decompiled smali resolution to crash trace reports ([670d76b](https://github.com/ihatenodejs/aidans-patches/commit/670d76b213538a21048a65274f5f219c23431ec8))
* **apk-lab:** add text value search and graceful split handling to res command ([a1039e1](https://github.com/ihatenodejs/aidans-patches/commit/a1039e1464665eb4c5dceb5cd7afbb09a7148fb3))
* **apk-lab:** add unified dev iteration command ([931406c](https://github.com/ihatenodejs/aidans-patches/commit/931406c2a610827376c970c7dedc88e9b5924eee))
* **apk-lab:** added keystore option to deploy command ([7091327](https://github.com/ihatenodejs/aidans-patches/commit/7091327309f7c1977df9b1fa7d6fc6335cd5784c))
* **apk-lab:** support automatic keystore negotiation and custom credentials ([419a89c](https://github.com/ihatenodejs/aidans-patches/commit/419a89c03f5beca4d980b053a05da87f146a7759))
* **apk-lab:** validate register aliasing and move-result sequences ([93aa649](https://github.com/ihatenodejs/aidans-patches/commit/93aa649f56c5651402c163c6f3b1c33620be5a32))
* **audit:** add phase and full-suite summary formatting ([1ff79c4](https://github.com/ihatenodejs/aidans-patches/commit/1ff79c4993fb112a2981e30d3a8fe8b4e96ec2ee))
* **geocaching:** added cookie support and pmo fallback to bridge ([015ace0](https://github.com/ihatenodejs/aidans-patches/commit/015ace068cb8c819e2a5715cc3133e1c9b226043))
* **geocaching:** added public cache data enrichment fallback ([32b5d14](https://github.com/ihatenodejs/aidans-patches/commit/32b5d148cbdbc9c8677f9995c99ba43d380c57e3))
* **geocaching:** added warning banner on PMO caches ([6f9e5ee](https://github.com/ihatenodejs/aidans-patches/commit/6f9e5eee94b0d991ffcd679da889867b72b5d8b9))
* **nhl:** added patches for ads, tracking, and hidden features ([5f33a90](https://github.com/ihatenodejs/aidans-patches/commit/5f33a9086d352a2906ef4814a68c70be5ef88d20))
* **play-metadata:** added enrichment pipeline and app detail views ([75f97e3](https://github.com/ihatenodejs/aidans-patches/commit/75f97e3fb9a13c4d526252a520e20ef4a4b4e8fb))
* **site/components:** added developer menu card and phone preview ([60aa338](https://github.com/ihatenodejs/aidans-patches/commit/60aa338faeda1a53bb73f5f92db66774d0d11d40))
* **site:** added app catalog and play store metadata support ([f42bb98](https://github.com/ihatenodejs/aidans-patches/commit/f42bb9805a623e62cdb78ae7bb39296e7760b3fc))
* **site:** redesigned landing page and ui components ([9f23004](https://github.com/ihatenodejs/aidans-patches/commit/9f23004674b9ea810b32eb0dffc1f7f7837e3e96))
* **worker:** added aggregate and per-package test passing badges ([5c46b59](https://github.com/ihatenodejs/aidans-patches/commit/5c46b596e6ad567b40b03188b46bcb494e2df61c))
* **worker:** added GITHUB_TOKEN support to download statistics ([a6d630f](https://github.com/ihatenodejs/aidans-patches/commit/a6d630f67851b7f53a7a129b7e474f22c8544a09))

## [1.8.0-dev.9](https://github.com/ihatenodejs/aidans-patches/compare/v1.8.0-dev.8...v1.8.0-dev.9) (2026-10-17)

### ✨ New Features

* **geocaching:** added warning banner on PMO caches ([6f9e5ee](https://github.com/ihatenodejs/aidans-patches/commit/6f9e5eee94b0d991ffcd679da889867b72b5d8b9))
* **worker:** added GITHUB_TOKEN support to download statistics ([a6d630f](https://github.com/ihatenodejs/aidans-patches/commit/a6d630f67851b7f53a7a129b7e474f22c8544a09))

## [1.8.0-dev.8](https://github.com/ihatenodejs/aidans-patches/compare/v1.8.0-dev.7...v1.8.0-dev.8) (2026-10-16)

### 🐛 Bug Fixes

* **apk-lab:** prevented overwriting input artifact during deploy ([0275070](https://github.com/ihatenodejs/aidans-patches/commit/0275070f7b758809063d82ef062bd61ef99d63dc))

### ✨ New Features

* **apk-lab:** add decompiled smali resolution to crash trace reports ([670d76b](https://github.com/ihatenodejs/aidans-patches/commit/670d76b213538a21048a65274f5f219c23431ec8))
* **apk-lab:** add text value search and graceful split handling to res command ([a1039e1](https://github.com/ihatenodejs/aidans-patches/commit/a1039e1464665eb4c5dceb5cd7afbb09a7148fb3))
* **apk-lab:** add unified dev iteration command ([931406c](https://github.com/ihatenodejs/aidans-patches/commit/931406c2a610827376c970c7dedc88e9b5924eee))
* **apk-lab:** added keystore option to deploy command ([7091327](https://github.com/ihatenodejs/aidans-patches/commit/7091327309f7c1977df9b1fa7d6fc6335cd5784c))
* **apk-lab:** support automatic keystore negotiation and custom credentials ([419a89c](https://github.com/ihatenodejs/aidans-patches/commit/419a89c03f5beca4d980b053a05da87f146a7759))
* **geocaching:** added cookie support and pmo fallback to bridge ([015ace0](https://github.com/ihatenodejs/aidans-patches/commit/015ace068cb8c819e2a5715cc3133e1c9b226043))

## [1.8.0-dev.7](https://github.com/ihatenodejs/aidans-patches/compare/v1.8.0-dev.6...v1.8.0-dev.7) (2026-10-16)

### 🐛 Bug Fixes

* **ci:** revert 5412f299237e486590dab6e8dda1e89ba6419150 ([dbffd82](https://github.com/ihatenodejs/aidans-patches/commit/dbffd8207faeaeb6a061d539a3361d5a41bf29f2))
* **geocaching:** sanitized loaded caches and fixed fallback enrichment ([de0a05b](https://github.com/ihatenodejs/aidans-patches/commit/de0a05b936420b8d5c908fb03f19ac2197120d45))

### ✨ New Features

* **geocaching:** added public cache data enrichment fallback ([32b5d14](https://github.com/ihatenodejs/aidans-patches/commit/32b5d148cbdbc9c8677f9995c99ba43d380c57e3))

## [1.8.0-dev.6](https://github.com/ihatenodejs/aidans-patches/compare/v1.8.0-dev.5...v1.8.0-dev.6) (2026-10-16)

### 🐛 Bug Fixes

* **apk-lab:** validate move-result variants and invoke producers ([823def9](https://github.com/ihatenodejs/aidans-patches/commit/823def9784b437dd180ce4342d6c796704eb482e))
* **nhl:** preserve synthetic default matching for Blaze analytics ([b6bd2b2](https://github.com/ihatenodejs/aidans-patches/commit/b6bd2b2b6ddb99d6a7ef9e5c284fba32ea1eb536))
* **worker:** use utf-16 code-unit order for monitored apps catalog ([964e01f](https://github.com/ihatenodejs/aidans-patches/commit/964e01f00da67982c7c259ced8a797f3449c09cc))

### ✨ New Features

* **apk-lab:** add crash-trace command to harvest device exceptions ([d99b76d](https://github.com/ihatenodejs/aidans-patches/commit/d99b76d38c6eacae0a117eb0083652366f0bb708))
* **apk-lab:** validate register aliasing and move-result sequences ([93aa649](https://github.com/ihatenodejs/aidans-patches/commit/93aa649f56c5651402c163c6f3b1c33620be5a32))
* **audit:** add phase and full-suite summary formatting ([1ff79c4](https://github.com/ihatenodejs/aidans-patches/commit/1ff79c4993fb112a2981e30d3a8fe8b4e96ec2ee))

## [1.8.0-dev.5](https://github.com/ihatenodejs/aidans-patches/compare/v1.8.0-dev.4...v1.8.0-dev.5) (2026-10-16)

### ✨ New Features

* **adobe-scan:** added compress pdf removal and card action cleanup ([ea76005](https://github.com/ihatenodejs/aidans-patches/commit/ea760053cb44a96d20de4e55736f0edf6027cfc9))
* **play-metadata:** added enrichment pipeline and app detail views ([75f97e3](https://github.com/ihatenodejs/aidans-patches/commit/75f97e3fb9a13c4d526252a520e20ef4a4b4e8fb))
* **site/components:** added developer menu card and phone preview ([60aa338](https://github.com/ihatenodejs/aidans-patches/commit/60aa338faeda1a53bb73f5f92db66774d0d11d40))
* **site:** added app catalog and play store metadata support ([f42bb98](https://github.com/ihatenodejs/aidans-patches/commit/f42bb9805a623e62cdb78ae7bb39296e7760b3fc))

## [1.8.0-dev.4](https://github.com/ihatenodejs/aidans-patches/compare/v1.8.0-dev.3...v1.8.0-dev.4) (2026-10-14)

### 🐛 Bug Fixes

* **nhl:** corrected developer settings gate to return false ([b555a5f](https://github.com/ihatenodejs/aidans-patches/commit/b555a5f4cfef68921be08d7de3e7c401b9ff77ec))

### ✨ New Features

* **365pay:** added display cutout adaptation patch ([be08c95](https://github.com/ihatenodejs/aidans-patches/commit/be08c954996edfaf0c310f99d4b5f6a0b6b0402a))

## [1.8.0-dev.3](https://github.com/ihatenodejs/aidans-patches/compare/v1.8.0-dev.2...v1.8.0-dev.3) (2026-10-14)

### 🐛 Bug Fixes

* **hermes:** corrected large function header offset decoding in bundle editor ([64bbd82](https://github.com/ihatenodejs/aidans-patches/commit/64bbd827e9e54e3bb8fb0488cabbeda5b9da0bb5))

### ✨ New Features

* **365pay:** add support for 365 pay ([52cd8a5](https://github.com/ihatenodejs/aidans-patches/commit/52cd8a5805cd84f1aab098a48ad774ff55c3dec8))
* **365pay:** expanded telemetry removal for crashlytics branch and onesignal ([1b11aee](https://github.com/ihatenodejs/aidans-patches/commit/1b11aeec83f2b15eff069d97f93ba5084b7889a8))

## [1.8.0-dev.2](https://github.com/ihatenodejs/aidans-patches/compare/v1.8.0-dev.1...v1.8.0-dev.2) (2026-10-14)

### 🐛 Bug Fixes

* **nhl:** validated patch targets and patched all constructor returns ([5c3b57b](https://github.com/ihatenodejs/aidans-patches/commit/5c3b57b13ed725c04f44d76f32000aa05c7cec01))

### ✨ New Features

* **nhl:** added patches for ads, tracking, and hidden features ([5f33a90](https://github.com/ihatenodejs/aidans-patches/commit/5f33a9086d352a2906ef4814a68c70be5ef88d20))

## [1.8.0-dev.1](https://github.com/ihatenodejs/aidans-patches/compare/v1.7.0...v1.8.0-dev.1) (2026-10-13)

### ✨ New Features

* **adobe-scan:** added remove attribution patch ([726f4f4](https://github.com/ihatenodejs/aidans-patches/commit/726f4f4ae2526707581faf8f285eb63feb57c282))

## [1.7.0](https://github.com/ihatenodejs/aidans-patches/compare/v1.6.0...v1.7.0) (2026-10-13)

### 🐛 Bug Fixes

* **geocaching:** neutralized server sync for experimental features ([ab635cb](https://github.com/ihatenodejs/aidans-patches/commit/ab635cb6170d5ba5fb1e821f4f5e2c9c4e59c7b2))
* **release:** update bun lockfile for semantic-release github plugin ([cea934f](https://github.com/ihatenodejs/aidans-patches/commit/cea934f8955192f773b46ea402fed80045e8c641))
* **site:** prevented linking prior workflow run url on queued badges ([97c50a9](https://github.com/ihatenodejs/aidans-patches/commit/97c50a92aa8eb35ba0039d3487246db23893c3c1))

### ✨ New Features

* **geocaching:** added developer tools patch and experimental toggles ([e99ec24](https://github.com/ihatenodejs/aidans-patches/commit/e99ec24ffaf7b78e5b1adfc0e4e68d1fbbabd8c9))
* **geocaching:** added experimental map sheet support for osm styles ([1f29c32](https://github.com/ihatenodejs/aidans-patches/commit/1f29c327d9c847bc34cad0dbb7970b331d0375e5))
* **sidelineswap:** added remove invite friends patch ([04328a9](https://github.com/ihatenodejs/aidans-patches/commit/04328a924ddd04de690da61b7b3937adbb26d789))
* **site:** added ViewMoreAppsCard and Crosshair components ([b66cb3d](https://github.com/ihatenodejs/aidans-patches/commit/b66cb3d1d7746bc24ad4f5964d4cbbfc0cc5a4e2))

## [1.7.0-dev.3](https://github.com/ihatenodejs/aidans-patches/compare/v1.7.0-dev.2...v1.7.0-dev.3) (2026-10-13)

### ✨ New Features

* **geocaching:** added developer tools patch and experimental toggles ([e99ec24](https://github.com/ihatenodejs/aidans-patches/commit/e99ec24ffaf7b78e5b1adfc0e4e68d1fbbabd8c9))

## [1.7.0-dev.2](https://github.com/ihatenodejs/aidans-patches/compare/v1.7.0-dev.1...v1.7.0-dev.2) (2026-10-13)

### 🐛 Bug Fixes

* **site:** prevented linking prior workflow run url on queued badges ([97c50a9](https://github.com/ihatenodejs/aidans-patches/commit/97c50a92aa8eb35ba0039d3487246db23893c3c1))

### ✨ New Features

* **sidelineswap:** added remove invite friends patch ([04328a9](https://github.com/ihatenodejs/aidans-patches/commit/04328a924ddd04de690da61b7b3937adbb26d789))

## [1.7.0-dev.1](https://github.com/ihatenodejs/aidans-patches/compare/v1.6.1-dev.1...v1.7.0-dev.1) (2026-10-12)

### 🐛 Bug Fixes

* **release:** update bun lockfile for semantic-release github plugin ([cea934f](https://github.com/ihatenodejs/aidans-patches/commit/cea934f8955192f773b46ea402fed80045e8c641))

### ✨ New Features

* **geocaching:** added experimental map sheet support for osm styles ([1f29c32](https://github.com/ihatenodejs/aidans-patches/commit/1f29c327d9c847bc34cad0dbb7970b331d0375e5))

## [1.6.1-dev.1](https://github.com/ihatenodejs/aidans-patches/compare/v1.6.0...v1.6.1-dev.1) (2026-10-12)

### 🐛 Bug Fixes

* **geocaching:** neutralized server sync for experimental features ([ab635cb](https://github.com/ihatenodejs/aidans-patches/commit/ab635cb6170d5ba5fb1e821f4f5e2c9c4e59c7b2))

### ✨ New Features

* **site:** added ViewMoreAppsCard and Crosshair components ([b66cb3d](https://github.com/ihatenodejs/aidans-patches/commit/b66cb3d1d7746bc24ad4f5964d4cbbfc0cc5a4e2))

## [1.7.0-dev.1](https://github.com/ihatenodejs/aidans-patches/compare/v1.6.0...v1.7.0-dev.1) (2026-10-12)

### ✨ New Features

* **site:** added ViewMoreAppsCard and Crosshair components ([b66cb3d](https://github.com/ihatenodejs/aidans-patches/commit/b66cb3d1d7746bc24ad4f5964d4cbbfc0cc5a4e2))

## [1.6.0](https://github.com/ihatenodejs/aidans-patches/compare/v1.5.1...v1.6.0) (2026-10-11)

### 🐛 Bug Fixes

* **apk-lab:** allowed jadx exit code 3 during analysis ([bf848ba](https://github.com/ihatenodejs/aidans-patches/commit/bf848ba47fc3b20f6de605cffe3896ed13ca2d8a))
* **audit:** address review comments ([a28833d](https://github.com/ihatenodejs/aidans-patches/commit/a28833d7d2ed7458560a8444244898dba7c5abf9))
* **audit:** resolve post-merge quality gates, formatting, and task dependencies ([43be30e](https://github.com/ihatenodejs/aidans-patches/commit/43be30e5c3e25ef9dc939ad6ebedd297c7ff872a))
* **ci:** reported terminal status to worker on pipeline failures ([9deac83](https://github.com/ihatenodejs/aidans-patches/commit/9deac838c87151580b8b71aa3b78ebeb38032107))
* **ci:** resolve nitpick comments ([50b19a9](https://github.com/ihatenodejs/aidans-patches/commit/50b19a9a848a5b2afb50bfb3221014b26062bb8f))
* **ci:** set User-Agent in compatibility submission and reuse acquired artifact for target check if version matches ([8384ed1](https://github.com/ihatenodejs/aidans-patches/commit/8384ed122976fa1cd99f094cec9fb37886cc5f62))
* **geocaching:** removed server-dependent profile statistics in local premium patch ([ee9aa13](https://github.com/ihatenodejs/aidans-patches/commit/ee9aa134b6459452b30b80fc68a89892c330f3bf))
* **patches/main:** fixed geocaching navigation map loading and MapLibre routing ([76f393e](https://github.com/ihatenodejs/aidans-patches/commit/76f393e8215354631a166145ad74146153f81f8c))
* **site:** corrected test verification scope on status page ([1c9d6a4](https://github.com/ihatenodejs/aidans-patches/commit/1c9d6a40fa8a63187a6a65e067726bd23323be88))
* **tools/apk-lab:** fixed anr package matching and smali validation ([63e2b11](https://github.com/ihatenodejs/aidans-patches/commit/63e2b1178a6d82256c792be6bb3835de7472468d))
* **tools/apk-lab:** fixed inspection, monitoring, and smali edge cases ([15c5aaa](https://github.com/ihatenodejs/aidans-patches/commit/15c5aaa701d42025a63ada5e5d8fd41d15811355))
* **worker:** reduced outstanding request ttl to one scrape cycle ([9eaf4fd](https://github.com/ihatenodejs/aidans-patches/commit/9eaf4fded0cb188e056b6858edbad5a52d9a623e))

### ✨ New Features

* **compatibility:** Added APKMirror fallback and missing-APK status ([fc536ae](https://github.com/ihatenodejs/aidans-patches/commit/fc536ae5dcfaaac2b81b31d11591c83b6f18230e))
* **compatibility:** added push matrix runs and failure stage tracking ([7f07e44](https://github.com/ihatenodejs/aidans-patches/commit/7f07e44e5a4718075cd4a0ba3d674d5570eeefb3))
* **geocaching:** added client-side map filtering and search fixes ([ff953c8](https://github.com/ihatenodejs/aidans-patches/commit/ff953c856a95cbcb7cacdd0ac6fed33dbc10a839))
* **geocaching:** added OpenStreetMap style switching and previews ([b4bc94a](https://github.com/ihatenodejs/aidans-patches/commit/b4bc94a5ffb68b797fce644863cff850851cef67))
* **geocaching:** added support and patches for Geocaching ([e9c822a](https://github.com/ihatenodejs/aidans-patches/commit/e9c822a62be2046c41cac8e4719cdd0d791ea16f))
* **patches/main:** Added Geocaching Shop removal patch ([310ceb1](https://github.com/ihatenodejs/aidans-patches/commit/310ceb1a3fc3e242e93c5be2b5155dbb10db6b6c))
* **site:** added app name filtering to patch search ([b0a8796](https://github.com/ihatenodejs/aidans-patches/commit/b0a879641e8d09d2d6c131b11c5b46e937592081))
* **tools/apk-lab:** added deploy, monitor, and APK inspection commands ([fea8c0c](https://github.com/ihatenodejs/aidans-patches/commit/fea8c0cec079e21f18227e86a5f9f9fa1df17686))
* **worker:** added release download statistics and SVG chart endpoints ([60fafd7](https://github.com/ihatenodejs/aidans-patches/commit/60fafd7c57cf97862b1bd4de99bce8cd1d34f77e))

## [1.6.0-dev.6](https://github.com/ihatenodejs/aidans-patches/compare/v1.6.0-dev.5...v1.6.0-dev.6) (2026-10-11)

### ✨ New Features

* **worker:** added release download statistics and SVG chart endpoints ([60fafd7](https://github.com/ihatenodejs/aidans-patches/commit/60fafd7c57cf97862b1bd4de99bce8cd1d34f77e))

## [1.6.0-dev.5](https://github.com/ihatenodejs/aidans-patches/compare/v1.6.0-dev.4...v1.6.0-dev.5) (2026-10-11)

### 🐛 Bug Fixes

* **geocaching:** removed server-dependent profile statistics in local premium patch ([ee9aa13](https://github.com/ihatenodejs/aidans-patches/commit/ee9aa134b6459452b30b80fc68a89892c330f3bf))

## [1.6.0-dev.4](https://github.com/ihatenodejs/aidans-patches/compare/v1.6.0-dev.3...v1.6.0-dev.4) (2026-10-11)

### 🐛 Bug Fixes

* **patches/main:** fixed geocaching navigation map loading and MapLibre routing ([76f393e](https://github.com/ihatenodejs/aidans-patches/commit/76f393e8215354631a166145ad74146153f81f8c))

## [1.6.0-dev.3](https://github.com/ihatenodejs/aidans-patches/compare/v1.6.0-dev.2...v1.6.0-dev.3) (2026-10-10)

### 🐛 Bug Fixes

* **tools/apk-lab:** fixed anr package matching and smali validation ([63e2b11](https://github.com/ihatenodejs/aidans-patches/commit/63e2b1178a6d82256c792be6bb3835de7472468d))
* **tools/apk-lab:** fixed inspection, monitoring, and smali edge cases ([15c5aaa](https://github.com/ihatenodejs/aidans-patches/commit/15c5aaa701d42025a63ada5e5d8fd41d15811355))

### ✨ New Features

* **tools/apk-lab:** added deploy, monitor, and APK inspection commands ([fea8c0c](https://github.com/ihatenodejs/aidans-patches/commit/fea8c0cec079e21f18227e86a5f9f9fa1df17686))

## [1.6.0-dev.2](https://github.com/ihatenodejs/aidans-patches/compare/v1.6.0-dev.1...v1.6.0-dev.2) (2026-10-10)

### 🐛 Bug Fixes

* **apk-lab:** allowed jadx exit code 3 during analysis ([bf848ba](https://github.com/ihatenodejs/aidans-patches/commit/bf848ba47fc3b20f6de605cffe3896ed13ca2d8a))
* **audit:** resolve post-merge quality gates, formatting, and task dependencies ([43be30e](https://github.com/ihatenodejs/aidans-patches/commit/43be30e5c3e25ef9dc939ad6ebedd297c7ff872a))

### ✨ New Features

* **geocaching:** added client-side map filtering and search fixes ([ff953c8](https://github.com/ihatenodejs/aidans-patches/commit/ff953c856a95cbcb7cacdd0ac6fed33dbc10a839))
* **geocaching:** added OpenStreetMap style switching and previews ([b4bc94a](https://github.com/ihatenodejs/aidans-patches/commit/b4bc94a5ffb68b797fce644863cff850851cef67))
* **geocaching:** added support and patches for Geocaching ([e9c822a](https://github.com/ihatenodejs/aidans-patches/commit/e9c822a62be2046c41cac8e4719cdd0d791ea16f))
* **patches/main:** Added Geocaching Shop removal patch ([310ceb1](https://github.com/ihatenodejs/aidans-patches/commit/310ceb1a3fc3e242e93c5be2b5155dbb10db6b6c))

## [1.6.0-dev.1](https://github.com/ihatenodejs/aidans-patches/compare/v1.5.1...v1.6.0-dev.1) (2026-10-09)

### 🐛 Bug Fixes

* **audit:** address review comments ([a28833d](https://github.com/ihatenodejs/aidans-patches/commit/a28833d7d2ed7458560a8444244898dba7c5abf9))
* **ci:** reported terminal status to worker on pipeline failures ([9deac83](https://github.com/ihatenodejs/aidans-patches/commit/9deac838c87151580b8b71aa3b78ebeb38032107))
* **ci:** resolve nitpick comments ([50b19a9](https://github.com/ihatenodejs/aidans-patches/commit/50b19a9a848a5b2afb50bfb3221014b26062bb8f))
* **ci:** set User-Agent in compatibility submission and reuse acquired artifact for target check if version matches ([8384ed1](https://github.com/ihatenodejs/aidans-patches/commit/8384ed122976fa1cd99f094cec9fb37886cc5f62))
* **site:** corrected test verification scope on status page ([1c9d6a4](https://github.com/ihatenodejs/aidans-patches/commit/1c9d6a40fa8a63187a6a65e067726bd23323be88))
* **worker:** reduced outstanding request ttl to one scrape cycle ([9eaf4fd](https://github.com/ihatenodejs/aidans-patches/commit/9eaf4fded0cb188e056b6858edbad5a52d9a623e))

### ✨ New Features

* **compatibility:** Added APKMirror fallback and missing-APK status ([fc536ae](https://github.com/ihatenodejs/aidans-patches/commit/fc536ae5dcfaaac2b81b31d11591c83b6f18230e))
* **compatibility:** added push matrix runs and failure stage tracking ([7f07e44](https://github.com/ihatenodejs/aidans-patches/commit/7f07e44e5a4718075cd4a0ba3d674d5570eeefb3))
* **site:** added app name filtering to patch search ([b0a8796](https://github.com/ihatenodejs/aidans-patches/commit/b0a879641e8d09d2d6c131b11c5b46e937592081))

## [1.5.1](https://github.com/ihatenodejs/aidans-patches/compare/v1.5.0...v1.5.1) (2026-10-09)

### 🐛 Bug Fixes

* **ci:** set User-Agent in compatibility submission and reuse acquired artifact for target check if version matches ([1901351](https://github.com/ihatenodejs/aidans-patches/commit/1901351bfddb604a4674695a86d9826f6a0a0caf))

## [1.5.0-dev.3](https://github.com/ihatenodejs/aidans-patches/compare/v1.5.0-dev.2...v1.5.0-dev.3) (2026-10-09)

### 🐛 Bug Fixes

* **audit:** address review comments ([a28833d](https://github.com/ihatenodejs/aidans-patches/commit/a28833d7d2ed7458560a8444244898dba7c5abf9))
* **ci:** reported terminal status to worker on pipeline failures ([9deac83](https://github.com/ihatenodejs/aidans-patches/commit/9deac838c87151580b8b71aa3b78ebeb38032107))
* **ci:** resolve nitpick comments ([50b19a9](https://github.com/ihatenodejs/aidans-patches/commit/50b19a9a848a5b2afb50bfb3221014b26062bb8f))
* **ci:** set User-Agent in compatibility submission and reuse acquired artifact for target check if version matches ([8384ed1](https://github.com/ihatenodejs/aidans-patches/commit/8384ed122976fa1cd99f094cec9fb37886cc5f62))
* **site:** corrected test verification scope on status page ([1c9d6a4](https://github.com/ihatenodejs/aidans-patches/commit/1c9d6a40fa8a63187a6a65e067726bd23323be88))
* **worker:** reduced outstanding request ttl to one scrape cycle ([9eaf4fd](https://github.com/ihatenodejs/aidans-patches/commit/9eaf4fded0cb188e056b6858edbad5a52d9a623e))

### ✨ New Features

* **compatibility:** Added APKMirror fallback and missing-APK status ([fc536ae](https://github.com/ihatenodejs/aidans-patches/commit/fc536ae5dcfaaac2b81b31d11591c83b6f18230e))
* **compatibility:** added push matrix runs and failure stage tracking ([7f07e44](https://github.com/ihatenodejs/aidans-patches/commit/7f07e44e5a4718075cd4a0ba3d674d5570eeefb3))
* **site:** added app name filtering to patch search ([b0a8796](https://github.com/ihatenodejs/aidans-patches/commit/b0a879641e8d09d2d6c131b11c5b46e937592081))

## [1.5.0-dev.2](https://github.com/ihatenodejs/aidans-patches/compare/v1.5.0-dev.1...v1.5.0-dev.2) (2026-10-09)

### 🐛 Bug Fixes

* **site:** clean up status page, worker, and apk lab ([#16](https://github.com/ihatenodejs/aidans-patches/issues/16)) ([a666f02](https://github.com/ihatenodejs/aidans-patches/commit/a666f02c74b7c2a5f2d716df0a94f38ec127624c))

## [1.5.0](https://github.com/ihatenodejs/aidans-patches/compare/v1.4.0...v1.5.0) (2026-10-09)

### 🐛 Bug Fixes

* **site:** clean up status page, worker, and apk lab ([#16](https://github.com/ihatenodejs/aidans-patches/issues/16)) ([a666f02](https://github.com/ihatenodejs/aidans-patches/commit/a666f02c74b7c2a5f2d716df0a94f38ec127624c))

### ✨ New Features

* **sezzle:** Added subscription removal patch and shortcut toggle ([47801e6](https://github.com/ihatenodejs/aidans-patches/commit/47801e691eca3274ceb00d21ac43932a48ca5545))

## [1.5.0-dev.1](https://github.com/ihatenodejs/aidans-patches/compare/v1.4.0...v1.5.0-dev.1) (2026-10-09)

### ✨ New Features

* **sezzle:** Added subscription removal patch and shortcut toggle ([47801e6](https://github.com/ihatenodejs/aidans-patches/commit/47801e691eca3274ceb00d21ac43932a48ca5545))

## [1.4.0](https://github.com/ihatenodejs/aidans-patches/compare/v1.3.1...v1.4.0) (2026-10-08)

### 🐛 Bug Fixes

* **apk-lab:** address review feedback on asm, native extraction, il2cpp layouts, and unity cli ([9e1addc](https://github.com/ihatenodejs/aidans-patches/commit/9e1addccd11969151c735345d231718afc730e21))
* **apk-lab:** Allowed overriding default kwargs in tool runner ([76801f6](https://github.com/ihatenodejs/aidans-patches/commit/76801f612922c52a52bb264a01eb9d78f0a1e53d))
* **apk-lab:** detect conflicting duplicate native library entries within an APK ([d85bc28](https://github.com/ihatenodejs/aidans-patches/commit/d85bc28f716251f3e0952a93e8bb5798aaf4693e))
* **apk-lab:** Included AndroidManifest in patchable member check ([af7fcd6](https://github.com/ihatenodejs/aidans-patches/commit/af7fcd6d19ec5d22d9f8494ac774a7ed3fa26124))
* **apk-lab:** Included resources.arsc in patch artifact detection ([845d8f1](https://github.com/ihatenodejs/aidans-patches/commit/845d8f1623fa68063ccf17ee9e5e62df1c746cb4))
* **blackjack:** update to 2.22.09 ([34eca78](https://github.com/ihatenodejs/aidans-patches/commit/34eca782360220175556fc587f37a8c5e011c9e0))
* **build:** Selected newest patch artifact ([185b8bc](https://github.com/ihatenodejs/aidans-patches/commit/185b8bce6097df0a9c9d435a482d1fd898115b43))
* **sezzle:** Corrected rewards dispatch and CodePush method matching ([0b60e45](https://github.com/ihatenodejs/aidans-patches/commit/0b60e450f3fbae774b4ec7888458bb8152f62e10))
* **sezzle:** Preserved Sezzle Send navigation in rewards patch ([9245a74](https://github.com/ihatenodejs/aidans-patches/commit/9245a74c3d49b921938e435bd31edf1114217147))
* **sezzle:** Stubbed CodePush bundle resolution in update patch ([c1cac39](https://github.com/ihatenodejs/aidans-patches/commit/c1cac396717618c97eae379f4328677b74975ca2))

### ✨ New Features

* **apk-lab:** add deterministic ARM64 instruction encoder and CLI ([58324dc](https://github.com/ihatenodejs/aidans-patches/commit/58324dc430cc9ed8659e29ced02be7776e257289))
* **apk-lab:** add IL2CPP metadata extractor and symbol search command ([13d41bd](https://github.com/ihatenodejs/aidans-patches/commit/13d41bdc0d492fdf705e48f8d497cf3ede376235))
* **apk-lab:** add Unity serialized asset inspector and GameObject locator ([4cdb737](https://github.com/ihatenodejs/aidans-patches/commit/4cdb7378b6f5d4ccf3204ff55eb34da95f524a80))
* **apk-lab:** added apk compatibility testing and status tracking ([c077d5a](https://github.com/ihatenodejs/aidans-patches/commit/c077d5a2b72447f455c9e80268f95c59a7bd7395))
* **apk-lab:** extract native libraries across splits during analyze ([55e7aa1](https://github.com/ihatenodejs/aidans-patches/commit/55e7aa13a782c5096b7ea564f2f1e43eb446c837))
* **fizz:** Added feed advertisement removal patch ([6c10066](https://github.com/ihatenodejs/aidans-patches/commit/6c100662f9516b125cf4ce3fe60808fb3c9e1678))
* **sezzle:** Added configurable shortcut filtering ([5716677](https://github.com/ihatenodejs/aidans-patches/commit/57166774f3ce51eed55606f7c9d783750397183f))
* **sidelineswap:** Added AMOLED theme patch ([619e112](https://github.com/ihatenodejs/aidans-patches/commit/619e112c7304eb0a0a03d677fb0d65f47448b99c))
* **sidelineswap:** Added cart item and CardView theming ([9603003](https://github.com/ihatenodejs/aidans-patches/commit/9603003d69265ffafd4ce4fbe5a39dfe27ea4469))
* **sidelineswap:** Added dark WebView support and expanded AMOLED theme ([d0de58a](https://github.com/ihatenodejs/aidans-patches/commit/d0de58a456ec9d9bd8a2f421cf216ef6f8420f75))
* **sidelineswap:** Added payment sheet theming and address list fix ([4290b27](https://github.com/ihatenodejs/aidans-patches/commit/4290b270af2c009b4ef1c1c52c483a0b5c9a0b57))
* **sidelineswap:** Added shipping badge and avatar stroke theming ([cdfed1e](https://github.com/ihatenodejs/aidans-patches/commit/cdfed1e4bbba03c8d94d789ca57824c5964a5498))

## [1.4.0-dev.7](https://github.com/ihatenodejs/aidans-patches/compare/v1.4.0-dev.6...v1.4.0-dev.7) (2026-10-08)

### 🐛 Bug Fixes

* **apk-lab:** Included AndroidManifest in patchable member check ([af7fcd6](https://github.com/ihatenodejs/aidans-patches/commit/af7fcd6d19ec5d22d9f8494ac774a7ed3fa26124))

### ✨ New Features

* **sidelineswap:** Added AMOLED theme patch ([619e112](https://github.com/ihatenodejs/aidans-patches/commit/619e112c7304eb0a0a03d677fb0d65f47448b99c))
* **sidelineswap:** Added cart item and CardView theming ([9603003](https://github.com/ihatenodejs/aidans-patches/commit/9603003d69265ffafd4ce4fbe5a39dfe27ea4469))
* **sidelineswap:** Added dark WebView support and expanded AMOLED theme ([d0de58a](https://github.com/ihatenodejs/aidans-patches/commit/d0de58a456ec9d9bd8a2f421cf216ef6f8420f75))
* **sidelineswap:** Added payment sheet theming and address list fix ([4290b27](https://github.com/ihatenodejs/aidans-patches/commit/4290b270af2c009b4ef1c1c52c483a0b5c9a0b57))
* **sidelineswap:** Added shipping badge and avatar stroke theming ([cdfed1e](https://github.com/ihatenodejs/aidans-patches/commit/cdfed1e4bbba03c8d94d789ca57824c5964a5498))

## [1.4.0-dev.6](https://github.com/ihatenodejs/aidans-patches/compare/v1.4.0-dev.5...v1.4.0-dev.6) (2026-10-08)

### 🐛 Bug Fixes

* **apk-lab:** Allowed overriding default kwargs in tool runner ([76801f6](https://github.com/ihatenodejs/aidans-patches/commit/76801f612922c52a52bb264a01eb9d78f0a1e53d))
* **apk-lab:** Included resources.arsc in patch artifact detection ([845d8f1](https://github.com/ihatenodejs/aidans-patches/commit/845d8f1623fa68063ccf17ee9e5e62df1c746cb4))
* **sezzle:** Corrected rewards dispatch and CodePush method matching ([0b60e45](https://github.com/ihatenodejs/aidans-patches/commit/0b60e450f3fbae774b4ec7888458bb8152f62e10))
* **sezzle:** Preserved Sezzle Send navigation in rewards patch ([9245a74](https://github.com/ihatenodejs/aidans-patches/commit/9245a74c3d49b921938e435bd31edf1114217147))
* **sezzle:** Stubbed CodePush bundle resolution in update patch ([c1cac39](https://github.com/ihatenodejs/aidans-patches/commit/c1cac396717618c97eae379f4328677b74975ca2))

## [1.4.0-dev.5](https://github.com/ihatenodejs/aidans-patches/compare/v1.4.0-dev.4...v1.4.0-dev.5) (2026-10-08)

### 🐛 Bug Fixes

* **apk-lab:** address review feedback on asm, native extraction, il2cpp layouts, and unity cli ([9e1addc](https://github.com/ihatenodejs/aidans-patches/commit/9e1addccd11969151c735345d231718afc730e21))
* **apk-lab:** detect conflicting duplicate native library entries within an APK ([d85bc28](https://github.com/ihatenodejs/aidans-patches/commit/d85bc28f716251f3e0952a93e8bb5798aaf4693e))

### ✨ New Features

* **apk-lab:** add deterministic ARM64 instruction encoder and CLI ([58324dc](https://github.com/ihatenodejs/aidans-patches/commit/58324dc430cc9ed8659e29ced02be7776e257289))
* **apk-lab:** add IL2CPP metadata extractor and symbol search command ([13d41bd](https://github.com/ihatenodejs/aidans-patches/commit/13d41bdc0d492fdf705e48f8d497cf3ede376235))
* **apk-lab:** add Unity serialized asset inspector and GameObject locator ([4cdb737](https://github.com/ihatenodejs/aidans-patches/commit/4cdb7378b6f5d4ccf3204ff55eb34da95f524a80))
* **apk-lab:** extract native libraries across splits during analyze ([55e7aa1](https://github.com/ihatenodejs/aidans-patches/commit/55e7aa13a782c5096b7ea564f2f1e43eb446c837))

## [1.4.0-dev.4](https://github.com/ihatenodejs/aidans-patches/compare/v1.4.0-dev.3...v1.4.0-dev.4) (2026-10-08)

### 🐛 Bug Fixes

* **blackjack:** update to 2.22.09 ([34eca78](https://github.com/ihatenodejs/aidans-patches/commit/34eca782360220175556fc587f37a8c5e011c9e0))

## [1.4.0-dev.3](https://github.com/ihatenodejs/aidans-patches/compare/v1.4.0-dev.2...v1.4.0-dev.3) (2026-10-07)

### ✨ New Features

* **sezzle:** Added configurable shortcut filtering ([5716677](https://github.com/ihatenodejs/aidans-patches/commit/57166774f3ce51eed55606f7c9d783750397183f))

## [1.4.0-dev.2](https://github.com/ihatenodejs/aidans-patches/compare/v1.4.0-dev.1...v1.4.0-dev.2) (2026-10-07)

### ✨ New Features

* **apk-lab:** added apk compatibility testing and status tracking ([c077d5a](https://github.com/ihatenodejs/aidans-patches/commit/c077d5a2b72447f455c9e80268f95c59a7bd7395))

## [1.4.0-dev.1](https://github.com/ihatenodejs/aidans-patches/compare/v1.3.1...v1.4.0-dev.1) (2026-10-06)

### 🐛 Bug Fixes

* **build:** Selected newest patch artifact ([185b8bc](https://github.com/ihatenodejs/aidans-patches/commit/185b8bce6097df0a9c9d435a482d1fd898115b43))

### ✨ New Features

* **fizz:** Added feed advertisement removal patch ([6c10066](https://github.com/ihatenodejs/aidans-patches/commit/6c100662f9516b125cf4ce3fe60808fb3c9e1678))

## [1.3.1](https://github.com/ihatenodejs/aidans-patches/compare/v1.3.0...v1.3.1) (2026-10-06)

### 🐛 Bug Fixes

* **ci:** allow release workflow to create backmerge PRs ([8bc1992](https://github.com/ihatenodejs/aidans-patches/commit/8bc1992dc6904a73ad205adc858db1847ff7afe3))

## [1.3.1-dev.1](https://github.com/ihatenodejs/aidans-patches/compare/v1.3.0...v1.3.1-dev.1) (2026-10-06)

### 🐛 Bug Fixes

* **ci:** allow release workflow to create backmerge PRs ([8bc1992](https://github.com/ihatenodejs/aidans-patches/commit/8bc1992dc6904a73ad205adc858db1847ff7afe3))

## [1.3.0](https://github.com/ihatenodejs/aidans-patches/compare/v1.2.0...v1.3.0) (2026-10-05)

### 🐛 Bug Fixes

* add semantic-release-backmerge plugin ([234cab1](https://github.com/ihatenodejs/aidans-patches/commit/234cab18dcce7c2eb3493ea0ceec0b001eaacac1))
* **release:** rollback incorrect version, implement @kilianpaquier/semantic-release-backmerge ([7df1007](https://github.com/ihatenodejs/aidans-patches/commit/7df1007579ff01f896aaee32bd791acb6e9c0bc6))
* **site:** fixed status data and deployment safeguards ([f4f1e77](https://github.com/ihatenodejs/aidans-patches/commit/f4f1e77a0e5577e72bf9d23964c97ac14021bcc9))

### ✨ New Features

* **sezzle:** added js bundle patch to remove ads and tracking ([a5c9937](https://github.com/ihatenodejs/aidans-patches/commit/a5c993787b66b2da0c9b6a37dafe6a7263b6ec0e))
* **site:** added documentation and showcase website ([4fba928](https://github.com/ihatenodejs/aidans-patches/commit/4fba9284c3a663680f2324b70210e50ba379a9b2))
* **site:** added live status polling and dynamic patch counts ([e1907ab](https://github.com/ihatenodejs/aidans-patches/commit/e1907ab24c827a4d8b490fbf5b6449353b494877))
* **worker:** added custom domain route and refresh authorization ([1acbd40](https://github.com/ihatenodejs/aidans-patches/commit/1acbd40067e180d51b7526de8ea96b401cf99823))

## [1.3.0-dev.1](https://github.com/ihatenodejs/aidans-patches/compare/v1.2.0...v1.3.0-dev.1) (2026-10-05)

### 🐛 Bug Fixes

* add semantic-release-backmerge plugin ([234cab1](https://github.com/ihatenodejs/aidans-patches/commit/234cab18dcce7c2eb3493ea0ceec0b001eaacac1))
* **release:** rollback incorrect version, implement @kilianpaquier/semantic-release-backmerge ([7df1007](https://github.com/ihatenodejs/aidans-patches/commit/7df1007579ff01f896aaee32bd791acb6e9c0bc6))

### ✨ New Features

* **sezzle:** added js bundle patch to remove ads and tracking ([a5c9937](https://github.com/ihatenodejs/aidans-patches/commit/a5c993787b66b2da0c9b6a37dafe6a7263b6ec0e))

## [1.2.0](https://github.com/ihatenodejs/aidans-patches/compare/v1.1.0...v1.2.0) (2026-10-03)

### 🐛 Bug Fixes

* **fizz:** fixed developer button visibility on single-feed layouts ([0bc1f31](https://github.com/ihatenodejs/aidans-patches/commit/0bc1f31358350260a0b437eb5ab8151255574f71))
* **fizz:** preserved 2048 UPM metrics for Apple Color Emoji ([cbd6f86](https://github.com/ihatenodejs/aidans-patches/commit/cbd6f86735c84fabe381ffc61e19c53567a4f684))

### ✨ New Features

* **adobe-scan:** added patch suite and documentation for adobe scan ([89eae8e](https://github.com/ihatenodejs/aidans-patches/commit/89eae8e7e9bfc36784f5c90798ba36fb4dd36bfe))
* **blackjack:** added internet and notification permission removal patches ([2f23c54](https://github.com/ihatenodejs/aidans-patches/commit/2f23c541fa089abf3fbc1f52f620b1e0a1457ba0))
* **fizz:** added developer settings mod menu patch ([aee93d8](https://github.com/ihatenodejs/aidans-patches/commit/aee93d8e05aa66e620b8d25cfbd498439dbcc8e1))
* **fizz:** added ios emoji font replacement patch and dynamic build pipeline ([0bf8157](https://github.com/ihatenodejs/aidans-patches/commit/0bf81570512fc4db0ff731d9b6b5e113236ed870))
* **fizz:** added patch suite and documentation for fizz ([1ecfacf](https://github.com/ihatenodejs/aidans-patches/commit/1ecfacffb6af05ac285dd006c0b18f353d00867e))

## [1.2.0-dev.5](https://github.com/ihatenodejs/aidans-patches/compare/v1.2.0-dev.4...v1.2.0-dev.5) (2026-10-03)

### 🐛 Bug Fixes

* **fizz:** preserved 2048 UPM metrics for Apple Color Emoji ([cbd6f86](https://github.com/ihatenodejs/aidans-patches/commit/cbd6f86735c84fabe381ffc61e19c53567a4f684))

## [1.2.0-dev.4](https://github.com/ihatenodejs/aidans-patches/compare/v1.2.0-dev.3...v1.2.0-dev.4) (2026-10-03)

### 🐛 Bug Fixes

* **fizz:** fixed developer button visibility on single-feed layouts ([0bc1f31](https://github.com/ihatenodejs/aidans-patches/commit/0bc1f31358350260a0b437eb5ab8151255574f71))

## [1.2.0-dev.3](https://github.com/ihatenodejs/aidans-patches/compare/v1.2.0-dev.2...v1.2.0-dev.3) (2026-10-03)

### ✨ New Features

* **fizz:** added developer settings mod menu patch ([aee93d8](https://github.com/ihatenodejs/aidans-patches/commit/aee93d8e05aa66e620b8d25cfbd498439dbcc8e1))

## [1.2.0-dev.2](https://github.com/ihatenodejs/aidans-patches/compare/v1.2.0-dev.1...v1.2.0-dev.2) (2026-10-03)

### ✨ New Features

* **fizz:** added ios emoji font replacement patch and dynamic build pipeline ([0bf8157](https://github.com/ihatenodejs/aidans-patches/commit/0bf81570512fc4db0ff731d9b6b5e113236ed870))

## [1.2.0-dev.1](https://github.com/ihatenodejs/aidans-patches/compare/v1.1.0...v1.2.0-dev.1) (2026-10-03)

### ✨ New Features

* **adobe-scan:** added patch suite and documentation for adobe scan ([89eae8e](https://github.com/ihatenodejs/aidans-patches/commit/89eae8e7e9bfc36784f5c90798ba36fb4dd36bfe))
* **blackjack:** added internet and notification permission removal patches ([2f23c54](https://github.com/ihatenodejs/aidans-patches/commit/2f23c541fa089abf3fbc1f52f620b1e0a1457ba0))
* **fizz:** added patch suite and documentation for fizz ([1ecfacf](https://github.com/ihatenodejs/aidans-patches/commit/1ecfacffb6af05ac285dd006c0b18f353d00867e))

## [1.1.0](https://github.com/ihatenodejs/aidans-patches/compare/v1.0.0...v1.1.0) (2026-10-02)

### 🐛 Bug Fixes

* **ads:** Disabled Braze in-app message registration ([06077d1](https://github.com/ihatenodejs/aidans-patches/commit/06077d14007ccfcf34cb5a01bee0598844eba2a6))
* **aftership:** aligned map controls with tracking toolbar ([4e03d67](https://github.com/ihatenodejs/aidans-patches/commit/4e03d674715136664b8c2fe1bde9560dd99de96c))
* **aftership:** resolved checkpoint coordinates from geo entities ([caf5cb9](https://github.com/ihatenodejs/aidans-patches/commit/caf5cb9c6df2d2d9252916ebe6cd37e44aef0ca7))
* **blackjack:** aligned skip level dialog touch target with safe area insets ([cc2ecca](https://github.com/ihatenodejs/aidans-patches/commit/cc2ecca92ef89df7c563dead4b212a0ef3d3028a))
* **blackjack:** closed chip file stream via try-with-resources ([a0758bc](https://github.com/ihatenodejs/aidans-patches/commit/a0758bca3149c77e89d8bfe683e25619a30b36ea))
* **blackjack:** logged 16 kb page alignment configuration failure ([780cdda](https://github.com/ihatenodejs/aidans-patches/commit/780cdda9343709841d0727189b09b80a49f7eaf2))
* **blackjack:** recalibrated skip level dialog touch target ([e128e1b](https://github.com/ihatenodejs/aidans-patches/commit/e128e1b844f8793897cd552b73c8803ccc95b66a))
* **generator:** mapped patch dependencies by display name ([328db4b](https://github.com/ihatenodejs/aidans-patches/commit/328db4b8c104ead3b6cea75255e809018a1f6e3d))
* **sezzle:** corrected byte sequences for hidden AI elements ([26f16bd](https://github.com/ihatenodejs/aidans-patches/commit/26f16bd1d30ff1fb60a84fb77d3cf287bd7dd1ac))
* **sezzle:** corrected jail monkey rooted check class targeting ([d556f97](https://github.com/ihatenodejs/aidans-patches/commit/d556f97c0c577a1c15b61e37b38802c986ddf243))
* **sezzle:** preserved total cents fallback in header navigation ([e480112](https://github.com/ihatenodejs/aidans-patches/commit/e480112d9a4a13f7ab3efb977d70bea22ae94e78))
* **sidelineswap:** refined method matching for telemetry overrides ([987a098](https://github.com/ihatenodejs/aidans-patches/commit/987a098ac588f5c5c0ae8b39175e89007f91636b))
* **sidelineswap:** validated hex format for brand color options ([67a83d1](https://github.com/ihatenodejs/aidans-patches/commit/67a83d1fd9575deffe25ae177e7dd8b07183c8c4))

### ✨ New Features

* **patches/main:** Expanded Sezzle navigation controls ([b76447c](https://github.com/ihatenodejs/aidans-patches/commit/b76447cda548ca22fdf981efaec51c11dd3ff43a))
* **patches:** Added multi-application patch suites ([9ba0e11](https://github.com/ihatenodejs/aidans-patches/commit/9ba0e11f7f9c81a2a940534eeb0baefdb863d927))

## 1.0.0 (2026-09-20)
