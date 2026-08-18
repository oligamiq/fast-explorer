package dev.oligami.fastexplorer;

import android.content.Context;
import android.media.MediaScannerConnection;
import android.net.Uri;
import android.os.Environment;
import android.provider.DocumentsContract;
import java.io.File;
import java.io.IOException;
import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Set;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import org.json.JSONArray;

final class FastExplorerFileChangeNotifier {
    private static final int SCAN_BATCH_PATHS = 128;
    private static final int SCAN_BATCH_CHARS = 128 * 1024;
    private static final ExecutorService EXECUTOR = Executors.newSingleThreadExecutor();

    private FastExplorerFileChangeNotifier() {}

    static void notifyChanges(Context context, String pathsJson) {
        final List<String> paths = parsePaths(pathsJson);
        if (paths.isEmpty()) return;
        Context appContext = context.getApplicationContext();
        try {
            EXECUTOR.execute(() -> notifyInBackground(appContext, paths));
        } catch (RuntimeException error) {
            android.util.Log.w("FastExplorer", "cannot schedule Android file changes", error);
        }
    }
    private static List<String> parsePaths(String pathsJson) {
        ArrayList<String> paths = new ArrayList<>();
        try {
            JSONArray array = new JSONArray(pathsJson);
            for (int index = 0; index < array.length(); index++) {
                String path = array.optString(index, "");
                if (!path.isBlank()) paths.add(path);
            }
        } catch (Exception error) {
            android.util.Log.w("FastExplorer", "cannot parse Android file changes", error);
        }
        return paths;
    }

    private static void notifyInBackground(Context context, List<String> paths) {
        Set<String> scanTargets = new LinkedHashSet<>();
        for (String raw : paths) {
            try {
                collectScanTargets(new File(raw), scanTargets);
            } catch (RuntimeException error) {
                android.util.Log.w("FastExplorer", "cannot prepare media scan for " + raw, error);
                scanTargets.add(new File(raw).getAbsolutePath());
            }
        }
        scanTargets(context, scanTargets);

        Uri providerChanges = Uri.parse(
                "content://" + context.getPackageName() + ".documents/changes");
        try {
            context.getContentResolver().notifyChange(providerChanges, null);
        } catch (RuntimeException error) {
            android.util.Log.w("FastExplorer", "cannot notify TailDrive DocumentsProvider", error);
        }
        // MediaScannerConnection emits granular MediaStore notifications for each
        // scanned path. Avoid globally invalidating every MediaStore observer here.
        notifyExternalStorageObservers(context, paths);
    }

    private static File canonicalOrAbsolute(File file) {
        try {
            return file.getCanonicalFile();
        } catch (IOException error) {
            return file.getAbsoluteFile();
        }
    }

    private static void collectScanTargets(File raw, Set<String> targets) {
        File changed = canonicalOrAbsolute(raw);
        targets.add(changed.getPath());
        if (!changed.exists()) {
            File parent = changed.getParentFile();
            while (parent != null && !parent.exists()) {
                parent = parent.getParentFile();
            }
            if (parent != null) targets.add(canonicalOrAbsolute(parent).getPath());
            return;
        }
        if (!changed.isDirectory()) return;

        String rootPath = changed.getPath();
        ArrayDeque<File> pending = new ArrayDeque<>();
        Set<String> visitedDirectories = new LinkedHashSet<>();
        pending.add(changed);
        while (!pending.isEmpty()) {
            File candidate = canonicalOrAbsolute(pending.removeFirst());
            String path = candidate.getPath();
            if (!path.equals(rootPath) && !path.startsWith(rootPath + File.separator)) continue;
            if (candidate.isDirectory()) {
                if (!visitedDirectories.add(path)) continue;
                File[] children = candidate.listFiles();
                if (children != null) {
                    for (File child : children) pending.addLast(child);
                }
            } else {
                targets.add(path);
            }
        }
    }

    static List<String> collectScanTargetsForTesting(File raw) {
        Set<String> targets = new LinkedHashSet<>();
        collectScanTargets(raw, targets);
        return new ArrayList<>(targets);
    }

    private static void scanTargets(Context context, Set<String> targets) {
        ArrayList<String> batch = new ArrayList<>();
        int batchChars = 0;
        for (String path : targets) {
            if (!batch.isEmpty()
                    && (batch.size() >= SCAN_BATCH_PATHS
                            || batchChars + path.length() > SCAN_BATCH_CHARS)) {
                scanBatch(context, batch);
                batch.clear();
                batchChars = 0;
            }
            batch.add(path);
            batchChars += path.length();
        }
        if (!batch.isEmpty()) scanBatch(context, batch);
    }

    private static void scanBatch(Context context, List<String> batch) {
        MediaScannerConnection.scanFile(context, batch.toArray(new String[0]), null, null);
    }
    private static void notifyExternalStorageObservers(Context context, List<String> paths) {
        File sharedRoot = canonicalOrAbsolute(Environment.getExternalStorageDirectory());
        String rootPath = sharedRoot.getPath();
        Set<String> observerDirectories = new LinkedHashSet<>();
        for (String raw : paths) {
            try {
                File changed = canonicalOrAbsolute(new File(raw));
                File parent = changed.getParentFile();
                if (parent != null && isAtOrBelow(parent.getPath(), rootPath)) {
                    observerDirectories.add(parent.getPath());
                }
                if (changed.isDirectory() && isAtOrBelow(changed.getPath(), rootPath)) {
                    observerDirectories.add(changed.getPath());
                }
            } catch (RuntimeException error) {
                android.util.Log.w(
                        "FastExplorer", "cannot prepare DocumentsProvider notification for " + raw, error);
            }
        }
        for (String observerPath : observerDirectories) {
            String relative = observerPath.equals(rootPath)
                    ? ""
                    : observerPath.substring(rootPath.length() + 1)
                            .replace(File.separatorChar, '/');
            Uri children = DocumentsContract.buildChildDocumentsUri(
                    "com.android.externalstorage.documents", "primary:" + relative);
            try {
                context.getContentResolver().notifyChange(children, null);
            } catch (RuntimeException error) {
                // Some OEM DocumentsProviders reject third-party notifications.
                android.util.Log.w("FastExplorer", "cannot notify external-storage observer", error);
            }
        }
    }
    private static boolean isAtOrBelow(String path, String rootPath) {
        return path.equals(rootPath) || path.startsWith(rootPath + File.separator);
    }
}
