package app.aidan.extension.fizz;

import java.lang.reflect.Field;
import java.util.ArrayList;
import java.util.List;

public final class FeedFilterBridge {
    private FeedFilterBridge() {
    }

    /**
     * Filters out both commercial advertisements and marketplace listings.
     */
    public static List filterDisplayItems(List items) {
        return filter(items, true);
    }

    /**
     * Filters out commercial advertisements only, keeping marketplace listings.
     */
    public static List filterAdsOnly(List items) {
        return filter(items, false);
    }

    private static List filter(List items, boolean removeMarketplace) {
        if (items == null || items.isEmpty()) {
            return items;
        }
        ArrayList result = new ArrayList(items.size());
        for (int i = 0; i < items.size(); i++) {
            Object item = items.get(i);
            if (item == null) {
                continue;
            }
            String className = item.getClass().getName();
            // Marketplace listing card is rc.t1
            if (removeMarketplace && "rc.t1".equals(className)) {
                continue;
            }
            // Commercial announcement / ad item is rc.k
            if ("rc.k".equals(className)) {
                try {
                    Field typeField = item.getClass().getDeclaredField("b");
                    typeField.setAccessible(true);
                    Object typeVal = typeField.get(item);
                    if (typeVal != null && "Advertisement".equals(((Enum<?>) typeVal).name())) {
                        continue;
                    }
                } catch (Throwable ignored) {
                }
            }
            result.add(item);
        }
        return result;
    }
}
