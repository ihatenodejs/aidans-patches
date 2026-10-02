package app.aidan.extension.aftership;

import android.content.Context;
import android.content.res.Resources;
import android.os.Build;
import android.os.Handler;
import android.os.Looper;
import android.util.Log;
import android.view.View;
import android.view.ViewGroup;
import android.view.WindowInsets;
import android.graphics.Insets;
import android.widget.FrameLayout;

import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.util.ArrayList;
import java.util.List;

public final class OsmMapBridge {
    private static final String TAG = "AfterShipOsmBridge";
    private static final Handler MAIN_HANDLER = new Handler(Looper.getMainLooper());

    private OsmMapBridge() {
    }

    public static void updateMap(final Object fragmentObj) {
        updateMap(fragmentObj, false);
    }

    public static void updateMap(final Object fragmentObj, final boolean showZoomButtons) {
        if (fragmentObj == null) {
            return;
        }

        View rootView = resolveRootView(fragmentObj);
        if (rootView == null) {
            MAIN_HANDLER.post(new Runnable() {
                @Override
                public void run() {
                    updateMapInternal(fragmentObj, showZoomButtons);
                }
            });
            return;
        }

        updateMapInternal(fragmentObj, showZoomButtons);
    }

    private static void updateMapInternal(final Object fragmentObj, final boolean showZoomButtons) {
        try {
            final View rootView = resolveRootView(fragmentObj);
            if (rootView == null) {
                return;
            }

            Context context = rootView.getContext();
            ViewGroup container = findTrackingMapContainer(rootView);
            if (container == null) {
                Log.w(TAG, "tracking_map_container not found in root view hierarchy");
                return;
            }

            View defaultImg = findViewByIdName(rootView, "tracking_map_default_img");
            View tipsTv = findViewByIdName(rootView, "tracking_map_tips_tv");

            Object viewModel = resolveViewModel(fragmentObj);
            if (viewModel == null) {
                Log.w(TAG, "ViewModel not found on fragment");
                return;
            }

            List<double[]> coordinates = extractCoordinatesFromViewModel(viewModel);
            Log.d(TAG, "Extracted " + coordinates.size() + " coordinates for OSM map");

            if (coordinates.isEmpty()) {
                if (defaultImg != null) {
                    defaultImg.setVisibility(View.VISIBLE);
                }
                if (tipsTv != null) {
                    tipsTv.setVisibility(View.VISIBLE);
                }
                OsmMapView existingMap = findOsmMapView(container);
                if (existingMap != null) {
                    existingMap.setVisibility(View.GONE);
                }
                return;
            }

            if (defaultImg != null) {
                defaultImg.setVisibility(View.GONE);
            }
            if (tipsTv != null) {
                tipsTv.setVisibility(View.GONE);
            }

            int color = resolveColor(viewModel);
            int bottomOffset = resolveBottomOffset(viewModel);
            int topOffsetCss = resolveTopOffsetCss(rootView);

            final OsmMapView osmMapView = getOrCreateOsmMapView(container, context);
            osmMapView.setVisibility(View.VISIBLE);
            osmMapView.renderRoute(coordinates, color, topOffsetCss, bottomOffset, showZoomButtons);

            // Re-measure after layout in case toolbar measurements were still pending
            rootView.post(new Runnable() {
                @Override
                public void run() {
                    int updatedTopOffsetCss = resolveTopOffsetCss(rootView);
                    Object vm = resolveViewModel(fragmentObj);
                    if (vm != null) {
                        List<double[]> coords = extractCoordinatesFromViewModel(vm);
                        int c = resolveColor(vm);
                        int b = resolveBottomOffset(vm);
                        osmMapView.renderRoute(coords, c, updatedTopOffsetCss, b, showZoomButtons);
                    }
                }
            });

        } catch (Exception e) {
            Log.e(TAG, "Error updating OSM map bridge", e);
        }
    }

