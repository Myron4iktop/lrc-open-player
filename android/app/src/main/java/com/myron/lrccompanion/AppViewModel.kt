package com.myron.lrccompanion

import android.app.Application
import android.content.Context
import android.content.Intent
import android.graphics.BitmapFactory
import android.net.Uri
import android.provider.DocumentsContract
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import com.myron.lrccompanion.data.CoverChange
import com.myron.lrccompanion.data.CoverImage
import com.myron.lrccompanion.data.DocEntry
import com.myron.lrccompanion.data.LrcResult
import com.myron.lrccompanion.data.LrclibClient
import com.myron.lrccompanion.data.StorageRepo
import com.myron.lrccompanion.data.TagKey
import com.myron.lrccompanion.data.TagRepo
import com.myron.lrccompanion.text.Lrc
import com.myron.lrccompanion.text.Romaji
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.File
import kotlin.math.abs

sealed interface Screen {
    data object Browser : Screen
    data object Editor : Screen
    data object Search : Screen
    data object Preview : Screen
    data object RomajiTool : Screen
}

data class FolderRef(val docId: String, val name: String)

enum class LyricsSource(val label: String) {
    NONE("No lyrics yet"),
    LRC_FILE("From .lrc file"),
    EMBEDDED("Embedded in tags"),
    LRCLIB("From LRCLIB, not saved yet"),
}

class EditorState(val file: DocEntry, val parentDocId: String) {
    var loading by mutableStateOf(true)
    var saving by mutableStateOf(false)
    var error by mutableStateOf<String?>(null)

    val fields = mutableStateMapOf<TagKey, String>()
    var originalFields by mutableStateOf<Map<TagKey, String>>(emptyMap())

    var cover by mutableStateOf<ByteArray?>(null)
    var newCover by mutableStateOf<CoverImage?>(null)
    var removeCover by mutableStateOf(false)

    var lyrics by mutableStateOf("")
    var savedLyrics by mutableStateOf("")
    var lyricsSource by mutableStateOf(LyricsSource.NONE)
    var embedLyrics by mutableStateOf(false)
    var showRomaji by mutableStateOf(false)

    var durationSec by mutableIntStateOf(0)
    var formatInfo by mutableStateOf("")

    var cacheFile: File? = null

    val tagsDirty: Boolean
        get() = TagKey.entries.any { fields[it].orEmpty() != originalFields[it].orEmpty() } ||
            newCover != null || removeCover

    val isDirty: Boolean get() = tagsDirty || lyrics != savedLyrics
}

class SearchState(track: String, artist: String, album: String, val fileDuration: Int) {
    var track by mutableStateOf(track)
    var artist by mutableStateOf(artist)
    var album by mutableStateOf(album)
    var loading by mutableStateOf(false)
    var searched by mutableStateOf(false)
    var error by mutableStateOf<String?>(null)
    var results by mutableStateOf<List<LrcResult>>(emptyList())
}

class AppViewModel(app: Application) : AndroidViewModel(app) {

    private val prefs = app.getSharedPreferences("lrc_companion", Context.MODE_PRIVATE)
    private val storage = StorageRepo(app)
    private val lrclib = LrclibClient()

    // ---------------- Navigation & messages ----------------
    val stack = mutableStateListOf<Screen>(Screen.Browser)
    var showDiscardDialog by mutableStateOf(false)
    var message by mutableStateOf<String?>(null)
        private set

    fun consumeMessage() {
        message = null
    }

    // ---------------- Folder browser ----------------
    var treeUri by mutableStateOf<Uri?>(null)
        private set
    val folderStack = mutableStateListOf<FolderRef>()
    var entries by mutableStateOf<List<DocEntry>>(emptyList())
        private set
    var lrcBaseNames by mutableStateOf<Set<String>>(emptySet())
        private set
    var browserLoading by mutableStateOf(false)
        private set
    var browserQuery by mutableStateOf("")

    // ---------------- Editor / search / preview ----------------
    var editor by mutableStateOf<EditorState?>(null)
        private set
    var search by mutableStateOf<SearchState?>(null)
        private set
    var selectedResult by mutableStateOf<LrcResult?>(null)
        private set

    // ---------------- Romaji tool ----------------
    var romajiReady by mutableStateOf(false)
        private set
    var romajiInput by mutableStateOf("")
    var romajiOutput by mutableStateOf("")
    var romajiBusy by mutableStateOf(false)
        private set

