package app.aidan.patches.sezzle.security

import app.morphe.patcher.patch.PatchException
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test

class SuppressUpdatesAndIntegrityPatchTest {
    private val sagaPrefixOne = byteArrayOf(0x40, 0x01, 0x07, 0x89.toByte())
    private val sagaPrefixTwo = byteArrayOf(0x34, 0x01, 0x00, 0x3b)
    private val replacement = byteArrayOf(0x93.toByte(), 0x00, 0x76, 0x00)
    private val unknownBytes = byteArrayOf(0xde.toByte(), 0xad.toByte(), 0xbe.toByte(), 0xef.toByte())

    private fun sagaFamily(vararg offsets: Int) = UpdateBytecodeFamily(
        "shouldForceUpdateAppSaga",
        offsets.toList(),
        listOf(sagaPrefixOne, sagaPrefixTwo)
    )

    @Test
    fun `patches both recognized saga prefixes`() {
        val data = sagaPrefixOne + sagaPrefixTwo

        patchUpdateFamilies(data, listOf(sagaFamily(0, 4)), replacement)

        assertArrayEquals(replacement, data.copyOfRange(0, 4))
        assertArrayEquals(replacement, data.copyOfRange(4, 8))
    }

    @Test
    fun `accepts an already patched replacement`() {
        val data = replacement + replacement

        patchUpdateFamilies(data, listOf(sagaFamily(0, 4)), replacement)

        assertArrayEquals(replacement, data.copyOfRange(0, 4))
        assertArrayEquals(replacement, data.copyOfRange(4, 8))
    }

    @Test
    fun `classifies expected prefix replacement and unknown bytecode`() {
        val data = sagaPrefixOne + replacement + unknownBytes

        assertEquals(
            UpdateBytecodeMatch.EXPECTED_PREFIX,
            classifyUpdateBytecode(data, 0, listOf(sagaPrefixOne), replacement)
        )
        assertEquals(
            UpdateBytecodeMatch.ALREADY_PATCHED,
            classifyUpdateBytecode(data, 4, listOf(sagaPrefixOne), replacement)
        )
        assertEquals(
            UpdateBytecodeMatch.UNKNOWN,
            classifyUpdateBytecode(data, 8, listOf(sagaPrefixOne), replacement)
        )
    }

    @Test
    fun `unknown prefix reports family offset and found bytes`() {
        val data = unknownBytes.copyOf()

        val error = assertThrows(PatchException::class.java) {
            patchUpdateFamilies(data, listOf(sagaFamily(0)), replacement)
        }

        val message = error.message.orEmpty()
        assertTrue(message.contains("shouldForceUpdateAppSaga"))
        assertTrue(message.contains("offset 0"))
        assertTrue(message.contains("de ad be ef"))
        assertArrayEquals(unknownBytes, data)
    }

    @Test
    fun `unknown occurrence leaves every family unmodified`() {
        val data = sagaPrefixOne + ByteArray(4) + unknownBytes
        val original = data.copyOf()

        val error = assertThrows(PatchException::class.java) {
            patchUpdateFamilies(
                data,
                listOf(
                    sagaFamily(0),
                    UpdateBytecodeFamily(
                        "shouldForceUpdateAppSagaWatcher",
                        listOf(8),
                        listOf(sagaPrefixOne)
                    )
                ),
                replacement
            )
        }

        assertTrue(error.message.orEmpty().contains("shouldForceUpdateAppSagaWatcher"))
        assertArrayEquals(original, data)
    }

    @Test
    fun `out of range offsets are diagnosed instead of failing as index errors`() {
        val data = sagaPrefixOne.copyOf()

        val error = assertThrows(PatchException::class.java) {
            patchUpdateFamilies(data, listOf(sagaFamily(0, 100)), replacement)
        }

        assertTrue(error.message.orEmpty().contains("offset 100"))
        assertTrue(error.message.orEmpty().contains("<out of range>"))
        assertArrayEquals(sagaPrefixOne, data)
    }
}
