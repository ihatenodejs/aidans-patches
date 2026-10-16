---
name: morphe-patches
description: Authoring, modifying, and debugging Morphe patches (bytecode, resource, raw asset, composite) in Kotlin, Java extension runtime bridges, Hermes bytecode, ARM64 assembly, and target app version upgrade workflows. Use when creating or editing patches under patches/, implementing runtime bridges under extensions/, manipulating Dalvik/Smali with dexlib2, editing Hermes HBC bundles or ARM64 ELF libraries, or upgrading supported target app versions.
---

# Morphe Patches: Authoring, Runtime Bridges & Target App Maintenance

This skill provides comprehensive instructions for authoring, modifying, and verifying Morphe patches under `patches/` and Java extension runtime bridges under `extensions/`. Morphe applies modular bytecode, resource, and asset transformations to third-party Android APKs and split bundles (`.apkm`, `.xapk`).

---

## 1. Core Principles & Hard Invariants

1. **Fail-Fast Rule**:
   - Every required class, method, XML node, or binary sequence must throw `PatchException` with clear diagnostic context when absent.
   - Never silently ignore missing targets or fall through unless a feature is explicitly documented as version-optional.
   - Example: `throw PatchException("Could not find method $methodName in $classDescriptor")`.

2. **Register & Dalvik Instruction Safety**:
   - Dalvik opcodes have strict register width constraints. For instance, format `22c` opcodes (such as `instance-of vA, vB, type@CCCC` or `check-cast`) restrict registers `vA` and `vB` to 4-bit register indices (`v0..v15`).
   - Distinguish parameter registers from local registers: in non-static (instance) methods, `p0` is `this`, while static method parameters start at `p0`.
   - Never clobber existing registers without verifying live ranges or allocating additional local registers. Pre-validate unfamiliar Smali using `apk-lab validate-smali`.

3. **ABI Parity Between Smali and Java Extensions**:
   - Java classes in `extensions/extension/` are compiled into `extensions/extension.mpe` and merged into target DEX.
   - Public class names, method names, parameter types, and return types form a rigid binary interface (ABI).
   - Any modification to Java bridge signatures requires a synchronized change to the injected Smali descriptor (e.g. `invoke-static {p0}, Lapp/aidan/extension/sezzle/ConsentGate;->maybeShow(Landroid/app/Activity;)V`).
   - Never import proprietary/obfuscated target classes in Java source; all target app reflection must occur dynamically at runtime.

4. **Idempotency & Bounds Safety**:
   - Binary and asset modifications must check for prior application (`alreadyPatched`) before mutating bytes.
   - Verify expected bytes and boundary sizes prior to writing: verify `bytes.size > offset + length`, check `matchesBytes(...)`, and throw `PatchException` on mismatch.
   - In Hermes bytecode (HBC) bundles, never shift section offsets or expand tables. Recycle existing donor strings, zero-pad remaining space, and always recalculate the SHA-1 checksum via `updateFooterHash()`.

5. **Metadata Synchronization**:
   - Patch declarations, names, descriptions, options, categories, and compatibility constants are public API consumed by `patches-list.json`, the Cloudflare Worker, and the Astro site.
   - Any change to patch declarations or options must be accompanied by `./gradlew generatePatchesList`. Never hand-edit `patches-list.json`.

---

## 2. Progressive Disclosure: Reference Directory Map

To conserve context tokens, read only the specific reference file needed for your current task:

| Task / Domain | Key Topics & Modules | Reference Document |
|---|---|---|
| **DSL & Architecture** | `compositeFeaturePatch`, `bytecodePatch`, `resourcePatch`, `rawResourcePatch`, `dependsOn`, `extendWith`, options, categories | [DSL & Architecture](references/dsl-architecture.md)<br>`skill://morphe-patches/references/dsl-architecture.md` |
| **Bytecode & Dexlib2** | Dexlib2 AST navigation, `BytecodeNeutralizers`, Smali instruction injection, register widths (`22c`), `validate-smali` | [Bytecode & Dexlib2](references/bytecode-dexlib2.md)<br>`skill://morphe-patches/references/bytecode-dexlib2.md` |
| **Java Extension Bridges** | `extensions/extension.mpe`, reflection over obfuscated classes, lifecycle guards, main-thread UI dispatch, WebView security | [Java Extension Bridges](references/extension-bridges.md)<br>`skill://morphe-patches/references/extension-bridges.md` |
| **Binary Assets & Hermes** | `HermesBundleEditor` (HBC v96, v98+), ARM64 ELF patching, `apk-lab asm`, XML/Manifest DOM manipulation | [Binary Assets, Hermes & Resources](references/binary-assets-hermes.md)<br>`skill://morphe-patches/references/binary-assets-hermes.md` |
| **App Version Upgrades** | 6-phase upgrade playbook: baseline diff, targeted decompilation, symbol remapping, compatibility bump, matrix checks | [App Upgrade Playbook](references/app-upgrade-playbook.md)<br>`skill://morphe-patches/references/app-upgrade-playbook.md` |

