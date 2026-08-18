package dev.oligami.fastexplorer;

import android.content.Context;
import android.content.IntentSender;
import android.content.pm.PackageInstaller;
import android.content.pm.PackageManager;
import android.content.res.Configuration;
import android.os.Build;
import android.os.LocaleList;
import com.android.bundle.Devices;
import com.android.tools.build.bundletool.commands.ExtractApksCommand;
import java.io.File;
import java.io.FileInputStream;
import java.io.InputStream;
import java.io.OutputStream;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Locale;
import java.util.Set;
import java.util.UUID;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

final class FastExplorerApksInstaller {
    private static final ExecutorService CLEANUP_EXECUTOR = Executors.newSingleThreadExecutor();

    private FastExplorerApksInstaller() {}

    static final class PreparedInstall {
        final File workingDirectory;
        final List<Path> apkPaths;

        PreparedInstall(File workingDirectory, List<Path> apkPaths) {
            this.workingDirectory = workingDirectory;
            this.apkPaths = apkPaths;
        }
    }

    static PreparedInstall prepare(Context context, File archive) throws Exception {
        File workingDirectory = createWorkingDirectory(context);
        boolean complete = false;
        try {
            List<Path> extracted = ExtractApksCommand.builder()
                    .setApksArchivePath(archive.toPath())
                    .setDeviceSpec(buildDeviceSpec(context))
                    .setOutputDirectory(workingDirectory.toPath())
                    .setIncludeInstallTimeAssetModules(true)
                    .setInstant(false)
                    .setIncludeMetadata(false)
                    .build()
                    .execute();
            List<Path> apkPaths = validateExtractedApks(workingDirectory, extracted);
            complete = true;
            return new PreparedInstall(workingDirectory, apkPaths);
        } finally {
            if (!complete) {
                deleteRecursively(workingDirectory);
            }
        }
    }

    static PreparedInstall prepareSingleApk(Context context, File apk) throws Exception {
        File canonical = apk.getCanonicalFile();
        if (!canonical.isFile() || canonical.length() == 0) {
            throw new IllegalArgumentException("Selected APK is empty or unavailable");
        }
        File workingDirectory = createWorkingDirectory(context);
        return new PreparedInstall(workingDirectory, List.of(canonical.toPath()));
    }

    private static File createWorkingDirectory(Context context) throws Exception {
        File root = new File(context.getFilesDir(), "apks-install");
        if (!root.exists() && !root.mkdirs()) {
            throw new IllegalStateException("Cannot create package installer cache");
        }
        cleanupStaleChildren(root);
        File workingDirectory = new File(root, UUID.randomUUID().toString());
        if (!workingDirectory.mkdirs()) {
            throw new IllegalStateException("Cannot create package installer working directory");
        }
        return workingDirectory;
    }

    static int commit(Context context, PreparedInstall prepared, IntentSender statusReceiver)
            throws Exception {
        PackageInstaller installer = context.getPackageManager().getPackageInstaller();
        PackageInstaller.SessionParams params = new PackageInstaller.SessionParams(
                PackageInstaller.SessionParams.MODE_FULL_INSTALL);
        params.setInstallReason(PackageManager.INSTALL_REASON_USER);
        long totalBytes = 0L;
        for (Path path : prepared.apkPaths) {
            totalBytes += path.toFile().length();
        }
        params.setSize(totalBytes);
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            // Tell Android this is a user-selected local file, not an app-store or
            // background downloader install. This is the package source intended for
            // file managers facilitating an APK installation.
            params.setPackageSource(PackageInstaller.PACKAGE_SOURCE_LOCAL_FILE);
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            params.setRequireUserAction(PackageInstaller.SessionParams.USER_ACTION_REQUIRED);
        }

