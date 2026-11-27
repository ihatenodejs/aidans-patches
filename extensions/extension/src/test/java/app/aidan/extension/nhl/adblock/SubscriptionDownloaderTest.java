package app.aidan.extension.nhl.adblock;

import static org.junit.Assert.assertArrayEquals;
import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertNull;
import static org.junit.Assert.assertTrue;
import static org.junit.Assert.fail;

import org.junit.Rule;
import org.junit.Test;
import org.junit.rules.TemporaryFolder;

import java.io.ByteArrayInputStream;
import java.io.File;
import java.io.IOException;
import java.io.InputStream;
import java.net.HttpURLConnection;
import java.net.URL;
import java.nio.charset.StandardCharsets;
import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.Deque;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;

public class SubscriptionDownloaderTest {
    @Rule
    public final TemporaryFolder mTemp = new TemporaryFolder();

    @Test
    public void successfulDownloadPublishesAndUpdatesMetadata() throws Exception {
        File target = mTemp.newFile("user_1.txt");
        write(target, "old rules");
        RuleRepository.SubscriptionInfo subscription = subscription();
        FakeConnection connection = connection(200, "new rules");
        connection.headers.put("ETag", "\"new-etag\"");
        connection.headers.put("Last-Modified", "Mon, 23 Nov 2026 10:00:00 GMT");

        SubscriptionDownloader.Result result = downloader(connection).download(subscription, target);

        assertFalse(result.notModified);
        assertEquals("new rules", read(target));
        assertEquals("\"new-etag\"", subscription.etag);
        assertEquals("Mon, 23 Nov 2026 10:00:00 GMT", subscription.lastModified);
        assertEquals("", subscription.lastError);
        assertTrue(subscription.lastUpdated > 0L);
        assertTrue(tmpFiles(target.getParentFile()).isEmpty());
    }

    @Test
    public void notModifiedPreservesBytesAndKeepsEtag() throws Exception {
        File target = mTemp.newFile("user_1.txt");
        write(target, "old rules");
        RuleRepository.SubscriptionInfo subscription = subscription();
        subscription.lastError = "previous failure";

        SubscriptionDownloader.Result result = downloader(connection(304, "")).download(subscription, target);

        assertTrue(result.notModified);
        assertEquals("old rules", read(target));
        assertEquals("\"old-etag\"", subscription.etag);
        assertEquals("", subscription.lastError);
    }

    @Test
    public void singleRedirectIsFollowed() throws Exception {
        File target = mTemp.newFile("user_1.txt");
        write(target, "old rules");
        RuleRepository.SubscriptionInfo subscription = subscription();
        FakeConnection redirect = connection(302, "");
        redirect.headers.put("Location", "/lists/new.txt");

        FakeFactory factory = new FakeFactory(redirect, connection(200, "redirected rules"));
        SubscriptionDownloader downloader = new SubscriptionDownloader(
                factory, SubscriptionDownloader.atomicPublisher(), RuleRepository.MAX_LIST_BYTES);

        downloader.download(subscription, target);

        assertEquals("redirected rules", read(target));
        assertEquals(2, factory.opened.size());
        assertEquals("https://example.com/lists/new.txt", factory.opened.get(1).toString());
    }

    @Test
    public void redirectBudgetExhaustionKeepsOldFileAndMetadata() throws Exception {
        File target = mTemp.newFile("user_1.txt");
        write(target, "old rules");
        RuleRepository.SubscriptionInfo subscription = subscription();
        Metadata before = Metadata.of(subscription);

        FakeConnection first = connection(302, "");
        first.headers.put("Location", "/r1");
        FakeConnection second = connection(302, "");
        second.headers.put("Location", "/r2");
        FakeConnection third = connection(302, "");
        third.headers.put("Location", "/r3");
        FakeConnection fourth = connection(302, "");
        fourth.headers.put("Location", "/r4");

        try {
            downloader(first, second, third, fourth).download(subscription, target);
            fail("Expected redirect exhaustion to fail");
        } catch (IOException expected) {
            assertTrue(expected.getMessage().contains("maximum"));
        }

        assertArrayEquals("old rules".getBytes(StandardCharsets.UTF_8), readBytes(target));
        before.assertUnchanged(subscription);
        assertTrue(tmpFiles(target.getParentFile()).isEmpty());
    }

