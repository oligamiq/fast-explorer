package dev.oligami.fastexplorer;

import android.database.Cursor;
import android.database.MatrixCursor;
import android.net.Uri;
import android.os.CancellationSignal;
import android.os.Environment;
import android.os.Handler;
import android.os.HandlerThread;
import android.os.ParcelFileDescriptor;
import android.provider.DocumentsContract;
import android.provider.DocumentsProvider;
import android.util.Base64;
import android.webkit.MimeTypeMap;
import java.io.File;
import java.io.FileNotFoundException;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;
import java.util.Locale;
import org.json.JSONArray;
import org.json.JSONException;
import org.json.JSONObject;

public final class TaildriveDocumentsProvider extends DocumentsProvider {
    private static final String ROOT_ID = "taildrive";
    private static final String ROOT_DOC_ID = "root";
    private static final String MIME_DIR = DocumentsContract.Document.MIME_TYPE_DIR;
    private static final String[] DEFAULT_ROOT_PROJECTION = new String[] {
        DocumentsContract.Root.COLUMN_ROOT_ID,
        DocumentsContract.Root.COLUMN_DOCUMENT_ID,
        DocumentsContract.Root.COLUMN_TITLE,
        DocumentsContract.Root.COLUMN_SUMMARY,
        DocumentsContract.Root.COLUMN_FLAGS,
        DocumentsContract.Root.COLUMN_ICON
    };
    private static final String[] DEFAULT_DOCUMENT_PROJECTION = new String[] {
        DocumentsContract.Document.COLUMN_DOCUMENT_ID,
        DocumentsContract.Document.COLUMN_DISPLAY_NAME,
        DocumentsContract.Document.COLUMN_MIME_TYPE,
        DocumentsContract.Document.COLUMN_FLAGS,
        DocumentsContract.Document.COLUMN_SIZE
    };

    static {
        System.loadLibrary("fast_explorer_android");
    }

    private static native String nativeCall(String operation, String payload);

    private HandlerThread closeThread;
    private Handler closeHandler;
    private File cacheDir;

    @Override
    public boolean onCreate() {
        if (getContext() == null) return false;
        File filesDir = getContext().getFilesDir();
        File shareRoot = Environment.getExternalStorageDirectory();
        cacheDir = new File(getContext().getCacheDir(), "taildrive-documents");
        if (!cacheDir.isDirectory() && !cacheDir.mkdirs()) {
            android.util.Log.e("FastExplorer", "cannot create DocumentsProvider cache");
        }
        closeThread = new HandlerThread("FastExplorerDocumentsClose");
        closeThread.start();
        closeHandler = new Handler(closeThread.getLooper());
        try {
            call("init", new JSONObject()
                    .put("files_dir", filesDir.getAbsolutePath())
                    .put("share_root", shareRoot.getAbsolutePath()));
            return true;
        } catch (IOException | JSONException error) {
            android.util.Log.e("FastExplorer", "DocumentsProvider init failed", error);
            return false;
        }
    }

    @Override
    public Cursor queryRoots(String[] projection) throws FileNotFoundException {
        MatrixCursor cursor = new MatrixCursor(resolveProjection(projection, DEFAULT_ROOT_PROJECTION));
        MatrixCursor.RowBuilder row = cursor.newRow();
        row.add(DocumentsContract.Root.COLUMN_ROOT_ID, ROOT_ID);
        row.add(DocumentsContract.Root.COLUMN_DOCUMENT_ID, ROOT_DOC_ID);
        row.add(DocumentsContract.Root.COLUMN_TITLE, "FastExplorer");
        row.add(DocumentsContract.Root.COLUMN_SUMMARY, "TailDrive");
        row.add(DocumentsContract.Root.COLUMN_FLAGS,
                DocumentsContract.Root.FLAG_SUPPORTS_CREATE
                        | DocumentsContract.Root.FLAG_SUPPORTS_IS_CHILD);
        row.add(DocumentsContract.Root.COLUMN_ICON, android.R.drawable.ic_menu_share);
        return cursor;
    }

    @Override
    public Cursor queryDocument(String documentId, String[] projection) throws FileNotFoundException {
        MatrixCursor cursor = new MatrixCursor(resolveProjection(projection, DEFAULT_DOCUMENT_PROJECTION));
        if (getContext() != null) {
            cursor.setNotificationUri(getContext().getContentResolver(), changeUri());
        }
        includeDocument(cursor, DocRef.parse(documentId));
        return cursor;
    }

