package app.aidan.extension.nhl.adblock;

import android.app.job.JobInfo;
import android.app.job.JobScheduler;
import android.content.ComponentName;
import android.content.Context;
import android.net.ConnectivityManager;
import android.net.Network;
import android.net.NetworkCapabilities;
import android.util.Log;

import java.util.List;
import java.util.concurrent.atomic.AtomicBoolean;

/**
 * Owns the restart-safe automatic subscription refresh. The persisted JobScheduler job is
 * network constrained and re-checks the user preference every time it executes.
 */
public final class AdBlockAutoUpdateScheduler {
    private static final String TAG = "AidanAdBlockScheduler";

    public static final long UPDATE_INTERVAL_MS = 12L * 60L * 60L * 1000L;
    static final int JOB_ID = 0x4E484C01;

    interface Backend {
        boolean isScheduled();

        boolean schedule(boolean requiresNetwork);

        void cancel();
    }

    interface Preference {
        boolean isEnabled();
    }

    interface NetworkStatus {
        boolean isConnected();
    }

    interface Refresher {
        interface Callback {
            void onComplete(boolean success);
        }

        void refresh(Callback callback, UpdateCancellation cancellation);
    }

    private static volatile AdBlockAutoUpdateScheduler sInstance;

    private final Backend mBackend;
    private final Preference mPreference;
    private final NetworkStatus mNetwork;
    private final Refresher mRefresher;
    private final AtomicBoolean mRunning = new AtomicBoolean(false);
    private final Object mLifecycleLock = new Object();
    private volatile UpdateCancellation mActiveCancellation;

    AdBlockAutoUpdateScheduler(
            Backend backend,
            Preference preference,
            NetworkStatus network,
            Refresher refresher) {
        mBackend = backend;
        mPreference = preference;
        mNetwork = network;
        mRefresher = refresher;
    }

    public static AdBlockAutoUpdateScheduler get(Context context) {
        if (sInstance == null) {
            synchronized (AdBlockAutoUpdateScheduler.class) {
                if (sInstance == null) {
                    Context app = context.getApplicationContext() != null
                            ? context.getApplicationContext()
                            : context;
                    sInstance = new AdBlockAutoUpdateScheduler(
                            new JobSchedulerBackend(app),
                            () -> AdBlockStore.get(app).isAutoUpdate(),
                            new ConnectivityNetwork(app),
                            (callback, cancellation) -> RuleRepository.get(app)
                                    .updateAllSubscriptionsAsync(cancellation, callback::onComplete));
                }
            }
        }
        return sInstance;
    }

    /**
     * Persists the preference and reconciles the scheduled job. Returns {@code false} when
     * scheduling was requested but the platform rejected it.
     */
    public static boolean setEnabled(Context context, boolean enabled) {
        AdBlockStore.get(context).setAutoUpdate(enabled);
        return get(context).sync();
    }

    /**
     * Re-establishes or cancels the job from the persisted preference. Safe to call on every
     * app/settings launch and after reboot.
     */
    public static boolean syncFromPreference(Context context) {
        return get(context).sync();
    }

    public static void runScheduled(Context context, Refresher.Callback callback) {
        get(context).run(callback);
    }

    public static void cancelRunning(Context context) {
        get(context).cancel();
    }

    boolean sync() {
        if (!mPreference.isEnabled()) {
            mBackend.cancel();
            return true;
        }
        if (mBackend.isScheduled()) {
            return true;
        }
        return mBackend.schedule(true);
    }

