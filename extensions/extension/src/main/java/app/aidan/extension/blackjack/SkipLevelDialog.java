package app.aidan.extension.blackjack;

import android.app.Activity;
import android.app.AlertDialog;
import android.content.DialogInterface;
import android.util.Log;
import android.view.MotionEvent;
import android.view.Window;
import java.io.File;
import java.io.FileInputStream;
import java.lang.reflect.InvocationHandler;
import java.lang.reflect.Method;
import java.lang.reflect.Proxy;
import org.json.JSONObject;

public final class SkipLevelDialog {
    private static final String TAG = "SkipLevelDialog";
    private static long lastDialogTime = 0L;

    private SkipLevelDialog() {
    }

    public static void install(final Activity activity) {
        if (activity == null) {
            return;
        }

        activity.runOnUiThread(new Runnable() {
            @Override
            public void run() {
                try {
                    final Window window = activity.getWindow();
                    if (window == null) {
                        Log.w(TAG, "Window is null, cannot install touch interceptor");
                        return;
                    }

                    final Window.Callback originalCallback = window.getCallback();
                    if (originalCallback == null) {
                        Log.w(TAG, "Original Window.Callback is null");
                        return;
                    }

                    InvocationHandler handler = new InvocationHandler() {
                        @Override
                        public Object invoke(Object proxy, Method method, Object[] args) throws Throwable {
                            if ("dispatchTouchEvent".equals(method.getName()) && args != null && args.length == 1) {
                                MotionEvent ev = (MotionEvent) args[0];
                                if (handleTouch(activity, ev)) {
                                    return Boolean.TRUE;
                                }
                            }
                            return method.invoke(originalCallback, args);
                        }
                    };

                    Window.Callback proxyCallback = (Window.Callback) Proxy.newProxyInstance(
                            Window.Callback.class.getClassLoader(),
                            new Class<?>[]{Window.Callback.class},
                            handler
                    );
                    window.setCallback(proxyCallback);
                    Log.i(TAG, "Successfully installed touch interceptor for Skip to Next Level");
                } catch (Throwable t) {
                    Log.e(TAG, "Failed to install touch interceptor", t);
                }
            }
        });
    }

    public static boolean handleTouch(Activity activity, MotionEvent event) {
        if (event == null || event.getAction() != MotionEvent.ACTION_UP) {
            return false;
        }

        long now = System.currentTimeMillis();
        if (now - lastDialogTime < 1500L) {
            return false;
        }

        int w = activity.getResources().getDisplayMetrics().widthPixels;
        int h = activity.getResources().getDisplayMetrics().heightPixels;
        if (w <= 0 || h <= 0) {
            return false;
        }

        float normX = event.getX() / (float) w;
        float normY = event.getY() / (float) h;

        // Next level circle in portrait mode:
        // Center is located at approximately X = 80.1%, Y = 8.1% of display.
        if (normX >= 0.74f && normX <= 0.88f && normY >= 0.04f && normY <= 0.14f) {
            lastDialogTime = now;
            show(activity);
            return true;
        }

        return false;
    }

    public static void show(final Activity activity) {
        if (activity == null || activity.isFinishing()) {
            return;
        }

        activity.runOnUiThread(new Runnable() {
            @Override
            public void run() {
                int currentLevel = loadCurrentLevel(activity);
                final int nextLevel = currentLevel + 1;

                new AlertDialog.Builder(activity)
                        .setTitle("Skip to Next Level")
                        .setMessage("Do you want to skip to Level " + nextLevel + "?")
                        .setPositiveButton("Skip", new DialogInterface.OnClickListener() {
                            @Override
                            public void onClick(DialogInterface dialog, int which) {
                                skipLevel();
                            }
                        })
                        .setNegativeButton("Cancel", null)
                        .show();
            }
        });
    }

    private static int loadCurrentLevel(Activity activity) {
        File[] candidates = new File[]{
                new File("/sdcard/Android/data/com.tripledot.blackjack/files/SimpleStorage/PlayerData.json"),
                new File(activity.getExternalFilesDir(null), "SimpleStorage/PlayerData.json"),
                new File(activity.getFilesDir(), "SimpleStorage/PlayerData.json")
        };

        for (File file : candidates) {
            if (!file.exists()) {
                continue;
            }

            try (FileInputStream fis = new FileInputStream(file)) {
                byte[] data = new byte[(int) file.length()];
                int read = fis.read(data);
                String jsonStr = new String(data, 0, read, "UTF-8");
                JSONObject json = new JSONObject(jsonStr);
                JSONObject valueObj = json.getJSONObject("value");
                return valueObj.getInt("Level");
            } catch (Throwable t) {
                Log.e(TAG, "Failed reading " + file.getAbsolutePath(), t);
            }
        }

        return 1;
    }

    private static void skipLevel() {
        try {
            Log.i(TAG, "Sending Unity message to skip level");
            Class<?> unityPlayerClass = Class.forName("com.unity3d.player.UnityPlayer");
            Method sendMethod = unityPlayerClass.getMethod(
                    "UnitySendMessage",
                    String.class,
                    String.class,
                    String.class
            );
            sendMethod.invoke(null, "BlackjackApplication", "CheckUpdateToVersion", "skip_level");
        } catch (Throwable t) {
            Log.e(TAG, "Failed to send Unity message to skip level", t);
        }
    }
}
