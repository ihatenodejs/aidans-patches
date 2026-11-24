package app.aidan.extension.adobescan;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertNotNull;
import static org.junit.Assert.assertTrue;

import java.io.File;
import java.io.FileOutputStream;
import java.nio.charset.StandardCharsets;
import java.util.Arrays;
import java.util.Collections;
import org.junit.After;
import org.junit.Before;
import org.junit.Test;

public class PdfOutputTransactionTest {

    private File tempDir;
    private File stagingDir;
    private File inputA;
    private File inputB;

    private static final byte[] BYTES_A = "%PDF-1.7\n1 0 obj Doc A\nendobj\n%%EOF".getBytes(StandardCharsets.UTF_8);
    private static final byte[] BYTES_B = "%PDF-1.7\n1 0 obj Doc B\nendobj\n%%EOF".getBytes(StandardCharsets.UTF_8);
    private static final byte[] COMBINED_BYTES = "%PDF-1.7\n1 0 obj Combined\nendobj\n%%EOF".getBytes(StandardCharsets.UTF_8);

    /** Fake database lookup; recovery must never call registration create APIs. */
    private static class MockRegistrationAdapter extends ScanFileAdapter {
        long registeredId;
        boolean throwOnLookup;
        int lookupCalls;
        int createCalls;

        @Override
        public long findRegisteredDocumentId(final String persistentUniqueId) throws Exception {
            lookupCalls++;
            if (throwOnLookup) {
                throw new IllegalStateException("Simulated database failure");
            }
            return registeredId;
        }

        @Override
        public Object createLocalDocument(
                final File pdfFile,
                final String displayName,
                final int pageCount,
                final String persistentUniqueId,
                final String registrationFileName) {
            createCalls++;
            return null;
        }
    }

    private PdfOutputTransaction beginCombine(final long opId, final String displayName) throws Exception {
        return PdfOutputTransaction.begin(
                stagingDir,
                opId,
                NativePdfEngine.COMBINE,
                Arrays.asList(inputA, inputB),
                Arrays.asList(1L, 2L),
                displayName);
    }

    private void writeStage(final PdfOutputTransaction tx) throws Exception {
        try (FileOutputStream fos = new FileOutputStream(tx.getStageFile())) {
            fos.write(COMBINED_BYTES);
            fos.flush();
        }
        assertTrue(tx.validateStageOutput());
    }

    private File publish(
            final PdfOutputTransaction tx,
            final String docId,
            final byte[] bytes,
            final String registrationFileName) throws Exception {
        final File dest = new File(tempDir, docId + ".pdf");
        tx.markPublishing(dest.getAbsolutePath(), docId, 2, registrationFileName);
        try (FileOutputStream fos = new FileOutputStream(dest)) {
            fos.write(bytes);
            fos.flush();
        }
        return dest;
    }

    private File journal(final long opId) {
        return new File(stagingDir, opId + ".output-journal");
    }

    @Before
    public void setUp() throws Exception {
        tempDir = File.createTempFile("pdf_out_tx_test_", "");
        tempDir.delete();
        tempDir.mkdirs();

        stagingDir = new File(tempDir, "staging");
        stagingDir.mkdirs();

        inputA = new File(tempDir, "input_a.pdf");
        try (FileOutputStream fos = new FileOutputStream(inputA)) {
            fos.write(BYTES_A);
            fos.flush();
        }

        inputB = new File(tempDir, "input_b.pdf");
        try (FileOutputStream fos = new FileOutputStream(inputB)) {
            fos.write(BYTES_B);
            fos.flush();
        }
    }

    @After
    public void tearDown() {
        deleteRecursively(tempDir);
    }

    private void deleteRecursively(final File file) {
        if (file == null || !file.exists()) return;
        if (file.isDirectory()) {
            final File[] children = file.listFiles();
            if (children != null) {
                for (final File child : children) {
                    deleteRecursively(child);
                }
            }
        }
        file.delete();
    }