    @Test
    public void redirectWithoutLocationFailsWithoutTouchingList() throws Exception {
        File target = mTemp.newFile("user_1.txt");
        write(target, "old rules");
        RuleRepository.SubscriptionInfo subscription = subscription();
        Metadata before = Metadata.of(subscription);

        try {
            downloader(connection(302, "")).download(subscription, target);
            fail("Expected missing Location to fail");
        } catch (IOException expected) {
            assertTrue(expected.getMessage().contains("Location"));
        }

        assertEquals("old rules", read(target));
        before.assertUnchanged(subscription);
    }

    @Test
    public void httpErrorKeepsOldFileAndMetadata() throws Exception {
        File target = mTemp.newFile("user_1.txt");
        write(target, "old rules");
        RuleRepository.SubscriptionInfo subscription = subscription();
        Metadata before = Metadata.of(subscription);

        try {
            downloader(connection(500, "server error")).download(subscription, target);
            fail("Expected HTTP failure");
        } catch (IOException expected) {
            assertTrue(expected.getMessage().contains("500"));
        }

        assertEquals("old rules", read(target));
        before.assertUnchanged(subscription);
    }

    @Test
    public void oversizedResponseFailsAndCleansTemporaryFile() throws Exception {
        File target = mTemp.newFile("user_1.txt");
        write(target, "old rules");
        RuleRepository.SubscriptionInfo subscription = subscription();
        Metadata before = Metadata.of(subscription);

        SubscriptionDownloader downloader = new SubscriptionDownloader(
                new FakeFactory(connection(200, "123456789")),
                SubscriptionDownloader.atomicPublisher(),
                8L);
        try {
            downloader.download(subscription, target);
            fail("Expected size limit failure");
        } catch (IOException expected) {
            assertTrue(expected.getMessage().contains("limit"));
        }

        assertEquals("old rules", read(target));
        before.assertUnchanged(subscription);
        assertTrue(tmpFiles(target.getParentFile()).isEmpty());
    }

    @Test
    public void ioFailureKeepsOldFileAndMetadata() throws Exception {
        File target = mTemp.newFile("user_1.txt");
        write(target, "old rules");
        RuleRepository.SubscriptionInfo subscription = subscription();
        Metadata before = Metadata.of(subscription);

        SubscriptionDownloader downloader = new SubscriptionDownloader(
                url -> {
                    throw new IOException("network gone");
                },
                SubscriptionDownloader.atomicPublisher(),
                RuleRepository.MAX_LIST_BYTES);
        try {
            downloader.download(subscription, target);
            fail("Expected I/O failure");
        } catch (IOException expected) {
            assertEquals("network gone", expected.getMessage());
        }

        assertEquals("old rules", read(target));
        before.assertUnchanged(subscription);
    }

    @Test
    public void failedPublicationKeepsOldFileAndMetadata() throws Exception {
        File target = mTemp.newFile("user_1.txt");
        write(target, "old rules");
        RuleRepository.SubscriptionInfo subscription = subscription();
        Metadata before = Metadata.of(subscription);

        SubscriptionDownloader downloader = new SubscriptionDownloader(
                new FakeFactory(connection(200, "new rules")),
                (source, destination) -> {
                    throw new IOException("rename failed");
                },
                RuleRepository.MAX_LIST_BYTES);
        try {
            downloader.download(subscription, target);
            fail("Expected publication failure");
        } catch (IOException expected) {
            assertEquals("rename failed", expected.getMessage());
        }

        assertEquals("old rules", read(target));
        before.assertUnchanged(subscription);
        assertTrue(tmpFiles(target.getParentFile()).isEmpty());
    }

    @Test
    public void conditionalHeadersAreSentWhenMetadataExists() throws Exception {
        File target = mTemp.newFile("user_1.txt");
        write(target, "old rules");
        RuleRepository.SubscriptionInfo subscription = subscription();
        FakeConnection connection = connection(304, "");

        downloader(connection).download(subscription, target);

        assertEquals("\"old-etag\"", connection.requestHeaders.get("If-None-Match"));
        assertEquals("Mon, 23 Nov 2026 09:00:00 GMT", connection.requestHeaders.get("If-Modified-Since"));
    }