    @Override
    public Cursor queryChildDocuments(
            String parentDocumentId, String[] projection, String sortOrder)
            throws FileNotFoundException {
        MatrixCursor cursor = new MatrixCursor(resolveProjection(projection, DEFAULT_DOCUMENT_PROJECTION));
        if (getContext() != null) {
            cursor.setNotificationUri(getContext().getContentResolver(), changeUri());
        }
        DocRef parent = DocRef.parse(parentDocumentId);
        try {
            if (parent.kind == Kind.ROOT) {
                JSONArray profiles = call("profiles", new JSONObject()).getJSONArray("profiles");
                for (int i = 0; i < profiles.length(); i++) {
                    JSONObject profile = profiles.getJSONObject(i);
                    String id = profile.getString("id");
                    String label = profile.optString("label", id);
                    includeDocument(cursor, DocRef.profile(id, label));
                }
                return cursor;
            }
            if (parent.kind == Kind.PROFILE) {
                JSONObject status = call("status", new JSONObject().put("profile", parent.profile));
                JSONArray devices = status.optJSONArray("taildrive_devices");
                if (devices == null) return cursor;
                for (int i = 0; i < devices.length(); i++) {
                    JSONObject device = devices.getJSONObject(i);
                    String id = device.getString("id");
                    String name = device.optString("hostname", id);
                    includeDocument(cursor, DocRef.device(parent.profile, id, name));
                }
                return cursor;
            }
            if (parent.kind == Kind.DEVICE) {
                JSONObject status = call("status", new JSONObject().put("profile", parent.profile));
                JSONArray devices = status.optJSONArray("taildrive_devices");
                if (devices == null) return cursor;
                for (int i = 0; i < devices.length(); i++) {
                    JSONObject device = devices.getJSONObject(i);
                    if (!parent.device.equals(device.optString("id"))) continue;
                    JSONArray shares = device.optJSONArray("shares");
                    if (shares != null) {
                        for (int j = 0; j < shares.length(); j++) {
                            String share = shares.getString(j);
                            includeDocument(cursor, DocRef.share(parent.profile, parent.device, share));
                        }
                    }
                    break;
                }
                return cursor;
            }
            if (parent.kind == Kind.SHARE || parent.kind == Kind.ENTRY) {
                if (parent.kind == Kind.ENTRY && !parent.directory) {
                    throw new FileNotFoundException("not a directory: " + parentDocumentId);
                }
                String path = parent.kind == Kind.SHARE ? "" : parent.path;
                JSONObject response = call("list", remoteArgs(parent).put("path", path));
                JSONArray entries = response.getJSONArray("entries");
                for (int i = 0; i < entries.length(); i++) {
                    JSONObject entry = entries.getJSONObject(i);
                    includeDocument(cursor, DocRef.entry(
                            parent.profile,
                            parent.device,
                            parent.share,
                            entry.getString("path"),
                            entry.getString("name"),
                            entry.optBoolean("directory"),
                            parseSize(entry.optString("size", ""))));
                }
                return cursor;
            }
            throw new FileNotFoundException("unknown parent document: " + parentDocumentId);
        } catch (IOException | JSONException error) {
            throw fileNotFound("cannot list TailDrive", error);
        }
    }

    @Override
    public ParcelFileDescriptor openDocument(
            String documentId, String mode, CancellationSignal signal) throws FileNotFoundException {
        DocRef doc = DocRef.parse(documentId);
        if (doc.kind != Kind.ENTRY || doc.directory) {
            throw new FileNotFoundException("not a file: " + documentId);
        }
        File local = cacheFile(doc);
        boolean writing = mode.indexOf('w') >= 0;
        boolean reading = mode.indexOf('r') >= 0;
        boolean truncate = mode.indexOf('t') >= 0;
        try {
            if (reading && !truncate) {
                call("download", remoteArgs(doc)
                        .put("path", doc.path)
                        .put("destination", local.getAbsolutePath()));
            } else if (!local.exists() && !local.createNewFile()) {
                throw new IOException("cannot create cache file");
            }
            int parsedMode = ParcelFileDescriptor.parseMode(mode);
            return ParcelFileDescriptor.open(local, parsedMode, closeHandler, error -> {
                try {
                    if (writing && error == null) {
                        call("upload", remoteArgs(doc)
                                .put("path", doc.path)
                                .put("source", local.getAbsolutePath()));
                        notifyProviderChanged();
                    }
                } catch (Exception uploadError) {
                    android.util.Log.e("FastExplorer", "TailDrive upload on close failed", uploadError);
                } finally {
                    if (!local.delete() && local.exists()) local.deleteOnExit();
                }
            });
        } catch (IOException | JSONException error) {
            if (!local.delete() && local.exists()) local.deleteOnExit();
            throw fileNotFound("cannot open TailDrive file", error);
        }
    }