---

## 3. Essential Build & Verification Commands

All commands run from the repository root:

```bash
# 1. Compile Kotlin patches and Java extension classes (fast compiler gate)
./gradlew :patches:compileKotlin :extensions:extension:compileReleaseJavaWithJavac --console=plain

# 2. Build complete Android patch bundle (.mpp) and extension bundle (.mpe)
./gradlew :patches:buildAndroid

# 3. Regenerate public metadata inventory (patches-list.json)
./gradlew generatePatchesList

# 4. Validate patch compatibility across baseline and all boolean option permutations
uv run --project tools/apk-lab apk-lab check path/to/app.apkm \
  --mpp patches/build/libs/patches-<version>.mpp \
  --package <package.name> \
  --all

# 5. Full repository audit gate (formatting, linting, typechecking, all tests)
bun run audit
```

---

## 4. Standard Operational Workflows

### Workflow A: Adding or Editing a Bytecode Patch
1. Read [Bytecode & Dexlib2](references/bytecode-dexlib2.md).
2. Determine if the patch requires companion XML/raw assets:
   - If yes and 1-to-1: use `compositeFeaturePatch`.
   - If purely Dalvik: use `bytecodePatch`.
3. Check `app.aidan.patches.shared.BytecodeNeutralizers` to see if existing neutralizers (`disableRequiredVoidMethods`, `returnRequiredBoolean`, etc.) suffice before writing custom Smali.
4. If writing custom Smali, check register allocation and validate using `apk-lab validate-smali`.
5. Run `./gradlew :patches:buildAndroid && ./gradlew generatePatchesList`.
6. Run `apk-lab check <artifact> --mpp <bundle> --package <pkg> --all`.

### Workflow B: Modifying Hermes or Native ARM64 Libraries
1. Read [Binary Assets, Hermes & Resources](references/binary-assets-hermes.md).
2. For Hermes (`index.android.bundle`): instantiate `HermesBundleEditor(bundleFile.readBytes())`, match target function offsets, apply modifications with `patchBytesIfMatches`, and invoke `updateFooterHash()`.
3. For ARM64 (`.so`): disassemble instruction using `apk-lab asm`, verify offsets, confirm idempotency with `alreadyPatched`, and write verified replacements.
4. Run `./gradlew :patches:buildAndroid` and verify via `apk-lab check`.

### Workflow C: Authoring a Java Extension Bridge
1. Read [Java Extension Bridges](references/extension-bridges.md).
2. Create or update `extensions/extension/src/main/java/app/aidan/extension/<app>/<BridgeName>.java`.
3. Keep class `public final` with a `private` constructor and static methods.
4. In the corresponding Kotlin patch, declare `extendWith("extensions/extension.mpe")` and inject the matching `invoke-static` instruction.
5. Compile both targets: `./gradlew :patches:compileKotlin :extensions:extension:compileReleaseJavaWithJavac --console=plain`.

### Workflow D: Upgrading a Supported Target App Version
1. Read [App Upgrade Playbook](references/app-upgrade-playbook.md).
2. Follow the 6-phase upgrade protocol:
   - Phase 1: Diff & Failure Baseline (`apk-lab compare`, `apk-lab check --force`).
   - Phase 2: Targeted Bytecode Analysis & De-obfuscation (`apk-lab inspect-code`, `apk-lab analyze --smali`).
   - Phase 3: Patch Remapping (updating class descriptors and instruction anchors).
   - Phase 4: Compatibility Metadata Bump (`shared/Constants.kt`).
   - Phase 5: Build & Verification (`apk-lab check --all`).
   - Phase 6: Documentation (`docs/<app>/`) and Conventional Commit (`bump(<app>): ...`).
