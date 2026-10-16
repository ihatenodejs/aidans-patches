# Binary Assets, Hermes Bytecode & Resources Reference

This reference covers manipulating Hermes bytecode bundles (`index.android.bundle`), in-place ARM64/ARMv7 native shared library patching (`.so`), and Android XML/Manifest DOM manipulation.

---

## 1. Hermes Bytecode (HBC v96, v98+) Patching

React Native Android applications often compile JavaScript into Hermes Bytecode (HBC). The bundle is located at `assets/index.android.bundle`. In Morphe, asset manipulation is executed within a `rawResourcePatch` or the `rawResources { ... }` block of a `compositeFeaturePatch`.

### `HermesBundleEditor` Overview
The repository provides a shared utility: `app.aidan.patches.shared.HermesBundleEditor`.

```kotlin
import app.aidan.patches.shared.HermesBundleEditor
import app.morphe.patcher.patch.PatchException

rawResources {
    val bundleFile = get("assets/index.android.bundle")
    if (!bundleFile.exists()) {
        throw PatchException("assets/index.android.bundle not found")
    }

    val editor = HermesBundleEditor(bundleFile.readBytes())
    if (editor.version != 96 && editor.version < 98) {
        throw PatchException("Unsupported Hermes version: ${editor.version}")
    }

    // Apply mutations...

    // Critical: Recalculate SHA-1 footer checksum before writing
    editor.updateFooterHash()
    bundleFile.writeBytes(editor.toByteArray())
}
```

### Core API Methods in `HermesBundleEditor`

| Method | Description |
|---|---|
| `findFunctionOffsetByName(name, predicate)` | Finds the code offset of the first function matching the given name and optional offset predicate. |
| `findFunctionOffsetsByName(name)` | Returns a `List<Int>` of all function code offsets matching the name. |
| `matchesBytes(offset, expected)` | Returns `true` if the byte slice starting at `offset` matches `expected`. |
| `patchBytes(offset, bytes)` | Writes raw bytes directly at `offset` (in-place replacement). |
| `patchBytesIfMatches(offset, expected, replacement)` | Validates that `expected.size == replacement.size` and that bytes at `offset` match `expected`, then writes `replacement`. Throws `IllegalArgumentException` on size mismatch or mismatching bytes. |
| `redirectFunctionHeader(funcIndex, newOffset, newSize)` | **(HBC v98+ only)** Redirects a function header to alternate bytecode (e.g. pointing a complex function to a donor no-op function). |
| `updateFooterHash()` | Recalculates the Hermes SHA-1 footer hash across `data.size - 20` bytes and updates the final 20 bytes. |
| `toByteArray()` | Returns the mutated bundle byte array. |

### Hard Invariants for Hermes Patching
1. **Never Shift or Expand Bundles**:
   - HBC offsets for functions, string tables, and identifiers are absolute. Never insert or delete bytes, which would invalidate all subsequent offsets.
   - Any in-place byte replacement must have `expected.size == replacement.size`.
2. **String Replacement & Donor Recycling**:
   - When replacing a string literal inside the string table, the new string cannot exceed the byte capacity of the original string.
   - Any unused remaining bytes in the donor string slot must be zero-padded (`0x00`).
3. **Always Update Footer Hash**:
   - Hermes runtimes verify the 20-byte SHA-1 hash at the end of `index.android.bundle`. If `editor.updateFooterHash()` is omitted, the app will crash instantly on startup with an invalid bundle checksum error.

### Example: Neutralizing a React Native Feature Flag Function
```kotlin
val fnOffset = editor.findFunctionOffsetByName("useIsPromotionsEnabled")
    ?: throw PatchException("Function useIsPromotionsEnabled not found in Hermes bundle")

// Opcode sequence returning boolean false:
// LoadConstFalse r0; Ret r0
val expectedBytes = byteArrayOf(0x30, 0x00, 0x76.toByte(), 0x00)
val returnFalseBytes = byteArrayOf(0x2f, 0x00, 0x76.toByte(), 0x00)

editor.patchBytesIfMatches(fnOffset, expectedBytes, returnFalseBytes)
```

---

## 2. Native Shared Library Patching (ARM64 / ARMv7)

When app integrity checks, signature validation, or core protections are implemented in C/C++ native shared libraries (`lib/**/*.so`), use in-place binary patching inside a `rawResourcePatch`.

### Standard ARM64 Assembly Sequences

