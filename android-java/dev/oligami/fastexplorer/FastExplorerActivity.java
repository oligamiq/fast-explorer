package dev.oligami.fastexplorer;

import com.google.androidgamesdk.GameActivity;
import androidx.core.content.FileProvider;
import androidx.core.view.WindowCompat;
import android.Manifest;
import android.content.ActivityNotFoundException;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.graphics.Color;
import android.net.ConnectivityManager;
import android.net.LinkProperties;
import android.net.Network;
import android.net.NetworkRequest;
import android.net.Uri;
import android.media.MediaScannerConnection;
import android.os.Build;
import android.os.Bundle;
import android.os.Environment;
import android.os.Handler;
import android.os.HandlerThread;
import android.provider.DocumentsContract;
import android.provider.MediaStore;
import android.provider.Settings;
import android.window.OnBackInvokedCallback;
import android.window.OnBackInvokedDispatcher;
import android.webkit.MimeTypeMap;
import java.io.File;
import java.io.IOException;
import java.util.concurrent.ConcurrentLinkedDeque;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.Locale;
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
    private static final int INSTALL_REQUEST_CODE_BASE = 3002;
    private static final int NOTIFICATION_PERMISSION_REQUEST_CODE = 3003;
    private static final int INSTALL_REQUEST_CODE_MAX = 60000;
    private final ConcurrentLinkedDeque<String> pendingInstallPaths = new ConcurrentLinkedDeque<>();
    private volatile String activeInstallPath;
    private volatile boolean activeInstallPaused;
    private volatile int activeInstallRequestCode = -1;
    private int nextInstallRequestCode = INSTALL_REQUEST_CODE_BASE;
    private boolean unknownSourceSettingsOpen;
    private final ExecutorService aabInstallerExecutor = Executors.newSingleThreadExecutor();

    private static native void nativeBackPressed();
    private static native void nativeActivityResumed();
    private static native void nativeActivityPaused();

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
    }

    @Override
    protected void onResume() {
        super.onResume();
        nativeActivityResumed();
        if (unknownSourceSettingsOpen) {
            unknownSourceSettingsOpen = false;
            if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O
                    || getPackageManager().canRequestPackageInstalls()) {
                getWindow().getDecorView().post(this::drainInstallQueue);
            }
        } else if (activeInstallPath != null && activeInstallPaused) {
            // Some OEM package installers do not reliably return an activity result.
            // Give onActivityResult a chance to run first, then release the serialized
            // installer slot if the external installer has already returned to us.
            getWindow().getDecorView().postDelayed(() -> {
                if (activeInstallPath != null && activeInstallPaused) {
                    releaseActiveInstall();
                    drainInstallQueue();
                }
            }, 250L);
        } else {
            getWindow().getDecorView().post(this::drainInstallQueue);
        }
    }

    @Override
    protected void onPause() {
        if (activeInstallPath != null) {
            activeInstallPaused = true;
        }
        nativeActivityPaused();
        super.onPause();
    }

    @Override
    protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        if (requestCode == activeInstallRequestCode) {
            releaseActiveInstall();
            getWindow().getDecorView().post(this::drainInstallQueue);
        }
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
    @SuppressWarnings("deprecation")
    public void onBackPressed() {
        nativeBackPressed();
    }

    public void backgroundFastExplorerTask() {
        moveTaskToBack(true);
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

    private static void deleteRecursively(File path) {
        File[] children = path.listFiles();
        if (children != null) {
            for (File child : children) {
                deleteRecursively(child);
            }
        }
        path.delete();
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

    private File resolveFastExplorerShareableFile(String absolutePath) throws IOException {
        File file = new File(absolutePath).getCanonicalFile();
        if (!file.isFile()) {
            throw new IOException("not a regular file");
        }
        File cache = new File(getFilesDir(), "remote-open").getCanonicalFile();
        File aabInstall = new File(getFilesDir(), "aab-install").getCanonicalFile();
        File shared = Environment.getExternalStorageDirectory().getCanonicalFile();
        String cachePrefix = cache.getPath() + File.separator;
        String aabInstallPrefix = aabInstall.getPath() + File.separator;
        String sharedPrefix = shared.getPath() + File.separator;
        if (!file.getPath().startsWith(cachePrefix)
                && !file.getPath().startsWith(aabInstallPrefix)
                && !file.getPath().startsWith(sharedPrefix)) {
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
        activeInstallPaused = false;
        activeInstallRequestCode = -1;
        if (path != null && isGeneratedAabInstallPath(path)) {
            new File(path).delete();
        }
    }

    private void enqueueInstall(String path) {
        if (path.equals(activeInstallPath) || pendingInstallPaths.contains(path)) {
            return;
        }
        pendingInstallPaths.addLast(path);
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
                    android.util.Log.e("FastExplorer", "cannot open unknown-app settings", error);
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
            Uri uri = FileProvider.getUriForFile(this, getPackageName() + ".files", file);
            activeInstallPath = path;
            activeInstallPaused = false;
            int requestCode = nextInstallRequestCode;
            nextInstallRequestCode++;
            if (nextInstallRequestCode > INSTALL_REQUEST_CODE_MAX) {
                nextInstallRequestCode = INSTALL_REQUEST_CODE_BASE;
            }
            activeInstallRequestCode = requestCode;
            Intent install = new Intent(Intent.ACTION_INSTALL_PACKAGE)
                    .setData(uri)
                    .putExtra(Intent.EXTRA_RETURN_RESULT, true)
                    .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
            startActivityForResult(install, requestCode);
        } catch (IOException | IllegalArgumentException | ActivityNotFoundException error) {
            releaseActiveInstall();
            android.util.Log.e("FastExplorer", "cannot install FastExplorer APK", error);
            getWindow().getDecorView().post(this::drainInstallQueue);
        }
    }

    private void prepareAabForInstall(File bundle) {
        Toast.makeText(this, "Preparing AAB for installation…", Toast.LENGTH_SHORT).show();
        aabInstallerExecutor.execute(() -> {
            try {
                File apk = FastExplorerAabInstaller.buildUniversalApk(this, bundle);
                runOnUiThread(() -> {
                    if (isFinishing() || isDestroyed()) {
                        apk.delete();
                        return;
                    }
                    enqueueInstall(apk.getAbsolutePath());
                    drainInstallQueue();
                });
            } catch (Exception error) {
                android.util.Log.e("FastExplorer", "cannot prepare AAB for installation", error);
                runOnUiThread(() -> {
                    if (!isFinishing() && !isDestroyed()) {
                        Toast.makeText(
                                this,
                                "Cannot install AAB: " + error.getMessage(),
                                Toast.LENGTH_LONG).show();
                    }
                });
            }
        });
    }

    public boolean openFastExplorerFile(String absolutePath) {
        try {
            File file = resolveFastExplorerShareableFile(absolutePath);
            String lowerName = file.getName().toLowerCase(Locale.ROOT);
            if (lowerName.endsWith(".aab")) {
                prepareAabForInstall(file);
                return true;
            }
            String mime = mimeForFile(file);
            boolean apk = "application/vnd.android.package-archive".equals(mime)
                    || file.getName().toLowerCase(Locale.ROOT).endsWith(".apk");
            if (apk) {
                String installPath = file.getAbsolutePath();
                getWindow().getDecorView().post(() -> {
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

    private Uri fastExplorerDocumentsChangeUri() {
        return Uri.parse("content://" + getPackageName() + ".documents/changes");
    }

    public void notifyFastExplorerDocumentsChanged() {
        getContentResolver().notifyChange(fastExplorerDocumentsChangeUri(), null);
    }

    public void notifyFastExplorerFileChanges(String pathsJson) {
        try {
            JSONArray array = new JSONArray(pathsJson);
            String[] paths = new String[array.length()];
            for (int i = 0; i < array.length(); i++) paths[i] = array.getString(i);
            if (paths.length != 0) {
                MediaScannerConnection.scanFile(this, paths, null, null);
            }
            try {
                getContentResolver().notifyChange(MediaStore.Files.getContentUri("external"), null);
            } catch (SecurityException ignored) {
                // MediaStore notifications may be restricted on OEM builds.
            }

            File sharedRoot = Environment.getExternalStorageDirectory().getCanonicalFile();
            String rootPath = sharedRoot.getPath();
            for (String raw : paths) {
                File changed = new File(raw).getCanonicalFile();
                File parent = changed.getParentFile();
                if (parent == null) continue;
                String parentPath = parent.getPath();
                if (!parentPath.equals(rootPath)
                        && !parentPath.startsWith(rootPath + File.separator)) continue;
                String relative = parentPath.equals(rootPath)
                        ? ""
                        : parentPath.substring(rootPath.length() + 1).replace(File.separatorChar, '/');
                String documentId = "primary:" + relative;
                Uri children = DocumentsContract.buildChildDocumentsUri(
                        "com.android.externalstorage.documents", documentId);
                try {
                    getContentResolver().notifyChange(children, null);
                } catch (SecurityException ignored) {
                    // Some OEM DocumentsProviders reject third-party notifications.
                }
            }
        } catch (Exception error) {
            android.util.Log.w("FastExplorer", "cannot notify Android file changes", error);
        }
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
