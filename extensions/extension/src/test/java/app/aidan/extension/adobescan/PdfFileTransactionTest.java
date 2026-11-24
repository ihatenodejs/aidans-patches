package app.aidan.extension.adobescan;

import static org.junit.Assert.assertArrayEquals;
import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertNotNull;
import static org.junit.Assert.assertTrue;
import static org.junit.Assert.fail;

import java.io.File;
import java.io.FileOutputStream;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import org.junit.After;
import org.junit.Before;
import org.junit.Test;

public class PdfFileTransactionTest {

    private File tempDir;
    private File sourceFile;
    private static final byte[] ORIGINAL_BYTES = "%PDF-1.7\n1 0 obj\n<< /Type /Catalog >>\nendobj\ntrailer\n<<>>\n%%EOF".getBytes(StandardCharsets.UTF_8);
    private static final byte[] TRANSFORMED_BYTES = "%PDF-1.7\n1 0 obj\n<< /Type /Catalog /Encrypted true >>\nendobj\ntrailer\n<<>>\n%%EOF".getBytes(StandardCharsets.UTF_8);

    private static class MockScanFileAdapter extends ScanFileAdapter {
        long updatedDocId = -1;
        int updatedNewStatus = -1;
        long updatedNewSize = -1;
        int updatedOldStatus = -1;
        long updatedOldSize = -1;
        boolean updateCalled = false;
        boolean shouldThrowOnUpdate = false;

        @Override
        public void updateMetadata(
                final long databaseId,
                final int newProtectionStatus,
                final long newFileSize,
                final int expectedOldStatus,
                final long expectedOldSize) throws Exception {
            if (shouldThrowOnUpdate) {
                throw new RuntimeException("Simulated DB transaction failure");
            }
            this.updatedDocId = databaseId;
            this.updatedNewStatus = newProtectionStatus;
            this.updatedNewSize = newFileSize;
            this.updatedOldStatus = expectedOldStatus;
            this.updatedOldSize = expectedOldSize;
            this.updateCalled = true;
        }
    }