        int sessionId = installer.createSession(params);
        boolean committed = false;
        try (PackageInstaller.Session session = installer.openSession(sessionId)) {
            int index = 0;
            for (Path path : prepared.apkPaths) {
                File apk = path.toFile();
                String name = String.format(Locale.ROOT, "%03d-%s", index++, apk.getName());
                copyIntoSession(session, apk, name);
            }
            session.commit(statusReceiver);
            committed = true;
            return sessionId;
        } finally {
            if (!committed) {
                try {
                    installer.abandonSession(sessionId);
                } catch (Exception ignored) {
                    // Best effort cleanup after a failed session write.
                }
            }
        }
    }

    private static void copyIntoSession(
            PackageInstaller.Session session, File apk, String sessionName) throws Exception {
        try (InputStream input = new FileInputStream(apk);
                OutputStream output = session.openWrite(sessionName, 0, apk.length())) {
            byte[] buffer = new byte[256 * 1024];
            int read;
            while ((read = input.read(buffer)) != -1) {
                output.write(buffer, 0, read);
            }
            session.fsync(output);
        }
    }

    static Devices.DeviceSpec buildDeviceSpec(Context context) {
        Devices.DeviceSpec.Builder builder = Devices.DeviceSpec.newBuilder()
                .addAllSupportedAbis(Arrays.asList(Build.SUPPORTED_ABIS))
                .setScreenDensity(context.getResources().getDisplayMetrics().densityDpi)
                .setSdkVersion(Build.VERSION.SDK_INT);

        Set<String> locales = new LinkedHashSet<>();
        Configuration configuration = context.getResources().getConfiguration();
        LocaleList localeList = configuration.getLocales();
        for (int index = 0; index < localeList.size(); index++) {
            Locale locale = localeList.get(index);
            String tag = locale.toLanguageTag();
            if (!tag.isEmpty()) {
                locales.add(tag);
            }
            String language = locale.getLanguage();
            if (!language.isEmpty()) {
                locales.add(language);
            }
        }
        builder.addAllSupportedLocales(locales);
        if (Build.VERSION.CODENAME != null && !"REL".equals(Build.VERSION.CODENAME)) {
            builder.setCodename(Build.VERSION.CODENAME);
        }
        if (Build.BRAND != null) {
            builder.setBuildBrand(Build.BRAND);
        }
        if (Build.DEVICE != null) {
            builder.setBuildDevice(Build.DEVICE);
        }
        return builder.build();
    }
    private static List<Path> validateExtractedApks(File root, List<Path> extracted) throws Exception {
        if (extracted.isEmpty()) {
            throw new IllegalStateException("APK set has no APKs compatible with this device");
        }
        String rootPrefix = root.getCanonicalPath() + File.separator;
        List<Path> apks = new ArrayList<>();
        for (Path path : extracted) {
            File apk = path.toFile().getCanonicalFile();
            if (!apk.getPath().startsWith(rootPrefix)
                    || !apk.isFile()
                    || apk.length() == 0
                    || !apk.getName().toLowerCase(Locale.ROOT).endsWith(".apk")) {
                throw new IllegalStateException("APK set produced an invalid split");
            }
            apks.add(apk.toPath());
        }
        return List.copyOf(apks);
    }

    static void deleteRecursively(File path) {
        if (!java.nio.file.Files.isSymbolicLink(path.toPath())) {
            File[] children = path.listFiles();
            if (children != null) {
                for (File child : children) {
                    deleteRecursively(child);
                }
            }
        }
        try {
            java.nio.file.Files.deleteIfExists(path.toPath());
        } catch (java.io.IOException error) {
            android.util.Log.w("FastExplorer", "Cannot delete APKS installer file: " + path, error);
        }
    }

    static void deleteRecursivelyAsync(File path) {
        try {
            CLEANUP_EXECUTOR.execute(() -> deleteRecursively(path));
        } catch (RuntimeException error) {
            android.util.Log.w("FastExplorer", "Cannot schedule installer cleanup", error);
        }
    }

    static boolean isWorkingDirectory(Context context, String absolutePath) {
        if (absolutePath == null || absolutePath.isEmpty()) {
            return false;
        }
        try {
            File root = new File(context.getFilesDir(), "apks-install").getCanonicalFile();
            File candidate = new File(absolutePath).getCanonicalFile();
            return candidate.isDirectory()
                    && candidate.getPath().startsWith(root.getPath() + File.separator);
        } catch (Exception error) {
            return false;
        }
    }

    static void cleanupWorkingDirectory(Context context, String absolutePath) {
        if (!isWorkingDirectory(context, absolutePath)) {
            return;
        }
        try {
            deleteRecursively(new File(absolutePath).getCanonicalFile());
        } catch (Exception error) {
            android.util.Log.w("FastExplorer", "Cannot clean APKS working directory", error);
        }
    }

    static void cleanupWorkingDirectoryAsync(Context context, String absolutePath) {
        Context appContext = context.getApplicationContext();
        try {
            CLEANUP_EXECUTOR.execute(() -> cleanupWorkingDirectory(appContext, absolutePath));
        } catch (RuntimeException error) {
            android.util.Log.w("FastExplorer", "Cannot schedule APKS working-directory cleanup", error);
        }
    }

    static void cleanupStaleArtifacts(Context context) {
        cleanupStaleChildren(new File(context.getFilesDir(), "apks-install"));
        FastExplorerAabInstaller.cleanupStaleArtifacts(context);
    }

    static void cleanupStaleArtifactsAsync(Context context) {
        Context appContext = context.getApplicationContext();
        try {
            CLEANUP_EXECUTOR.execute(() -> cleanupStaleArtifacts(appContext));
        } catch (RuntimeException error) {
            android.util.Log.w("FastExplorer", "Cannot schedule installer cache cleanup", error);
        }
    }

    private static void cleanupStaleChildren(File root) {
        File[] children = root.listFiles();
        if (children == null) {
            return;
        }
        long cutoff = System.currentTimeMillis() - 24L * 60L * 60L * 1000L;
        for (File child : children) {
            if (child.lastModified() < cutoff) {
                deleteRecursively(child);
            }
        }
    }
}