    private static int resolveTopOffsetCss(View rootView) {
        if (rootView == null) return 120;
        Resources res = rootView.getResources();
        float density = res.getDisplayMetrics().density;
        if (density <= 0) density = 1.0f;

        int topOffsetPx = 0;

        // 1. Try finding the actual toolbar in the window hierarchy
        View root = rootView.getRootView();
        View toolbar = findViewByIdName(root, "tracking_detail_toolbar_rl");
        if (toolbar == null) {
            toolbar = findViewByIdName(root, "tracking_detail_toolbar");
        }
        if (toolbar == null) {
            toolbar = findViewByIdName(root, "layout_tracking_detail_title");
        }

        if (toolbar != null && toolbar.getHeight() > 0) {
            int[] loc = new int[2];
            toolbar.getLocationOnScreen(loc);
            topOffsetPx = loc[1] + toolbar.getHeight();
        }

        // 2. Fallback: derive from status bar height + 56dp action bar
        if (topOffsetPx <= 0) {
            int statusBarHeight = 0;
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R && rootView.getRootWindowInsets() != null) {
                Insets insets = rootView.getRootWindowInsets().getInsetsIgnoringVisibility(
                        WindowInsets.Type.statusBars() | WindowInsets.Type.displayCutout()
                );
                statusBarHeight = insets.top;
            }
            if (statusBarHeight <= 0) {
                int resourceId = res.getIdentifier("status_bar_height", "dimen", "android");
                if (resourceId > 0) {
                    statusBarHeight = res.getDimensionPixelSize(resourceId);
                } else {
                    statusBarHeight = (int) (36 * density);
                }
            }
            int toolbarHeight = (int) (56 * density);
            topOffsetPx = statusBarHeight + toolbarHeight;
        }

        // Add 16dp clearance below toolbar
        topOffsetPx += (int) (16 * density);

