package org.ohmycloud.readwise;

import android.app.Activity;
import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.Service;
import android.content.ClipData;
import android.content.Intent;
import android.database.Cursor;
import android.net.Uri;
import android.os.Build;
import android.os.Handler;
import android.os.IBinder;
import android.os.Looper;
import android.os.PowerManager;
import android.content.pm.ServiceInfo;
import android.provider.OpenableColumns;
import android.provider.Settings;

import java.io.BufferedInputStream;
import java.io.BufferedOutputStream;
import java.io.File;
import java.io.FileOutputStream;
import java.io.InputStream;
import java.util.ArrayList;
import java.util.LinkedHashSet;
import java.util.Locale;

import dev.makepad.android.MakepadActivity;

/** App-specific Activity additions that cargo-makepad's generated subclass lacks. */
public class MakepadApp extends MakepadActivity {
    private static final int EPUB_PICKER_REQUEST = 0x5257;
    private static String lanSession;

    private static native void nativeOnLanTransferServiceStopped(String session);

    public String startLanTransferService(String session, boolean english) {
        synchronized (MakepadApp.class) {
            try {
                Intent intent = new Intent(this, LanTransferService.class);
                intent.putExtra("session", session);
                intent.putExtra("english", english);
                startForegroundService(intent);
                lanSession = session;
                return "";
            } catch (Exception error) {
                return messageFor(error);
            }
        }
    }

    public void stopLanTransferService(String session) {
        synchronized (MakepadApp.class) {
            if (session.equals(lanSession)) {
                lanSession = null;
                stopService(new Intent(this, LanTransferService.class));
            }
        }
    }

