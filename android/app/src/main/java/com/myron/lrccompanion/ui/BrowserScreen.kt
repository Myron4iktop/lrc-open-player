package com.myron.lrccompanion.ui

import android.provider.DocumentsContract
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Search
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.OutlinedTextField
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.text.input.ImeAction
import java.text.Normalizer
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.myron.lrccompanion.AppViewModel

// Opens the system picker in Download/AUDIO when it exists.
private val AUDIO_FOLDER_HINT = DocumentsContract.buildDocumentUri(
    "com.android.externalstorage.documents",
    "primary:Download/AUDIO",
)

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun BrowserScreen(vm: AppViewModel, snackbar: SnackbarHostState) {
    val picker = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocumentTree()) { uri ->
        if (uri != null) vm.onFolderPicked(uri)
    }
    val pickFolder: () -> Unit = { picker.launch(AUDIO_FOLDER_HINT) }

    Scaffold(
        topBar = {
            TopAppBar(
                title = {
                    Text(
                        vm.folderStack.lastOrNull()?.name ?: "LRC Companion",
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                },
                navigationIcon = {
                    if (vm.folderStack.size > 1) {
                        IconButton(onClick = { vm.back() }) {
                            Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Up one folder")
                        }
                    }
                },
                actions = {
                    TextButton(onClick = { vm.openRomajiTool() }) { Text("あ→A") }
                    if (vm.treeUri != null) {
                        IconButton(onClick = { vm.refresh() }) {
                            Icon(Icons.Filled.Refresh, contentDescription = "Refresh")
                        }
                        TextButton(onClick = pickFolder) { Text("Folder") }
                    }
                },
            )
        },
        snackbarHost = { SnackbarHost(snackbar) },
    ) { padding ->
        Box(Modifier.fillMaxSize().padding(padding)) {
            if (vm.treeUri == null) NoFolder(pickFolder) else FolderList(vm)
        }
    }
}

@Composable
private fun NoFolder(onPick: () -> Unit) {
    Column(
        Modifier.fillMaxSize().padding(32.dp),
        verticalArrangement = Arrangement.Center,
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text("Choose your music folder", style = MaterialTheme.typography.headlineSmall)
        Spacer(Modifier.height(12.dp))
        Text(
            "Pick the folder with the songs you want to edit, like Download/AUDIO. " +
                "Android won't let apps open the whole Download folder, but any folder inside it works.",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
        )
        Spacer(Modifier.height(24.dp))
        Button(onClick = onPick) { Text("Choose folder") }
    }
}

@Composable
private fun FolderList(vm: AppViewModel) {
    val entries = vm.entries
    val allFolders = remember(entries) { entries.filter { it.isDirectory }.sortedBy { it.name.lowercase() } }
    val allSongs = remember(entries) { entries.filter { it.isAudio }.sortedBy { it.name.lowercase() } }
    val query = vm.browserQuery
    val terms = remember(query) { searchKey(query).split(' ').filter { it.isNotEmpty() } }
    val folders = remember(allFolders, terms) { allFolders.filter { matches(it.name, terms) } }
    val songs = remember(allSongs, terms) { allSongs.filter { matches(it.name, terms) } }
    val focus = LocalFocusManager.current

    Column(Modifier.fillMaxSize()) {
        OutlinedTextField(
            value = query,
            onValueChange = { vm.browserQuery = it },
            placeholder = { Text("Search this folder") },
            leadingIcon = { Icon(Icons.Filled.Search, contentDescription = null) },
            trailingIcon = {
                if (query.isNotEmpty()) {
                    IconButton(onClick = { vm.browserQuery = "" }) {
                        Icon(Icons.Filled.Close, contentDescription = "Clear search")
                    }
                }
            },
            singleLine = true,
            keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
            keyboardActions = KeyboardActions(onSearch = { focus.clearFocus() }),
            shape = RoundedCornerShape(28.dp),
            modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 4.dp),
        )

        if (vm.folderStack.size > 1) {
            Text(
                vm.folderStack.joinToString(" / ") { it.name },
                style = MaterialTheme.typography.labelMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp),
            )
        }
        if (vm.browserLoading) LinearProgressIndicator(Modifier.fillMaxWidth())

        if (!vm.browserLoading && terms.isNotEmpty() && folders.isEmpty() && songs.isEmpty()) {
            Text(
                "Nothing in this folder matches \"${query.trim()}\".",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(24.dp),
            )
        } else if (!vm.browserLoading && folders.isEmpty() && songs.isEmpty()) {
            Text(
                "No audio files here. Add songs to this folder, or tap Folder to pick another one.",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(24.dp),
            )
        }

        LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(bottom = 24.dp)) {
            items(folders, key = { it.docId }) { folder ->
                EntryRow(icon = "▸", title = folder.name, subtitle = "Folder", onClick = { vm.openFolder(folder) })
            }
            items(songs, key = { it.docId }) { song ->
                val hasLrc = song.baseName.lowercase() in vm.lrcBaseNames
                val subtitle = buildString {
                    append(song.extension.uppercase())
                    append(" / ")
                    append(formatSize(song.size))
                    if (hasLrc) append(" / has .lrc")
                }
                EntryRow(icon = "♪", title = song.name, subtitle = subtitle, onClick = { vm.openEditor(song) })
            }
        }
    }
}

@Composable
private fun EntryRow(icon: String, title: String, subtitle: String, onClick: () -> Unit) {
    ListItem(
        headlineContent = { Text(title, maxLines = 2, overflow = TextOverflow.Ellipsis) },
        supportingContent = { Text(subtitle, style = MaterialTheme.typography.bodySmall) },
        leadingContent = {
            Box(
                Modifier.size(40.dp).clip(RoundedCornerShape(10.dp)).background(MaterialTheme.colorScheme.surfaceVariant),
                contentAlignment = Alignment.Center,
            ) {
                Text(icon, color = MaterialTheme.colorScheme.primary)
            }
        },
        colors = ListItemDefaults.colors(containerColor = Color.Transparent),
        modifier = Modifier.clickable(onClick = onClick),
    )
}

/** Lowercase, width-normalised, punctuation turned into spaces: "06. Fear.flac" -> "06 fear flac". */
private fun searchKey(s: String): String =
    Normalizer.normalize(s, Normalizer.Form.NFKC)
        .lowercase()
        .map { if (it.isLetterOrDigit()) it else ' ' }
        .joinToString("")

/** Every word typed must appear somewhere in the name, in any order. */
private fun matches(name: String, terms: List<String>): Boolean {
    if (terms.isEmpty()) return true
    val key = searchKey(name)
    return terms.all { key.contains(it) }
}