    @Override
    public String createDocument(String parentDocumentId, String mimeType, String displayName)
            throws FileNotFoundException {
        DocRef parent = DocRef.parse(parentDocumentId);
        if (parent.kind != Kind.SHARE && !(parent.kind == Kind.ENTRY && parent.directory)) {
            throw new FileNotFoundException("parent is not writable");
        }
        String path = joinPath(parent.kind == Kind.SHARE ? "" : parent.path, displayName);
        try {
            if (MIME_DIR.equals(mimeType)) {
                call("mkdir", remoteArgs(parent).put("path", path));
                notifyProviderChanged();
                return DocRef.entry(parent.profile, parent.device, parent.share,
                        path, displayName, true, null).id();
            }
            File empty = File.createTempFile("create-", ".tmp", cacheDir);
            try {
                call("upload", remoteArgs(parent)
                        .put("path", path)
                        .put("source", empty.getAbsolutePath()));
            } finally {
                if (!empty.delete() && empty.exists()) empty.deleteOnExit();
            }
            notifyProviderChanged();
            return DocRef.entry(parent.profile, parent.device, parent.share,
                    path, displayName, false, 0L).id();
        } catch (IOException | JSONException error) {
            throw fileNotFound("cannot create TailDrive document", error);
        }
    }

    @Override
    public void deleteDocument(String documentId) throws FileNotFoundException {
        DocRef doc = DocRef.parse(documentId);
        if (doc.kind != Kind.ENTRY) throw new FileNotFoundException("document is not deletable");
        try {
            call("delete", remoteArgs(doc).put("path", doc.path));
            notifyProviderChanged();
        } catch (IOException | JSONException error) {
            throw fileNotFound("cannot delete TailDrive document", error);
        }
    }
    @Override
    public String renameDocument(String documentId, String displayName) throws FileNotFoundException {
        DocRef doc = DocRef.parse(documentId);
        if (doc.kind != Kind.ENTRY) throw new FileNotFoundException("document is not renameable");
        try {
            call("rename", remoteArgs(doc)
                    .put("path", doc.path)
                    .put("new_name", displayName));
            notifyProviderChanged();
            String parent = parentPath(doc.path);
            String renamedPath = joinPath(parent, displayName);
            return DocRef.entry(doc.profile, doc.device, doc.share,
                    renamedPath, displayName, doc.directory, doc.size).id();
        } catch (IOException | JSONException error) {
            throw fileNotFound("cannot rename TailDrive document", error);
        }
    }

    @Override
    public boolean isChildDocument(String parentDocumentId, String documentId) {
        try {
            DocRef parent = DocRef.parse(parentDocumentId);
            DocRef child = DocRef.parse(documentId);
            if (parent.kind == Kind.ROOT) return child.kind != Kind.ROOT;
            if (!sameProfile(parent, child)) return false;
            if (parent.kind == Kind.PROFILE) return child.kind != Kind.ROOT;
            if (!sameDevice(parent, child)) return false;
            if (parent.kind == Kind.DEVICE) return child.kind == Kind.SHARE || child.kind == Kind.ENTRY;
            if (!sameShare(parent, child)) return false;
            if (parent.kind == Kind.SHARE) return child.kind == Kind.ENTRY;
            if (parent.kind == Kind.ENTRY && parent.directory && child.kind == Kind.ENTRY) {
                String prefix = parent.path.endsWith("/") ? parent.path : parent.path + "/";
                return child.path.startsWith(prefix);
            }
        } catch (Exception ignored) {
        }
        return false;
    }