    void run(Refresher.Callback callback) {
        if (!mPreference.isEnabled()) {
            if (callback != null) callback.onComplete(true);
            return;
        }
        if (!mNetwork.isConnected()) {
            if (callback != null) callback.onComplete(false);
            return;
        }
        final UpdateCancellation cancellation = beginRun();
        if (cancellation == null) {
            if (callback != null) callback.onComplete(false);
            return;
        }

        try {
            mRefresher.refresh(success -> {
                if (!completeRun(cancellation)) {
                    return;
                }
                if (cancellation.isCancelled()) {
                    return;
                }
                if (callback != null) callback.onComplete(success);
            }, cancellation);
        } catch (RuntimeException e) {
            boolean owned = completeRun(cancellation);
            Log.w(TAG, "Automatic subscription refresh failed to start", e);
            if (owned && !cancellation.isCancelled() && callback != null) {
                callback.onComplete(false);
            }
        }
    }

    private UpdateCancellation beginRun() {
        synchronized (mLifecycleLock) {
            if (!mRunning.compareAndSet(false, true)) {
                return null;
            }
            UpdateCancellation cancellation = new UpdateCancellation();
            mActiveCancellation = cancellation;
            return cancellation;
        }
    }

    /**
     * Clears the completed run's ownership before releasing the single-flight state. Returns
     * false for a stale callback whose run no longer owns the active token.
     */
    boolean completeRun(UpdateCancellation cancellation) {
        synchronized (mLifecycleLock) {
            if (mActiveCancellation != cancellation) {
                return false;
            }
            mActiveCancellation = null;
            mRunning.set(false);
            return true;
        }
    }

    void cancel() {
        synchronized (mLifecycleLock) {
            if (mActiveCancellation != null) {
                mActiveCancellation.cancel();
            }
        }
    }

    static final class JobSchedulerBackend implements Backend {
        private final Context mContext;

        JobSchedulerBackend(Context context) {
            mContext = context;
        }

        @Override
        public boolean isScheduled() {
            JobScheduler scheduler = getScheduler();
            if (scheduler == null) return false;
            try {
                List<JobInfo> jobs = scheduler.getAllPendingJobs();
                if (jobs == null) return false;
                for (JobInfo job : jobs) {
                    if (job.getId() == JOB_ID) return true;
                }
            } catch (RuntimeException e) {
                Log.w(TAG, "Unable to inspect scheduled update jobs", e);
            }
            return false;
        }

        @Override
        public boolean schedule(boolean requiresNetwork) {
            JobScheduler scheduler = getScheduler();
            if (scheduler == null) return false;
            try {
                JobInfo.Builder builder = new JobInfo.Builder(
                        JOB_ID,
                        new ComponentName(mContext, AdBlockUpdateJobService.class))
                        .setPeriodic(UPDATE_INTERVAL_MS)
                        .setPersisted(true);
                if (requiresNetwork) {
                    builder.setRequiredNetworkType(JobInfo.NETWORK_TYPE_ANY);
                }
                return scheduler.schedule(builder.build()) == JobScheduler.RESULT_SUCCESS;
            } catch (RuntimeException e) {
                Log.w(TAG, "Unable to schedule automatic subscription updates", e);
                return false;
            }
        }

        @Override
        public void cancel() {
            JobScheduler scheduler = getScheduler();
            if (scheduler == null) return;
            try {
                scheduler.cancel(JOB_ID);
            } catch (RuntimeException e) {
                Log.w(TAG, "Unable to cancel automatic subscription updates", e);
            }
        }

        private JobScheduler getScheduler() {
            return (JobScheduler) mContext.getSystemService(Context.JOB_SCHEDULER_SERVICE);
        }
    }

    static final class ConnectivityNetwork implements NetworkStatus {
        private final Context mContext;

        ConnectivityNetwork(Context context) {
            mContext = context;
        }

        @Override
        public boolean isConnected() {
            ConnectivityManager manager =
                    (ConnectivityManager) mContext.getSystemService(Context.CONNECTIVITY_SERVICE);
            if (manager == null) return false;
            Network network = manager.getActiveNetwork();
            if (network == null) return false;
            NetworkCapabilities capabilities = manager.getNetworkCapabilities(network);
            return capabilities != null
                    && capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET);
        }
    }
}
