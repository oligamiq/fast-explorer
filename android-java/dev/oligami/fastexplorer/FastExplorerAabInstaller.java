package dev.oligami.fastexplorer;

import android.content.Context;
import android.security.keystore.KeyGenParameterSpec;
import android.security.keystore.KeyProperties;
import com.android.tools.build.bundletool.androidtools.Aapt2Command;
import com.android.tools.build.bundletool.commands.BuildApksCommand;
import com.android.tools.build.bundletool.model.SigningConfiguration;
import java.io.File;
import java.io.FileOutputStream;
import java.io.InputStream;
import java.math.BigInteger;
import java.security.KeyPairGenerator;
import java.security.KeyStore;
import java.security.PrivateKey;
import java.security.cert.X509Certificate;
import java.util.Calendar;
import java.util.Enumeration;
import java.util.UUID;
import java.util.zip.ZipEntry;
import java.util.zip.ZipFile;
import javax.security.auth.x500.X500Principal;

final class FastExplorerAabInstaller {
    private static final String KEY_ALIAS = "fast_explorer_aab_installer_v1";

    private FastExplorerAabInstaller() {}

    static File buildUniversalApk(Context context, File bundle) throws Exception {
        File outputDir = new File(context.getFilesDir(), "aab-install");
        if (!outputDir.exists() && !outputDir.mkdirs()) {
            throw new IllegalStateException("Cannot create AAB installer cache");
        }
        String token = UUID.randomUUID().toString();
        File apkSet = new File(outputDir, token + ".apks");
        File universalApk = new File(outputDir, token + ".apk");
        boolean complete = false;
        try {
            File aapt2 = new File(context.getApplicationInfo().nativeLibraryDir, "libaapt2.so");
            if (!aapt2.isFile() || !aapt2.canExecute()) {
                throw new IllegalStateException("Bundled Android aapt2 is unavailable");
            }

            SigningConfiguration signing = signingConfiguration();
            BuildApksCommand.builder()
                    .setBundlePath(bundle.toPath())
                    .setOutputFile(apkSet.toPath())
                    .setOverwriteOutput(true)
                    .setApkBuildMode(BuildApksCommand.ApkBuildMode.UNIVERSAL)
                    .setAapt2Command(Aapt2Command.createFromExecutablePath(aapt2.toPath()))
                    .setSigningConfiguration(signing)
                    .build()
                    .execute();

            extractUniversalApk(apkSet, universalApk);
            if (!universalApk.isFile() || universalApk.length() == 0) {
                throw new IllegalStateException("bundletool did not produce a universal APK");
            }
            complete = true;
            return universalApk;
        } finally {
            deleteTemporaryFile(apkSet);
            if (!complete) {
                deleteTemporaryFile(universalApk);
            }
        }
    }


    private static void deleteTemporaryFile(File file) {
        if (file.exists() && !file.delete()) {
            android.util.Log.w("FastExplorer", "Cannot delete temporary AAB install file: " + file);
        }
    }

    static SigningConfiguration signingConfiguration() throws Exception {
        KeyStore keyStore = KeyStore.getInstance("AndroidKeyStore");
        keyStore.load(null);
        if (!keyStore.containsAlias(KEY_ALIAS)) {
            generateSigningKey();
            keyStore.load(null);
        }
        PrivateKey privateKey = (PrivateKey) keyStore.getKey(KEY_ALIAS, null);
        X509Certificate certificate = (X509Certificate) keyStore.getCertificate(KEY_ALIAS);
        if (privateKey == null || certificate == null) {
            throw new IllegalStateException("AAB signing key is unavailable");
        }
        return SigningConfiguration.builder()
                .setSignerConfig(privateKey, certificate)
                .build();
    }

    private static void generateSigningKey() throws Exception {
        Calendar start = Calendar.getInstance();
        start.add(Calendar.DAY_OF_YEAR, -1);
        Calendar end = Calendar.getInstance();
        end.add(Calendar.YEAR, 30);
        KeyGenParameterSpec spec = new KeyGenParameterSpec.Builder(
                KEY_ALIAS, KeyProperties.PURPOSE_SIGN | KeyProperties.PURPOSE_VERIFY)
                .setDigests(KeyProperties.DIGEST_SHA256, KeyProperties.DIGEST_SHA512)
                .setSignaturePaddings(KeyProperties.SIGNATURE_PADDING_RSA_PKCS1)
                .setKeySize(2048)
                .setCertificateSubject(new X500Principal("CN=FastExplorer AAB Installer"))
                .setCertificateSerialNumber(BigInteger.ONE)
                .setCertificateNotBefore(start.getTime())
                .setCertificateNotAfter(end.getTime())
                .build();
        KeyPairGenerator generator = KeyPairGenerator.getInstance(
                KeyProperties.KEY_ALGORITHM_RSA, "AndroidKeyStore");
        generator.initialize(spec);
        generator.generateKeyPair();
    }

    private static void extractUniversalApk(File apkSet, File destination) throws Exception {
        try (ZipFile zip = new ZipFile(apkSet)) {
            ZipEntry universal = zip.getEntry("universal.apk");
            if (universal == null) {
                Enumeration<? extends ZipEntry> entries = zip.entries();
                while (entries.hasMoreElements()) {
                    ZipEntry candidate = entries.nextElement();
                    if (!candidate.isDirectory()
                            && candidate.getName().toLowerCase().endsWith("/universal.apk")) {
                        universal = candidate;
                        break;
                    }
                }
            }
            if (universal == null) {
                throw new IllegalStateException("APK set does not contain universal.apk");
            }
            try (InputStream input = zip.getInputStream(universal);
                    FileOutputStream output = new FileOutputStream(destination)) {
                byte[] buffer = new byte[256 * 1024];
                int read;
                while ((read = input.read(buffer)) != -1) {
                    output.write(buffer, 0, read);
                }
                output.getFD().sync();
            }
        }
    }
}
