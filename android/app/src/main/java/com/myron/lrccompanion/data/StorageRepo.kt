package com.myron.lrccompanion.data

import android.content.Context
import android.net.Uri
import android.provider.DocumentsContract
import java.io.File
import java.io.FileInputStream
import java.io.FileOutputStream
import java.io.IOException

/** One file or folder inside the folder the user granted access to. */
data class DocEntry(
    val docId: String,
    val uri: Uri,
    val name: String,
    val mime: String,
    val size: Long,
) {
    val isDirectory: Boolean get() = mime == DocumentsContract.Document.MIME_TYPE_DIR
    val extension: String get() = name.substringAfterLast('.', "").lowercase()
    val baseName: String get() = if ('.' in name) name.substringBeforeLast('.') else name
    val isAudio: Boolean get() = !isDirectory && extension in AUDIO_EXTENSIONS
    val isLrc: Boolean get() = !isDirectory && extension == "lrc"

    companion object {
        val AUDIO_EXTENSIONS = setOf(
            "mp3", "flac", "m4a", "mp4", "aac", "ogg", "oga", "opus",
            "wav", "aif", "aiff", "wma", "dsf",
        )
    }
}

/**
 * Everything that touches the shared folder goes through the Storage Access Framework.
 * jaudiotagger needs a real java.io.File, so audio is copied into the app cache,
 * edited there, and then written back over the original.
 */
class StorageRepo(private val context: Context) {

    private val resolver get() = context.contentResolver
    private val editDir: File get() = File(context.cacheDir, "edit").apply { mkdirs() }

    fun listChildren(treeUri: Uri, parentDocId: String): List<DocEntry> {
        val childrenUri = DocumentsContract.buildChildDocumentsUriUsingTree(treeUri, parentDocId)
        val projection = arrayOf(
            DocumentsContract.Document.COLUMN_DOCUMENT_ID,
            DocumentsContract.Document.COLUMN_DISPLAY_NAME,
            DocumentsContract.Document.COLUMN_MIME_TYPE,
            DocumentsContract.Document.COLUMN_SIZE,
        )
        val out = ArrayList<DocEntry>()
        resolver.query(childrenUri, projection, null, null, null)?.use { c ->
            while (c.moveToNext()) {
                val id = c.getString(0) ?: continue
                val name = c.getString(1) ?: continue
                val mime = c.getString(2) ?: ""
                val size = if (c.isNull(3)) 0L else c.getLong(3)
                out += DocEntry(id, DocumentsContract.buildDocumentUriUsingTree(treeUri, id), name, mime, size)
            }
        }
        return out
    }

    fun findChild(treeUri: Uri, parentDocId: String, name: String): DocEntry? =
        listChildren(treeUri, parentDocId).firstOrNull { !it.isDirectory && it.name.equals(name, ignoreCase = true) }

    /** Copies the audio file into the cache. Only one file is edited at a time, so old copies are cleared. */
    fun copyToCache(uri: Uri, extension: String): File {
        clearCache()
        val file = File(editDir, "track_${System.currentTimeMillis()}.$extension")
        val input = resolver.openInputStream(uri) ?: throw IOException("Couldn't open the file for reading")
        input.use { i -> FileOutputStream(file).use { o -> i.copyTo(o) } }
        return file
    }

    fun clearCache() {
        editDir.listFiles()?.forEach { it.delete() }
    }

    /** Overwrites the original document with the edited cache copy ("wt" = write + truncate). */
    fun writeBack(file: File, target: Uri) {
        val out = resolver.openOutputStream(target, "wt") ?: throw IOException("Couldn't open the file for writing")
        out.use { o -> FileInputStream(file).use { i -> i.copyTo(o) } }
    }

    fun readText(uri: Uri): String {
        val input = resolver.openInputStream(uri) ?: return ""
        return input.use { it.readBytes().toString(Charsets.UTF_8) }
            .removePrefix("\uFEFF")
            .replace("\r\n", "\n")
    }

    fun readBytes(uri: Uri): ByteArray {
        val input = resolver.openInputStream(uri) ?: throw IOException("Couldn't open the image")
        return input.use { it.readBytes() }
    }

    /** Creates or overwrites a text file (e.g. Song.lrc) in the given folder. */
    fun writeTextFile(treeUri: Uri, parentDocId: String, name: String, text: String) {
        val existing = findChild(treeUri, parentDocId, name)
        val target = existing?.uri ?: DocumentsContract.createDocument(
            resolver,
            DocumentsContract.buildDocumentUriUsingTree(treeUri, parentDocId),
            "application/octet-stream", // keeps the .lrc extension exactly as given
            name,
        ) ?: throw IOException("Couldn't create $name")
        val out = resolver.openOutputStream(target, "wt") ?: throw IOException("Couldn't write $name")
        out.use { it.write(text.toByteArray(Charsets.UTF_8)) }
    }
}