        return (int) (topOffsetPx / density);
    }

    private static View resolveRootView(Object fragmentObj) {
        if (fragmentObj == null) return null;
        try {
            Method getViewMethod = fragmentObj.getClass().getMethod("getView");
            View v = (View) getViewMethod.invoke(fragmentObj);
            if (v != null) {
                return v;
            }
        } catch (Exception ignored) {
        }

        for (Field f : fragmentObj.getClass().getDeclaredFields()) {
            try {
                f.setAccessible(true);
                Object val = f.get(fragmentObj);
                if (val instanceof View) {
                    return (View) val;
                }
                if (val != null) {
                    for (Field bf : val.getClass().getDeclaredFields()) {
                        bf.setAccessible(true);
                        Object bval = bf.get(val);
                        if (bval instanceof View) {
                            return (View) bval;
                        }
                    }
                }
            } catch (Exception ignored) {
            }
        }
        return null;
    }

    private static ViewGroup findTrackingMapContainer(View root) {
        View v = findViewByIdName(root, "tracking_map_container");
        if (v instanceof ViewGroup) {
            return (ViewGroup) v;
        }
        if (root instanceof ViewGroup) {
            return findViewGroupByCriteria((ViewGroup) root);
        }
        return null;
    }

    private static ViewGroup findViewGroupByCriteria(ViewGroup root) {
        for (int i = 0; i < root.getChildCount(); i++) {
            View child = root.getChildAt(i);
            if (child instanceof FrameLayout) {
                int id = child.getId();
                if (id != View.NO_ID) {
                    try {
                        String name = root.getResources().getResourceEntryName(id);
                        if ("tracking_map_container".equals(name)) {
                            return (ViewGroup) child;
                        }
                    } catch (Exception ignored) {
                    }
                }
            }
            if (child instanceof ViewGroup) {
                ViewGroup res = findViewGroupByCriteria((ViewGroup) child);
                if (res != null) {
                    return res;
                }
            }
        }
        return null;
    }

    private static View findViewByIdName(View root, String name) {
        if (root == null) return null;
        try {
            Resources res = root.getResources();
            int id = res.getIdentifier(name, "id", root.getContext().getPackageName());
            if (id != 0) {
                View target = root.findViewById(id);
                if (target != null) return target;
            }
        } catch (Exception ignored) {
        }

        if (root.getId() != View.NO_ID) {
            try {
                String entry = root.getResources().getResourceEntryName(root.getId());
                if (name.equals(entry)) return root;
            } catch (Exception ignored) {
            }
        }

        if (root instanceof ViewGroup) {
            ViewGroup vg = (ViewGroup) root;
            for (int i = 0; i < vg.getChildCount(); i++) {
                View v = findViewByIdName(vg.getChildAt(i), name);
                if (v != null) return v;
            }
        }
        return null;
    }

    private static Object resolveViewModel(Object fragmentObj) {
        try {
            Method m = fragmentObj.getClass().getMethod("a3");
            return m.invoke(fragmentObj);
        } catch (Exception ignored) {
        }

        for (Method m : fragmentObj.getClass().getDeclaredMethods()) {
            if (m.getParameterTypes().length == 0 && m.getReturnType().getName().contains("TrackingMapViewModel")) {
                try {
                    m.setAccessible(true);
                    return m.invoke(fragmentObj);
                } catch (Exception ignored) {
                }
            }
        }
        return null;
    }

    private static int resolveColor(Object viewModel) {
        try {
            Method eMethod = viewModel.getClass().getMethod("e");
            return (Integer) eMethod.invoke(viewModel);
        } catch (Exception ignored) {
        }
        return 0x5B7BFE;
    }

    private static int resolveBottomOffset(Object viewModel) {
        for (Field f : viewModel.getClass().getDeclaredFields()) {
            if (f.getType() == int.class) {
                try {
                    f.setAccessible(true);
                    int val = f.getInt(viewModel);
                    if (val > 0 && val < 2000) {
                        return val;
                    }
                } catch (Exception ignored) {
                }
            }
        }
        return 120;
    }

    private static OsmMapView findOsmMapView(ViewGroup container) {
        for (int i = 0; i < container.getChildCount(); i++) {
            View child = container.getChildAt(i);
            if (child instanceof OsmMapView) {
                return (OsmMapView) child;
            }
        }
        return null;
    }

    private static OsmMapView getOrCreateOsmMapView(ViewGroup container, Context context) {
        OsmMapView osmMapView = findOsmMapView(container);
        if (osmMapView != null) {
            return osmMapView;
        }
        osmMapView = new OsmMapView(context);
        container.addView(osmMapView, new FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.MATCH_PARENT
        ));
        return osmMapView;
    }

    private static List<double[]> extractCoordinatesFromViewModel(Object viewModel) {
        List<double[]> result = new ArrayList<>();
        try {
            Method fMethod = viewModel.getClass().getMethod("f");
            List<?> list = (List<?>) fMethod.invoke(viewModel);
            if (list == null) return result;

            for (Object checkpoint : list) {
                if (checkpoint == null) continue;
                double[] pt = extractPointFromCheckpoint(checkpoint);
                if (pt != null) {
                    result.add(pt);
                }
            }
        } catch (Exception e) {
            Log.e(TAG, "Failed extracting coordinates from ViewModel", e);
        }
        return result;
    }

    private static double[] extractPointFromCheckpoint(Object checkpoint) {
        for (Field f : checkpoint.getClass().getDeclaredFields()) {
            try {
                f.setAccessible(true);
                Object geoObj = f.get(checkpoint);
                if (geoObj == null || geoObj.getClass().isPrimitive() || geoObj instanceof String || geoObj instanceof Number) {
                    continue;
                }

                String latStr = null;
                String lngStr = null;
                for (Field gf : geoObj.getClass().getDeclaredFields()) {
                    gf.setAccessible(true);
                    Object gval = gf.get(geoObj);
                    if (gval instanceof String) {
                        String s = ((String) gval).trim();
                        if (s.isEmpty()) continue;
                        try {
                            double d = Double.parseDouble(s);
                            if (latStr == null && d >= -90.0 && d <= 90.0) {
                                latStr = s;
                            } else if (lngStr == null && d >= -180.0 && d <= 180.0) {
                                lngStr = s;
                            }
                        } catch (NumberFormatException ignored) {
                        }
                    }
                }

                if (latStr != null && lngStr != null) {
                    return new double[]{Double.parseDouble(latStr), Double.parseDouble(lngStr)};
                }
            } catch (Exception ignored) {
            }
        }
        return null;
    }
}