    private void includeDocument(MatrixCursor cursor, DocRef doc) {
        MatrixCursor.RowBuilder row = cursor.newRow();
        row.add(DocumentsContract.Document.COLUMN_DOCUMENT_ID, doc.id());
        row.add(DocumentsContract.Document.COLUMN_DISPLAY_NAME, doc.name);
        row.add(DocumentsContract.Document.COLUMN_MIME_TYPE, doc.directory ? MIME_DIR : mimeType(doc.name));
        int flags = 0;
        if (doc.kind == Kind.SHARE) {
            flags |= DocumentsContract.Document.FLAG_DIR_SUPPORTS_CREATE;
        } else if (doc.kind == Kind.ENTRY) {
            flags |= DocumentsContract.Document.FLAG_SUPPORTS_DELETE
                    | DocumentsContract.Document.FLAG_SUPPORTS_RENAME;
            if (doc.directory) flags |= DocumentsContract.Document.FLAG_DIR_SUPPORTS_CREATE;
            else flags |= DocumentsContract.Document.FLAG_SUPPORTS_WRITE;
        }
        row.add(DocumentsContract.Document.COLUMN_FLAGS, flags);
        if (!doc.directory && doc.size != null) {
            row.add(DocumentsContract.Document.COLUMN_SIZE, doc.size);
        }
    }

    private Uri changeUri() {
        if (getContext() == null) return Uri.EMPTY;
        return Uri.parse("content://" + getContext().getPackageName() + ".documents/changes");
    }

    private void notifyProviderChanged() {
        if (getContext() != null) {
            getContext().getContentResolver().notifyChange(changeUri(), null);
        }
    }

    private JSONObject call(String operation, JSONObject payload) throws IOException {
        try {
            if (getContext() != null) {
                payload.put("_network_interfaces", FastExplorerNetwork.snapshot(getContext()));
            }
        } catch (JSONException error) {
            throw new IOException("cannot encode Android network state", error);
        }
        String raw = nativeCall(operation, payload.toString());
        try {
            JSONObject response = new JSONObject(raw);
            if (!response.optBoolean("ok", false)) {
                throw new IOException(response.optString("error", "TailDrive operation failed"));
            }
            return response;
        } catch (JSONException error) {
            throw new IOException("invalid TailDrive native response", error);
        }
    }

    private static JSONObject remoteArgs(DocRef doc) throws JSONException {
        return new JSONObject()
                .put("profile", doc.profile)
                .put("device", doc.device)
                .put("share", doc.share);
    }

    private File cacheFile(DocRef doc) {
        String key = sha256Hex(doc.id());
        String suffix = doc.name.contains(".") ? doc.name.substring(doc.name.lastIndexOf('.')) : ".bin";
        if (suffix.length() > 24) suffix = ".bin";
        return new File(cacheDir, key + suffix);
    }

    private static String sha256Hex(String value) {
        try {
            byte[] digest = MessageDigest.getInstance("SHA-256")
                    .digest(value.getBytes(StandardCharsets.UTF_8));
            StringBuilder output = new StringBuilder(digest.length * 2);
            for (byte item : digest) output.append(String.format(Locale.ROOT, "%02x", item & 0xff));
            return output.toString();
        } catch (NoSuchAlgorithmException impossible) {
            throw new IllegalStateException("SHA-256 is unavailable", impossible);
        }
    }

    private static String[] resolveProjection(String[] requested, String[] fallback) {
        return requested == null ? fallback : requested;
    }

    private static Long parseSize(String value) {
        try {
            return value == null || value.isEmpty() ? null : Long.parseLong(value);
        } catch (NumberFormatException ignored) {
            return null;
        }
    }

    private static String mimeType(String name) {
        int dot = name.lastIndexOf('.');
        if (dot < 0 || dot == name.length() - 1) return "application/octet-stream";
        String extension = name.substring(dot + 1).toLowerCase(Locale.ROOT);
        String mime = MimeTypeMap.getSingleton().getMimeTypeFromExtension(extension);
        return mime == null ? "application/octet-stream" : mime;
    }

    private static String joinPath(String parent, String child) {
        String cleanChild = child.replace("/", "_");
        if (parent == null || parent.isEmpty() || "/".equals(parent)) return "/" + cleanChild;
        return (parent.endsWith("/") ? parent : parent + "/") + cleanChild;
    }

    private static String parentPath(String path) {
        int slash = path.lastIndexOf('/');
        return slash <= 0 ? "" : path.substring(0, slash);
    }

