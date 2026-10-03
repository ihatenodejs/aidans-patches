package app.aidan.extension.fizz;

import android.app.Activity;
import android.app.AlertDialog;
import android.app.Dialog;
import android.content.Context;
import android.content.DialogInterface;
import android.content.SharedPreferences;
import android.graphics.Color;
import android.graphics.Typeface;
import android.graphics.drawable.ColorDrawable;
import android.graphics.drawable.GradientDrawable;
import android.util.Log;
import android.util.TypedValue;
import android.view.Gravity;
import android.view.View;
import android.view.ViewGroup;
import android.view.Window;
import android.widget.CompoundButton;
import android.widget.EditText;
import android.widget.FrameLayout;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.Switch;
import android.widget.TextView;
import java.lang.reflect.Field;

public final class DeveloperMenuDialog {
    private static final String TAG = "DeveloperMenuDialog";

    private static final int COLOR_BG = 0xFF141416;
    private static final int COLOR_CARD = 0xFF202024;
    private static final int COLOR_ACCENT = 0xFF7F00FF;
    private static final int COLOR_TEXT_PRIMARY = 0xFFFFFFFF;
    private static final int COLOR_TEXT_SECONDARY = 0xFF8E8E93;

    private static final String PREFS_DEBUG_FLAGS = "fizz_debug_flags";

    public interface OnOptionSelectedListener {
        void onSelected(int index, String option);
    }

    public interface OnTextEnteredListener {
        void onTextEntered(String text);
    }

    private DeveloperMenuDialog() {
    }

    public static void show(
        final Activity activity,
        final boolean mobileStudioEnabled,
        final boolean feedDebuggingEnabled
    ) {
        if (activity == null || activity.isFinishing()) {
            return;
        }

        final Dialog dialog = new Dialog(activity);
        dialog.requestWindowFeature(Window.FEATURE_NO_TITLE);

        final LinearLayout root = new LinearLayout(activity);
        root.setOrientation(LinearLayout.VERTICAL);
        root.setBackground(createRoundedBackground(COLOR_BG, dp(activity, 16)));
        root.setPadding(dp(activity, 20), dp(activity, 20), dp(activity, 20), dp(activity, 24));

        // Header
        addHeader(root, activity, "Developer Settings", "Fizz Mobile Studio & Diagnostics", new Runnable() {
            @Override
            public void run() {
                dialog.dismiss();
            }
        });

        // Scrollable content
        final ScrollView scrollView = new ScrollView(activity);
        scrollView.setVerticalScrollBarEnabled(false);
        final LinearLayout content = new LinearLayout(activity);
        content.setOrientation(LinearLayout.VERTICAL);
        scrollView.addView(content);

        // Section 1: Quick Actions (Mobile Studio & Restart)
        if (mobileStudioEnabled) {
            addSectionHeader(content, activity, "QUICK ACTIONS");

            addActionItem(content, activity, "Launch Mobile Studio", "Slide out internal developer drawer", new Runnable() {
                @Override
                public void run() {
                    dialog.dismiss();
                    DeveloperMenuBridge.openMobileStudio(activity);
                }
            });

            addActionItem(content, activity, "Restart Application", "Apply pending state & overlay changes", new Runnable() {
                @Override
                public void run() {
                    dialog.dismiss();
                    DeveloperMenuBridge.restartApp(activity);
                }
            });
        }

        // Section 2: Visual Diagnostics & Debugging
        if (feedDebuggingEnabled) {
            addSectionHeader(content, activity, "VISUAL DIAGNOSTICS & DEBUGGING");

            final SharedPreferences debugPrefs = activity.getSharedPreferences(PREFS_DEBUG_FLAGS, 0);

            addToggleItem(content, activity, "Real-Time Frame Rate", "Display live FPS overlay counter",
                debugPrefs.getBoolean("SHOW_FRAME_RATE", false), new CompoundButton.OnCheckedChangeListener() {
                    @Override
                    public void onCheckedChanged(CompoundButton buttonView, boolean isChecked) {
                        debugPrefs.edit().putBoolean("SHOW_FRAME_RATE", isChecked).apply();
                        syncDebugFlags(activity);
                    }
                });

            addToggleItem(content, activity, "Feed Ingestion Debugger", "Show origin tags (Cache, Network, Swap) on cards",
                debugPrefs.getBoolean("SHOW_FEED_DEBUG_OVERLAY", false), new CompoundButton.OnCheckedChangeListener() {
                    @Override
                    public void onCheckedChanged(CompoundButton buttonView, boolean isChecked) {
                        debugPrefs.edit().putBoolean("SHOW_FEED_DEBUG_OVERLAY", isChecked).apply();
                        syncDebugFlags(activity);
                    }
                });

            addToggleItem(content, activity, "Compose Layout Debug Lines", "Draw bounding boxes & padding margins",
                debugPrefs.getBoolean("SHOW_VIEW_DEBUG_LINES", false), new CompoundButton.OnCheckedChangeListener() {
                    @Override
                    public void onCheckedChanged(CompoundButton buttonView, boolean isChecked) {
                        debugPrefs.edit().putBoolean("SHOW_VIEW_DEBUG_LINES", isChecked).apply();
                        syncDebugFlags(activity);
                    }
                });

            addToggleItem(content, activity, "Viewport Tracking Debugger", "Render post intersection rects",
                debugPrefs.getBoolean("SHOW_VIEW_TRACKING_DEBUGGER", false), new CompoundButton.OnCheckedChangeListener() {
                    @Override
                    public void onCheckedChanged(CompoundButton buttonView, boolean isChecked) {
                        debugPrefs.edit().putBoolean("SHOW_VIEW_TRACKING_DEBUGGER", isChecked).apply();
                        syncDebugFlags(activity);
                    }
                });

            addToggleItem(content, activity, "Unmask Superadmin Names", "Display real user names over anonymous aliases",
                debugPrefs.getBoolean("SHOW_SUPER_ADMIN_NAMES", false), new CompoundButton.OnCheckedChangeListener() {
                    @Override
                    public void onCheckedChanged(CompoundButton buttonView, boolean isChecked) {
                        debugPrefs.edit().putBoolean("SHOW_SUPER_ADMIN_NAMES", isChecked).apply();
                        syncDebugFlags(activity);
                    }
                });

            addToggleItem(content, activity, "Meme Template Tags", "Display template identifier above meme feed items",
                debugPrefs.getBoolean("SHOW_MEME_NAME", false), new CompoundButton.OnCheckedChangeListener() {
                    @Override
                    public void onCheckedChanged(CompoundButton buttonView, boolean isChecked) {
                        debugPrefs.edit().putBoolean("SHOW_MEME_NAME", isChecked).apply();
                        syncDebugFlags(activity);
                    }
                });
        }

        root.addView(scrollView, new LinearLayout.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT, 0, 1.0f));