    @Test
    public void testSuccessfulCombineTransactionLifecycle() throws Exception {
        long opId = 1001L;
        PdfOutputTransaction tx = PdfOutputTransaction.begin(
                stagingDir,
                opId,
                NativePdfEngine.COMBINE,
                Arrays.asList(inputA, inputB),
                Arrays.asList(1L, 2L),
                "Combined Scan");

        assertNotNull(tx.getStageFile());
        assertEquals(PdfOutputTransaction.PHASE_PREPARING, tx.getRecord().phase);

        // Simulate worker writing stage file
        try (FileOutputStream fos = new FileOutputStream(tx.getStageFile())) {
            fos.write(COMBINED_BYTES);
            fos.flush();
        }

        assertTrue(tx.validateStageOutput());
        assertEquals(PdfOutputTransaction.PHASE_VALIDATED, tx.getRecord().phase);
        assertEquals(COMBINED_BYTES.length, tx.getRecord().outputSize);
        assertNotNull(tx.getRecord().outputSha256);

        assertTrue(tx.revalidateSources());

        File destinationFile = new File(tempDir, "combined_final.pdf");
        tx.markPublishing(destinationFile.getAbsolutePath(), "doc-123", 3, "Combined Scan");
        assertEquals(PdfOutputTransaction.PHASE_PUBLISHING, tx.getRecord().phase);
        assertEquals("doc-123", tx.getRecord().registrationId);
        assertEquals("Combined Scan", tx.getRecord().registrationFileName);
        assertEquals(3, tx.getRecord().pageCount);

        // Write destination file
        try (FileOutputStream fos = new FileOutputStream(destinationFile)) {
            fos.write(COMBINED_BYTES);
            fos.flush();
        }

        tx.markPublished();
        assertFalse(tx.getStageFile().exists());

        // Verify inputs remained unchanged
        assertEquals(BYTES_A.length, inputA.length());
        assertEquals(BYTES_B.length, inputB.length());
    }

    @Test
    public void testRevalidateSourcesFailsIfInputModified() throws Exception {
        long opId = 1002L;
        PdfOutputTransaction tx = PdfOutputTransaction.begin(
                stagingDir,
                opId,
                NativePdfEngine.COMBINE,
                Arrays.asList(inputA, inputB),
                Arrays.asList(1L, 2L),
                "Combined Scan");

        // Write stage file
        try (FileOutputStream fos = new FileOutputStream(tx.getStageFile())) {
            fos.write(COMBINED_BYTES);
            fos.flush();
        }
        assertTrue(tx.validateStageOutput());

        // Modify inputA
        try (FileOutputStream fos = new FileOutputStream(inputA)) {
            fos.write("Modified content".getBytes(StandardCharsets.UTF_8));
            fos.flush();
        }

        assertFalse(tx.revalidateSources());
        tx.abort();
        assertFalse(tx.getStageFile().exists());
    }

    @Test
    public void testRecoveryOfInterruptedPreparingCleansUp() throws Exception {
        long opId = 1003L;
        PdfOutputTransaction tx = PdfOutputTransaction.begin(
                stagingDir,
                opId,
                NativePdfEngine.COMBINE,
                Collections.singletonList(inputA),
                Collections.singletonList(1L),
                "Test Scan");

        // Staging file exists
        try (FileOutputStream fos = new FileOutputStream(tx.getStageFile())) {
            fos.write(COMBINED_BYTES);
            fos.flush();
        }
        assertTrue(tx.getStageFile().exists());

        // Simulate crash / restart: recoverAll
        PdfOutputTransaction.recoverAll(stagingDir, new MockRegistrationAdapter());

        // Stage file and journal should be removed, inputs untouched
        assertFalse(tx.getStageFile().exists());
        assertFalse(journal(opId).exists());
        assertTrue(inputA.exists());
        assertEquals(BYTES_A.length, inputA.length());
    }

    @Test
    public void testAbortCleansUpStaging() throws Exception {
        long opId = 1004L;
        PdfOutputTransaction tx = PdfOutputTransaction.begin(
                stagingDir,
                opId,
                NativePdfEngine.EXPORT_DOCX,
                Collections.singletonList(inputA),
                Collections.singletonList(1L),
                "Exported Doc");

        try (FileOutputStream fos = new FileOutputStream(tx.getStageFile())) {
            fos.write("DOCX STUFF".getBytes(StandardCharsets.UTF_8));
            fos.flush();
        }
        assertTrue(tx.getStageFile().exists());

        tx.abort();
        assertFalse(tx.getStageFile().exists());
        assertTrue(inputA.exists());
    }