    // =========================================================
    // Navigation
    // =========================================================
    fun back() {
        when (stack.last()) {
            Screen.Browser -> if (folderStack.size > 1) {
                folderStack.removeAt(folderStack.lastIndex)
                entries = emptyList()
                browserQuery = ""
                refresh()
            }

            Screen.Editor -> {
                val st = editor
                when {
                    st == null -> pop()
                    st.saving -> Unit
                    st.isDirty -> showDiscardDialog = true
                    else -> closeEditor()
                }
            }

            else -> pop()
        }
    }

    private fun pop() {
        if (stack.size > 1) stack.removeAt(stack.lastIndex)
    }

    fun closeEditor() {
        showDiscardDialog = false
        storage.clearCache()
        editor = null
        search = null
        selectedResult = null
        while (stack.size > 1 && stack.last() != Screen.Browser) stack.removeAt(stack.lastIndex)
        refresh()
    }

    fun openRomajiTool() {
        stack.add(Screen.RomajiTool)
    }

    // =========================================================
    // Folder access
    // =========================================================
    private fun restoreFolder() {
        val saved = prefs.getString(KEY_TREE, null) ?: return
        val uri = Uri.parse(saved)
        val granted = getApplication<Application>().contentResolver.persistedUriPermissions
            .any { it.uri == uri && it.isReadPermission && it.isWritePermission }
        if (granted) setTree(uri) else prefs.edit().remove(KEY_TREE).apply()
    }

    fun onFolderPicked(uri: Uri) {
        val resolver = getApplication<Application>().contentResolver
        val flags = Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION
        try {
            resolver.takePersistableUriPermission(uri, flags)
        } catch (e: SecurityException) {
            message = "Android didn't grant access to that folder. Pick a subfolder such as Download/AUDIO."
            return
        }
        val old = treeUri
        if (old != null && old != uri) runCatching { resolver.releasePersistableUriPermission(old, flags) }
        prefs.edit().putString(KEY_TREE, uri.toString()).apply()
        setTree(uri)
    }

    private fun setTree(uri: Uri) {
        treeUri = uri
        folderStack.clear()
        entries = emptyList()
        browserQuery = ""
        val rootId = DocumentsContract.getTreeDocumentId(uri)
        val name = rootId.substringAfterLast(':').substringAfterLast('/').ifBlank { "Music" }
        folderStack.add(FolderRef(rootId, name))
        refresh()
    }

    fun openFolder(entry: DocEntry) {
        folderStack.add(FolderRef(entry.docId, entry.name))
        entries = emptyList()
        browserQuery = ""
        refresh()
    }

    fun refresh() {
        val tree = treeUri ?: return
        val folder = folderStack.lastOrNull() ?: return
        browserLoading = true
        viewModelScope.launch {
            try {
                val list = withContext(Dispatchers.IO) { storage.listChildren(tree, folder.docId) }
                if (folderStack.lastOrNull() == folder) {
                    entries = list
                    lrcBaseNames = list.filter { it.isLrc }.map { it.baseName.lowercase() }.toSet()
                }
            } catch (e: CancellationException) {
                throw e
            } catch (t: Throwable) {
                message = "Couldn't read the folder: ${t.readable()}"
            }
            browserLoading = false
        }
    }

    // =========================================================
    // Editor
    // =========================================================
    fun openEditor(entry: DocEntry) {
        val tree = treeUri ?: return
        val parent = folderStack.lastOrNull()?.docId ?: return
        val st = EditorState(entry, parent)
        editor = st
        search = null
        selectedResult = null
        stack.add(Screen.Editor)

        val hasLrc = entry.baseName.lowercase() in lrcBaseNames
        viewModelScope.launch {
            try {
                val (file, data, lrc) = withContext(Dispatchers.IO) {
                    val file = storage.copyToCache(entry.uri, entry.extension)
                    val data = TagRepo.read(file)
                    val lrc = if (hasLrc) {
                        storage.findChild(tree, parent, entry.baseName + ".lrc")?.let { storage.readText(it.uri) }
                    } else null
                    Triple(file, data, lrc)
                }
                st.cacheFile = file
                TagKey.entries.forEach { st.fields[it] = data.fields[it].orEmpty() }
                st.originalFields = data.fields
                st.cover = data.cover
                st.durationSec = data.durationSec
                st.formatInfo = data.formatInfo
                when {
                    !lrc.isNullOrBlank() -> {
                        st.lyrics = lrc
                        st.lyricsSource = LyricsSource.LRC_FILE
                    }

                    data.lyrics.isNotBlank() -> {
                        st.lyrics = data.lyrics
                        st.lyricsSource = LyricsSource.EMBEDDED
                        st.embedLyrics = true
                    }
                }
                st.savedLyrics = st.lyrics
            } catch (e: CancellationException) {
                throw e
            } catch (t: Throwable) {
                st.error = t.readable()
            }
            st.loading = false
        }
    }

