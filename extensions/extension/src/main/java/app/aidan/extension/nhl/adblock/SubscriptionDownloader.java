package app.aidan.extension.nhl.adblock;

import java.io.File;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.net.HttpURLConnection;
import java.net.InetAddress;
import java.net.URL;
import java.nio.file.AtomicMoveNotSupportedException;
import java.nio.file.Files;
import java.nio.file.StandardCopyOption;
import java.util.Locale;
import java.util.zip.GZIPInputStream;

import javax.net.ssl.HttpsURLConnection;

/**
 * Performs a bounded, conditional HTTP(S) download of a filter list and publishes it
 * with a checked same-directory replacement. Metadata on the subscription is mutated
 * only after the new file has been published successfully.
 */
final class SubscriptionDownloader {
    static final int MAX_REDIRECTS = 3;

    interface ConnectionFactory {
        HttpURLConnection open(URL url) throws IOException;
    }

    interface Publisher {
        void publish(File source, File target) throws IOException;
    }

    static final class Result {
        final boolean notModified;
        final long bytesWritten;

        Result(boolean notModified, long bytesWritten) {
            this.notModified = notModified;
            this.bytesWritten = bytesWritten;
        }
    }

    private final ConnectionFactory mFactory;
    private final Publisher mPublisher;
    private final long mMaxBytes;

    SubscriptionDownloader() {
        this(new DefaultConnectionFactory(), atomicPublisher(), RuleRepository.MAX_LIST_BYTES);
    }

    SubscriptionDownloader(ConnectionFactory factory, Publisher publisher, long maxBytes) {
        mFactory = factory;
        mPublisher = publisher;
        mMaxBytes = maxBytes;
    }

    Result download(RuleRepository.SubscriptionInfo subscription, File targetFile) throws IOException {
        return download(subscription, targetFile, null);
    }

    Result download(
            RuleRepository.SubscriptionInfo subscription,
            File targetFile,
            UpdateCancellation cancellation) throws IOException {
        HttpURLConnection conn = null;
        URL currentUrl = new URL(subscription.url);
        int redirects = 0;

        try {
            while (true) {
                throwIfCancelled(cancellation);
                if (!"https".equalsIgnoreCase(currentUrl.getProtocol())) {
                    throw new IOException("Redirect to non-HTTPS URL disallowed: " + currentUrl);
                }

                conn = mFactory.open(currentUrl);
                if (cancellation != null) {
                    cancellation.attach(conn);
                }
                throwIfCancelled(cancellation);
                if (subscription.lastUpdated > 0L && subscription.etag != null && !subscription.etag.isEmpty()) {
                    conn.setRequestProperty("If-None-Match", subscription.etag);
                    if (subscription.lastModified != null && !subscription.lastModified.isEmpty()) {
                        conn.setRequestProperty("If-Modified-Since", subscription.lastModified);
                    }
                }

                int code = conn.getResponseCode();
                throwIfCancelled(cancellation);
                if (code == HttpURLConnection.HTTP_NOT_MODIFIED) {
                    boolean committed = commit(cancellation, () -> {
                        subscription.lastUpdated = System.currentTimeMillis();
                        subscription.lastError = "";
                    });
                    if (!committed) {
                        throw new IOException("Update cancelled");
                    }
                    return new Result(true, 0L);
                }

                if (isRedirect(code)) {
                    String location = conn.getHeaderField("Location");
                    if (location == null || location.trim().isEmpty()) {
                        throw new IOException("Redirect response missing Location header (HTTP " + code + ")");
                    }
                    if (redirects >= MAX_REDIRECTS) {
                        throw new IOException("Exceeded maximum of " + MAX_REDIRECTS + " redirects");
                    }
                    URL nextUrl = new URL(currentUrl, location.trim());
                    if (cancellation != null) {
                        cancellation.detach(conn);
                    }
                    conn.disconnect();
                    conn = null;
                    currentUrl = nextUrl;
                    redirects++;
                    continue;
                }

                if (code != HttpURLConnection.HTTP_OK) {
                    closeQuietly(conn.getErrorStream());
                    throw new IOException("HTTP error " + code);
                }
                break;
            }

            throwIfCancelled(cancellation);
            String encoding = conn.getContentEncoding();
            InputStream rawStream = conn.getInputStream();
            InputStream stream;
            try {
                stream = encoding != null && encoding.toLowerCase(Locale.ROOT).contains("gzip")
                        ? new GZIPInputStream(rawStream)
                        : rawStream;
            } catch (IOException e) {
                closeQuietly(rawStream);
                throw e;
            }

            File targetDir = targetFile.getParentFile();
            if (targetDir == null || (!targetDir.exists() && !targetDir.mkdirs())) {
                closeQuietly(stream);
                throw new IOException("Unable to create list directory for " + targetFile.getName());
            }

            File tmpFile = null;
            boolean published = false;
            long written = 0L;
            try {
                throwIfCancelled(cancellation);
                tmpFile = File.createTempFile(targetFile.getName() + ".", ".tmp", targetDir);
                final File stagedFile = tmpFile;
                try (InputStream in = stream; FileOutputStream out = new FileOutputStream(stagedFile)) {
                    byte[] buffer = new byte[8192];
                    int read;
                    while ((read = in.read(buffer)) != -1) {
                        throwIfCancelled(cancellation);
                        written += read;
                        if (written > mMaxBytes) {
                            throw new IOException("Subscription exceeds " + mMaxBytes + " byte limit");
                        }
                        out.write(buffer, 0, read);
                    }
                    throwIfCancelled(cancellation);
                    out.flush();
                    out.getFD().sync();
                }

                throwIfCancelled(cancellation);
                final String etag = headerOrEmpty(conn, "ETag");
                final String lastModified = headerOrEmpty(conn, "Last-Modified");
                boolean committed = commit(cancellation, () -> {
                    mPublisher.publish(stagedFile, targetFile);
                    subscription.etag = etag;
                    subscription.lastModified = lastModified;
                    subscription.lastUpdated = System.currentTimeMillis();
                    subscription.lastError = "";
                });
                if (!committed) {
                    throw new IOException("Update cancelled");
                }
                published = true;
            } finally {
                if (!published) {
                    closeQuietly(stream);
                    if (tmpFile != null && tmpFile.exists() && !tmpFile.delete()) {
                        tmpFile.deleteOnExit();
                    }
                }
            }

            return new Result(false, written);
        } finally {
            if (conn != null) {
                if (cancellation != null) {
                    cancellation.detach(conn);
                }
                conn.disconnect();
            }
        }
    }

