package dev.oligami.fastexplorer;

import com.google.androidgamesdk.GameActivity;
import androidx.core.content.FileProvider;
import androidx.core.view.WindowCompat;
import android.Manifest;
import android.annotation.SuppressLint;
import android.app.AlertDialog;
import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.content.ActivityNotFoundException;
import android.content.ClipData;
import android.content.ClipboardManager;
import android.content.Intent;
import android.content.pm.PackageInstaller;
import android.content.pm.PackageManager;
import android.graphics.Color;
import android.net.ConnectivityManager;
import android.net.LinkProperties;
import android.net.Network;
import android.net.NetworkRequest;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.os.Environment;
import android.os.Handler;
import android.os.HandlerThread;
import android.os.storage.StorageManager;
import android.os.storage.StorageVolume;
import android.provider.Settings;
import android.window.OnBackInvokedCallback;
import android.window.OnBackInvokedDispatcher;
import android.webkit.MimeTypeMap;
import java.io.File;
import java.io.IOException;
import java.util.ArrayList;
import java.util.Locale;
import java.util.UUID;
import java.util.concurrent.ConcurrentLinkedDeque;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import android.widget.Toast;
import org.json.JSONArray;

public final class FastExplorerActivity extends GameActivity {
    static {
        System.loadLibrary("fast_explorer_android");
    }

    private OnBackInvokedCallback backCallback;
    private ConnectivityManager connectivityManager;
    private ConnectivityManager.NetworkCallback networkCallback;
    private HandlerThread networkThread;
    private Handler networkHandler;
    private volatile String networkInterfacesJson = "[]";
    private static final int NOTIFICATION_PERMISSION_REQUEST_CODE = 3003;
    private static final String ACTION_FAST_EXPLORER_SYNC =
            "dev.oligami.fastexplorer.action.OPEN_DEVICE_SYNC";
    static final String ACTION_APKS_INSTALL_STATUS =
            "dev.oligami.fastexplorer.action.APKS_INSTALL_STATUS";
    static final String EXTRA_APKS_WORK_DIR =
            "dev.oligami.fastexplorer.extra.APKS_WORK_DIR";
    static final String EXTRA_INSTALL_SOURCE_PATH =
            "dev.oligami.fastexplorer.extra.INSTALL_SOURCE_PATH";
    static final String EXTRA_INSTALL_CALLBACK_TOKEN =
            "dev.oligami.fastexplorer.extra.INSTALL_CALLBACK_TOKEN";
    private static final String STATE_PENDING_INSTALL_PATHS =
            "dev.oligami.fastexplorer.state.PENDING_INSTALL_PATHS";
    private static final String STATE_ACTIVE_INSTALL_PATH =
            "dev.oligami.fastexplorer.state.ACTIVE_INSTALL_PATH";
    private static final String STATE_ACTIVE_INSTALL_CALLBACK_TOKEN =
            "dev.oligami.fastexplorer.state.ACTIVE_INSTALL_CALLBACK_TOKEN";
    private static final String STATE_UNKNOWN_SOURCE_SETTINGS_OPEN =
            "dev.oligami.fastexplorer.state.UNKNOWN_SOURCE_SETTINGS_OPEN";
    private static final String STATE_SYNC_INTENT_CONSUMED =
            "dev.oligami.fastexplorer.state.SYNC_INTENT_CONSUMED";
    private static final String STATE_LAST_INSTALL_CALLBACK_TOKEN =
            "dev.oligami.fastexplorer.state.LAST_INSTALL_CALLBACK_TOKEN";
    private static final String INTENT_DEDUPE_PREFS = "fast_explorer_intent_dedupe";
    private static final String KEY_LAST_SYNC_TOKEN = "last_sync_notification_token";
    private static final String KEY_LAST_INSTALL_CALLBACK_TOKEN = "last_install_callback_token";
    private final ConcurrentLinkedDeque<String> pendingInstallPaths = new ConcurrentLinkedDeque<>();
    private volatile String activeInstallPath;
    private volatile String activeInstallCallbackToken;
    private int nextApksCallbackRequestCode = 70001;
    private boolean unknownSourceSettingsOpen;
    private boolean syncIntentConsumed;
    private String lastHandledInstallCallbackToken;
    private final ExecutorService aabInstallerExecutor = Executors.newSingleThreadExecutor();
    private static volatile boolean installFlowActivityVisible;

    private static native void nativeBackPressed();
    private static native void nativeActivityResumed();
    private static native void nativeActivityPaused();
    private static native void nativeSyncNotificationOpened();

