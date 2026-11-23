package app.aidan.extension.nhl.adblock;

import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;

import org.junit.Test;

import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.atomic.AtomicInteger;

public class RuleRepositoryRefreshTest {
    @Test
    public void successfulFetchAndCompileReturnsSuccess() {
        AtomicInteger fetched = new AtomicInteger();
        AtomicInteger compiled = new AtomicInteger();

        boolean success = RuleRepository.refreshAll(
                subscriptions(subscription("user_1", true)),
                (subscription, cancellation) -> {
                    fetched.incrementAndGet();
                    return true;
                },
                cancellation -> {
                    compiled.incrementAndGet();
                    return true;
                },
                new UpdateCancellation());

        assertTrue(success);
        assertTrue(fetched.get() == 1);
        assertTrue(compiled.get() == 1);
    }

    @Test
    public void downloadFailureMakesRefreshFail() {
        boolean success = RuleRepository.refreshAll(
                subscriptions(subscription("user_1", true)),
                (subscription, cancellation) -> false,
                cancellation -> true,
                new UpdateCancellation());

        assertFalse(success);
    }

    @Test
    public void compilationFailureMakesRefreshFail() {
        boolean success = RuleRepository.refreshAll(
                subscriptions(subscription("user_1", true)),
                (subscription, cancellation) -> true,
                cancellation -> false,
                new UpdateCancellation());

        assertFalse(success);
    }

    @Test
    public void cancellationDuringFetchSkipsCompilationAndFails() {
        AtomicInteger compiled = new AtomicInteger();
        UpdateCancellation cancellation = new UpdateCancellation();

        boolean success = RuleRepository.refreshAll(
                subscriptions(subscription("user_1", true)),
                (subscription, token) -> {
                    token.cancel();
                    return false;
                },
                token -> {
                    compiled.incrementAndGet();
                    return true;
                },
                cancellation);

        assertFalse(success);
        assertTrue(compiled.get() == 0);
    }

    @Test
    public void disabledAndBuiltinSubscriptionsAreSkipped() {
        AtomicInteger fetched = new AtomicInteger();
        AtomicInteger compiled = new AtomicInteger();
        List<RuleRepository.SubscriptionInfo> subscriptions = new ArrayList<>();
        subscriptions.add(subscription("disabled", false));
        subscriptions.add(new RuleRepository.SubscriptionInfo(
                "nhl-default", "Bundled", "https://example.com/default", true, true));

        boolean success = RuleRepository.refreshAll(
                subscriptions,
                (subscription, cancellation) -> {
                    fetched.incrementAndGet();
                    return true;
                },
                cancellation -> {
                    compiled.incrementAndGet();
                    return true;
                },
                new UpdateCancellation());

        assertTrue(success);
        assertTrue(fetched.get() == 0);
        assertTrue(compiled.get() == 1);
    }

    @Test
    public void preCancelledRefreshDoesNothing() {
        AtomicInteger fetched = new AtomicInteger();
        AtomicInteger compiled = new AtomicInteger();
        UpdateCancellation cancellation = new UpdateCancellation();
        cancellation.cancel();

        boolean success = RuleRepository.refreshAll(
                subscriptions(subscription("user_1", true)),
                (subscription, token) -> {
                    fetched.incrementAndGet();
                    return true;
                },
                token -> {
                    compiled.incrementAndGet();
                    return true;
                },
                cancellation);

        assertFalse(success);
        assertTrue(fetched.get() == 0);
        assertTrue(compiled.get() == 0);
    }

    private static RuleRepository.SubscriptionInfo subscription(String id, boolean enabled) {
        return new RuleRepository.SubscriptionInfo(
                id, id, "https://example.com/" + id + ".txt", false, enabled);
    }

    private static List<RuleRepository.SubscriptionInfo> subscriptions(
            RuleRepository.SubscriptionInfo... subscriptions) {
        List<RuleRepository.SubscriptionInfo> list = new ArrayList<>();
        for (RuleRepository.SubscriptionInfo subscription : subscriptions) {
            list.add(subscription);
        }
        return list;
    }
}