    @Test
    public void testRecoveryPreservesRegisteredCombineOutput() throws Exception {
        final PdfOutputTransaction tx = beginCombine(1101L, "Registered Combine");
        writeStage(tx);
        final File dest = publish(tx, "doc-registered", COMBINED_BYTES, "Registered Combine");
        final String originalHash = PdfOutputTransaction.sha256(dest);

        final MockRegistrationAdapter adapter = new MockRegistrationAdapter();
        adapter.registeredId = 77L;
        PdfOutputTransaction.recoverAll(stagingDir, adapter);

        assertTrue("Registered output must be preserved", dest.exists());
        assertEquals("Registered document bytes must not change",
                originalHash, PdfOutputTransaction.sha256(dest));
        assertFalse("Journal must be retired once registration is proven",
                journal(1101L).exists());
        assertFalse("Our stage artifact must be cleaned up", tx.getStageFile().exists());
        assertEquals(1, adapter.lookupCalls);
        assertEquals("Recovery must not issue a registration create", 0, adapter.createCalls);
    }

    @Test
    public void testRecoveryRetainsUnregisteredCompleteOutput() throws Exception {
        final PdfOutputTransaction tx = beginCombine(1103L, "Unregistered Combine");
        writeStage(tx);
        final File dest = publish(tx, "doc-unregistered", COMBINED_BYTES, "Unregistered Combine");
        final String originalHash = PdfOutputTransaction.sha256(dest);

        final MockRegistrationAdapter adapter = new MockRegistrationAdapter();
        PdfOutputTransaction.recoverAll(stagingDir, adapter);

        assertTrue("Unregistered output must not be deleted", dest.exists());
        assertEquals("Unregistered output must not be modified",
                originalHash, PdfOutputTransaction.sha256(dest));
        assertTrue("Complete stage must be retained as evidence", tx.getStageFile().exists());
        assertTrue("Journal must be retained for manual recovery", journal(1103L).exists());
        assertEquals("Recovery must not issue a registration create", 0, adapter.createCalls);
    }

    @Test
    public void testRecoveryRetainsUnregisteredPartialOutput() throws Exception {
        final PdfOutputTransaction tx = beginCombine(1105L, "Partial Output");
        writeStage(tx);
        final byte[] partialBytes = "user modified or partial output".getBytes(StandardCharsets.UTF_8);
        final File dest = publish(tx, "doc-partial", partialBytes, "Partial Output");
        final String originalHash = PdfOutputTransaction.sha256(dest);

        final MockRegistrationAdapter adapter = new MockRegistrationAdapter();
        PdfOutputTransaction.recoverAll(stagingDir, adapter);

        assertTrue(dest.exists());
        assertEquals("Partial or ambiguous output must not be overwritten",
                originalHash, PdfOutputTransaction.sha256(dest));
        assertTrue(tx.getStageFile().exists());
        assertTrue(journal(1105L).exists());
    }

    @Test
    public void testRecoveryRetainsMissingOutputAndStage() throws Exception {
        final PdfOutputTransaction tx = beginCombine(1106L, "Missing Output");
        writeStage(tx);
        final File dest = new File(tempDir, "doc-missing.pdf");
        tx.markPublishing(dest.getAbsolutePath(), "doc-missing", 2, "Missing Output");
        assertFalse(dest.exists());

        final MockRegistrationAdapter adapter = new MockRegistrationAdapter();
        PdfOutputTransaction.recoverAll(stagingDir, adapter);

        assertFalse("Recovery must not republish an output", dest.exists());
        assertTrue("Complete stage must be retained", tx.getStageFile().exists());
        assertTrue("Journal must be retained", journal(1106L).exists());
    }