    @Test
    public void cancellationDuringReadDoesNotPublishOrChangeMetadata() throws Exception {
        File target = mTemp.newFile("user_1.txt");
        write(target, "old rules");
        RuleRepository.SubscriptionInfo subscription = subscription();
        Metadata before = Metadata.of(subscription);
        UpdateCancellation cancellation = new UpdateCancellation();
        FakeConnection connection = connection(200, "new rules");
        connection.onRead = cancellation::cancel;

        try {
            downloader(connection).download(subscription, target, cancellation);
            fail("Expected cancellation");
        } catch (IOException expected) {
            assertTrue(expected.getMessage().contains("cancelled"));
        }

        assertEquals("old rules", read(target));
        before.assertUnchanged(subscription);
        assertTrue(tmpFiles(target.getParentFile()).isEmpty());
    }

    @Test
    public void preCancelledRunDoesNotOpenAConnection() throws Exception {
        File target = mTemp.newFile("user_1.txt");
        write(target, "old rules");
        RuleRepository.SubscriptionInfo subscription = subscription();
        Metadata before = Metadata.of(subscription);
        UpdateCancellation cancellation = new UpdateCancellation();
        cancellation.cancel();
        FakeFactory factory = new FakeFactory(connection(200, "new rules"));

        try {
            new SubscriptionDownloader(
                    factory, SubscriptionDownloader.atomicPublisher(), RuleRepository.MAX_LIST_BYTES)
                    .download(subscription, target, cancellation);
            fail("Expected cancellation");
        } catch (IOException expected) {
            assertTrue(expected.getMessage().contains("cancelled"));
        }

        assertTrue(factory.opened.isEmpty());
        assertEquals("old rules", read(target));
        before.assertUnchanged(subscription);
    }

    @Test
    public void publicationAndMetadataCompleteBeforeCancelReturns() throws Exception {
        File target = mTemp.newFile("user_1.txt");
        write(target, "old rules");
        RuleRepository.SubscriptionInfo subscription = subscription();
        UpdateCancellation cancellation = new UpdateCancellation();
        CountDownLatch publisherStarted = new CountDownLatch(1);
        CountDownLatch releasePublisher = new CountDownLatch(1);
        CountDownLatch cancelReturned = new CountDownLatch(1);
        java.util.concurrent.atomic.AtomicReference<Throwable> failure =
                new java.util.concurrent.atomic.AtomicReference<>();

        SubscriptionDownloader downloader = new SubscriptionDownloader(
                new FakeFactory(connection(200, "new rules")),
                (source, destination) -> {
                    publisherStarted.countDown();
                    try {
                        if (!releasePublisher.await(1, TimeUnit.SECONDS)) {
                            throw new IOException("Timed out waiting to release publisher");
                        }
                    } catch (InterruptedException e) {
                        Thread.currentThread().interrupt();
                        throw new IOException(e);
                    }
                    SubscriptionDownloader.atomicPublisher().publish(source, destination);
                },
                RuleRepository.MAX_LIST_BYTES);
        Thread downloadThread = new Thread(() -> {
            try {
                downloader.download(subscription, target, cancellation);
            } catch (Throwable t) {
                failure.set(t);
            }
        }, "download-thread");
        downloadThread.start();
        assertTrue(publisherStarted.await(1, TimeUnit.SECONDS));

        Thread cancelThread = new Thread(() -> {
            cancellation.cancel();
            cancelReturned.countDown();
        }, "cancel-thread");
        cancelThread.start();
        assertFalse(cancelReturned.await(50, TimeUnit.MILLISECONDS));

        releasePublisher.countDown();
        downloadThread.join(1000);
        cancelThread.join(1000);

        assertNull(failure.get());
        assertEquals("new rules", read(target));
        assertEquals("", subscription.lastError);
        assertTrue(cancelReturned.getCount() == 0);
    }

    private RuleRepository.SubscriptionInfo subscription() {
        RuleRepository.SubscriptionInfo subscription =
                new RuleRepository.SubscriptionInfo("user_1", "Test", "https://example.com/list.txt", false, true);
        subscription.etag = "\"old-etag\"";
        subscription.lastModified = "Mon, 23 Nov 2026 09:00:00 GMT";
        subscription.lastUpdated = 1234L;
        return subscription;
    }

    private SubscriptionDownloader downloader(FakeConnection... connections) {
        return new SubscriptionDownloader(
                new FakeFactory(connections),
                SubscriptionDownloader.atomicPublisher(),
                RuleRepository.MAX_LIST_BYTES);
    }