    fun onCoverPicked(uri: Uri) {
        val st = editor ?: return
        viewModelScope.launch {
            try {
                val image = withContext(Dispatchers.IO) {
                    val bytes = storage.readBytes(uri)
                    val mime = when {
                        bytes.size > 3 && bytes[0] == 0xFF.toByte() && bytes[1] == 0xD8.toByte() -> "image/jpeg"
                        bytes.size > 8 && bytes[0] == 0x89.toByte() && bytes[1] == 'P'.code.toByte() -> "image/png"
                        else -> throw IllegalArgumentException("Pick a JPEG or PNG image for the cover.")
                    }
                    val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
                    BitmapFactory.decodeByteArray(bytes, 0, bytes.size, bounds)
                    CoverImage(bytes, mime, bounds.outWidth, bounds.outHeight)
                }
                st.newCover = image
                st.removeCover = false
            } catch (e: CancellationException) {
                throw e
            } catch (t: Throwable) {
                message = t.readable()
            }
        }
    }

    fun removeCover() {
        val st = editor ?: return
        st.newCover = null
        st.removeCover = st.cover != null
    }

    fun saveTags() {
        val st = editor ?: return
        if (st.saving) return
        val changed = TagKey.entries
            .filter { st.fields[it].orEmpty() != st.originalFields[it].orEmpty() }
            .associateWith { st.fields[it].orEmpty() }
        val lyricsToEmbed = if (st.embedLyrics) st.lyrics else null
        val pendingCover = st.newCover
        val coverChange: CoverChange = when {
            pendingCover != null -> CoverChange.Replace(pendingCover)
            st.removeCover -> CoverChange.Remove
            else -> CoverChange.Keep
        }
        if (changed.isEmpty() && coverChange == CoverChange.Keep && lyricsToEmbed == null) {
            message = "No tag changes to save"
            return
        }

        st.saving = true
        viewModelScope.launch {
            try {
                withContext(Dispatchers.IO) {
                    val file = st.cacheFile?.takeIf { it.exists() }
                        ?: storage.copyToCache(st.file.uri, st.file.extension).also { st.cacheFile = it }
                    val backup = File(file.parentFile, "backup.${st.file.extension}")
                    file.copyTo(backup, overwrite = true)

                    TagRepo.write(file, changed, lyricsToEmbed, coverChange)
                    TagRepo.read(file) // sanity check: the edited file must still parse

                    try {
                        storage.writeBack(file, st.file.uri)
                    } catch (e: Exception) {
                        // Put the untouched original back if the copy failed half way.
                        runCatching { storage.writeBack(backup, st.file.uri) }
                        throw e
                    } finally {
                        backup.delete()
                    }
                }
                st.originalFields = st.fields.toMap()
                if (pendingCover != null) st.cover = pendingCover.bytes
                if (coverChange == CoverChange.Remove) st.cover = null
                st.newCover = null
                st.removeCover = false
                if (lyricsToEmbed != null) {
                    st.savedLyrics = lyricsToEmbed
                    if (st.lyricsSource == LyricsSource.LRCLIB || st.lyricsSource == LyricsSource.NONE) {
                        st.lyricsSource = LyricsSource.EMBEDDED
                    }
                }
                message = "Tags saved"
            } catch (e: CancellationException) {
                throw e
            } catch (t: Throwable) {
                // The cache copy may be half-edited; start from the original next time.
                st.cacheFile?.delete()
                st.cacheFile = null
                message = "Couldn't save tags: ${t.readable()}"
            }
            st.saving = false
        }
    }

    fun saveLrc() {
        val st = editor ?: return
        val tree = treeUri ?: return
        val text = st.lyrics
        if (text.isBlank()) {
            message = "There are no lyrics to save"
            return
        }
        val name = st.file.baseName + ".lrc"
        st.saving = true
        viewModelScope.launch {
            try {
                withContext(Dispatchers.IO) {
                    storage.writeTextFile(tree, st.parentDocId, name, text.trimEnd() + "\n")
                }
                st.savedLyrics = text
                st.lyricsSource = LyricsSource.LRC_FILE
                lrcBaseNames = lrcBaseNames + st.file.baseName.lowercase()
                message = "Saved $name"
            } catch (e: CancellationException) {
                throw e
            } catch (t: Throwable) {
                message = "Couldn't save $name: ${t.readable()}"
            }
            st.saving = false
        }
    }