    @Test
    public void testRecoveryRetainsOutputWhenRegistrationLookupFails() throws Exception {
        final PdfOutputTransaction tx = beginCombine(1107L, "Lookup Failure");
        writeStage(tx);
        final File dest = publish(tx, "doc-lookup-failure", COMBINED_BYTES, "Lookup Failure");

        final MockRegistrationAdapter adapter = new MockRegistrationAdapter();
        adapter.throwOnLookup = true;
        PdfOutputTransaction.recoverAll(stagingDir, adapter);

        assertTrue("Ambiguous output must be preserved", dest.exists());
        assertTrue("Journal must be retained on unknown registration", journal(1107L).exists());
        assertTrue("Stage must be retained on unknown registration", tx.getStageFile().exists());
    }

    @Test
    public void testRecoveryWithoutAdapterRetainsEvidence() throws Exception {
        final PdfOutputTransaction tx = beginCombine(1108L, "No Adapter");
        writeStage(tx);
        final File dest = publish(tx, "doc-no-adapter", COMBINED_BYTES, "No Adapter");

        PdfOutputTransaction.recoverAll(stagingDir, null);

        assertTrue(dest.exists());
        assertTrue(tx.getStageFile().exists());
        assertTrue(journal(1108L).exists());
    }

    @Test
    public void testRecoveryRetainsLegacyCombineJournalWithoutIdentity() throws Exception {
        final PdfOutputTransaction tx = beginCombine(1110L, "Legacy Journal");
        writeStage(tx);
        final File dest = publish(tx, "doc-legacy", COMBINED_BYTES, "Legacy Journal");

        // Simulate a journal written by an older extension version.
        tx.getRecord().registrationId = null;
        try (java.io.FileWriter fw = new java.io.FileWriter(journal(1110L))) {
            fw.write(tx.getRecord().toJson());
        }

        final MockRegistrationAdapter adapter = new MockRegistrationAdapter();
        PdfOutputTransaction.recoverAll(stagingDir, adapter);

        assertTrue("Legacy output must not be deleted", dest.exists());
        assertTrue("Legacy stage must not be deleted", tx.getStageFile().exists());
        assertTrue("Legacy journal must be retained as evidence", journal(1110L).exists());
    }

    @Test
    public void testRecoveryRetainsMalformedJournal() throws Exception {
        final File malformed = journal(1111L);
        try (java.io.FileWriter fw = new java.io.FileWriter(malformed)) {
            fw.write("{\n  \"operationId\": not-a-number\n}");
        }

        PdfOutputTransaction.recoverAll(stagingDir, new MockRegistrationAdapter());

        assertTrue("Malformed journal must be retained for diagnosis", malformed.exists());
    }

    @Test
    public void testRecoveryIsIdempotentAndNeverCreatesOrMovesFiles() throws Exception {
        final PdfOutputTransaction tx = beginCombine(1109L, "Idempotent Recovery");
        writeStage(tx);
        final File dest = publish(tx, "doc-idempotent", COMBINED_BYTES, "Idempotent Recovery");
        final String originalHash = PdfOutputTransaction.sha256(dest);
        final String stageHash = PdfOutputTransaction.sha256(tx.getStageFile());

        final MockRegistrationAdapter adapter = new MockRegistrationAdapter();
        PdfOutputTransaction.recoverAll(stagingDir, adapter);
        PdfOutputTransaction.recoverAll(stagingDir, adapter);
        PdfOutputTransaction.recoverAll(stagingDir, adapter);

        assertTrue(dest.exists());
        assertEquals(originalHash, PdfOutputTransaction.sha256(dest));
        assertTrue(tx.getStageFile().exists());
        assertEquals("Stage evidence must be unchanged", stageHash, PdfOutputTransaction.sha256(tx.getStageFile()));
        assertTrue(journal(1109L).exists());
        assertEquals("Each run performs one read-only lookup", 3, adapter.lookupCalls);
    }

