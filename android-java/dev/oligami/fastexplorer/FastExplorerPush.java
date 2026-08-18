package dev.oligami.fastexplorer;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.content.Context;
import android.content.Intent;
import android.content.SharedPreferences;
import android.net.Uri;
import android.os.Build;
import androidx.annotation.Nullable;
import com.google.firebase.FirebaseApp;
import com.google.firebase.messaging.FirebaseMessaging;
import java.util.UUID;

final class FastExplorerPush {
    static final String CHANNEL_ID = "fast_explorer_sync";
    static final String ACTION_OPEN_DEVICE_SYNC =
            "dev.oligami.fastexplorer.action.OPEN_DEVICE_SYNC";
    static final String EXTRA_SYNC_NOTIFICATION_TOKEN =
            "dev.oligami.fastexplorer.extra.SYNC_NOTIFICATION_TOKEN";
    private static final String PREFS = "fast_explorer_push";
    private static final String KEY_FCM_TOKEN = "fcm_token";

    private FastExplorerPush() {}

    static void initialize(Context context) {
        ensureChannel(context);
        FirebaseApp app = FirebaseApp.initializeApp(context);
        if (app == null) {
            android.util.Log.i("FastExplorer", "Firebase is not configured; killed-state push disabled");
            return;
        }
        FirebaseMessaging.getInstance().getToken().addOnCompleteListener(task -> {
            if (!task.isSuccessful()) {
                android.util.Log.w("FastExplorer", "Cannot obtain FCM token", task.getException());
                return;
            }
            saveToken(context, task.getResult());
        });
    }

    static void saveToken(Context context, @Nullable String token) {
        if (token == null || token.isBlank()) {
            return;
        }
        prefs(context).edit().putString(KEY_FCM_TOKEN, token).apply();
    }

    static String cachedToken(Context context) {
        return prefs(context).getString(KEY_FCM_TOKEN, "");
    }

    static void ensureChannel(Context context) {
        NotificationManager manager = context.getSystemService(NotificationManager.class);
        manager.createNotificationChannel(new NotificationChannel(
                CHANNEL_ID,
                "FastExplorer device sync",
                NotificationManager.IMPORTANCE_DEFAULT));
    }

    static void showNotification(Context context, String title, String detail) {
        ensureChannel(context);
        String token = UUID.randomUUID().toString();
        Intent open = new Intent(context, FastExplorerActivity.class)
                .setAction(ACTION_OPEN_DEVICE_SYNC)
                .setData(Uri.parse("fastexplorer-sync://notification/" + token))
                .putExtra(EXTRA_SYNC_NOTIFICATION_TOKEN, token)
                .addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP | Intent.FLAG_ACTIVITY_CLEAR_TOP);
        PendingIntent pending = PendingIntent.getActivity(
                context,
                9101,
                open,
                PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_IMMUTABLE);
        Notification notification = new Notification.Builder(context, CHANNEL_ID)
                .setSmallIcon(android.R.drawable.stat_sys_download_done)
                .setContentTitle(title)
                .setContentText(detail)
                .setStyle(new Notification.BigTextStyle().bigText(detail))
                .setAutoCancel(true)
                .setOnlyAlertOnce(true)
                .setContentIntent(pending)
                .build();
        NotificationManager manager = context.getSystemService(NotificationManager.class);
        manager.notify(9101, notification);
    }

    private static SharedPreferences prefs(Context context) {
        return context.getSharedPreferences(PREFS, Context.MODE_PRIVATE);
    }
}