    public void requestLanBackgroundAccess() {
        runOnUiThread(() -> {
            PowerManager power = getSystemService(PowerManager.class);
            Intent settings = new Intent(power.isIgnoringBatteryOptimizations(getPackageName())
                    ? Settings.ACTION_APPLICATION_DETAILS_SETTINGS
                    : Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS);
            settings.setData(Uri.parse("package:" + getPackageName()));
            try { startActivity(settings); }
            catch (Exception unavailable) {
                Intent fallback = new Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS);
                fallback.setData(Uri.parse("package:" + getPackageName()));
                startActivity(fallback);
            }
        });
    }

    /** Keeps the explicitly enabled receiver alive when the Activity is paused. */
    public static class LanTransferService extends Service {
        private static final String CHANNEL = "readwise-lan-transfer";
        private static final String STOP = "org.ohmycloud.readwise.STOP_LAN_TRANSFER";
        private static final int NOTIFICATION_ID = 0x5258;
        private final Handler timer = new Handler(Looper.getMainLooper());
        private String session;
        private PowerManager.WakeLock receiverWakeLock;
        private final Runnable expire = () -> stopReceiver();

        @Override
        public IBinder onBind(Intent intent) { return null; }

        @Override
        public int onStartCommand(Intent intent, int flags, int startId) {
            if (intent == null) { stopSelf(); return START_NOT_STICKY; }
            String requested = intent.getStringExtra("session");
            if (STOP.equals(intent.getAction())) {
                if (requested != null && requested.equals(session)) { stopReceiver(); }
                return START_NOT_STICKY;
            }
            synchronized (MakepadApp.class) {
                // A delayed start belonging to an already closed session must
                // neither resurrect that session nor stop a newer receiver.
                if (requested == null || !requested.equals(lanSession)) {
                    if (session == null) { stopSelf(startId); }
                    return START_NOT_STICKY;
                }
                session = requested;
            }
            boolean english = intent.getBooleanExtra("english", false);
            NotificationManager manager = getSystemService(NotificationManager.class);
            manager.createNotificationChannel(new NotificationChannel(
                    CHANNEL, english ? "LAN book transfer" : "局域网传书",
                    NotificationManager.IMPORTANCE_LOW));
            Intent open = new Intent(this, MakepadApp.class);
            open.addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP | Intent.FLAG_ACTIVITY_CLEAR_TOP);
            PendingIntent reopen = PendingIntent.getActivity(this, 0, open,
                    PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_IMMUTABLE);
            Intent stop = new Intent(this, LanTransferService.class);
            stop.setAction(STOP);
            stop.putExtra("session", session);
            PendingIntent stopAction = PendingIntent.getService(this, 0, stop,
                    PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_IMMUTABLE);
            Notification notification = new Notification.Builder(this, CHANNEL)
                    .setSmallIcon(android.R.drawable.stat_sys_upload)
                    .setContentTitle(english ? "Receiving EPUB books" : "正在接收 EPUB 书籍")
                    .setContentText(english ? "Background receiving enabled · expires in 30 minutes"
                            : "可在后台接收 · 开启后 30 分钟到期")
                    .setContentIntent(reopen).setOngoing(true)
                    .addAction(new Notification.Action.Builder(null,
                            english ? "Stop receiving" : "停止接收", stopAction).build())
                    .build();
            if (Build.VERSION.SDK_INT >= 29) {
                startForeground(NOTIFICATION_ID, notification,
                        ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC);
            } else { startForeground(NOTIFICATION_ID, notification); }
            // A foreground service alone does not keep the CPU awake. Hold a
            // bounded lock only for the explicitly enabled transfer session.
            if (receiverWakeLock == null) {
                PowerManager power = getSystemService(PowerManager.class);
                receiverWakeLock = power.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK,
                        "ReadWise:LanTransfer");
                receiverWakeLock.setReferenceCounted(false);
            }
            receiverWakeLock.acquire(30L * 60L * 1000L + 5000L);
            timer.removeCallbacks(expire);
            timer.postDelayed(expire, 30L * 60L * 1000L);
            return START_NOT_STICKY;
        }

        private void stopReceiver() {
            if (session != null) { nativeOnLanTransferServiceStopped(session); }
            stopForeground(true);
            stopSelf();
        }

        // Called by Android 15 if its data-sync foreground-service quota expires.
        public void onTimeout(int startId, int foregroundServiceType) { stopReceiver(); }

        @Override
        public void onDestroy() {
            timer.removeCallbacks(expire);
            if (receiverWakeLock != null && receiverWakeLock.isHeld()) {
                receiverWakeLock.release();
            }
            if (session != null) { nativeOnLanTransferServiceStopped(session); }
            super.onDestroy();
        }
    }

    private static native void nativeOnEpubPickerResult(
            int status, String[] paths, String error);

    public void switchActivity() {
        switchActivityClass(MakepadAppXr.class);
    }

    public void openEpubPicker() {
        Intent picker = new Intent(Intent.ACTION_OPEN_DOCUMENT);
        picker.addCategory(Intent.CATEGORY_OPENABLE);
        picker.setType("application/epub+zip");
        picker.putExtra(Intent.EXTRA_MIME_TYPES, new String[] {
                "application/epub+zip",
                "application/octet-stream",
                "application/zip"
        });
        picker.putExtra(Intent.EXTRA_ALLOW_MULTIPLE, true);
        picker.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
        try {
            startActivityForResult(picker, EPUB_PICKER_REQUEST);
        } catch (Exception error) {
            nativeOnEpubPickerResult(2, null, messageFor(error));
        }
    }

    @Override
    protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        if (requestCode != EPUB_PICKER_REQUEST) {
            return;
        }
        if (resultCode != Activity.RESULT_OK || data == null) {
            nativeOnEpubPickerResult(0, null, null);
            return;
        }

        ArrayList<File> copiedFiles = new ArrayList<>();
        try {
            LinkedHashSet<Uri> uris = new LinkedHashSet<>();
            ClipData clip = data.getClipData();
            if (clip != null) {
                for (int index = 0; index < clip.getItemCount(); index++) {
                    Uri uri = clip.getItemAt(index).getUri();
                    if (uri != null) {
                        uris.add(uri);
                    }
                }
            }
            if (data.getData() != null) {
                uris.add(data.getData());
            }
            if (uris.isEmpty()) {
                nativeOnEpubPickerResult(2, null, "系统没有返回所选文件");
                return;
            }

            File importDir = new File(getCacheDir(), "readwise-import");
            if (!importDir.exists() && !importDir.mkdirs()) {
                throw new IllegalStateException("无法创建 EPUB 导入缓存目录");
            }

            ArrayList<String> paths = new ArrayList<>();
            int index = 0;
            for (Uri uri : uris) {
                File copy = copyToImportCache(uri, importDir, index++);
                copiedFiles.add(copy);
                paths.add(copy.getAbsolutePath());
            }
            nativeOnEpubPickerResult(1, paths.toArray(new String[0]), null);
        } catch (Exception error) {
            for (File copiedFile : copiedFiles) {
                copiedFile.delete();
            }
            nativeOnEpubPickerResult(2, null, messageFor(error));
        }
    }

    private File copyToImportCache(Uri uri, File importDir, int index) throws Exception {
        String displayName = queryDisplayName(uri);
        if (displayName == null || displayName.trim().isEmpty()) {
            displayName = "book.epub";
        }
        displayName = displayName.replaceAll("[^\\p{L}\\p{N}._ -]", "_");
        if (!displayName.toLowerCase(Locale.ROOT).endsWith(".epub")) {
            displayName += ".epub";
        }
        File output = new File(
                importDir,
                System.currentTimeMillis() + "-" + index + "-" + displayName);
        try (InputStream rawInput = getContentResolver().openInputStream(uri)) {
            if (rawInput == null) {
                throw new IllegalStateException("无法读取所选文件：" + displayName);
            }
            try (BufferedInputStream input = new BufferedInputStream(rawInput);
                 BufferedOutputStream target = new BufferedOutputStream(new FileOutputStream(output))) {
                byte[] buffer = new byte[64 * 1024];
                int count;
                while ((count = input.read(buffer)) != -1) {
                    target.write(buffer, 0, count);
                }
            }
        } catch (Exception error) {
            // Avoid leaving a partial EPUB that could be mistaken for a future import.
            output.delete();
            throw error;
        }
        return output;
    }

    private String queryDisplayName(Uri uri) {
        try (Cursor cursor = getContentResolver().query(
                uri, new String[] { OpenableColumns.DISPLAY_NAME }, null, null, null)) {
            if (cursor != null && cursor.moveToFirst()) {
                int column = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME);
                if (column >= 0) {
                    return cursor.getString(column);
                }
            }
        } catch (Exception ignored) {
            // A provider may omit metadata; the fallback filename is sufficient.
        }
        return null;
    }

    private static String messageFor(Exception error) {
        String message = error.getMessage();
        return message == null || message.trim().isEmpty()
                ? error.getClass().getSimpleName()
                : message;
    }
}