    @Test
    public void testRegisteredRecoveryIsIdempotent() throws Exception {
        final PdfOutputTransaction tx = beginCombine(1114L, "Registered Idempotent");
        writeStage(tx);
        final File dest = publish(tx, "doc-registered-idempotent", COMBINED_BYTES, "Registered Idempotent");

        final MockRegistrationAdapter adapter = new MockRegistrationAdapter();
        adapter.registeredId = 42L;
        PdfOutputTransaction.recoverAll(stagingDir, adapter);
        PdfOutputTransaction.recoverAll(stagingDir, adapter);

        assertTrue(dest.exists());
        assertFalse(journal(1114L).exists());
        assertFalse(tx.getStageFile().exists());
    }

    @Test
    public void testPublishedCombineJournalRetainsEvidenceWhenUnregistered() throws Exception {
        final PdfOutputTransaction tx = beginCombine(1115L, "Published Unregistered");
        writeStage(tx);
        final File dest = publish(tx, "doc-published-unregistered", COMBINED_BYTES, "Published Unregistered");
        final String originalHash = PdfOutputTransaction.sha256(dest);

        // Simulate a legacy/crashed journal that reached PUBLISHED before the
        // library registration was durable.
        tx.getRecord().phase = PdfOutputTransaction.PHASE_PUBLISHED;
        try (java.io.FileWriter fw = new java.io.FileWriter(journal(1115L))) {
            fw.write(tx.getRecord().toJson());
        }

        final MockRegistrationAdapter adapter = new MockRegistrationAdapter();
        PdfOutputTransaction.recoverAll(stagingDir, adapter);

        assertTrue("Unregistered PUBLISHED output must not be deleted", dest.exists());
        assertEquals("Unregistered PUBLISHED output must not be modified",
                originalHash, PdfOutputTransaction.sha256(dest));
        assertTrue("Stage may be the only complete copy; it must be retained",
                tx.getStageFile().exists());
        assertTrue("Journal must be retained for manual recovery", journal(1115L).exists());
        assertEquals("Recovery must not issue a registration create", 0, adapter.createCalls);
    }

    @Test
    public void testPublishedCombineJournalRetiresWhenRegistered() throws Exception {
        final PdfOutputTransaction tx = beginCombine(1116L, "Published Registered");
        writeStage(tx);
        final File dest = publish(tx, "doc-published-registered", COMBINED_BYTES, "Published Registered");
        final String originalHash = PdfOutputTransaction.sha256(dest);

        tx.getRecord().phase = PdfOutputTransaction.PHASE_PUBLISHED;
        try (java.io.FileWriter fw = new java.io.FileWriter(journal(1116L))) {
            fw.write(tx.getRecord().toJson());
        }

        final MockRegistrationAdapter adapter = new MockRegistrationAdapter();
        adapter.registeredId = 55L;
        PdfOutputTransaction.recoverAll(stagingDir, adapter);

        assertTrue("Registered document must be preserved", dest.exists());
        assertEquals(originalHash, PdfOutputTransaction.sha256(dest));
        assertFalse("Journal bookkeeping must be retired", journal(1116L).exists());
        assertFalse("Stage artifact must be cleaned up", tx.getStageFile().exists());
        assertEquals(0, adapter.createCalls);
    }

    @Test
    public void testExportPublishingRecoveryBehaviorUnchanged() throws Exception {
        final PdfOutputTransaction tx = PdfOutputTransaction.begin(
                stagingDir,
                1112L,
                NativePdfEngine.EXPORT_DOCX,
                Collections.singletonList(inputA),
                Collections.singletonList(1L),
                "Exported Doc");
        try (FileOutputStream fos = new FileOutputStream(tx.getStageFile())) {
            fos.write("DOCX STUFF".getBytes(StandardCharsets.UTF_8));
            fos.flush();
        }
        assertTrue(tx.validateStageOutput());
        final File export = new File(tempDir, "export.docx");
        tx.markPublishing(export.getAbsolutePath(), "1112");
        try (FileOutputStream fos = new FileOutputStream(export)) {
            fos.write("DOCX STUFF".getBytes(StandardCharsets.UTF_8));
            fos.flush();
        }

        PdfOutputTransaction.recoverAll(stagingDir, new MockRegistrationAdapter());

        assertTrue("Export output is cache-scoped and left untouched", export.exists());
        assertFalse(journal(1112L).exists());
        assertFalse(tx.getStageFile().exists());
    }
}