    @Override
    protected void onCreate(Bundle state) {
        super.onCreate(state);
        // Draw the file surface through the system-bar regions. Rust/Xilem
        // applies the status-bar inset itself and adds a scrollable tail to the
        // file list for the bottom navigation/gesture inset.
        WindowCompat.setDecorFitsSystemWindows(getWindow(), false);
        getWindow().setNavigationBarColor(Color.TRANSPARENT);
        getWindow().setNavigationBarContrastEnforced(false);
        if (Build.VERSION.SDK_INT >= 33) {
            backCallback = FastExplorerActivity::nativeBackPressed;
            getOnBackInvokedDispatcher().registerOnBackInvokedCallback(
                    OnBackInvokedDispatcher.PRIORITY_DEFAULT, backCallback);
        }
        startNetworkSnapshotter();
        FastExplorerPush.initialize(this);
        FastExplorerApksInstaller.cleanupStaleArtifactsAsync(this);
        restoreTransientState(state);
        handleApksInstallStatus(getIntent());
        handlePendingInstallStatus();
        handleFastExplorerSyncIntent(getIntent());
    }

    @Override
    protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        setIntent(intent);
        if (intent != null && ACTION_FAST_EXPLORER_SYNC.equals(intent.getAction())) {
            syncIntentConsumed = false;
        }
        handleApksInstallStatus(intent);
        handleFastExplorerSyncIntent(intent);
    }

    private void restoreTransientState(Bundle state) {
        if (state == null) return;
        ArrayList<String> pending = state.getStringArrayList(STATE_PENDING_INSTALL_PATHS);
        if (pending != null) pendingInstallPaths.addAll(pending);
        String restoredActivePath = state.getString(STATE_ACTIVE_INSTALL_PATH);
        String restoredCallbackToken = state.getString(STATE_ACTIVE_INSTALL_CALLBACK_TOKEN);
        if (restoredActivePath != null && !restoredActivePath.isBlank()) {
            if (restoredCallbackToken == null || restoredCallbackToken.isBlank()) {
                // Preparation was interrupted before PackageInstaller owned the work.
                // Restart it from the serialized queue instead of leaving a dead slot.
                pendingInstallPaths.addFirst(restoredActivePath);
            } else {
                activeInstallPath = restoredActivePath;
                activeInstallCallbackToken = restoredCallbackToken;
            }
        }
        unknownSourceSettingsOpen = state.getBoolean(STATE_UNKNOWN_SOURCE_SETTINGS_OPEN, false);
        syncIntentConsumed = state.getBoolean(STATE_SYNC_INTENT_CONSUMED, false);
        lastHandledInstallCallbackToken = state.getString(STATE_LAST_INSTALL_CALLBACK_TOKEN);
    }

    @Override
    protected void onSaveInstanceState(Bundle outState) {
        super.onSaveInstanceState(outState);
        outState.putStringArrayList(
                STATE_PENDING_INSTALL_PATHS, new ArrayList<>(pendingInstallPaths));
        outState.putString(STATE_ACTIVE_INSTALL_PATH, activeInstallPath);
        outState.putString(STATE_ACTIVE_INSTALL_CALLBACK_TOKEN, activeInstallCallbackToken);
        outState.putBoolean(STATE_UNKNOWN_SOURCE_SETTINGS_OPEN, unknownSourceSettingsOpen);
        outState.putBoolean(STATE_SYNC_INTENT_CONSUMED, syncIntentConsumed);
        outState.putString(STATE_LAST_INSTALL_CALLBACK_TOKEN, lastHandledInstallCallbackToken);
    }

    private void handleFastExplorerSyncIntent(Intent intent) {
        if (intent == null || !ACTION_FAST_EXPLORER_SYNC.equals(intent.getAction())) {
            return;
        }
        String token = intent.getStringExtra(FastExplorerPush.EXTRA_SYNC_NOTIFICATION_TOKEN);
        if (token != null && !token.isBlank()) {
            String lastToken = getSharedPreferences(INTENT_DEDUPE_PREFS, MODE_PRIVATE)
                    .getString(KEY_LAST_SYNC_TOKEN, "");
            if (!token.equals(lastToken)) {
                getSharedPreferences(INTENT_DEDUPE_PREFS, MODE_PRIVATE)
                        .edit()
                        .putString(KEY_LAST_SYNC_TOKEN, token)
                        .apply();
                nativeSyncNotificationOpened();
            }
            intent.setAction(null);
            return;
        }
        // Legacy notifications did not carry an identity token. Saved instance state
        // still prevents them from replaying across a normal Activity recreation.
        if (!syncIntentConsumed) {
            syncIntentConsumed = true;
            nativeSyncNotificationOpened();
        }
        intent.setAction(null);
    }

    @Override
    protected void onStop() {
        installFlowActivityVisible = false;
        super.onStop();
    }

    static boolean isVisibleForInstallFlow() {
        return installFlowActivityVisible;
    }

    @Override
    protected void onResume() {
        super.onResume();
        installFlowActivityVisible = true;
        nativeActivityResumed();
        handlePendingInstallStatus();
        if (unknownSourceSettingsOpen) {
            unknownSourceSettingsOpen = false;
            if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O
                    || getPackageManager().canRequestPackageInstalls()) {
                getWindow().getDecorView().post(this::drainInstallQueue);
            } else {
                clearPendingInstallQueue();
                Toast.makeText(
                        this,
                        "App installation permission was not granted",
                        Toast.LENGTH_LONG).show();
            }
        } else {
            getWindow().getDecorView().post(this::drainInstallQueue);
        }
    }

    @Override
    protected void onPause() {
        installFlowActivityVisible = false;
        nativeActivityPaused();
        super.onPause();
    }

    @Override
    protected void onDestroy() {
        if (Build.VERSION.SDK_INT >= 33 && backCallback != null) {
            getOnBackInvokedDispatcher().unregisterOnBackInvokedCallback(backCallback);
            backCallback = null;
        }
        aabInstallerExecutor.shutdownNow();
        stopNetworkSnapshotter();
        super.onDestroy();
    }

    @Override
    @SuppressLint("MissingSuperCall")
    @SuppressWarnings("deprecation")
    public void onBackPressed() {
        nativeBackPressed();
    }

    public void backgroundFastExplorerTask() {
        moveTaskToBack(true);
    }

    public void setFastExplorerClipboardText(String text) {
        ClipboardManager clipboard = (ClipboardManager) getSystemService(CLIPBOARD_SERVICE);
        clipboard.setPrimaryClip(ClipData.newPlainText("FastExplorer", text == null ? "" : text));
    }

    public String getFastExplorerClipboardText() {
        ClipboardManager clipboard = (ClipboardManager) getSystemService(CLIPBOARD_SERVICE);
        if (!clipboard.hasPrimaryClip() || clipboard.getPrimaryClip() == null
                || clipboard.getPrimaryClip().getItemCount() == 0) {
            return "";
        }
        CharSequence value = clipboard.getPrimaryClip().getItemAt(0).coerceToText(this);
        return value == null ? "" : value.toString();
    }

    public void ensureFastExplorerNotificationPermission() {
        if (Build.VERSION.SDK_INT < 33
                || checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS)
                        == PackageManager.PERMISSION_GRANTED) {
            return;
        }
        android.content.SharedPreferences prefs = getSharedPreferences(
                "fast_explorer_permissions", MODE_PRIVATE);
        if (!prefs.getBoolean("notification_prompted", false)) {
            prefs.edit().putBoolean("notification_prompted", true).apply();
            requestPermissions(
                    new String[] { Manifest.permission.POST_NOTIFICATIONS },
                    NOTIFICATION_PERMISSION_REQUEST_CODE);
        }
    }

    public String getFastExplorerFcmToken() {
        return FastExplorerPush.cachedToken(this);
    }

    public void notifyFastExplorerIncomingSync(String title, String detail) {
        if (Build.VERSION.SDK_INT >= 33
                && checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS)
                        != PackageManager.PERMISSION_GRANTED) {
            ensureFastExplorerNotificationPermission();
            return;
        }
        FastExplorerPush.showNotification(this, title, detail);
    }

    public void startFastExplorerTransferService() {
        Intent service = new Intent(this, FastExplorerTransferService.class);
        try {
            startForegroundService(service);
        } catch (RuntimeException error) {
            android.util.Log.e("FastExplorer", "cannot start transfer foreground service", error);
        }

        if (Build.VERSION.SDK_INT >= 33
                && checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS)
                        != PackageManager.PERMISSION_GRANTED) {
            android.content.SharedPreferences prefs = getSharedPreferences(
                    "fast_explorer_permissions", MODE_PRIVATE);
            if (!prefs.getBoolean("notification_prompted", false)) {
                prefs.edit().putBoolean("notification_prompted", true).apply();
                requestPermissions(
                        new String[] { Manifest.permission.POST_NOTIFICATIONS },
                        NOTIFICATION_PERMISSION_REQUEST_CODE);
            }
        }
    }

    public String getFastExplorerNetworkInterfacesJson() {
        // Native startup may race with the asynchronous ConnectivityManager callback.
        // Always take a fresh synchronous snapshot so tsnet's initial netmon state is
        // populated before Server.Start(). Android otherwise polls netmon only very
        // infrequently and can remain stuck in NoState until a manual reconnect.
        refreshNetworkInterfacesJson();
        return networkInterfacesJson;
    }


    public String getFastExplorerRemoteOpenCacheDir() {
        // Return only the location here. Directory creation, legacy cleanup, sizing,
        // and eviction are intentionally performed by Rust background workers so a
        // JNI call from the UI thread never turns into filesystem I/O.
        return new File(getFilesDir(), "remote-open").getAbsolutePath();
    }

    public String getFastExplorerRemoteOpenLeasesJson() {
        JSONArray protectedFiles = new JSONArray();
        try {
            JSONArray providerLeases = new JSONArray(RemoteOpenFileProvider.leasedFilesJson());
            for (int index = 0; index < providerLeases.length(); index++) {
                String name = providerLeases.optString(index, "");
                if (!name.isEmpty()) protectedFiles.put(name);
            }
        } catch (org.json.JSONException error) {
            android.util.Log.w("FastExplorer", "cannot parse remote-open leases", error);
        }
        if (activeInstallPath != null) {
            protectedFiles.put(new File(activeInstallPath).getName());
        }
        for (String path : pendingInstallPaths) {
            protectedFiles.put(new File(path).getName());
        }
        return protectedFiles.toString();
    }

    private static boolean isBelowRoot(File file, File root) {
        String rootPath = root.getPath();
        String filePath = file.getPath();
        return filePath.equals(rootPath) || filePath.startsWith(rootPath + File.separator);
    }

    private boolean isOnMountedStorageVolume(File file) {
        StorageManager storageManager = getSystemService(StorageManager.class);
        if (storageManager == null) return false;
        for (StorageVolume volume : storageManager.getStorageVolumes()) {
            File directory = volume.getDirectory();
            if (directory == null) continue;
            try {
                if (isBelowRoot(file, directory.getCanonicalFile())) return true;
            } catch (IOException ignored) {
                // Ignore an unavailable/unmounted volume and continue with the rest.
            }
        }
        return false;
    }

    private File resolveFastExplorerShareableFile(String absolutePath) throws IOException {
        File file = new File(absolutePath).getCanonicalFile();
        if (!file.isFile()) {
            throw new IOException("not a regular file");
        }
        File cache = new File(getFilesDir(), "remote-open").getCanonicalFile();
        File aabInstall = new File(getFilesDir(), "aab-install").getCanonicalFile();
        if (!isBelowRoot(file, cache)
                && !isBelowRoot(file, aabInstall)
                && !isOnMountedStorageVolume(file)) {
            throw new IOException("refusing to expose file outside FastExplorer roots");
        }
        return file;
    }

    private static String mimeForFile(File file) {
        String name = file.getName();
        int dot = name.lastIndexOf('.');
        String extension = dot >= 0 ? name.substring(dot + 1).toLowerCase(Locale.ROOT) : "";
        String mime = MimeTypeMap.getSingleton().getMimeTypeFromExtension(extension);
        return (mime == null || mime.isEmpty()) ? "application/octet-stream" : mime;
    }

    private boolean isGeneratedAabInstallPath(String path) {
        try {
            File root = new File(getFilesDir(), "aab-install").getCanonicalFile();
            File file = new File(path).getCanonicalFile();
            return file.getPath().startsWith(root.getPath() + File.separator);
        } catch (IOException error) {
            return false;
        }
    }

    private void releaseActiveInstall() {
        String path = activeInstallPath;
        activeInstallPath = null;
        activeInstallCallbackToken = null;
        if (path != null && isGeneratedAabInstallPath(path)) {
            FastExplorerApksInstaller.deleteRecursivelyAsync(new File(path));
        }
    }

    private boolean releaseActiveInstallIfMatches(String callbackToken, String sourcePath) {
        if (callbackToken == null
                || sourcePath == null
                || !callbackToken.equals(activeInstallCallbackToken)
                || !sourcePath.equals(activeInstallPath)) {
            return false;
        }
        releaseActiveInstall();
        return true;
    }

    private void enqueueInstall(String path) {
        if (path.equals(activeInstallPath) || pendingInstallPaths.contains(path)) {
            return;
        }
        pendingInstallPaths.addLast(path);
    }

    private void clearPendingInstallQueue() {
        String path;
        while ((path = pendingInstallPaths.pollFirst()) != null) {
            if (isGeneratedAabInstallPath(path)) {
                FastExplorerApksInstaller.deleteRecursivelyAsync(new File(path));
            }
        }
    }

    private void drainInstallQueue() {
        if (activeInstallPath != null || pendingInstallPaths.isEmpty()) {
            return;
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O
                && !getPackageManager().canRequestPackageInstalls()) {
            if (!unknownSourceSettingsOpen) {
                unknownSourceSettingsOpen = true;
                Intent permission = new Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES)
                        .setData(Uri.parse("package:" + getPackageName()));
                try {
                    startActivity(permission);
                } catch (ActivityNotFoundException error) {
                    unknownSourceSettingsOpen = false;
                    clearPendingInstallQueue();
                    android.util.Log.e("FastExplorer", "cannot open unknown-app settings", error);
                    Toast.makeText(
                            this,
                            "Cannot open Android's app installation permission settings",
                            Toast.LENGTH_LONG).show();
                }
            }
            return;
        }

        String path = pendingInstallPaths.pollFirst();
        if (path == null) {
            return;
        }
        try {
            File file = resolveFastExplorerShareableFile(path);
            String lowerName = file.getName().toLowerCase(Locale.ROOT);
            activeInstallPath = path;
            if (lowerName.endsWith(".aab")) {
                prepareAabForInstall(file);
            } else if (lowerName.endsWith(".apks")) {
                prepareApksForInstall(file);
            } else {
                prepareApkForInstall(file);
            }
        } catch (IOException | IllegalArgumentException error) {
            releaseActiveInstall();
            android.util.Log.e("FastExplorer", "cannot prepare app package installation", error);
            if (!isFinishing() && !isDestroyed()) {
                Toast.makeText(
                        this,
                        "Cannot install app package: " + safeErrorMessage(error),
                        Toast.LENGTH_LONG).show();
            }
            getWindow().getDecorView().post(this::drainInstallQueue);
        }
    }

    private void prepareApkForInstall(File apk) {
        aabInstallerExecutor.execute(() -> {
            try {
                FastExplorerApksInstaller.PreparedInstall prepared =
                        FastExplorerApksInstaller.prepareSingleApk(this, apk);
                runOnUiThread(() -> commitPreparedApksInstall(prepared));
            } catch (Exception error) {
                android.util.Log.e("FastExplorer", "cannot prepare APK for installation", error);
                runOnUiThread(() -> failActiveInstall("Cannot install APK", error));
            }
        });
    }

    private void prepareApksForInstall(File archive) {
        Toast.makeText(this, "Preparing APKS for installation…", Toast.LENGTH_SHORT).show();
        aabInstallerExecutor.execute(() -> {
            try {
                FastExplorerApksInstaller.PreparedInstall prepared =
                        FastExplorerApksInstaller.prepare(this, archive);
                runOnUiThread(() -> commitPreparedApksInstall(prepared));
            } catch (Exception error) {
                android.util.Log.e("FastExplorer", "cannot prepare APKS for installation", error);
                runOnUiThread(() -> failActiveInstall("Cannot install APKS", error));
            }
        });
    }

    private void commitPreparedApksInstall(FastExplorerApksInstaller.PreparedInstall prepared) {
        if (isFinishing() || isDestroyed()) {
            FastExplorerApksInstaller.deleteRecursivelyAsync(prepared.workingDirectory);
            releaseActiveInstall();
            return;
        }
        final String sourcePath = activeInstallPath;
        if (sourcePath == null) {
            FastExplorerApksInstaller.deleteRecursivelyAsync(prepared.workingDirectory);
            return;
        }
        final String callbackToken = UUID.randomUUID().toString();
        activeInstallCallbackToken = callbackToken;
        final PendingIntent status;
        try {
            Intent statusIntent = new Intent(this, FastExplorerInstallStatusReceiver.class)
                    .setData(Uri.parse("fastexplorer-install://callback/" + callbackToken))
                    .putExtra(EXTRA_APKS_WORK_DIR, prepared.workingDirectory.getAbsolutePath())
                    .putExtra(EXTRA_INSTALL_SOURCE_PATH, sourcePath)
                    .putExtra(EXTRA_INSTALL_CALLBACK_TOKEN, callbackToken);
            int pendingIntentFlags = PendingIntent.FLAG_UPDATE_CURRENT;
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                pendingIntentFlags |= PendingIntent.FLAG_MUTABLE;
            }
            int requestCode = nextApksCallbackRequestCode++;
            if (nextApksCallbackRequestCode == Integer.MAX_VALUE) {
                nextApksCallbackRequestCode = 70001;
            }
            status = PendingIntent.getBroadcast(this, requestCode, statusIntent, pendingIntentFlags);
        } catch (RuntimeException error) {
            FastExplorerApksInstaller.deleteRecursivelyAsync(prepared.workingDirectory);
            android.util.Log.e("FastExplorer", "cannot create package install callback", error);
            failActiveInstall("Cannot install app package", error);
            return;
        }

        try {
            aabInstallerExecutor.execute(() -> {
                try {
                    FastExplorerApksInstaller.commit(this, prepared, status.getIntentSender());
                } catch (Exception error) {
                    FastExplorerApksInstaller.deleteRecursively(prepared.workingDirectory);
                    android.util.Log.e("FastExplorer", "cannot commit package installation", error);
                    runOnUiThread(() -> failActiveInstall("Cannot install app package", error));
                }
            });
        } catch (RuntimeException error) {
            FastExplorerApksInstaller.deleteRecursivelyAsync(prepared.workingDirectory);
            android.util.Log.e("FastExplorer", "cannot schedule package installation", error);
            failActiveInstall("Cannot install app package", error);
        }
    }

    private static String safeErrorMessage(Throwable error) {
        String detail = error == null ? null : error.getMessage();
        return detail == null || detail.isBlank() ? "unknown error" : detail;
    }

    private void failActiveInstall(String prefix, Throwable error) {
        releaseActiveInstall();
        if (!isFinishing() && !isDestroyed()) {
            Toast.makeText(
                    this,
                    prefix + ": " + safeErrorMessage(error),
                    Toast.LENGTH_LONG).show();
            drainInstallQueue();
        }
    }

    private void handlePendingInstallStatus() {
        Intent pending = FastExplorerInstallStatusReceiver.pendingTerminalStatus(this);
        if (pending != null) handleApksInstallStatus(pending);
    }

    private void handleApksInstallStatus(Intent intent) {
        if (intent == null || !ACTION_APKS_INSTALL_STATUS.equals(intent.getAction())) {
            return;
        }
        String workDir = intent.getStringExtra(EXTRA_APKS_WORK_DIR);
        String sourcePath = intent.getStringExtra(EXTRA_INSTALL_SOURCE_PATH);
        String callbackToken = intent.getStringExtra(EXTRA_INSTALL_CALLBACK_TOKEN);
        String persistedCallbackToken = getSharedPreferences(INTENT_DEDUPE_PREFS, MODE_PRIVATE)
                .getString(KEY_LAST_INSTALL_CALLBACK_TOKEN, "");
        if (callbackToken != null
                && (callbackToken.equals(lastHandledInstallCallbackToken)
                        || callbackToken.equals(persistedCallbackToken))) {
            // PackageInstaller may relaunch an Activity from a historic Intent after
            // process death. Persist the callback identity so a completed install is
            // never reported or cleaned up twice.
            FastExplorerInstallStatusReceiver.clearPendingTerminalStatus(this, callbackToken);
            intent.setAction(null);
            return;
        }
        if (callbackToken == null
                || callbackToken.isBlank()
                || !FastExplorerApksInstaller.isWorkingDirectory(this, workDir)) {
            android.util.Log.w("FastExplorer", "Ignoring invalid package install callback");
            FastExplorerInstallStatusReceiver.clearPendingTerminalStatus(this, callbackToken);
            intent.setAction(null);
            return;
        }
        int status = intent.getIntExtra(
                PackageInstaller.EXTRA_STATUS, PackageInstaller.STATUS_FAILURE);
        if (status == PackageInstaller.STATUS_PENDING_USER_ACTION) {
            // Pending user action is handled only by our non-exported callback receiver.
            // Never launch a nested Intent delivered directly to the exported main Activity.
            android.util.Log.w("FastExplorer", "Ignoring pending install action in main Activity");
            intent.setAction(null);
            return;
        }

        lastHandledInstallCallbackToken = callbackToken;
        getSharedPreferences(INTENT_DEDUPE_PREFS, MODE_PRIVATE)
                .edit()
                .putString(KEY_LAST_INSTALL_CALLBACK_TOKEN, callbackToken)
                .apply();
        FastExplorerInstallStatusReceiver.clearPendingTerminalStatus(this, callbackToken);
        FastExplorerApksInstaller.cleanupWorkingDirectoryAsync(this, workDir);
        if (sourcePath != null
                && isGeneratedAabInstallPath(sourcePath)
                && !sourcePath.equals(activeInstallPath)) {
            // Stale/process-restored callbacks no longer own an Activity slot, so
            // clean their generated AAB artifact directly. The matching active slot
            // is cleaned by releaseActiveInstall() below to avoid double scheduling.
            FastExplorerApksInstaller.deleteRecursivelyAsync(new File(sourcePath));
        }
        String statusMessage = intent.getStringExtra(PackageInstaller.EXTRA_STATUS_MESSAGE);
        String blocker = intent.getStringExtra(PackageInstaller.EXTRA_OTHER_PACKAGE_NAME);
        if (!isFinishing() && !isDestroyed()) {
            String message = status == PackageInstaller.STATUS_SUCCESS
                    ? "App installed"
                    : packageInstallFailureMessage(status, statusMessage, blocker);
            Toast.makeText(this, message, Toast.LENGTH_LONG).show();
        }
        intent.setAction(null);
        boolean releasedCurrent = releaseActiveInstallIfMatches(callbackToken, sourcePath);
        if (releasedCurrent || activeInstallPath == null) {
            getWindow().getDecorView().post(this::drainInstallQueue);
        }
    }

    static String packageInstallFailureMessage(int status, String detail, String blocker) {
        String message;
        switch (status) {
            case PackageInstaller.STATUS_FAILURE_ABORTED:
                message = "Installation was cancelled or rejected by Android";
                break;
            case PackageInstaller.STATUS_FAILURE_BLOCKED:
                message = "Installation was blocked by Android or a security verifier";
                break;
            case PackageInstaller.STATUS_FAILURE_CONFLICT:
                message = "Cannot install app: package signature or installed app conflicts";
                break;
            case PackageInstaller.STATUS_FAILURE_INCOMPATIBLE:
                message = "Cannot install app: package is incompatible with this device";
                break;
            case PackageInstaller.STATUS_FAILURE_INVALID:
                message = "Cannot install app: package is invalid or incomplete";
                break;
            case PackageInstaller.STATUS_FAILURE_STORAGE:
                message = "Cannot install app: not enough storage";
                break;
            case PackageInstaller.STATUS_FAILURE_TIMEOUT:
                message = "Cannot install app: Android timed out";
                break;
            default:
                message = "Cannot install app";
                break;
        }
        if (detail != null && !detail.isBlank()) {
            message += ": " + detail;
        }
        if (blocker != null && !blocker.isBlank()) {
            message += status == PackageInstaller.STATUS_FAILURE_BLOCKED
                    ? " (blocked by " + blocker + ")"
                    : " (related package: " + blocker + ")";
        }
        return message;
    }

    private void confirmAppPackageInstall(File file, Runnable onConfirm) {
        getWindow().getDecorView().post(() -> {
            if (isFinishing() || isDestroyed()) {
                return;
            }
            new AlertDialog.Builder(this)
                    .setTitle("Install app package?")
                    .setMessage(
                            "FastExplorer will hand \"" + file.getName()
                                    + "\" to Android only because you selected it. "
                                    + "Android will make the final installation decision and may show "
                                    + "its own security confirmation or block the install.")
                    .setNegativeButton("Cancel", null)
                    .setPositiveButton("Continue", (dialog, which) -> onConfirm.run())
                    .show();
        });
    }

    private void prepareAabForInstall(File bundle) {
        final String sourcePath = bundle.getAbsolutePath();
        Toast.makeText(this, "Preparing AAB for installation…", Toast.LENGTH_SHORT).show();
        try {
            aabInstallerExecutor.execute(() -> {
                try {
                    File apk = FastExplorerAabInstaller.buildUniversalApk(this, bundle);
                    runOnUiThread(() -> {
                        if (isFinishing() || isDestroyed()) {
                            FastExplorerApksInstaller.deleteRecursivelyAsync(apk);
                            return;
                        }
                        if (!sourcePath.equals(activeInstallPath)) {
                            // A stale preparation result must never hijack a newer queued install.
                            FastExplorerApksInstaller.deleteRecursivelyAsync(apk);
                            return;
                        }
                        // Keep this as the same durable queue slot. If Android recreates the
                        // Activity after conversion but before PackageInstaller commits, the
                        // generated APK path is saved as the active path and can be retried.
                        activeInstallPath = apk.getAbsolutePath();
                        prepareApkForInstall(apk);
                    });
                } catch (Exception error) {
                    android.util.Log.e("FastExplorer", "cannot prepare AAB for installation", error);
                    runOnUiThread(() -> {
                        if (sourcePath.equals(activeInstallPath)) {
                            failActiveInstall("Cannot install AAB", error);
                        }
                    });
                }
            });
        } catch (RuntimeException error) {
            android.util.Log.e("FastExplorer", "cannot schedule AAB preparation", error);
            failActiveInstall("Cannot install AAB", error);
        }
    }

    public boolean openFastExplorerFile(String absolutePath) {
        try {
            File file = resolveFastExplorerShareableFile(absolutePath);
            String lowerName = file.getName().toLowerCase(Locale.ROOT);
            if (lowerName.endsWith(".aab")) {
                String installPath = file.getAbsolutePath();
                confirmAppPackageInstall(file, () -> {
                    enqueueInstall(installPath);
                    drainInstallQueue();
                });
                return true;
            }
            if (lowerName.endsWith(".apks")) {
                String installPath = file.getAbsolutePath();
                confirmAppPackageInstall(file, () -> {
                    enqueueInstall(installPath);
                    drainInstallQueue();
                });
                return true;
            }
            String mime = mimeForFile(file);
            boolean apk = "application/vnd.android.package-archive".equals(mime)
                    || lowerName.endsWith(".apk");
            if (apk) {
                String installPath = file.getAbsolutePath();
                confirmAppPackageInstall(file, () -> {
                    enqueueInstall(installPath);
                    drainInstallQueue();
                });
                return true;
            }
            Uri uri = FileProvider.getUriForFile(this, getPackageName() + ".files", file);
            Intent intent = new Intent(Intent.ACTION_VIEW)
                    .setDataAndType(uri, mime)
                    .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
            startActivity(intent);
            return true;
        } catch (IOException | IllegalArgumentException | ActivityNotFoundException error) {
            android.util.Log.e("FastExplorer", "cannot open FastExplorer file", error);
            return false;
        }
    }

    public boolean shareFastExplorerFile(String absolutePath) {
        try {
            File file = resolveFastExplorerShareableFile(absolutePath);
            Uri uri = FileProvider.getUriForFile(this, getPackageName() + ".files", file);
            Intent send = new Intent(Intent.ACTION_SEND)
                    .setType(mimeForFile(file))
                    .putExtra(Intent.EXTRA_STREAM, uri)
                    .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
            startActivity(Intent.createChooser(send, "Share file"));
            return true;
        } catch (IOException | IllegalArgumentException | ActivityNotFoundException error) {
            android.util.Log.e("FastExplorer", "cannot share FastExplorer file", error);
            return false;
        }
    }

    public long getFastExplorerLocalDayEndUnixMs() {
        java.time.ZoneId zone = java.time.ZoneId.systemDefault();
        java.time.LocalDate tomorrow = java.time.LocalDate.now(zone).plusDays(1);
        return tomorrow.atStartOfDay(zone).toInstant().toEpochMilli();
    }

    public void notifyFastExplorerFileChanges(String pathsJson) {
        FastExplorerFileChangeNotifier.notifyChanges(this, pathsJson);
    }

    private void startNetworkSnapshotter() {
        connectivityManager = getSystemService(ConnectivityManager.class);
        if (connectivityManager == null) return;
        networkThread = new HandlerThread("FastExplorerNetwork");
        networkThread.start();
        networkHandler = new Handler(networkThread.getLooper());
        networkCallback = new ConnectivityManager.NetworkCallback() {
            @Override
            public void onAvailable(Network network) {
                refreshNetworkInterfacesJson();
            }

            @Override
            public void onLost(Network network) {
                refreshNetworkInterfacesJson();
            }

            @Override
            public void onLinkPropertiesChanged(Network network, LinkProperties properties) {
                refreshNetworkInterfacesJson();
            }
        };
        try {
            // Seed the snapshot synchronously before native code can consume it.
            refreshNetworkInterfacesJson();
            NetworkRequest request = new NetworkRequest.Builder().clearCapabilities().build();
            connectivityManager.registerNetworkCallback(request, networkCallback, networkHandler);
        } catch (Exception error) {
            android.util.Log.e("FastExplorer", "network callback registration failed", error);
        }
    }

    private void stopNetworkSnapshotter() {
        if (connectivityManager != null && networkCallback != null) {
            try {
                connectivityManager.unregisterNetworkCallback(networkCallback);
            } catch (Exception ignored) {
            }
        }
        networkCallback = null;
        connectivityManager = null;
        networkHandler = null;
        if (networkThread != null) {
            networkThread.quitSafely();
            networkThread = null;
        }
    }

    private void refreshNetworkInterfacesJson() {
        networkInterfacesJson = FastExplorerNetwork.snapshot(this);
    }
}
