package app.aidan.extension.nhl.adblock;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;

import org.junit.Test;

import java.util.ArrayList;
import java.util.List;

public class AdBlockAutoUpdateSchedulerTest {
    @Test
    public void enablingSchedulesOneNetworkConstrainedJob() {
        FakeBackend backend = new FakeBackend();
        FakePreference preference = new FakePreference(true);
        CountingRefresher refresher = new CountingRefresher();
        AdBlockAutoUpdateScheduler scheduler = scheduler(backend, preference, new FakeNetwork(true), refresher);

        assertTrue(scheduler.sync());

        assertEquals(1, backend.scheduleCalls);
        assertTrue(backend.lastRequiresNetwork);
        assertTrue(backend.scheduled);
        assertEquals(0, backend.cancelCalls);
    }

    @Test
    public void disablingCancelsTheJob() {
        FakeBackend backend = new FakeBackend();
        backend.scheduled = true;
        FakePreference preference = new FakePreference(false);
        AdBlockAutoUpdateScheduler scheduler =
                scheduler(backend, preference, new FakeNetwork(true), new CountingRefresher());

        assertTrue(scheduler.sync());

        assertEquals(1, backend.cancelCalls);
        assertFalse(backend.scheduled);
        assertEquals(0, backend.scheduleCalls);
    }

    @Test
    public void repeatedTogglesReconcileScheduleState() {
        FakeBackend backend = new FakeBackend();
        FakePreference preference = new FakePreference(true);
        AdBlockAutoUpdateScheduler scheduler =
                scheduler(backend, preference, new FakeNetwork(true), new CountingRefresher());

        scheduler.sync();
        preference.enabled = false;
        scheduler.sync();
        preference.enabled = true;
        scheduler.sync();

        assertEquals(2, backend.scheduleCalls);
        assertEquals(1, backend.cancelCalls);
        assertTrue(backend.scheduled);
    }

    @Test
    public void reinitializationDoesNotDuplicateThePersistedJob() {
        FakeBackend backend = new FakeBackend();
        FakePreference preference = new FakePreference(true);
        AdBlockAutoUpdateScheduler first =
                scheduler(backend, preference, new FakeNetwork(true), new CountingRefresher());
        first.sync();

        AdBlockAutoUpdateScheduler second =
                scheduler(backend, preference, new FakeNetwork(true), new CountingRefresher());
        assertTrue(second.sync());

        assertEquals(1, backend.scheduleCalls);
    }

    @Test
    public void offlineExecutionDoesNotStartARefresh() {
        CountingRefresher refresher = new CountingRefresher();
        AdBlockAutoUpdateScheduler scheduler =
                scheduler(new FakeBackend(), new FakePreference(true), new FakeNetwork(false), refresher);
        final boolean[] result = new boolean[] { true };

        scheduler.run(success -> result[0] = success);

        assertEquals(0, refresher.calls);
        assertFalse(result[0]);
    }

    @Test
    public void disabledPreferenceSkipsExecutionEvenWhenScheduled() {
        CountingRefresher refresher = new CountingRefresher();
        AdBlockAutoUpdateScheduler scheduler =
                scheduler(new FakeBackend(), new FakePreference(false), new FakeNetwork(true), refresher);
        final boolean[] result = new boolean[] { false };

        scheduler.run(success -> result[0] = success);

        assertEquals(0, refresher.calls);
        assertTrue(result[0]);
    }

    @Test
    public void concurrentRunIsRejectedInsteadOfOverlapping() {
        ControllableRefresher refresher = new ControllableRefresher();
        AdBlockAutoUpdateScheduler scheduler =
                scheduler(new FakeBackend(), new FakePreference(true), new FakeNetwork(true), refresher);
        final List<Boolean> results = new ArrayList<>();

        scheduler.run(results::add);
        scheduler.run(results::add);
        assertEquals(1, refresher.calls);
        assertEquals(1, results.size());
        assertFalse(results.get(0));

        refresher.callback.onComplete(true);
        assertEquals(2, results.size());
        assertTrue(results.get(1));
    }