    private static void throwIfCancelled(UpdateCancellation cancellation) throws IOException {
        if (cancellation != null && cancellation.isCancelled()) {
            throw new IOException("Update cancelled");
        }
    }

    private static boolean commit(
            UpdateCancellation cancellation,
            UpdateCancellation.CommitAction action) throws IOException {
        if (cancellation == null) {
            action.run();
            return true;
        }
        return cancellation.commit(action);
    }

    private static boolean isRedirect(int code) {
        return code == HttpURLConnection.HTTP_MOVED_PERM
                || code == HttpURLConnection.HTTP_MOVED_TEMP
                || code == 307
                || code == 308;
    }

    private static String headerOrEmpty(HttpURLConnection connection, String name) {
        String value = connection.getHeaderField(name);
        return value != null ? value : "";
    }

    private static void closeQuietly(InputStream stream) {
        if (stream == null) return;
        try {
            stream.close();
        } catch (IOException ignored) {
            // Best effort while reporting the original failure.
        }
    }

    static Publisher atomicPublisher() {
        return (source, target) -> {
            try {
                Files.move(
                        source.toPath(),
                        target.toPath(),
                        StandardCopyOption.REPLACE_EXISTING,
                        StandardCopyOption.ATOMIC_MOVE);
            } catch (AtomicMoveNotSupportedException e) {
                throw new IOException("Atomic replacement is not supported for " + target, e);
            }
        };
    }

    static final class DefaultConnectionFactory implements ConnectionFactory {
        @Override
        public HttpURLConnection open(URL url) throws IOException {
            if (!"https".equalsIgnoreCase(url.getProtocol())) {
                throw new IOException("Redirect to non-HTTPS URL disallowed: " + url);
            }

            InetAddress address = InetAddress.getByName(url.getHost());
            if (address.isLoopbackAddress()
                    || address.isSiteLocalAddress()
                    || address.isLinkLocalAddress()
                    || address.isAnyLocalAddress()) {
                throw new IOException("Host resolves to private or loopback IP: " + address);
            }

            HttpsURLConnection connection = (HttpsURLConnection) url.openConnection();
            connection.setInstanceFollowRedirects(false);
            connection.setConnectTimeout(10_000);
            connection.setReadTimeout(30_000);
            connection.setRequestProperty("User-Agent", "NHL-AdBlock/1.0");
            connection.setRequestProperty("Accept-Encoding", "gzip");
            return connection;
        }
    }
}
