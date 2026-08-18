package dev.oligami.fastexplorer;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.Service;
import android.content.Intent;
import android.content.pm.ServiceInfo;
import android.os.Build;
import android.os.Handler;
import android.os.IBinder;
import android.os.Looper;
import androidx.annotation.Nullable;
import org.json.JSONException;
import org.json.JSONObject;

public final class FastExplorerTransferService extends Service {
    private static final String CHANNEL_ID = "fast_explorer_transfers";
    private static final int FOREGROUND_NOTIFICATION_ID = 4101;
    private static final int COMPLETION_NOTIFICATION_ID = 4102;
    private static final long POLL_INTERVAL_MS = 500L;

    static {
        System.loadLibrary("fast_explorer_android");
    }

    private static native String nativeTransferSnapshotJson();
    private static native String nativeDrainFileChangesJson();
    private static native void nativeUpdateNetworkInterfaces(String json);
    private static native String nativeControlTransfer(String transferId, String action);

    private final Handler handler = new Handler(Looper.getMainLooper());
    private NotificationManager notificationManager;
    private boolean sawActiveTransfer;
    private int emptyPolls;
    private int networkPolls;

    private final Runnable pollTransfers = new Runnable() {
        @Override
        public void run() {
            pollNativeTransfers();
        }
    };

    @Override
    public void onCreate() {
        super.onCreate();
        notificationManager = getSystemService(NotificationManager.class);
        createNotificationChannel();
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        if (notificationManager != null) {
            notificationManager.cancel(COMPLETION_NOTIFICATION_ID);
        }
        String transferId = intent == null ? null : intent.getStringExtra("transfer_id");
        String action = intent == null ? null : intent.getStringExtra("transfer_action");
        Notification initial = buildStartingNotification();
        if (transferId != null && action != null) {
            try {
                JSONObject snapshot = new JSONObject(nativeTransferSnapshotJson());
                int activeCount = snapshot.optInt("active_count", 0);
                JSONObject primary = snapshot.optJSONObject("primary");
                if (activeCount > 0 && primary != null) {
                    initial = buildProgressNotification(activeCount, primary);
                }
            } catch (JSONException | RuntimeException error) {
                android.util.Log.w("FastExplorer", "cannot restore transfer notification", error);
            }
        }
        // Every startForegroundService entry still acknowledges the foreground contract,
        // but notification actions reuse the current snapshot instead of flashing Starting….
        startAsForeground(initial);
        if (transferId != null && action != null) {
            String controlError = nativeControlTransfer(transferId, action);
            if (controlError != null && !controlError.isEmpty()) {
                android.util.Log.w(
                        "FastExplorer",
                        "cannot " + action + " transfer " + transferId + ": " + controlError);
            }
        }
        handler.removeCallbacks(pollTransfers);
        handler.postDelayed(pollTransfers, 120L);
        return START_NOT_STICKY;
    }

    @Nullable
    @Override
    public IBinder onBind(Intent intent) {
        return null;
    }

    @Override
    public void onDestroy() {
        handler.removeCallbacks(pollTransfers);
        super.onDestroy();
    }

    @Override
    public void onTaskRemoved(Intent rootIntent) {
        // Intentionally keep running. android:stopWithTask is also false in the manifest.
        super.onTaskRemoved(rootIntent);
    }

    @Override
    public void onTimeout(int startId, int fgsType) {
        if (Build.VERSION.SDK_INT >= 35
                && (fgsType & ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC) != 0) {
            // onTimeout and pollTransfers both run on the main looper. Remove the
            // queued poll before posting the terminal warning; otherwise a final poll
            // can immediately cancel or overwrite that warning before onDestroy runs.
            handler.removeCallbacks(pollTransfers);
            boolean quiesced = false;
            String detail;
            try {
                // Android is about to revoke the data-sync foreground-service window.
                // Quiesce every native transfer before dropping foreground state so no
                // second transfer is left doing background network or filesystem work.
                // Pausing is preferred; workers that cannot pause are cancelled safely.
                String controlError = nativeControlTransfer("", "quiesce_all");
                quiesced = controlError == null || controlError.isEmpty();
                detail = quiesced
                        ? "Android limited long-running data sync. Active transfers were paused, "
                                + "or cancelled when pausing was not safe. Reopen FastExplorer to continue."
                        : "Android stopped the transfer service, but some transfer work could not be "
                                + "safely suspended: " + controlError
                                + ". Reopen FastExplorer to verify its status.";
            } catch (Exception error) {
                android.util.Log.w("FastExplorer", "cannot suspend transfers after service timeout", error);
                detail = "Android stopped the transfer service and FastExplorer could not confirm whether "
                        + "all transfers were suspended. Reopen FastExplorer to check their status.";
            }
            postTerminalNotification(
                    quiesced ? "Transfers suspended" : "Transfer service stopped", detail);
            stopForeground(STOP_FOREGROUND_REMOVE);
            stopSelf(startId);
        }
    }

    private void createNotificationChannel() {
        if (Build.VERSION.SDK_INT < 26 || notificationManager == null) return;
        NotificationChannel channel = new NotificationChannel(
                CHANNEL_ID,
                "File transfers",
                NotificationManager.IMPORTANCE_LOW);
        channel.setDescription("FastExplorer file transfer progress");
        channel.setShowBadge(false);
        notificationManager.createNotificationChannel(channel);
    }

    private Notification buildStartingNotification() {
        return baseBuilder()
                .setContentTitle("FastExplorer transfer")
                .setContentText("Starting…")
                .setProgress(0, 0, true)
                .setOngoing(true)
                .build();
    }