    @Test
    public void cancelledRunSuppressesStaleCallbackAndAllowsRestart() {
        ControllableRefresher refresher = new ControllableRefresher();
        AdBlockAutoUpdateScheduler scheduler =
                scheduler(new FakeBackend(), new FakePreference(true), new FakeNetwork(true), refresher);
        final List<Boolean> results = new ArrayList<>();

        scheduler.run(results::add);
        scheduler.cancel();
        assertTrue(refresher.cancellation.isCancelled());
        refresher.callback.onComplete(true);
        assertTrue(results.isEmpty());

        scheduler.run(results::add);
        assertEquals(2, refresher.calls);
        assertTrue(results.isEmpty());
    }

    @Test
    public void staleCompletionCannotClearNewRunOwnership() {
        ControllableRefresher refresher = new ControllableRefresher();
        AdBlockAutoUpdateScheduler scheduler =
                scheduler(new FakeBackend(), new FakePreference(true), new FakeNetwork(true), refresher);

        scheduler.run(success -> {
        });
        UpdateCancellation first = refresher.cancellation;
        assertTrue(scheduler.completeRun(first));

        scheduler.run(success -> {
        });
        UpdateCancellation second = refresher.cancellation;
        assertFalse(scheduler.completeRun(first));

        scheduler.cancel();
        assertTrue(second.isCancelled());
    }

    @Test
    public void failedRefreshIsReportedAndAllowsTheNextRun() {
        ControllableRefresher refresher = new ControllableRefresher();
        AdBlockAutoUpdateScheduler scheduler =
                scheduler(new FakeBackend(), new FakePreference(true), new FakeNetwork(true), refresher);
        final List<Boolean> results = new ArrayList<>();

        scheduler.run(results::add);
        refresher.callback.onComplete(false);
        assertFalse(results.get(0));

        scheduler.run(results::add);
        assertEquals(2, refresher.calls);
    }

    @Test
    public void refusedScheduleIsReported() {
        FakeBackend backend = new FakeBackend();
        backend.allowSchedule = false;
        AdBlockAutoUpdateScheduler scheduler =
                scheduler(backend, new FakePreference(true), new FakeNetwork(true), new CountingRefresher());

        assertFalse(scheduler.sync());
        assertFalse(backend.scheduled);
    }

    private static AdBlockAutoUpdateScheduler scheduler(
            FakeBackend backend,
            FakePreference preference,
            FakeNetwork network,
            AdBlockAutoUpdateScheduler.Refresher refresher) {
        return new AdBlockAutoUpdateScheduler(backend, preference, network, refresher);
    }

    private static final class FakeBackend implements AdBlockAutoUpdateScheduler.Backend {
        boolean scheduled;
        boolean allowSchedule = true;
        boolean lastRequiresNetwork;
        int scheduleCalls;
        int cancelCalls;

        @Override
        public boolean isScheduled() {
            return scheduled;
        }

        @Override
        public boolean schedule(boolean requiresNetwork) {
            scheduleCalls++;
            lastRequiresNetwork = requiresNetwork;
            scheduled = allowSchedule;
            return allowSchedule;
        }

        @Override
        public void cancel() {
            cancelCalls++;
            scheduled = false;
        }
    }

    private static final class FakePreference implements AdBlockAutoUpdateScheduler.Preference {
        boolean enabled;

        FakePreference(boolean enabled) {
            this.enabled = enabled;
        }

        @Override
        public boolean isEnabled() {
            return enabled;
        }
    }

    private static final class FakeNetwork implements AdBlockAutoUpdateScheduler.NetworkStatus {
        private final boolean connected;

        FakeNetwork(boolean connected) {
            this.connected = connected;
        }

        @Override
        public boolean isConnected() {
            return connected;
        }
    }

    private static final class CountingRefresher implements AdBlockAutoUpdateScheduler.Refresher {
        int calls;

        @Override
        public void refresh(Callback callback, UpdateCancellation cancellation) {
            calls++;
            callback.onComplete(true);
        }
    }

    private static final class ControllableRefresher implements AdBlockAutoUpdateScheduler.Refresher {
        int calls;
        Callback callback;
        UpdateCancellation cancellation;

        @Override
        public void refresh(Callback callback, UpdateCancellation cancellation) {
            calls++;
            this.callback = callback;
            this.cancellation = cancellation;
        }
    }
}