    private static String b64(String value) {
        return Base64.encodeToString(value.getBytes(StandardCharsets.UTF_8),
                Base64.URL_SAFE | Base64.NO_WRAP | Base64.NO_PADDING);
    }

    private static String unb64(String value) {
        byte[] decoded = Base64.decode(value, Base64.URL_SAFE | Base64.NO_WRAP | Base64.NO_PADDING);
        return new String(decoded, StandardCharsets.UTF_8);
    }

    private static boolean sameProfile(DocRef a, DocRef b) {
        return a.profile != null && a.profile.equals(b.profile);
    }

    private static boolean sameDevice(DocRef a, DocRef b) {
        return a.device != null && a.device.equals(b.device);
    }

    private static boolean sameShare(DocRef a, DocRef b) {
        return a.share != null && a.share.equals(b.share);
    }

    private static FileNotFoundException fileNotFound(String message, Exception cause) {
        FileNotFoundException error = new FileNotFoundException(message + ": " + cause.getMessage());
        error.initCause(cause);
        return error;
    }

    private enum Kind { ROOT, PROFILE, DEVICE, SHARE, ENTRY }

    private static final class DocRef {
        final Kind kind;
        final String profile;
        final String device;
        final String share;
        final String path;
        final String name;
        final boolean directory;
        final Long size;

        private DocRef(
                Kind kind, String profile, String device, String share,
                String path, String name, boolean directory, Long size) {
            this.kind = kind;
            this.profile = profile;
            this.device = device;
            this.share = share;
            this.path = path;
            this.name = name;
            this.directory = directory;
            this.size = size;
        }

        static DocRef root() {
            return new DocRef(Kind.ROOT, null, null, null, null, "TailDrive", true, null);
        }

        static DocRef profile(String profile, String label) {
            return new DocRef(Kind.PROFILE, profile, null, null, null, label, true, null);
        }

        static DocRef device(String profile, String device, String label) {
            return new DocRef(Kind.DEVICE, profile, device, null, null, label, true, null);
        }

        static DocRef share(String profile, String device, String share) {
            return new DocRef(Kind.SHARE, profile, device, share, null, share, true, null);
        }

        static DocRef entry(
                String profile, String device, String share,
                String path, String name, boolean directory, Long size) {
            return new DocRef(Kind.ENTRY, profile, device, share,
                    path, name, directory, size);
        }

        String id() {
            switch (kind) {
                case ROOT:
                    return ROOT_DOC_ID;
                case PROFILE:
                    return "p:" + b64(profile) + ":" + b64(name);
                case DEVICE:
                    return "d:" + b64(profile) + ":" + b64(device) + ":" + b64(name);
                case SHARE:
                    return "s:" + b64(profile) + ":" + b64(device) + ":" + b64(share);
                case ENTRY:
                    return "e:" + (directory ? "1" : "0")
                            + ":" + b64(profile) + ":" + b64(device) + ":" + b64(share)
                            + ":" + b64(path) + ":" + b64(name)
                            + ":" + (size == null ? "" : Long.toString(size));
                default:
                    throw new IllegalStateException("unknown document kind");
            }
        }

        static DocRef parse(String id) throws FileNotFoundException {
            try {
                if (ROOT_DOC_ID.equals(id)) return root();
                String[] parts = id.split(":", -1);
                if (parts.length >= 3 && "p".equals(parts[0])) {
                    return profile(unb64(parts[1]), unb64(parts[2]));
                }
                if (parts.length >= 4 && "d".equals(parts[0])) {
                    return device(unb64(parts[1]), unb64(parts[2]), unb64(parts[3]));
                }
                if (parts.length >= 4 && "s".equals(parts[0])) {
                    return share(unb64(parts[1]), unb64(parts[2]), unb64(parts[3]));
                }
                if (parts.length >= 8 && "e".equals(parts[0])) {
                    Long size = parts[7].isEmpty() ? null : Long.parseLong(parts[7]);
                    return entry(unb64(parts[2]), unb64(parts[3]), unb64(parts[4]),
                            unb64(parts[5]), unb64(parts[6]), "1".equals(parts[1]), size);
                }
            } catch (IllegalArgumentException error) {
                throw fileNotFound("invalid document ID", error);
            }
            throw new FileNotFoundException("unknown document ID: " + id);
        }
    }
}
