package app.aidan.extension.nhl.adblock;

import android.app.job.JobParameters;
import android.app.job.JobService;
import android.os.Handler;
import android.os.Looper;

import java.util.concurrent.atomic.AtomicBoolean;

/**
 * Persistent JobScheduler entry point. Work is delegated to the repository's single-threaded
 * executor; completion is always marshalled to the main thread, which also owns lifecycle
 * state for start/stop races.
 */
public final class AdBlockUpdateJobService extends JobService {
    private static final Handler MAIN_HANDLER = new Handler(Looper.getMainLooper());

    private final AtomicBoolean mStopped = new AtomicBoolean(true);
    private volatile JobParameters mRunningParams;

    @Override
    public boolean onStartJob(JobParameters params) {
        mRunningParams = params;
        mStopped.set(false);
        AdBlockAutoUpdateScheduler.runScheduled(this, success ->
                MAIN_HANDLER.post(() -> finish(params, success)));
        return true;
    }

    private void finish(JobParameters params, boolean success) {
        if (mStopped.get() || mRunningParams != params) {
            return;
        }
        mRunningParams = null;
        mStopped.set(true);
        jobFinished(params, !success);
    }

    @Override
    public boolean onStopJob(JobParameters params) {
        if (mRunningParams == params) {
            mStopped.set(true);
            mRunningParams = null;
            AdBlockAutoUpdateScheduler.cancelRunning(this);
        }
        // The preference and network constraint are re-checked on the next execution.
        return true;
    }
}