| Operation | ARM64 Disassembly | Kotlin Byte Sequence (`byteArrayOf(...)`) |
|---|---|---|
| `RETURN_VOID` | `ret` | `byteArrayOf(0xc0.toByte(), 0x03, 0x5f, 0xd6.toByte())` |
| `RETURN_TRUE` | `mov w0, #1`<br>`ret` | `byteArrayOf(0x20, 0x00, 0x80.toByte(), 0x52, 0xc0.toByte(), 0x03, 0x5f, 0xd6.toByte())` |
| `RETURN_FALSE` | `mov w0, #0`<br>`ret` | `byteArrayOf(0x00, 0x00, 0x80.toByte(), 0x52, 0xc0.toByte(), 0x03, 0x5f, 0xd6.toByte())` |
| `NOP` | `nop` | `byteArrayOf(0x1f, 0x20, 0x03, 0xd5.toByte())` |

### Disassembling & Calculating Offsets with APK Lab
Use `apk-lab asm` to calculate branch targets and convert instructions directly to Kotlin byte arrays:

```bash
# Assemble ARM64 instruction into Kotlin byteArrayOf
uv run --project tools/apk-lab apk-lab asm --arch arm64 "mov w0, #1" "ret"

# Disassemble byte hex string to ARM64 instructions
uv run --project tools/apk-lab apk-lab asm --arch arm64 --disasm "20008052c0035fd6"
```

### Binary Patching Pattern: Bounds, Match & Idempotency
Always follow this exact structure to ensure idempotency and safety:

```kotlin
val arm64So = get("lib/arm64-v8a/libsecurity.so")
if (arm64So.exists()) {
    val bytes = arm64So.readBytes()
    val offset = 0x48dd8 // Verified target offset

    val expected = byteArrayOf(
        0xff.toByte(), 0x43, 0x02, 0xd1.toByte(), // sub sp, sp, #0x90
        0xf7.toByte(), 0x2b, 0x00, 0xf9.toByte()  // str x23, [sp, #0x50]
    )
    val replacement = byteArrayOf(
        0x20.toByte(), 0x00.toByte(), 0x80.toByte(), 0x52.toByte(), // mov w0, #1
        0xc0.toByte(), 0x03.toByte(), 0x5f.toByte(), 0xd6.toByte()  // ret
    )

    // 1. Bounds check
    if (bytes.size <= offset + replacement.size) {
        throw PatchException("libsecurity.so is too small for patch offset 0x${offset.toString(16)}")
    }

    // 2. Idempotency check: skip if already patched
    val alreadyPatched = replacement.indices.all { bytes[offset + it] == replacement[it] }
    if (!alreadyPatched) {
        // 3. Expected bytes match check
        val matches = expected.indices.all { bytes[offset + it] == expected[it] }
        if (!matches) {
            throw PatchException("libsecurity.so byte sequence mismatch at offset 0x${offset.toString(16)}")
        }

        // 4. In-place write
        System.arraycopy(replacement, 0, bytes, offset, replacement.size)
        arm64So.writeBytes(bytes)
    }
}
```

---

## 3. Android XML & Manifest DOM Manipulation

XML resources (such as `res/xml/*.xml`, `res/values/*.xml`, and `AndroidManifest.xml`) are modified inside a `resourcePatch` or the `resources { ... }` block of a `compositeFeaturePatch`.

### XML Namespaces
Android resource documents use standard XML namespaces:
- Android: `http://schemas.android.com/apk/res/android`
- App/Custom: `http://schemas.android.com/apk/res-auto`

### Accessing Documents
Use `document(path).use { doc -> ... }` to ensure safe parsing and streaming:

```kotlin
resources {
    document("AndroidManifest.xml").use { doc ->
        val manifest = doc.documentElement
        val application = manifest.getElementsByTagName("application").item(0) as? org.w3c.dom.Element
            ?: throw PatchException("Missing <application> tag in AndroidManifest.xml")

        // 1. Modify attributes
        application.setAttributeNS(
            "http://schemas.android.com/apk/res/android",
            "android:debuggable",
            "true"
        )

        // 2. Remove unwanted receiver/service nodes
        val receivers = application.getElementsByTagName("receiver")
        for (i in receivers.length - 1 downTo 0) {
            val receiver = receivers.item(i) as org.w3c.dom.Element
            val name = receiver.getAttributeNS("http://schemas.android.com/apk/res/android", "name")
            if (name.contains("CrashReportReceiver")) {
                application.removeChild(receiver)
            }
        }
    }
}
```

### Hard Invariants for Resource Patching
1. **Always Use `.use { ... }`**: Ensures XML output is flushed and underlying streams are closed.
2. **Explicit Namespace URI**: Never use unqualified attribute names (e.g. `setAttribute("android:name", ...)`); always use `setAttributeNS("http://schemas.android.com/apk/res/android", "android:name", ...)`.
3. **Fail-Fast on Missing Required Nodes**: If an XML document or an anchor node is expected, throw `PatchException` rather than silently doing nothing.
