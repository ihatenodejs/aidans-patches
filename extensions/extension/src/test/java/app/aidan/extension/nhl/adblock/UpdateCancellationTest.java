package app.aidan.extension.nhl.adblock;

import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;
import static org.junit.Assert.fail;

import org.junit.Test;

import java.io.IOException;
import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicBoolean;

public class UpdateCancellationTest {
    @Test
    public void cancelBeforeCommitPreventsAction() throws Exception {
        UpdateCancellation cancellation = new UpdateCancellation();
        cancellation.cancel();

        assertFalse(cancellation.commit(() -> fail("Action must not run after cancellation")));
    }

    @Test
    public void sequentialCommitsAreAllowedUntilCancellation() throws Exception {
        UpdateCancellation cancellation = new UpdateCancellation();
        AtomicBoolean first = new AtomicBoolean(false);
        AtomicBoolean second = new AtomicBoolean(false);

        assertTrue(cancellation.commit(() -> first.set(true)));
        assertTrue(cancellation.commit(() -> second.set(true)));
        cancellation.cancel();
        assertFalse(cancellation.commit(() -> fail("Commit must not run after cancellation")));

        assertTrue(first.get());
        assertTrue(second.get());
    }

    @Test
    public void cancelWaitsForCommitInProgress() throws Exception {
        UpdateCancellation cancellation = new UpdateCancellation();
        CountDownLatch actionStarted = new CountDownLatch(1);
        CountDownLatch releaseAction = new CountDownLatch(1);
        CountDownLatch cancelReturned = new CountDownLatch(1);
        AtomicBoolean actionCompleted = new AtomicBoolean(false);
        AtomicBoolean committed = new AtomicBoolean(false);

        Thread committer = new Thread(() -> {
            try {
                committed.set(cancellation.commit(() -> {
                    actionStarted.countDown();
                    await(releaseAction);
                    actionCompleted.set(true);
                }));
            } catch (IOException e) {
                throw new AssertionError(e);
            }
        }, "commit-thread");
        committer.start();
        assertTrue(actionStarted.await(1, TimeUnit.SECONDS));

        Thread canceller = new Thread(() -> {
            cancellation.cancel();
            cancelReturned.countDown();
        }, "cancel-thread");
        canceller.start();
        waitUntilBlocked(canceller);
        assertFalse(cancelReturned.await(50, TimeUnit.MILLISECONDS));

        releaseAction.countDown();
        committer.join(1000);
        canceller.join(1000);

        assertFalse(committer.isAlive());
        assertFalse(canceller.isAlive());
        assertTrue(actionCompleted.get());
        assertTrue(committed.get());
        assertTrue(cancelReturned.getCount() == 0);
    }

    @Test
    public void cancelWinsAfterCommitCompletes() throws Exception {
        UpdateCancellation cancellation = new UpdateCancellation();
        List<String> events = new ArrayList<>();

        assertTrue(cancellation.commit(() -> events.add("commit")));
        cancellation.cancel();
        events.add("cancel-returned");

        assertTrue(events.size() == 2);
        assertTrue("commit".equals(events.get(0)));
        assertTrue("cancel-returned".equals(events.get(1)));
    }

    @Test
    public void cancellationNeverRunsAnActionThatLostTheRace() throws Exception {
        UpdateCancellation cancellation = new UpdateCancellation();
        AtomicBoolean ran = new AtomicBoolean(false);
        CountDownLatch cancelReturned = new CountDownLatch(1);

        Thread canceller = new Thread(() -> {
            cancellation.cancel();
            cancelReturned.countDown();
        });
        canceller.start();
        assertTrue(cancelReturned.await(1, TimeUnit.SECONDS));

        assertFalse(cancellation.commit(() -> ran.set(true)));
        assertFalse(ran.get());
    }

    private static void await(CountDownLatch latch) {
        try {
            if (!latch.await(1, TimeUnit.SECONDS)) {
                throw new AssertionError("Timed out waiting for latch");
            }
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            throw new AssertionError(e);
        }
    }

    private static void waitUntilBlocked(Thread thread) throws InterruptedException {
        long deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(1);
        while (System.nanoTime() < deadline) {
            Thread.State state = thread.getState();
            if (state == Thread.State.BLOCKED || state == Thread.State.TERMINATED) {
                return;
            }
            Thread.sleep(1);
        }
        fail("Cancellation thread did not block on the in-flight commit");
    }
}
