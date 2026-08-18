package dev.oligami.fastexplorer;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;
import android.content.pm.ApplicationInfo;
import android.content.pm.PackageInstaller;
import android.content.pm.PackageManager;
import android.content.pm.ResolveInfo;
import android.os.Build;

public final class FastExplorerInstallStatusReceiver extends BroadcastReceiver {
    private static final String PREFS = "fast_explorer_install_callback";
    private static final String KEY_TOKEN = "token";
    private static final String KEY_WORK_DIR = "work_dir";
    private static final String KEY_SOURCE_PATH = "source_path";
    private static final String KEY_STATUS = "status";
    private static final String KEY_MESSAGE = "message";
    private static final String KEY_BLOCKER = "blocker";
    private static final String KEY_DATA = "data";
    private static final String CHANNEL_ID = "fast_explorer_installs";
    private static final int CONFIRM_NOTIFICATION_ID = 4201;
    private static final int RESULT_NOTIFICATION_ID = 4202;

    @Override
    public void onReceive(Context context, Intent intent) {
        if (intent == null) return;
        int status = intent.getIntExtra(
                PackageInstaller.EXTRA_STATUS, PackageInstaller.STATUS_FAILURE);
        if (status == PackageInstaller.STATUS_PENDING_USER_ACTION) {
            handlePendingUserAction(context, intent);
            return;
        }
        forwardTerminalStatus(context, intent, status, null);
    }

    private static void handlePendingUserAction(Context context, Intent statusIntent) {
        Intent confirmation;
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            confirmation = statusIntent.getParcelableExtra(Intent.EXTRA_INTENT, Intent.class);
        } else {
            confirmation = statusIntent.getParcelableExtra(Intent.EXTRA_INTENT);
        }
        if (confirmation == null || !isTrustedSystemConfirmation(context, confirmation)) {
            abandonPendingSession(context, statusIntent);
            forwardTerminalStatus(
                    context,
                    statusIntent,
                    PackageInstaller.STATUS_FAILURE,
                    confirmation == null
                            ? "Android did not provide an installation confirmation screen"
                            : "Android provided an untrusted installation confirmation target");
            return;
        }

