package app.aidan.patches.canvas.tracking

import app.aidan.patches.canvas.shared.COMPATIBILITY_CANVAS
import app.morphe.patcher.patch.PatchException
import app.morphe.patcher.patch.rawResourcePatch
import java.nio.ByteBuffer
import java.nio.ByteOrder

private const val ELF64_HEADER_SIZE = 64
private const val ELF64_PROGRAM_HEADER_TYPE_OFFSET = 0
private const val ELF64_PROGRAM_HEADER_VIRTUAL_ADDRESS_OFFSET = 16
private const val ELF64_PROGRAM_HEADER_MEMORY_SIZE_OFFSET = 40
private const val PT_GNU_RELRO = 0x6474e552
private const val PT_NULL = 0
private const val PAGE_SIZE_16_KB = 16_384L

@Suppress("unused")
val fix16KbPageCompatibilityPatch = rawResourcePatch(
    name = "Fix 16 KB Page Compatibility",
    description = "Repairs Canvas's native-library packaging and disables incompatible GNU RELRO segments so Android 15+ does not show its 16 KB compatibility warning.",
    default = true
) {
    compatibleWith(COMPATIBILITY_CANVAS)

    execute {
        fun patchLibrary(path: String) {
            val library = get(path)
            if (!library.exists()) {
                throw PatchException("Missing native library: $path")
            }
            library.writeBytes(removeIncompatibleRelro(path, library.readBytes()))
        }

        ensure16KbPageAlignment()
        patchLibrary("lib/arm64-v8a/libandroidx.graphics.path.so")
        patchLibrary("lib/arm64-v8a/libdatastore_shared_counter.so")
        patchLibrary("lib/arm64-v8a/libpspdfkit.so")
    }
}

private fun removeIncompatibleRelro(path: String, bytes: ByteArray): ByteArray {
    if (bytes.size < ELF64_HEADER_SIZE || !bytes.copyOfRange(0, 4).contentEquals(byteArrayOf(0x7f, 0x45, 0x4c, 0x46))) {
        throw PatchException("$path is not an ELF file")
    }
    if (bytes[4] != 2.toByte() || bytes[5] != 1.toByte()) {
        throw PatchException("$path is not a little-endian ELF64 library")
    }

    val elf = ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN)
    val programHeaderOffset = elf.getLong(32)
    val programHeaderSize = elf.getShort(54).toInt() and 0xffff
    val programHeaderCount = elf.getShort(56).toInt() and 0xffff
    val programHeadersEnd = programHeaderOffset + programHeaderSize.toLong() * programHeaderCount
    if (
        programHeaderSize < ELF64_PROGRAM_HEADER_MEMORY_SIZE_OFFSET + Long.SIZE_BYTES ||
        programHeaderOffset < ELF64_HEADER_SIZE ||
        programHeadersEnd > bytes.size ||
        programHeadersEnd < programHeaderOffset
    ) {
        throw PatchException("$path has invalid ELF program headers")
    }

    var removedSegments = 0
    repeat(programHeaderCount) { index ->
        val offset = programHeaderOffset.toInt() + index * programHeaderSize
        if (elf.getInt(offset) != PT_GNU_RELRO) return@repeat

        val virtualAddress = elf.getLong(offset + ELF64_PROGRAM_HEADER_VIRTUAL_ADDRESS_OFFSET)
        val memorySize = elf.getLong(offset + ELF64_PROGRAM_HEADER_MEMORY_SIZE_OFFSET)
        if ((virtualAddress + memorySize) % PAGE_SIZE_16_KB != 0L) {
            elf.putInt(offset + ELF64_PROGRAM_HEADER_TYPE_OFFSET, PT_NULL)
            removedSegments++
        }
    }

    if (removedSegments != 1) {
        throw PatchException("Expected one 16 KB-incompatible GNU RELRO segment in $path, found $removedSegments")
    }
    return bytes
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
