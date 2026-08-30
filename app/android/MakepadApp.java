package org.ohmycloud.readwise;

import android.app.Activity;
import android.content.ClipData;
import android.content.Intent;
import android.database.Cursor;
import android.net.Uri;
import android.provider.OpenableColumns;

import java.io.BufferedInputStream;
import java.io.BufferedOutputStream;
import java.io.File;
import java.io.FileOutputStream;
import java.io.InputStream;
import java.util.ArrayList;
import java.util.LinkedHashSet;
import java.util.Locale;

import dev.makepad.android.MakepadActivity;

/** App-specific Activity additions that cargo-makepad's generated subclass lacks. */
public class MakepadApp extends MakepadActivity {
    private static final int EPUB_PICKER_REQUEST = 0x5257;

    private static native void nativeOnEpubPickerResult(
            int status, String[] paths, String error);

    public void switchActivity() {
        switchActivityClass(MakepadAppXr.class);
    }

    public void openEpubPicker() {
        Intent picker = new Intent(Intent.ACTION_OPEN_DOCUMENT);
        picker.addCategory(Intent.CATEGORY_OPENABLE);
        picker.setType("application/epub+zip");
        picker.putExtra(Intent.EXTRA_MIME_TYPES, new String[] {
                "application/epub+zip",
                "application/octet-stream",
                "application/zip"
        });
        picker.putExtra(Intent.EXTRA_ALLOW_MULTIPLE, true);
        picker.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
        try {
            startActivityForResult(picker, EPUB_PICKER_REQUEST);
        } catch (Exception error) {
            nativeOnEpubPickerResult(2, null, messageFor(error));
        }
    }

    @Override
    protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        if (requestCode != EPUB_PICKER_REQUEST) {
            return;
        }
        if (resultCode != Activity.RESULT_OK || data == null) {
            nativeOnEpubPickerResult(0, null, null);
            return;
        }

        ArrayList<File> copiedFiles = new ArrayList<>();
        try {
            LinkedHashSet<Uri> uris = new LinkedHashSet<>();
            ClipData clip = data.getClipData();
            if (clip != null) {
                for (int index = 0; index < clip.getItemCount(); index++) {
                    Uri uri = clip.getItemAt(index).getUri();
                    if (uri != null) {
                        uris.add(uri);
                    }
                }
            }
            if (data.getData() != null) {
                uris.add(data.getData());
            }
            if (uris.isEmpty()) {
                nativeOnEpubPickerResult(2, null, "系统没有返回所选文件");
                return;
            }

            File importDir = new File(getCacheDir(), "readwise-import");
            if (!importDir.exists() && !importDir.mkdirs()) {
                throw new IllegalStateException("无法创建 EPUB 导入缓存目录");
            }

            ArrayList<String> paths = new ArrayList<>();
            int index = 0;
            for (Uri uri : uris) {
                File copy = copyToImportCache(uri, importDir, index++);
                copiedFiles.add(copy);
                paths.add(copy.getAbsolutePath());
            }
            nativeOnEpubPickerResult(1, paths.toArray(new String[0]), null);
        } catch (Exception error) {
            for (File copiedFile : copiedFiles) {
                copiedFile.delete();
            }
            nativeOnEpubPickerResult(2, null, messageFor(error));
        }
    }

    private File copyToImportCache(Uri uri, File importDir, int index) throws Exception {
        String displayName = queryDisplayName(uri);
        if (displayName == null || displayName.trim().isEmpty()) {
            displayName = "book.epub";
        }
        displayName = displayName.replaceAll("[^\\p{L}\\p{N}._ -]", "_");
        if (!displayName.toLowerCase(Locale.ROOT).endsWith(".epub")) {
            displayName += ".epub";
        }
        File output = new File(
                importDir,
                System.currentTimeMillis() + "-" + index + "-" + displayName);
        try (InputStream rawInput = getContentResolver().openInputStream(uri)) {
            if (rawInput == null) {
                throw new IllegalStateException("无法读取所选文件：" + displayName);
            }
            try (BufferedInputStream input = new BufferedInputStream(rawInput);
                 BufferedOutputStream target = new BufferedOutputStream(new FileOutputStream(output))) {
                byte[] buffer = new byte[64 * 1024];
                int count;
                while ((count = input.read(buffer)) != -1) {
                    target.write(buffer, 0, count);
                }
            }
        } catch (Exception error) {
            // Avoid leaving a partial EPUB that could be mistaken for a future import.
            output.delete();
            throw error;
        }
        return output;
    }

    private String queryDisplayName(Uri uri) {
        try (Cursor cursor = getContentResolver().query(
                uri, new String[] { OpenableColumns.DISPLAY_NAME }, null, null, null)) {
            if (cursor != null && cursor.moveToFirst()) {
                int column = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME);
                if (column >= 0) {
                    return cursor.getString(column);
                }
            }
        } catch (Exception ignored) {
            // A provider may omit metadata; the fallback filename is sufficient.
        }
        return null;
    }

    private static String messageFor(Exception error) {
        String message = error.getMessage();
        return message == null || message.trim().isEmpty()
                ? error.getClass().getSimpleName()
                : message;
    }
}