    fun convertLyricsToRomaji() {
        val st = editor ?: return
        val text = st.lyrics
        st.saving = true
        viewModelScope.launch {
            try {
                val result = withContext(Dispatchers.Default) { Lrc.toRomaji(text) }
                if (result.changed == 0) {
                    message = "No Japanese lines to convert"
                } else {
                    st.lyrics = result.text
                    st.showRomaji = false
                    message = "Converted ${result.changed} lines. Save the .lrc to keep the romaji version."
                }
            } catch (e: CancellationException) {
                throw e
            } catch (t: Throwable) {
                message = "Romaji conversion failed: ${t.readable()}"
            }
            st.saving = false
        }
    }

    // =========================================================
    // LRCLIB
    // =========================================================
    fun openSearch() {
        val st = editor ?: return
        val title = st.fields[TagKey.TITLE].orEmpty().ifBlank { st.file.baseName }
        search = SearchState(
            track = cleanQuery(title),
            artist = cleanQuery(st.fields[TagKey.ARTIST].orEmpty()),
            album = st.fields[TagKey.ALBUM].orEmpty().trim(),
            fileDuration = st.durationSec,
        )
        stack.add(Screen.Search)
        runSearch()
    }

    fun runSearch() {
        val s = search ?: return
        if (s.track.isBlank() && s.artist.isBlank()) {
            s.error = "Enter a track name to search"
            return
        }
        s.loading = true
        s.error = null
        viewModelScope.launch {
            try {
                val found = lrclib.search(s.track.trim(), s.artist.trim(), s.album.trim())
                s.results = sortResults(found, s.fileDuration)
            } catch (e: CancellationException) {
                throw e
            } catch (t: Throwable) {
                s.results = emptyList()
                s.error = "Search failed: ${t.readable()}. Check your internet connection."
            }
            s.loading = false
            s.searched = true
        }
    }

    /** Close duration match first, then synced before plain, then by duration difference. */
    private fun sortResults(list: List<LrcResult>, fileDuration: Int): List<LrcResult> {
        fun diff(r: LrcResult) =
            if (fileDuration > 0 && r.duration > 0) abs(r.duration - fileDuration) else Double.MAX_VALUE
        return list.sortedWith(
            compareBy<LrcResult>(
                { if (diff(it) <= 3.0) 0 else 1 },
                { if (it.syncedLyrics != null) 0 else 1 },
                { diff(it) },
            )
        )
    }

    fun openResult(result: LrcResult) {
        selectedResult = result
        stack.add(Screen.Preview)
    }

    fun useLyrics(text: String) {
        val st = editor ?: return
        st.lyrics = text.replace("\r\n", "\n")
        st.lyricsSource = LyricsSource.LRCLIB
        st.showRomaji = false
        while (stack.size > 1 && stack.last() != Screen.Editor) stack.removeAt(stack.lastIndex)
        message = "Lyrics loaded. Save them as .lrc or embed them in the tags."
    }

    // =========================================================
    // Romaji tool
    // =========================================================
    fun convertRomaji() {
        val input = romajiInput
        if (input.isBlank()) return
        romajiBusy = true
        viewModelScope.launch {
            try {
                romajiOutput = withContext(Dispatchers.Default) { Lrc.toRomaji(input).text.trimEnd() }
            } catch (e: CancellationException) {
                throw e
            } catch (t: Throwable) {
                message = "Romaji conversion failed: ${t.readable()}"
            }
            romajiBusy = false
        }
    }

    private fun cleanQuery(s: String): String = s
        .replace(Regex("^\\d+[\\s.\\-_]+"), "")
        .replace(Regex("\\s*\\(\\d+\\)$"), "")
        .replace(Regex("\\[(FLAC|MP3|WAV|HQ|Hi-?Res)]", RegexOption.IGNORE_CASE), "")
        .trim()

    // Runs after every property above has been initialised.
    init {
        restoreFolder()
        viewModelScope.launch {
            val ok = withContext(Dispatchers.Default) { runCatching { Romaji.warmUp() }.isSuccess }
            romajiReady = ok
            if (!ok) message = "Couldn't load the Japanese dictionary, so romaji is unavailable."
        }
    }

    companion object {
        private const val KEY_TREE = "tree_uri"
    }
}

fun Throwable.readable(): String =
    message?.takeIf { it.isNotBlank() } ?: javaClass.simpleName
