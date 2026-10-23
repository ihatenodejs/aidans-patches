package app.aidan.extension.adobescan;

import android.app.Activity;
import android.app.AlertDialog;
import android.content.DialogInterface;
import android.os.Handler;
import android.os.Looper;
import android.util.Log;
import java.io.File;

public final class PdfExportBridge {

    private static final String TAG = "PdfExportBridge";

    private PdfExportBridge() {}

    public static void show(final Activity activity, final Object scanFile) {
        if (activity == null || activity.isFinishing() || activity.isDestroyed()) {
            return;
        }

        if (Looper.myLooper() != Looper.getMainLooper()) {
            new Handler(Looper.getMainLooper()).post(new Runnable() {
                @Override
                public void run() {
                    show(activity, scanFile);
                }
            });
            return;
        }

        if (scanFile == null) {
            PdfOperationUi.showSnackbar(activity, "No document selected for export.");
            return;
        }

        final File file = ScanFileAdapter.getInstance().getFile(scanFile);
        if (file == null || !file.exists() || !file.isFile() || file.length() <= 0) {
            PdfOperationUi.showSnackbar(activity, "Target document file not found on disk.");
            return;
        }

        if (ScanFileAdapter.getInstance().isProtected(scanFile) ||
                ScanFileAdapter.getInstance().getProtectionStatus(scanFile) == 2) {
            PdfOperationUi.showSnackbar(activity, "Cannot export password-protected scan. Remove password first.");
            return;
        }

        final String[] options = new String[]{
                "Word (.docx)",
                "Excel (.xlsx)",
                "PowerPoint (.pptx)"
        };
        final int[] formats = new int[]{
                NativePdfEngine.EXPORT_DOCX,
                NativePdfEngine.EXPORT_XLSX,
                NativePdfEngine.EXPORT_PPTX
        };
        final int[] selected = new int[]{0};

        new AlertDialog.Builder(activity)
                .setTitle("Export to Office")
                .setSingleChoiceItems(options, 0, new DialogInterface.OnClickListener() {
                    @Override
                    public void onClick(DialogInterface dialog, int which) {
                        selected[0] = which;
                    }
                })
                .setMessage("Export this scan as an editable Office document. Text and layout are converted on device.")
                .setPositiveButton("Export", new DialogInterface.OnClickListener() {
                    @Override
                    public void onClick(DialogInterface dialog, int which) {
                        final int format = formats[selected[0]];
                        PdfOperationCoordinator.getInstance().startExport(activity, scanFile, format);
                    }
                })
                .setNegativeButton("Cancel", null)
                .show();
    }

    public static void exportDocxDirect(final Activity activity, final Object scanFile) {
        if (activity == null || activity.isFinishing() || activity.isDestroyed()) {
            return;
        }

        if (Looper.myLooper() != Looper.getMainLooper()) {
            new Handler(Looper.getMainLooper()).post(new Runnable() {
                @Override
                public void run() {
                    exportDocxDirect(activity, scanFile);
                }
            });
            return;
        }

        if (scanFile == null) {
            PdfOperationUi.showSnackbar(activity, "No document selected for export.");
            return;
        }

        final File file = ScanFileAdapter.getInstance().getFile(scanFile);
        if (file == null || !file.exists() || !file.isFile() || file.length() <= 0) {
            PdfOperationUi.showSnackbar(activity, "Target document file not found on disk.");
            return;
        }

        if (ScanFileAdapter.getInstance().isProtected(scanFile) ||
                ScanFileAdapter.getInstance().getProtectionStatus(scanFile) == 2) {
            PdfOperationUi.showSnackbar(activity, "Cannot export password-protected scan. Remove password first.");
            return;
        }

        PdfOperationCoordinator.getInstance().startExport(activity, scanFile, NativePdfEngine.EXPORT_DOCX);
    }
}