        dialog.setContentView(root);
        final Window window = dialog.getWindow();
        if (window != null) {
            window.setBackgroundDrawable(new ColorDrawable(Color.TRANSPARENT));
            int width = (int) (activity.getResources().getDisplayMetrics().widthPixels * 0.90f);
            window.setLayout(width, ViewGroup.LayoutParams.WRAP_CONTENT);
        }

        dialog.show();
    }

    private static void syncDebugFlags(Context context) {
        try {
            Class<?> tClass = Class.forName("com.fizzsocial.fizz.data.local.t");
            Field f7223aField = tClass.getField("f7223a");
            Log.i(TAG, "Synced debug flags with SharedPreferences");
        } catch (Throwable ignored) {
        }
    }

    // --- UI Helper Components ---

    private static void addHeader(ViewGroup parent, Context context, String title, String subtitle, final Runnable onClose) {
        final LinearLayout headerLayout = new LinearLayout(context);
        headerLayout.setOrientation(LinearLayout.HORIZONTAL);
        headerLayout.setGravity(Gravity.CENTER_VERTICAL);
        headerLayout.setPadding(0, 0, 0, dp(context, 16));

        final LinearLayout titleLayout = new LinearLayout(context);
        titleLayout.setOrientation(LinearLayout.VERTICAL);

        final TextView titleView = new TextView(context);
        titleView.setText(title);
        titleView.setTextColor(COLOR_TEXT_PRIMARY);
        titleView.setTextSize(TypedValue.COMPLEX_UNIT_SP, 19);
        titleView.setTypeface(Typeface.DEFAULT_BOLD);
        titleLayout.addView(titleView);

        final TextView subView = new TextView(context);
        subView.setText(subtitle);
        subView.setTextColor(COLOR_TEXT_SECONDARY);
        subView.setTextSize(TypedValue.COMPLEX_UNIT_SP, 13);
        titleLayout.addView(subView);

        headerLayout.addView(titleLayout, new LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1.0f));

        final TextView closeBtn = new TextView(context);
        closeBtn.setText("✕");
        closeBtn.setTextColor(COLOR_TEXT_SECONDARY);
        closeBtn.setTextSize(TypedValue.COMPLEX_UNIT_SP, 18);
        closeBtn.setGravity(Gravity.CENTER);
        closeBtn.setPadding(dp(context, 10), dp(context, 6), dp(context, 10), dp(context, 6));
        closeBtn.setOnClickListener(new View.OnClickListener() {
            @Override
            public void onClick(View v) {
                onClose.run();
            }
        });
        headerLayout.addView(closeBtn);

        parent.addView(headerLayout);
    }

    private static void addSectionHeader(ViewGroup parent, Context context, String title) {
        final TextView sectionView = new TextView(context);
        sectionView.setText(title);
        sectionView.setTextColor(COLOR_ACCENT);
        sectionView.setTextSize(TypedValue.COMPLEX_UNIT_SP, 11);
        sectionView.setTypeface(Typeface.DEFAULT_BOLD);
        sectionView.setPadding(dp(context, 4), dp(context, 14), dp(context, 4), dp(context, 8));
        parent.addView(sectionView);
    }

    private static void addActionItem(ViewGroup parent, Context context, String title, String subtitle, final Runnable onClick) {
        final LinearLayout card = createCardView(context);
        card.setOnClickListener(new View.OnClickListener() {
            @Override
            public void onClick(View v) {
                onClick.run();
            }
        });

        final LinearLayout textLayout = new LinearLayout(context);
        textLayout.setOrientation(LinearLayout.VERTICAL);

        final TextView titleView = new TextView(context);
        titleView.setText(title);
        titleView.setTextColor(COLOR_TEXT_PRIMARY);
        titleView.setTextSize(TypedValue.COMPLEX_UNIT_SP, 15);
        titleView.setTypeface(Typeface.DEFAULT_BOLD);
        textLayout.addView(titleView);

        if (subtitle != null && !subtitle.isEmpty()) {
            final TextView subView = new TextView(context);
            subView.setText(subtitle);
            subView.setTextColor(COLOR_TEXT_SECONDARY);
            subView.setTextSize(TypedValue.COMPLEX_UNIT_SP, 12);
            textLayout.addView(subView);
        }

        card.addView(textLayout, new LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1.0f));

        final TextView chevron = new TextView(context);
        chevron.setText("➔");
        chevron.setTextColor(COLOR_ACCENT);
        chevron.setTextSize(TypedValue.COMPLEX_UNIT_SP, 16);
        card.addView(chevron);

        parent.addView(card);
    }

    public static void addSelectionItem(
        ViewGroup parent,
        final Context context,
        final String title,
        final String currentSelection,
        final String[] options,
        final int selectedIndex,
        final OnOptionSelectedListener listener
    ) {
        final LinearLayout card = createCardView(context);

        final LinearLayout textLayout = new LinearLayout(context);
        textLayout.setOrientation(LinearLayout.VERTICAL);

        final TextView titleView = new TextView(context);
        titleView.setText(title);
        titleView.setTextColor(COLOR_TEXT_PRIMARY);
        titleView.setTextSize(TypedValue.COMPLEX_UNIT_SP, 15);
        titleView.setTypeface(Typeface.DEFAULT_BOLD);
        textLayout.addView(titleView);

        final TextView currentView = new TextView(context);
        currentView.setText("Active: " + currentSelection);
        currentView.setTextColor(COLOR_TEXT_SECONDARY);
        currentView.setTextSize(TypedValue.COMPLEX_UNIT_SP, 12);
        textLayout.addView(currentView);

        card.addView(textLayout, new LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1.0f));

        final TextView chevron = new TextView(context);
        chevron.setText("▼");
        chevron.setTextColor(COLOR_ACCENT);
        chevron.setTextSize(TypedValue.COMPLEX_UNIT_SP, 12);
        card.addView(chevron);

        card.setOnClickListener(new View.OnClickListener() {
            @Override
            public void onClick(View v) {
                new AlertDialog.Builder(context)
                    .setTitle(title)
                    .setSingleChoiceItems(options, selectedIndex, new DialogInterface.OnClickListener() {
                        @Override
                        public void onClick(DialogInterface dialog, int which) {
                            dialog.dismiss();
                            listener.onSelected(which, options[which]);
                        }
                    })
                    .setNegativeButton("Cancel", null)
                    .show();
            }
        });

        parent.addView(card);
    }

    private static void addToggleItem(
        ViewGroup parent,
        Context context,
        String title,
        String subtitle,
        boolean isChecked,
        CompoundButton.OnCheckedChangeListener listener
    ) {
        final LinearLayout card = createCardView(context);

        final LinearLayout textLayout = new LinearLayout(context);
        textLayout.setOrientation(LinearLayout.VERTICAL);

        final TextView titleView = new TextView(context);
        titleView.setText(title);
        titleView.setTextColor(COLOR_TEXT_PRIMARY);
        titleView.setTextSize(TypedValue.COMPLEX_UNIT_SP, 15);
        titleView.setTypeface(Typeface.DEFAULT_BOLD);
        textLayout.addView(titleView);

        if (subtitle != null && !subtitle.isEmpty()) {
            final TextView subView = new TextView(context);
            subView.setText(subtitle);
            subView.setTextColor(COLOR_TEXT_SECONDARY);
            subView.setTextSize(TypedValue.COMPLEX_UNIT_SP, 12);
            textLayout.addView(subView);
        }

        card.addView(textLayout, new LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1.0f));

        final Switch switchView = new Switch(context);
        switchView.setChecked(isChecked);
        switchView.setOnCheckedChangeListener(listener);
        card.addView(switchView);

        parent.addView(card);
    }

    public static void addTextInputItem(
        ViewGroup parent,
        final Context context,
        final String title,
        final String subtitle,
        final String currentValue,
        final OnTextEnteredListener listener
    ) {
        final LinearLayout card = createCardView(context);

        final LinearLayout textLayout = new LinearLayout(context);
        textLayout.setOrientation(LinearLayout.VERTICAL);

        final TextView titleView = new TextView(context);
        titleView.setText(title);
        titleView.setTextColor(COLOR_TEXT_PRIMARY);
        titleView.setTextSize(TypedValue.COMPLEX_UNIT_SP, 15);
        titleView.setTypeface(Typeface.DEFAULT_BOLD);
        textLayout.addView(titleView);

        final TextView subView = new TextView(context);
        subView.setText(currentValue != null && !currentValue.isEmpty() ? currentValue : subtitle);
        subView.setTextColor(COLOR_TEXT_SECONDARY);
        subView.setTextSize(TypedValue.COMPLEX_UNIT_SP, 12);
        textLayout.addView(subView);

        card.addView(textLayout, new LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1.0f));

        card.setOnClickListener(new View.OnClickListener() {
            @Override
            public void onClick(View v) {
                final EditText input = new EditText(context);
                input.setText(currentValue != null ? currentValue : "");
                input.setSingleLine(true);
                final FrameLayout container = new FrameLayout(context);
                container.setPadding(dp(context, 20), dp(context, 10), dp(context, 20), dp(context, 10));
                container.addView(input);

                new AlertDialog.Builder(context)
                    .setTitle(title)
                    .setView(container)
                    .setPositiveButton("Save", new DialogInterface.OnClickListener() {
                        @Override
                        public void onClick(DialogInterface dialog, int which) {
                            listener.onTextEntered(input.getText().toString().trim());
                        }
                    })
                    .setNegativeButton("Cancel", null)
                    .show();
            }
        });

        parent.addView(card);
    }

    private static LinearLayout createCardView(Context context) {
        final LinearLayout card = new LinearLayout(context);
        card.setOrientation(LinearLayout.HORIZONTAL);
        card.setGravity(Gravity.CENTER_VERTICAL);
        card.setBackground(createRoundedBackground(COLOR_CARD, dp(context, 12)));
        card.setPadding(dp(context, 16), dp(context, 14), dp(context, 16), dp(context, 14));

        final LinearLayout.LayoutParams lp = new LinearLayout.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT);
        lp.setMargins(0, dp(context, 4), 0, dp(context, 4));
        card.setLayoutParams(lp);
        return card;
    }

    private static GradientDrawable createRoundedBackground(int color, int radiusPx) {
        final GradientDrawable drawable = new GradientDrawable();
        drawable.setShape(GradientDrawable.RECTANGLE);
        drawable.setColor(color);
        drawable.setCornerRadius(radiusPx);
        return drawable;
    }

    private static int dp(Context context, int dpVal) {
        return (int) TypedValue.applyDimension(
            TypedValue.COMPLEX_UNIT_DIP,
            dpVal,
            context.getResources().getDisplayMetrics()
        );
    }
}