    private Notification.Builder baseBuilder() {
        Intent open = new Intent(this, FastExplorerActivity.class)
                .addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP | Intent.FLAG_ACTIVITY_CLEAR_TOP);
        PendingIntent pending = PendingIntent.getActivity(
                this,
                0,
                open,
                PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_IMMUTABLE);
        return new Notification.Builder(this, CHANNEL_ID)
                .setSmallIcon(android.R.drawable.stat_sys_upload)
                .setContentIntent(pending)
                .setCategory(Notification.CATEGORY_PROGRESS)
                .setOnlyAlertOnce(true)
                .setShowWhen(false);
    }

    private void startAsForeground(Notification notification) {
        if (Build.VERSION.SDK_INT >= 29) {
            startForeground(
                    FOREGROUND_NOTIFICATION_ID,
                    notification,
                    ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC);
        } else {
            startForeground(FOREGROUND_NOTIFICATION_ID, notification);
        }
    }

    private void pollNativeTransfers() {
        try {
            if ((networkPolls++ % 4) == 0) {
                nativeUpdateNetworkInterfaces(FastExplorerNetwork.snapshot(this));
            }
            FastExplorerFileChangeNotifier.notifyChanges(this, nativeDrainFileChangesJson());
            JSONObject snapshot = new JSONObject(nativeTransferSnapshotJson());
            int activeCount = snapshot.optInt("active_count", 0);
            JSONObject primary = snapshot.optJSONObject("primary");
            if (activeCount > 0 && primary != null) {
                sawActiveTransfer = true;
                emptyPolls = 0;
                notificationManager.notify(
                        FOREGROUND_NOTIFICATION_ID,
                        buildProgressNotification(activeCount, primary));
                handler.postDelayed(pollTransfers, POLL_INTERVAL_MS);
                return;
            }

            emptyPolls++;
            if (!sawActiveTransfer && emptyPolls < 10) {
                handler.postDelayed(pollTransfers, POLL_INTERVAL_MS);
                return;
            }
        } catch (JSONException | RuntimeException error) {
            android.util.Log.w("FastExplorer", "cannot update transfer notification", error);
            if (emptyPolls++ < 10) {
                handler.postDelayed(pollTransfers, POLL_INTERVAL_MS);
                return;
            }
        }
        if (sawActiveTransfer && notificationManager != null) {
            try {
                JSONObject snapshot = new JSONObject(nativeTransferSnapshotJson());
                JSONObject primary = snapshot.optJSONObject("primary");
                if (primary != null) {
                    String phase = primary.optString("phase", "Completed");
                    String label = primary.optString("label", "File transfer");
                    String detail = primary.optString("detail", phase);
                    postTerminalNotification(phase + ": " + label, detail);
                }
            } catch (JSONException | RuntimeException error) {
                android.util.Log.w("FastExplorer", "cannot post terminal transfer notification", error);
            }
        }
        stopForeground(STOP_FOREGROUND_REMOVE);
        if (notificationManager != null) {
            notificationManager.cancel(FOREGROUND_NOTIFICATION_ID);
        }
        stopSelf();
    }

    private Notification.Action transferAction(
            String transferId, String action, String title, int icon) {
        Intent intent = new Intent(this, FastExplorerTransferService.class)
                .putExtra("transfer_id", transferId)
                .putExtra("transfer_action", action);
        int requestCode = 5000 + Math.abs((transferId + action).hashCode() % 20000);
        PendingIntent pending = PendingIntent.getService(
                this,
                requestCode,
                intent,
                PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_IMMUTABLE);
        return new Notification.Action.Builder(icon, title, pending).build();
    }

    private Notification buildProgressNotification(int activeCount, JSONObject primary) {
        String transferId = primary.optString("transfer_id", "");
        String label = primary.optString("label", "File transfer");
        String phase = primary.optString("phase", "Transferring");
        String detail = primary.optString("detail", "Working…");
        boolean paused = primary.optBoolean("paused", false);
        boolean canPause = primary.optBoolean("can_pause", false);
        boolean canCancel = primary.optBoolean("can_cancel", false);
        int progress = primary.optInt("percent", -1);
        String title = activeCount == 1
                ? phase + ": " + label
                : activeCount + " transfers active";
        Notification.Builder builder = baseBuilder()
                .setContentTitle(title)
                .setContentText(detail)
                .setStyle(new Notification.BigTextStyle().bigText(detail))
                .setOngoing(true);
        if (!transferId.isEmpty() && canPause) {
            builder.addAction(transferAction(
                    transferId,
                    paused ? "resume" : "pause",
                    paused ? "Resume" : "Pause",
                    paused ? android.R.drawable.ic_media_play : android.R.drawable.ic_media_pause));
        }
        if (!transferId.isEmpty() && canCancel) {
            builder.addAction(transferAction(
                    transferId,
                    "cancel",
                    "Cancel",
                    android.R.drawable.ic_menu_close_clear_cancel));
        }
        if (progress >= 0) {
            builder.setProgress(100, Math.min(100, progress), false);
        } else {
            builder.setProgress(0, 0, true);
        }
        return builder.build();
    }

    private void postTerminalNotification(String title, String detail) {
        if (notificationManager == null) return;
        Notification notification = baseBuilder()
                .setContentTitle(title)
                .setContentText(detail)
                .setStyle(new Notification.BigTextStyle().bigText(detail))
                .setOngoing(false)
                .setAutoCancel(true)
                .setProgress(0, 0, false)
                .build();
        notificationManager.notify(COMPLETION_NOTIFICATION_ID, notification);
    }
}