    @Before
    public void setUp() throws Exception {
        tempDir = File.createTempFile("pdf_tx_test_", "");
        tempDir.delete();
        tempDir.mkdirs();

        sourceFile = new File(tempDir, "test_document.pdf");
        try (FileOutputStream fos = new FileOutputStream(sourceFile)) {
            fos.write(ORIGINAL_BYTES);
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
    public void testBeginCreatesSnapshotAndPreparingJournal() throws Exception {
        final long opId = 1001L;
        final long docId = 42L;
        final MockScanFileAdapter adapter = new MockScanFileAdapter();

        final PdfFileTransaction tx = PdfFileTransaction.begin(
                tempDir,
                opId,
                docId,
                NativePdfEngine.PROTECT,
                sourceFile,
                1, // oldStatus (unprotected)
                ORIGINAL_BYTES.length,
                2  // newStatus (protected)
        );

        assertNotNull(tx);
        final File snapshot = tx.getSnapshotFile();
        assertTrue("Snapshot file must exist", snapshot.exists());
        assertEquals("Snapshot length must match original", ORIGINAL_BYTES.length, snapshot.length());
        assertEquals("Snapshot hash must match source", PdfFileTransaction.sha256(sourceFile), PdfFileTransaction.sha256(snapshot));

        final File journal = new File(tempDir, opId + ".journal");
        assertTrue("Journal file must exist", journal.exists());
        assertEquals(PdfFileTransaction.PHASE_PREPARING, tx.getRecord().phase);
    }

    @Test
    public void testCommitSuccessfullyOverwritesSourceAndCleansUp() throws Exception {
        final long opId = 1002L;
        final long docId = 43L;
        final MockScanFileAdapter adapter = new MockScanFileAdapter();

        final PdfFileTransaction tx = PdfFileTransaction.begin(
                tempDir,
                opId,
                docId,
                NativePdfEngine.COMPRESS,
                sourceFile,
                1,
                ORIGINAL_BYTES.length,
                1
        );

        // Simulate transformed output written to stage file
        final File stageFile = tx.getStageFile();
        try (FileOutputStream fos = new FileOutputStream(stageFile)) {
            fos.write(TRANSFORMED_BYTES);
            fos.flush();
        }

        final String outHash = PdfFileTransaction.sha256(stageFile);
        tx.markValidated(TRANSFORMED_BYTES.length, outHash);
        assertEquals(PdfFileTransaction.PHASE_VALIDATED, tx.getRecord().phase);

        tx.commit(adapter);

        // Verify source file has transformed bytes
        assertEquals(TRANSFORMED_BYTES.length, sourceFile.length());
        assertEquals(outHash, PdfFileTransaction.sha256(sourceFile));

        // Verify adapter was called with correct metadata
        assertTrue(adapter.updateCalled);
        assertEquals(docId, adapter.updatedDocId);
        assertEquals(TRANSFORMED_BYTES.length, adapter.updatedNewSize);
        assertEquals(ORIGINAL_BYTES.length, adapter.updatedOldSize);

        // Verify stage, snapshot, and journal files are cleaned up
        assertFalse("Stage file should be deleted", stageFile.exists());
        assertFalse("Snapshot file should be deleted", tx.getSnapshotFile().exists());
        assertFalse("Journal file should be deleted", new File(tempDir, opId + ".journal").exists());
    }

    @Test
    public void testCommitAbortsIfSourceFileModifiedExternally() throws Exception {
        final long opId = 1003L;
        final long docId = 44L;
        final MockScanFileAdapter adapter = new MockScanFileAdapter();

        final PdfFileTransaction tx = PdfFileTransaction.begin(
                tempDir,
                opId,
                docId,
                NativePdfEngine.PROTECT,
                sourceFile,
                1,
                ORIGINAL_BYTES.length,
                2
        );

        final File stageFile = tx.getStageFile();
        try (FileOutputStream fos = new FileOutputStream(stageFile)) {
            fos.write(TRANSFORMED_BYTES);
            fos.flush();
        }
        tx.markValidated(TRANSFORMED_BYTES.length, PdfFileTransaction.sha256(stageFile));

        // Tamper with source file before commit
        try (FileOutputStream fos = new FileOutputStream(sourceFile)) {
            fos.write("corrupted content".getBytes(StandardCharsets.UTF_8));
            fos.flush();
        }

        try {
            tx.commit(adapter);
            fail("Commit should have thrown IOException due to modified source");
        } catch (final IOException expected) {
            // Expected
        }

        assertFalse("Adapter should not have been called", adapter.updateCalled);
    }

    @Test
    public void testRollbackRestoresSnapshotOverSourceAfterFileCommitted() throws Exception {
        final long opId = 1004L;
        final long docId = 45L;
        final MockScanFileAdapter adapter = new MockScanFileAdapter();
        adapter.shouldThrowOnUpdate = true; // DB update failure simulation

        final PdfFileTransaction tx = PdfFileTransaction.begin(
                tempDir,
                opId,
                docId,
                NativePdfEngine.PROTECT,
                sourceFile,
                1,
                ORIGINAL_BYTES.length,
                2
        );

        final File stageFile = tx.getStageFile();
        try (FileOutputStream fos = new FileOutputStream(stageFile)) {
            fos.write(TRANSFORMED_BYTES);
            fos.flush();
        }
        tx.markValidated(TRANSFORMED_BYTES.length, PdfFileTransaction.sha256(stageFile));

        try {
            tx.commit(adapter);
            fail("Commit should fail on DB update");
        } catch (final Exception expected) {
            // DB failed; now rollback
            adapter.shouldThrowOnUpdate = false;
            tx.rollback(adapter);
        }

        // Verify source file was restored from snapshot to original content
        assertEquals(ORIGINAL_BYTES.length, sourceFile.length());
        assertArrayEquals("Bytes must equal the original snapshot",
                ORIGINAL_BYTES, java.nio.file.Files.readAllBytes(sourceFile.toPath()));
        assertEquals(PdfFileTransaction.sha256(sourceFile), tx.getRecord().sourceSha256);
        assertTrue("Old metadata must be restored", adapter.updateCalled);
        assertEquals(1, adapter.updatedNewStatus);
        assertEquals(ORIGINAL_BYTES.length, adapter.updatedNewSize);
        assertEquals(2, adapter.updatedOldStatus);
        assertEquals(TRANSFORMED_BYTES.length, adapter.updatedOldSize);

        // Verify journal deleted
        assertFalse(new File(tempDir, opId + ".journal").exists());
    }

    @Test
    public void testRecoveryCleansUncommittedPreparingJournal() throws Exception {
        final long opId = 1005L;
        final long docId = 46L;
        final MockScanFileAdapter adapter = new MockScanFileAdapter();

        final PdfFileTransaction tx = PdfFileTransaction.begin(
                tempDir,
                opId,
                docId,
                NativePdfEngine.COMPRESS,
                sourceFile,
                1,
                ORIGINAL_BYTES.length,
                1
        );

        // Simulate crash right after begin (phase PREPARING)
        assertTrue(new File(tempDir, opId + ".journal").exists());
        assertTrue(tx.getSnapshotFile().exists());

        PdfFileTransaction.recover(tempDir, adapter);

        assertFalse("Journal should be cleaned up by recovery", new File(tempDir, opId + ".journal").exists());
        assertFalse("Snapshot should be cleaned up by recovery", tx.getSnapshotFile().exists());
        assertEquals(ORIGINAL_BYTES.length, sourceFile.length());
    }

    @Test
    public void testRecoveryFinalizesFileCommittedPhaseWhenOutputHashMatches() throws Exception {
        final long opId = 1006L;
        final long docId = 47L;
        final MockScanFileAdapter adapter = new MockScanFileAdapter();

        final PdfFileTransaction tx = PdfFileTransaction.begin(
                tempDir,
                opId,
                docId,
                NativePdfEngine.PROTECT,
                sourceFile,
                1,
                ORIGINAL_BYTES.length,
                2
        );

        final File stageFile = tx.getStageFile();
        try (FileOutputStream fos = new FileOutputStream(stageFile)) {
            fos.write(TRANSFORMED_BYTES);
            fos.flush();
        }
        final String outHash = PdfFileTransaction.sha256(stageFile);
        tx.markValidated(TRANSFORMED_BYTES.length, outHash);

        // Simulate file move succeeded, recorded FILE_COMMITTED, then app died before DB
        PdfFileTransaction.atomicMove(stageFile, sourceFile);
        tx.getRecord().phase = PdfFileTransaction.PHASE_FILE_COMMITTED;

        // Save simulated FILE_COMMITTED journal
        final File journalFile = new File(tempDir, opId + ".journal");
        try (java.io.FileWriter fw = new java.io.FileWriter(journalFile)) {
            fw.write(tx.getRecord().toJson());
        }

        // Run recovery
        PdfFileTransaction.recover(tempDir, adapter);

        // DB update should have been applied
        assertTrue(adapter.updateCalled);
        assertEquals(2, adapter.updatedNewStatus);
        assertEquals(TRANSFORMED_BYTES.length, adapter.updatedNewSize);

        // Cleanup completed
        assertFalse(journalFile.exists());
        assertFalse(tx.getSnapshotFile().exists());
    }

    @Test
    public void testRecoveryValidatedPhaseWithOverwrittenSource() throws Exception {
        final long opId = 1007L;
        final long docId = 48L;
        final MockScanFileAdapter adapter = new MockScanFileAdapter();

        final PdfFileTransaction tx = PdfFileTransaction.begin(
                tempDir,
                opId,
                docId,
                NativePdfEngine.PROTECT,
                sourceFile,
                1,
                ORIGINAL_BYTES.length,
                2
        );

        final File stageFile = tx.getStageFile();
        try (FileOutputStream fos = new FileOutputStream(stageFile)) {
            fos.write(TRANSFORMED_BYTES);
            fos.flush();
        }
        final String outHash = PdfFileTransaction.sha256(stageFile);
        tx.markValidated(TRANSFORMED_BYTES.length, outHash);

        // Simulate atomic move succeeded before FILE_COMMITTED journal entry was written
        PdfFileTransaction.atomicMove(stageFile, sourceFile);

        // The journal on disk is still in PHASE_VALIDATED
        final File journalFile = new File(tempDir, opId + ".journal");
        assertTrue(journalFile.exists());

        PdfFileTransaction.recover(tempDir, adapter);

        assertTrue("adapter.updateMetadata should have been called", adapter.updateCalled);
        assertEquals(docId, adapter.updatedDocId);
        assertEquals(2, adapter.updatedNewStatus);
        assertEquals(TRANSFORMED_BYTES.length, adapter.updatedNewSize);
        assertEquals(1, adapter.updatedOldStatus);
        assertEquals(ORIGINAL_BYTES.length, adapter.updatedOldSize);

        assertFalse("Journal file should be deleted on completion", journalFile.exists());
        assertFalse("Snapshot file should be deleted on completion", tx.getSnapshotFile().exists());
    }

    @Test
    public void testRecoveryPreservesJournalOnDatabaseFailure() throws Exception {
        final long opId = 1008L;
        final long docId = 49L;
        final MockScanFileAdapter adapter = new MockScanFileAdapter();
        adapter.shouldThrowOnUpdate = true;

        final PdfFileTransaction tx = PdfFileTransaction.begin(
                tempDir,
                opId,
                docId,
                NativePdfEngine.PROTECT,
                sourceFile,
                1,
                ORIGINAL_BYTES.length,
                2
        );

        final File stageFile = tx.getStageFile();
        try (FileOutputStream fos = new FileOutputStream(stageFile)) {
            fos.write(TRANSFORMED_BYTES);
            fos.flush();
        }
        final String outHash = PdfFileTransaction.sha256(stageFile);
        tx.markValidated(TRANSFORMED_BYTES.length, outHash);

        // Simulate file committed
        PdfFileTransaction.atomicMove(stageFile, sourceFile);
        tx.getRecord().phase = PdfFileTransaction.PHASE_FILE_COMMITTED;
        final File journalFile = new File(tempDir, opId + ".journal");
        try (java.io.FileWriter fw = new java.io.FileWriter(journalFile)) {
            fw.write(tx.getRecord().toJson());
        }

        PdfFileTransaction.recover(tempDir, adapter);

        assertTrue("Journal file should still exist on DB failure", journalFile.exists());
        assertTrue("Snapshot file should still exist on DB failure", tx.getSnapshotFile().exists());
    }

    @Test
    public void testRollbackWithMissingSnapshotRetainsJournalAndSkipsMetadata() throws Exception {
        final long opId = 1010L;
        final long docId = 51L;
        final MockScanFileAdapter adapter = new MockScanFileAdapter();

        final PdfFileTransaction tx = PdfFileTransaction.begin(
                tempDir, opId, docId, NativePdfEngine.PROTECT, sourceFile,
                1, ORIGINAL_BYTES.length, 2);

        final File stageFile = tx.getStageFile();
        try (FileOutputStream fos = new FileOutputStream(stageFile)) {
            fos.write(TRANSFORMED_BYTES);
            fos.flush();
        }
        final String outHash = PdfFileTransaction.sha256(stageFile);
        tx.markValidated(TRANSFORMED_BYTES.length, outHash);
        PdfFileTransaction.atomicMove(stageFile, sourceFile);
        tx.getRecord().phase = PdfFileTransaction.PHASE_FILE_COMMITTED;

        assertTrue("Snapshot must exist before removal", tx.getSnapshotFile().delete());

        tx.rollback(adapter);

        assertEquals("Source must not be modified without a valid snapshot",
                outHash, PdfFileTransaction.sha256(sourceFile));
        assertFalse("Metadata must not be updated without a restore", adapter.updateCalled);
        assertTrue("Journal must be retained", new File(tempDir, opId + ".journal").exists());
    }

    @Test
    public void testRollbackWithUnreadableSourceRetainsJournalAndSkipsMetadata() throws Exception {
        final long opId = 1017L;
        final long docId = 58L;
        final MockScanFileAdapter adapter = new MockScanFileAdapter();

        final PdfFileTransaction tx = PdfFileTransaction.begin(
                tempDir, opId, docId, NativePdfEngine.PROTECT, sourceFile,
                1, ORIGINAL_BYTES.length, 2);

        final File stageFile = tx.getStageFile();
        try (FileOutputStream fos = new FileOutputStream(stageFile)) {
            fos.write(TRANSFORMED_BYTES);
            fos.flush();
        }
        tx.markValidated(TRANSFORMED_BYTES.length, PdfFileTransaction.sha256(stageFile));
        PdfFileTransaction.atomicMove(stageFile, sourceFile);
        tx.getRecord().phase = PdfFileTransaction.PHASE_FILE_COMMITTED;

        assertTrue("Source must exist before removal", sourceFile.delete());

        tx.rollback(adapter);

        assertFalse("Missing source must not be replaced blindly", sourceFile.exists());
        assertFalse("Metadata must not be updated without a restore", adapter.updateCalled);
        assertTrue("Snapshot must be retained", tx.getSnapshotFile().exists());
        assertTrue("Journal must be retained", new File(tempDir, opId + ".journal").exists());
    }

    @Test
    public void testRollbackRefusesExternallyModifiedSource() throws Exception {
        final long opId = 1011L;
        final long docId = 52L;
        final MockScanFileAdapter adapter = new MockScanFileAdapter();

        final PdfFileTransaction tx = PdfFileTransaction.begin(
                tempDir, opId, docId, NativePdfEngine.PROTECT, sourceFile,
                1, ORIGINAL_BYTES.length, 2);

        final File stageFile = tx.getStageFile();
        try (FileOutputStream fos = new FileOutputStream(stageFile)) {
            fos.write(TRANSFORMED_BYTES);
            fos.flush();
        }
        tx.markValidated(TRANSFORMED_BYTES.length, PdfFileTransaction.sha256(stageFile));
        PdfFileTransaction.atomicMove(stageFile, sourceFile);
        tx.getRecord().phase = PdfFileTransaction.PHASE_FILE_COMMITTED;

        final byte[] externalBytes = "user edited this document".getBytes(StandardCharsets.UTF_8);
        try (FileOutputStream fos = new FileOutputStream(sourceFile)) {
            fos.write(externalBytes);
            fos.flush();
        }
        final String externalHash = PdfFileTransaction.sha256(sourceFile);

        tx.rollback(adapter);

        assertEquals("User-modified source must not be overwritten",
                externalHash, PdfFileTransaction.sha256(sourceFile));
        assertFalse(adapter.updateCalled);
        assertTrue("Snapshot evidence must be retained", tx.getSnapshotFile().exists());
        assertTrue("Journal must be retained", new File(tempDir, opId + ".journal").exists());
    }

    @Test
    public void testRollbackAlreadyRestoredReconcilesMetadataIdempotently() throws Exception {
        final long opId = 1012L;
        final long docId = 53L;
        final MockScanFileAdapter adapter = new MockScanFileAdapter();

        final PdfFileTransaction tx = PdfFileTransaction.begin(
                tempDir, opId, docId, NativePdfEngine.PROTECT, sourceFile,
                1, ORIGINAL_BYTES.length, 2);

        final File stageFile = tx.getStageFile();
        try (FileOutputStream fos = new FileOutputStream(stageFile)) {
            fos.write(TRANSFORMED_BYTES);
            fos.flush();
        }
        tx.markValidated(TRANSFORMED_BYTES.length, PdfFileTransaction.sha256(stageFile));
        PdfFileTransaction.atomicMove(stageFile, sourceFile);
        tx.getRecord().phase = PdfFileTransaction.PHASE_FILE_COMMITTED;

        // Simulate a previous rollback that restored the snapshot but died before
        // it could reconcile metadata and remove the journal.
        try (FileOutputStream fos = new FileOutputStream(sourceFile)) {
            fos.write(ORIGINAL_BYTES);
            fos.flush();
        }
        assertTrue(tx.getSnapshotFile().delete());

        tx.rollback(adapter);

        assertArrayEquals("Already-restored source must not be clobbered",
                ORIGINAL_BYTES, java.nio.file.Files.readAllBytes(sourceFile.toPath()));
        assertEquals(tx.getRecord().sourceSha256, PdfFileTransaction.sha256(sourceFile));
        assertTrue("Old metadata must be reconciled", adapter.updateCalled);
        assertEquals(1, adapter.updatedNewStatus);
        assertEquals(ORIGINAL_BYTES.length, adapter.updatedNewSize);
        assertFalse("Journal must be removed after reconciliation",
                new File(tempDir, opId + ".journal").exists());
    }

    @Test
    public void testRollbackMetadataFailureRetainsJournalAfterRestore() throws Exception {
        final long opId = 1013L;
        final long docId = 54L;
        final MockScanFileAdapter adapter = new MockScanFileAdapter();

        final PdfFileTransaction tx = PdfFileTransaction.begin(
                tempDir, opId, docId, NativePdfEngine.PROTECT, sourceFile,
                1, ORIGINAL_BYTES.length, 2);

        final File stageFile = tx.getStageFile();
        try (FileOutputStream fos = new FileOutputStream(stageFile)) {
            fos.write(TRANSFORMED_BYTES);
            fos.flush();
        }
        tx.markValidated(TRANSFORMED_BYTES.length, PdfFileTransaction.sha256(stageFile));
        PdfFileTransaction.atomicMove(stageFile, sourceFile);
        tx.getRecord().phase = PdfFileTransaction.PHASE_FILE_COMMITTED;

        adapter.shouldThrowOnUpdate = true;
        tx.rollback(adapter);

        assertArrayEquals("File restore must still complete",
                ORIGINAL_BYTES, java.nio.file.Files.readAllBytes(sourceFile.toPath()));
        assertEquals(tx.getRecord().sourceSha256, PdfFileTransaction.sha256(sourceFile));
        assertTrue("Journal must be retained after metadata failure",
                new File(tempDir, opId + ".journal").exists());
    }

    @Test
    public void testRecoveryFileCommittedWithOriginalHashReconcilesOldMetadata() throws Exception {
        final long opId = 1014L;
        final long docId = 55L;
        final MockScanFileAdapter adapter = new MockScanFileAdapter();

        final PdfFileTransaction tx = PdfFileTransaction.begin(
                tempDir, opId, docId, NativePdfEngine.PROTECT, sourceFile,
                1, ORIGINAL_BYTES.length, 2);

        final File stageFile = tx.getStageFile();
        try (FileOutputStream fos = new FileOutputStream(stageFile)) {
            fos.write(TRANSFORMED_BYTES);
            fos.flush();
        }
        tx.markValidated(TRANSFORMED_BYTES.length, PdfFileTransaction.sha256(stageFile));

        // Simulate a completed rollback file restore whose journal phase never advanced.
        tx.getRecord().phase = PdfFileTransaction.PHASE_FILE_COMMITTED;
        final File journalFile = new File(tempDir, opId + ".journal");
        try (java.io.FileWriter fw = new java.io.FileWriter(journalFile)) {
            fw.write(tx.getRecord().toJson());
        }

        PdfFileTransaction.recover(tempDir, adapter);

        assertTrue("Rollback metadata must be reconciled", adapter.updateCalled);
        assertEquals(1, adapter.updatedNewStatus);
        assertEquals(ORIGINAL_BYTES.length, adapter.updatedNewSize);
        assertArrayEquals("Already-restored source must not be clobbered",
                ORIGINAL_BYTES, java.nio.file.Files.readAllBytes(sourceFile.toPath()));
        assertFalse("Journal must be removed after reconciliation", journalFile.exists());
        assertFalse("Snapshot must be cleaned up", tx.getSnapshotFile().exists());
    }

    @Test
    public void testRecoveryFileCommittedWithOutputHashRestoresNothingButReconcilesMetadata() throws Exception {
        final long opId = 1018L;
        final long docId = 59L;
        final MockScanFileAdapter adapter = new MockScanFileAdapter();

        final PdfFileTransaction tx = PdfFileTransaction.begin(
                tempDir, opId, docId, NativePdfEngine.PROTECT, sourceFile,
                1, ORIGINAL_BYTES.length, 2);

        final File stageFile = tx.getStageFile();
        try (FileOutputStream fos = new FileOutputStream(stageFile)) {
            fos.write(TRANSFORMED_BYTES);
            fos.flush();
        }
        tx.markValidated(TRANSFORMED_BYTES.length, PdfFileTransaction.sha256(stageFile));
        PdfFileTransaction.atomicMove(stageFile, sourceFile);
        tx.getRecord().phase = PdfFileTransaction.PHASE_FILE_COMMITTED;
        final File journalFile = new File(tempDir, opId + ".journal");
        try (java.io.FileWriter fw = new java.io.FileWriter(journalFile)) {
            fw.write(tx.getRecord().toJson());
        }

        PdfFileTransaction.recover(tempDir, adapter);

        assertArrayEquals("Committed output must be preserved",
                TRANSFORMED_BYTES, java.nio.file.Files.readAllBytes(sourceFile.toPath()));
        assertTrue(adapter.updateCalled);
        assertEquals(2, adapter.updatedNewStatus);
        assertEquals(TRANSFORMED_BYTES.length, adapter.updatedNewSize);
        assertFalse("Journal must be removed after successful reconciliation", journalFile.exists());
    }

    @Test
    public void testRecoveryFileCommittedAmbiguousSourceRetainsEvidence() throws Exception {
        final long opId = 1015L;
        final long docId = 56L;
        final MockScanFileAdapter adapter = new MockScanFileAdapter();

        final PdfFileTransaction tx = PdfFileTransaction.begin(
                tempDir, opId, docId, NativePdfEngine.PROTECT, sourceFile,
                1, ORIGINAL_BYTES.length, 2);

        final File stageFile = tx.getStageFile();
        try (FileOutputStream fos = new FileOutputStream(stageFile)) {
            fos.write(TRANSFORMED_BYTES);
            fos.flush();
        }
        tx.markValidated(TRANSFORMED_BYTES.length, PdfFileTransaction.sha256(stageFile));

        tx.getRecord().phase = PdfFileTransaction.PHASE_FILE_COMMITTED;
        final File journalFile = new File(tempDir, opId + ".journal");
        try (java.io.FileWriter fw = new java.io.FileWriter(journalFile)) {
            fw.write(tx.getRecord().toJson());
        }

        final byte[] ambiguousBytes = "ambiguous external content".getBytes(StandardCharsets.UTF_8);
        try (FileOutputStream fos = new FileOutputStream(sourceFile)) {
            fos.write(ambiguousBytes);
            fos.flush();
        }
        final String ambiguousHash = PdfFileTransaction.sha256(sourceFile);

        PdfFileTransaction.recover(tempDir, adapter);

        assertEquals("Ambiguous source must not be overwritten",
                ambiguousHash, PdfFileTransaction.sha256(sourceFile));
        assertArrayEquals(ambiguousBytes, java.nio.file.Files.readAllBytes(sourceFile.toPath()));
        assertFalse(adapter.updateCalled);
        assertTrue("Journal must be retained", journalFile.exists());
        assertTrue("Snapshot must be retained", tx.getSnapshotFile().exists());
    }

    @Test
    public void testRecoveryValidatedAmbiguousSourceRetainsEvidence() throws Exception {
        final long opId = 1016L;
        final long docId = 57L;
        final MockScanFileAdapter adapter = new MockScanFileAdapter();

        final PdfFileTransaction tx = PdfFileTransaction.begin(
                tempDir, opId, docId, NativePdfEngine.PROTECT, sourceFile,
                1, ORIGINAL_BYTES.length, 2);

        final File stageFile = tx.getStageFile();
        try (FileOutputStream fos = new FileOutputStream(stageFile)) {
            fos.write(TRANSFORMED_BYTES);
            fos.flush();
        }
        tx.markValidated(TRANSFORMED_BYTES.length, PdfFileTransaction.sha256(stageFile));

        final byte[] ambiguousBytes = "validated ambiguous content".getBytes(StandardCharsets.UTF_8);
        try (FileOutputStream fos = new FileOutputStream(sourceFile)) {
            fos.write(ambiguousBytes);
            fos.flush();
        }
        final String ambiguousHash = PdfFileTransaction.sha256(sourceFile);

        PdfFileTransaction.recover(tempDir, adapter);

        assertEquals(ambiguousHash, PdfFileTransaction.sha256(sourceFile));
        assertArrayEquals(ambiguousBytes, java.nio.file.Files.readAllBytes(sourceFile.toPath()));
        assertFalse(adapter.updateCalled);
        assertTrue(new File(tempDir, opId + ".journal").exists());
        assertTrue(tx.getSnapshotFile().exists());
    }

    @Test
    public void testFailedRollbackPreservesSnapshotFile() throws Exception {
        final long opId = 1009L;
        final long docId = 50L;
        final MockScanFileAdapter adapter = new MockScanFileAdapter();

        final PdfFileTransaction tx = PdfFileTransaction.begin(
                tempDir,
                opId,
                docId,
                NativePdfEngine.PROTECT,
                sourceFile,
                1,
                ORIGINAL_BYTES.length,
                2
        );

        final File stageFile = tx.getStageFile();
        try (FileOutputStream fos = new FileOutputStream(stageFile)) {
            fos.write(TRANSFORMED_BYTES);
            fos.flush();
        }
        tx.markValidated(TRANSFORMED_BYTES.length, PdfFileTransaction.sha256(stageFile));

        // Simulate move to source and file committed
        PdfFileTransaction.atomicMove(stageFile, sourceFile);
        tx.getRecord().phase = PdfFileTransaction.PHASE_FILE_COMMITTED;

        // Corrupt snapshot file hash so rollback fails
        try (FileOutputStream fos = new FileOutputStream(tx.getSnapshotFile())) {
            fos.write("corrupted snapshot bytes".getBytes(StandardCharsets.UTF_8));
            fos.flush();
        }

        tx.rollback(adapter);

        final String committedHash = tx.getRecord().outputSha256;
        assertTrue("Snapshot file must be preserved on failed rollback", tx.getSnapshotFile().exists());
        assertTrue("Journal file must be preserved on failed rollback", new File(tempDir, opId + ".journal").exists());
        assertEquals("Committed bytes must not be replaced by a corrupted snapshot",
                committedHash, PdfFileTransaction.sha256(sourceFile));
        assertArrayEquals(TRANSFORMED_BYTES, java.nio.file.Files.readAllBytes(sourceFile.toPath()));
        assertFalse("Metadata must not be updated after a failed restore", adapter.updateCalled);
    }
}
