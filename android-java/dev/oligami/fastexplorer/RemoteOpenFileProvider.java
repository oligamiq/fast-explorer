package dev.oligami.fastexplorer;

import android.net.Uri;
import android.os.Handler;
import android.os.Looper;
import android.os.ParcelFileDescriptor;

import androidx.core.content.FileProvider;

import org.json.JSONArray;

import java.io.File;
import java.io.FileNotFoundException;
import java.io.IOException;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.atomic.AtomicInteger;

public final class RemoteOpenFileProvider extends FileProvider {
    private static final ConcurrentHashMap<String, AtomicInteger> LEASES =
            new ConcurrentHashMap<>();

    private static void acquire(String name) {
        LEASES.compute(name, (key, count) -> {
            if (count == null) count = new AtomicInteger();
            count.incrementAndGet();
            return count;
        });
    }
    private static void release(String name) {
        LEASES.computeIfPresent(name, (key, count) ->
                count.decrementAndGet() <= 0 ? null : count);
    }

    public static String leasedFilesJson() {
        return new JSONArray(LEASES.keySet()).toString();
    }

    @Override
    public ParcelFileDescriptor openFile(Uri uri, String mode) throws FileNotFoundException {
        if (!"r".equals(mode)) {
            throw new FileNotFoundException("Remote open cache is read-only");
        }
        String name = uri.getLastPathSegment();
        if (name == null || name.isEmpty()) {
            throw new FileNotFoundException("Missing remote cache file name");
        }
        try {
            File root = new File(requireContext().getFilesDir(), "remote-open").getCanonicalFile();
            File file = new File(root, name).getCanonicalFile();
            if (!root.equals(file.getParentFile()) || !file.isFile()) {
                throw new FileNotFoundException("Invalid remote cache file");
            }
            file.setLastModified(System.currentTimeMillis());
            acquire(name);
            try {
                Handler handler = new Handler(Looper.getMainLooper());
                return ParcelFileDescriptor.open(
                        file,
                        ParcelFileDescriptor.MODE_READ_ONLY,
                        handler,
                        error -> release(name));
            } catch (IOException error) {
                release(name);
                throw new FileNotFoundException(error.getMessage());
            }
        } catch (IOException error) {
            throw new FileNotFoundException(error.getMessage());
        }
    }

}