        try {
            confirmation.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
            if (FastExplorerActivity.isVisibleForInstallFlow()) {
                context.startActivity(confirmation);
                return;
            }
            NotificationManager manager = context.getSystemService(NotificationManager.class);
            ensureChannel(context);
            NotificationChannel channel = manager == null ? null : manager.getNotificationChannel(CHANNEL_ID);
            if (manager == null
                    || !manager.areNotificationsEnabled()
                    || channel == null
                    || channel.getImportance() == NotificationManager.IMPORTANCE_NONE) {
                abandonPendingSession(context, statusIntent);
                forwardTerminalStatus(
                        context,
                        statusIntent,
                        PackageInstaller.STATUS_FAILURE,
                        "Installation confirmation requires FastExplorer in the foreground or notifications enabled");
                return;
            }
            PendingIntent pending = PendingIntent.getActivity(
                    context,
                    4200 + Math.abs(statusIntent.getIntExtra(PackageInstaller.EXTRA_SESSION_ID, 0) % 1000),
                    confirmation,
                    PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_IMMUTABLE);
            ensureChannel(context);
            Notification notification = new Notification.Builder(context, CHANNEL_ID)
                    .setSmallIcon(android.R.drawable.stat_sys_download_done)
                    .setContentTitle("Confirm app installation")
                    .setContentText("Tap to continue the Android package installer")
                    .setContentIntent(pending)
                    .setAutoCancel(true)
                    .setCategory(Notification.CATEGORY_SYSTEM)
                    .build();
            manager.notify(CONFIRM_NOTIFICATION_ID, notification);
        } catch (RuntimeException error) {
            android.util.Log.e("FastExplorer", "cannot post Android package confirmation", error);
            abandonPendingSession(context, statusIntent);
            forwardTerminalStatus(
                    context,
                    statusIntent,
                    PackageInstaller.STATUS_FAILURE,
                    "Cannot request Android package confirmation: " + safeMessage(error));
        }
    }

    private static boolean isTrustedSystemConfirmation(Context context, Intent confirmation) {
        PackageManager packageManager = context.getPackageManager();
        ResolveInfo resolved = packageManager.resolveActivity(confirmation, PackageManager.MATCH_DEFAULT_ONLY);
        if (resolved == null || resolved.activityInfo == null || resolved.activityInfo.applicationInfo == null) {
            return false;
        }
        ApplicationInfo app = resolved.activityInfo.applicationInfo;
        return (app.flags & (ApplicationInfo.FLAG_SYSTEM | ApplicationInfo.FLAG_UPDATED_SYSTEM_APP)) != 0;
    }

    private static void abandonPendingSession(Context context, Intent intent) {
        int sessionId = intent.getIntExtra(PackageInstaller.EXTRA_SESSION_ID, -1);
        if (sessionId < 0) return;
        try {
            context.getPackageManager().getPackageInstaller().abandonSession(sessionId);
        } catch (RuntimeException error) {
            android.util.Log.w("FastExplorer", "cannot abandon failed install session", error);
        }
    }

    private static void forwardTerminalStatus(
            Context context, Intent source, int status, String overrideMessage) {
        Intent target = new Intent(context, FastExplorerActivity.class)
                .setAction(FastExplorerActivity.ACTION_APKS_INSTALL_STATUS)
                .setData(source.getData())
                .putExtra(
                        FastExplorerActivity.EXTRA_APKS_WORK_DIR,
                        source.getStringExtra(FastExplorerActivity.EXTRA_APKS_WORK_DIR))
                .putExtra(
                        FastExplorerActivity.EXTRA_INSTALL_SOURCE_PATH,
                        source.getStringExtra(FastExplorerActivity.EXTRA_INSTALL_SOURCE_PATH))
                .putExtra(
                        FastExplorerActivity.EXTRA_INSTALL_CALLBACK_TOKEN,
                        source.getStringExtra(FastExplorerActivity.EXTRA_INSTALL_CALLBACK_TOKEN))
                .putExtra(PackageInstaller.EXTRA_STATUS, status)
                .addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP | Intent.FLAG_ACTIVITY_CLEAR_TOP);
        String statusMessage = overrideMessage != null
                ? overrideMessage
                : source.getStringExtra(PackageInstaller.EXTRA_STATUS_MESSAGE);
        String blocker = source.getStringExtra(PackageInstaller.EXTRA_OTHER_PACKAGE_NAME);
        if (statusMessage != null) {
            target.putExtra(PackageInstaller.EXTRA_STATUS_MESSAGE, statusMessage);
        }
        if (blocker != null) {
            target.putExtra(PackageInstaller.EXTRA_OTHER_PACKAGE_NAME, blocker);
        }
        persistTerminalStatus(context, target);
        NotificationManager manager = context.getSystemService(NotificationManager.class);
        if (manager != null) manager.cancel(CONFIRM_NOTIFICATION_ID);
        if (FastExplorerActivity.isVisibleForInstallFlow()) {
            try {
                target.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
                context.startActivity(target);
                return;
            } catch (RuntimeException error) {
                android.util.Log.w("FastExplorer", "cannot deliver visible install result", error);
            }
        }
        postTerminalNotification(context, target, status);
    }

    private static void ensureChannel(Context context) {
        NotificationManager manager = context.getSystemService(NotificationManager.class);
        if (manager != null) {
            manager.createNotificationChannel(new NotificationChannel(
                    CHANNEL_ID,
                    "FastExplorer app installs",
                    NotificationManager.IMPORTANCE_DEFAULT));
        }
    }

    private static void postTerminalNotification(Context context, Intent target, int status) {
        NotificationManager manager = context.getSystemService(NotificationManager.class);
        if (manager == null) return;
        ensureChannel(context);
        PendingIntent open = PendingIntent.getActivity(
                context,
                RESULT_NOTIFICATION_ID,
                target,
                PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_IMMUTABLE);
        String title = status == PackageInstaller.STATUS_SUCCESS
                ? "App installed"
                : "App installation failed";
        String detail = target.getStringExtra(PackageInstaller.EXTRA_STATUS_MESSAGE);
        if (detail == null || detail.isBlank()) {
            detail = status == PackageInstaller.STATUS_SUCCESS
                    ? "The selected package was installed"
                    : "Open FastExplorer for installation details";
        }
        manager.notify(
                RESULT_NOTIFICATION_ID,
                new Notification.Builder(context, CHANNEL_ID)
                        .setSmallIcon(android.R.drawable.stat_sys_download_done)
                        .setContentTitle(title)
                        .setContentText(detail)
                        .setStyle(new Notification.BigTextStyle().bigText(detail))
                        .setContentIntent(open)
                        .setAutoCancel(true)
                        .build());
    }

    private static void persistTerminalStatus(Context context, Intent terminal) {
        String token = terminal.getStringExtra(FastExplorerActivity.EXTRA_INSTALL_CALLBACK_TOKEN);
        if (token == null || token.isBlank()) return;
        android.content.SharedPreferences.Editor editor =
                context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit()
                        .putString(KEY_TOKEN, token)
                        .putString(KEY_WORK_DIR, terminal.getStringExtra(FastExplorerActivity.EXTRA_APKS_WORK_DIR))
                        .putString(KEY_SOURCE_PATH, terminal.getStringExtra(FastExplorerActivity.EXTRA_INSTALL_SOURCE_PATH))
                        .putInt(KEY_STATUS, terminal.getIntExtra(
                                PackageInstaller.EXTRA_STATUS, PackageInstaller.STATUS_FAILURE))
                        .putString(KEY_MESSAGE, terminal.getStringExtra(PackageInstaller.EXTRA_STATUS_MESSAGE))
                        .putString(KEY_BLOCKER, terminal.getStringExtra(PackageInstaller.EXTRA_OTHER_PACKAGE_NAME));
        // The receiver may be the only live component when Android reports the result.
        // Commit this tiny record before attempting a background Activity launch so a
        // process kill cannot lose the terminal status or leave the install queue stuck.
        if (!editor.putString(KEY_DATA, terminal.getDataString()).commit()) {
            android.util.Log.w("FastExplorer", "cannot persist package install result");
        }
    }

    static Intent pendingTerminalStatus(Context context) {
        android.content.SharedPreferences prefs =
                context.getSharedPreferences(PREFS, Context.MODE_PRIVATE);
        String token = prefs.getString(KEY_TOKEN, "");
        if (token == null || token.isBlank()) return null;
        Intent intent = new Intent(context, FastExplorerActivity.class)
                .setAction(FastExplorerActivity.ACTION_APKS_INSTALL_STATUS)
                .putExtra(FastExplorerActivity.EXTRA_INSTALL_CALLBACK_TOKEN, token)
                .putExtra(FastExplorerActivity.EXTRA_APKS_WORK_DIR, prefs.getString(KEY_WORK_DIR, null))
                .putExtra(FastExplorerActivity.EXTRA_INSTALL_SOURCE_PATH, prefs.getString(KEY_SOURCE_PATH, null))
                .putExtra(PackageInstaller.EXTRA_STATUS,
                        prefs.getInt(KEY_STATUS, PackageInstaller.STATUS_FAILURE));
        String data = prefs.getString(KEY_DATA, null);
        if (data != null) intent.setData(android.net.Uri.parse(data));
        String message = prefs.getString(KEY_MESSAGE, null);
        String blocker = prefs.getString(KEY_BLOCKER, null);
        if (message != null) intent.putExtra(PackageInstaller.EXTRA_STATUS_MESSAGE, message);
        if (blocker != null) intent.putExtra(PackageInstaller.EXTRA_OTHER_PACKAGE_NAME, blocker);
        return intent;
    }

    static void clearPendingTerminalStatus(Context context, String callbackToken) {
        android.content.SharedPreferences prefs =
                context.getSharedPreferences(PREFS, Context.MODE_PRIVATE);
        if (callbackToken != null && callbackToken.equals(prefs.getString(KEY_TOKEN, ""))) {
            prefs.edit().clear().apply();
            NotificationManager manager = context.getSystemService(NotificationManager.class);
            if (manager != null) {
                manager.cancel(CONFIRM_NOTIFICATION_ID);
                manager.cancel(RESULT_NOTIFICATION_ID);
            }
        }
    }

    private static String safeMessage(Throwable error) {
        String detail = error == null ? null : error.getMessage();
        return detail == null || detail.isBlank() ? "unknown error" : detail;
    }
}
