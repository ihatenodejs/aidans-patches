package app.aidan.extension.nhl.adblock;

import java.io.IOException;
import java.net.HttpURLConnection;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicReference;

/**
 * Cooperative cancellation for a subscription refresh. Cancellation disconnects the active
 * HTTP connection so a blocked read is released. Publication and engine installation go
 * through {@link #commit(CommitAction)} so a cancel either observes an already-finished
 * commit or prevents the commit from running at all.
 */
final class UpdateCancellation {
    interface CommitAction {
        void run() throws IOException;
    }

    private final AtomicBoolean mCancelled = new AtomicBoolean(false);
    private final AtomicReference<HttpURLConnection> mConnection = new AtomicReference<>();
    private final Object mCommitLock = new Object();

    boolean isCancelled() {
        return mCancelled.get();
    }

    void cancel() {
        mCancelled.set(true);
        HttpURLConnection connection = mConnection.getAndSet(null);
        if (connection != null) {
            connection.disconnect();
        }
        synchronized (mCommitLock) {
            // Wait for an in-flight commit to finish before returning to the caller.
        }
    }

    /**
     * Runs {@code action} as a single commit point. Returns false when cancellation won the
     * race and the action was not run. If the action starts first, cancellation waits for it.
     */
    boolean commit(CommitAction action) throws IOException {
        synchronized (mCommitLock) {
            if (mCancelled.get()) {
                return false;
            }
            action.run();
            return true;
        }
    }

    void attach(HttpURLConnection connection) {
        mConnection.set(connection);
        if (mCancelled.get() && mConnection.compareAndSet(connection, null)) {
            connection.disconnect();
        }
    }

    void detach(HttpURLConnection connection) {
        if (connection != null) {
            mConnection.compareAndSet(connection, null);
        }
    }
}