    private FakeConnection connection(int code, String body) throws Exception {
        return new FakeConnection(new URL("https://example.com/list.txt"), code, body);
    }

    private static void write(File file, String content) throws IOException {
        java.nio.file.Files.write(file.toPath(), content.getBytes(StandardCharsets.UTF_8));
    }

    private static String read(File file) throws IOException {
        return new String(readBytes(file), StandardCharsets.UTF_8);
    }

    private static byte[] readBytes(File file) throws IOException {
        return java.nio.file.Files.readAllBytes(file.toPath());
    }

    private static List<File> tmpFiles(File directory) {
        List<File> result = new ArrayList<>();
        File[] files = directory.listFiles();
        if (files != null) {
            for (File file : files) {
                if (file.getName().endsWith(".tmp")) result.add(file);
            }
        }
        return result;
    }

    private static final class Metadata {
        private final String etag;
        private final String lastModified;
        private final long lastUpdated;
        private final String lastError;
        private final int accepted;
        private final int unsupported;
        private final int invalid;

        private Metadata(RuleRepository.SubscriptionInfo subscription) {
            etag = subscription.etag;
            lastModified = subscription.lastModified;
            lastUpdated = subscription.lastUpdated;
            lastError = subscription.lastError;
            accepted = subscription.acceptedCount;
            unsupported = subscription.unsupportedCount;
            invalid = subscription.invalidCount;
        }

        static Metadata of(RuleRepository.SubscriptionInfo subscription) {
            return new Metadata(subscription);
        }

        void assertUnchanged(RuleRepository.SubscriptionInfo subscription) {
            assertEquals(etag, subscription.etag);
            assertEquals(lastModified, subscription.lastModified);
            assertEquals(lastUpdated, subscription.lastUpdated);
            assertEquals(lastError, subscription.lastError);
            assertEquals(accepted, subscription.acceptedCount);
            assertEquals(unsupported, subscription.unsupportedCount);
            assertEquals(invalid, subscription.invalidCount);
        }
    }

    private static final class FakeFactory implements SubscriptionDownloader.ConnectionFactory {
        private final Deque<FakeConnection> responses = new ArrayDeque<>();
        private final List<URL> opened = new ArrayList<>();

        FakeFactory(FakeConnection... connections) {
            for (FakeConnection connection : connections) {
                responses.addLast(connection);
            }
        }

        @Override
        public HttpURLConnection open(URL url) throws IOException {
            opened.add(url);
            FakeConnection connection = responses.pollFirst();
            if (connection == null) {
                throw new IOException("No fake response for " + url);
            }
            return connection;
        }
    }

    private static final class FakeConnection extends HttpURLConnection {
        private final int code;
        private final byte[] body;
        final Map<String, String> headers = new HashMap<>();
        final Map<String, String> requestHeaders = new HashMap<>();
        Runnable onRead;

        FakeConnection(URL url, int code, String body) {
            super(url);
            this.code = code;
            this.body = body.getBytes(StandardCharsets.UTF_8);
        }

        @Override
        public void connect() {
        }

        @Override
        public void disconnect() {
        }

        @Override
        public boolean usingProxy() {
            return false;
        }

        @Override
        public int getResponseCode() {
            return code;
        }

        @Override
        public String getHeaderField(String name) {
            if (name == null) return null;
            for (Map.Entry<String, String> entry : headers.entrySet()) {
                if (entry.getKey().equalsIgnoreCase(name)) return entry.getValue();
            }
            return null;
        }

        @Override
        public void setRequestProperty(String key, String value) {
            requestHeaders.put(key, value);
        }

        @Override
        public String getContentEncoding() {
            String value = getHeaderField("Content-Encoding");
            return value == null ? null : value;
        }

        @Override
        public InputStream getInputStream() throws IOException {
            if (code >= 400) throw new IOException("HTTP " + code);
            return new ByteArrayInputStream(body) {
                @Override
                public synchronized int read(byte[] buffer, int offset, int length) {
                    Runnable trigger = onRead;
                    if (trigger != null) {
                        onRead = null;
                        trigger.run();
                    }
                    return super.read(buffer, offset, length);
                }
            };
        }

        @Override
        public InputStream getErrorStream() {
            return code >= 400 ? new ByteArrayInputStream(body) : null;
        }
    }
}
