package dev.oligami.fastexplorer;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertNotEquals;
import static org.junit.Assert.assertNotNull;
import static org.junit.Assert.assertTrue;

import android.content.Context;
import android.content.pm.PackageInfo;
import android.os.Build;
import com.android.bundle.Devices;
import com.android.tools.build.bundletool.androidtools.Aapt2Command;
import com.android.tools.build.bundletool.commands.BuildApksCommand;
import androidx.test.core.app.ApplicationProvider;
import androidx.test.ext.junit.runners.AndroidJUnit4;
import androidx.test.platform.app.InstrumentationRegistry;
import java.io.File;
import java.io.FileOutputStream;
import java.io.InputStream;
import org.junit.Test;
import org.junit.runner.RunWith;

@RunWith(AndroidJUnit4.class)
public final class FastExplorerAabInstallerTest {

    @Test
    public void sameBundleGetsUniqueInstallOutputs() throws Exception {
        Context context = ApplicationProvider.getApplicationContext();
        Context testContext = InstrumentationRegistry.getInstrumentation().getContext();
        File bundle = new File(context.getCacheDir(), "aab-fixture-unique.aab");
        try (InputStream input = testContext.getAssets().open("aab-fixture.aab");
                FileOutputStream output = new FileOutputStream(bundle)) {
            input.transferTo(output);
        }

        File first = FastExplorerAabInstaller.buildUniversalApk(context, bundle);
        File second = FastExplorerAabInstaller.buildUniversalApk(context, bundle);
        try {
            assertNotEquals(first.getAbsolutePath(), second.getAbsolutePath());
            assertTrue(first.isFile());
            assertTrue(second.isFile());
        } finally {
            first.delete();
            second.delete();
            bundle.delete();
        }
    }

    @Test
    public void buildsSignedUniversalApkOnAndroid() throws Exception {
        Context context = ApplicationProvider.getApplicationContext();
        Context testContext = InstrumentationRegistry.getInstrumentation().getContext();
        File bundle = new File(context.getCacheDir(), "aab-fixture.aab");
        try (InputStream input = testContext.getAssets().open("aab-fixture.aab");
                FileOutputStream output = new FileOutputStream(bundle)) {
            input.transferTo(output);
        }

        File apk = FastExplorerAabInstaller.buildUniversalApk(context, bundle);
        try {
            assertTrue(apk.isFile());
            assertTrue(apk.length() > 0);

            PackageInfo info = context.getPackageManager().getPackageArchiveInfo(apk.getAbsolutePath(), 0);
            assertNotNull(info);
            assertEquals("dev.oligami.aabfixture", info.packageName);
        } finally {
            apk.delete();
            bundle.delete();
        }
    }

    @Test
    public void apksDeviceSpecMatchesCurrentAndroidDevice() {
        Context context = ApplicationProvider.getApplicationContext();
        Devices.DeviceSpec spec = FastExplorerApksInstaller.buildDeviceSpec(context);

        assertEquals(Build.VERSION.SDK_INT, spec.getSdkVersion());
        assertEquals(
                context.getResources().getDisplayMetrics().densityDpi,
                spec.getScreenDensity());
        assertTrue(spec.getSupportedAbisList().contains(Build.SUPPORTED_ABIS[0]));
        assertTrue(!spec.getSupportedLocalesList().isEmpty());
    }

    @Test
    public void extractsDeviceCompatibleApksFromBundletoolSet() throws Exception {
        Context context = ApplicationProvider.getApplicationContext();
        Context testContext = InstrumentationRegistry.getInstrumentation().getContext();
        File bundle = new File(context.getCacheDir(), "apks-fixture.aab");
        File apkSet = new File(context.getCacheDir(), "apks-fixture.apks");
        try (InputStream input = testContext.getAssets().open("aab-fixture.aab");
                FileOutputStream output = new FileOutputStream(bundle)) {
            input.transferTo(output);
        }

        File aapt2 = new File(context.getApplicationInfo().nativeLibraryDir, "libaapt2.so");
        BuildApksCommand.builder()
                .setBundlePath(bundle.toPath())
                .setOutputFile(apkSet.toPath())
                .setOverwriteOutput(true)
                .setAapt2Command(Aapt2Command.createFromExecutablePath(aapt2.toPath()))
                .setSigningConfiguration(FastExplorerAabInstaller.signingConfiguration())
                .build()
                .execute();

        FastExplorerApksInstaller.PreparedInstall prepared = null;
        try {
            prepared = FastExplorerApksInstaller.prepare(context, apkSet);
            assertTrue(!prepared.apkPaths.isEmpty());
            for (java.nio.file.Path path : prepared.apkPaths) {
                File apk = path.toFile();
                assertTrue(apk.isFile());
                assertTrue(apk.length() > 0);
                assertTrue(apk.getName().endsWith(".apk"));
            }
        } finally {
            if (prepared != null) {
                FastExplorerApksInstaller.deleteRecursively(prepared.workingDirectory);
            }
            apkSet.delete();
            bundle.delete();
        }
    }
}
