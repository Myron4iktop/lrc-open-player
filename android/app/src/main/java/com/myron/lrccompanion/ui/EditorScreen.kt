package com.myron.lrccompanion.ui

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Search
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.myron.lrccompanion.AppViewModel
import com.myron.lrccompanion.EditorState
import com.myron.lrccompanion.data.TagKey
import com.myron.lrccompanion.text.Lrc
import com.myron.lrccompanion.text.Romaji

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun EditorScreen(vm: AppViewModel, st: EditorState, snackbar: SnackbarHostState) {
    val coverPicker = rememberLauncherForActivityResult(ActivityResultContracts.GetContent()) { uri ->
        if (uri != null) vm.onCoverPicked(uri)
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(st.file.name, maxLines = 1, overflow = TextOverflow.Ellipsis) },
                navigationIcon = {
                    IconButton(onClick = { vm.back() }) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                    }
                },
            )
        },
        snackbarHost = { SnackbarHost(snackbar) },
    ) { padding ->
        Box(Modifier.fillMaxSize().padding(padding)) {
            when {
                st.loading -> CircularProgressIndicator(Modifier.align(Alignment.Center))
                st.error != null -> Column(
                    Modifier.align(Alignment.Center).padding(24.dp),
                    horizontalAlignment = Alignment.CenterHorizontally,
                ) {
                    Text("Couldn't open this file", style = MaterialTheme.typography.titleMedium)
                    Spacer(Modifier.height(8.dp))
                    Text(
                        st.error.orEmpty(),
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        textAlign = TextAlign.Center,
                    )
                }

                else -> EditorContent(vm, st, onPickCover = { coverPicker.launch("image/*") })
            }
        }
    }
}

@Composable
private fun EditorContent(vm: AppViewModel, st: EditorState, onPickCover: () -> Unit) {
    Column(
        Modifier
            .fillMaxSize()
            .imePadding()
            .verticalScroll(rememberScrollState())
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        // ---------- Header ----------
        val shownCover = st.newCover?.bytes ?: if (st.removeCover) null else st.cover
        Row {
            CoverBox(shownCover, 112.dp)
            Spacer(Modifier.width(14.dp))
            Column(Modifier.weight(1f)) {
                Text(
                    st.fields[TagKey.TITLE].orEmpty().ifBlank { st.file.baseName },
                    style = MaterialTheme.typography.titleMedium,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                )
                val artist = st.fields[TagKey.ARTIST].orEmpty()
                if (artist.isNotBlank()) {
                    Text(
                        artist,
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
                Spacer(Modifier.height(4.dp))
                Text(
                    listOf(st.formatInfo, if (st.durationSec > 0) Lrc.formatTime(st.durationSec * 1000L) else "")
                        .filter { it.isNotBlank() }.joinToString(" / "),
                    style = MaterialTheme.typography.labelMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Row {
                    TextButton(onClick = onPickCover, contentPadding = PaddingValues(horizontal = 0.dp)) {
                        Text("Change cover")
                    }
                    if (shownCover != null) {
                        Spacer(Modifier.width(12.dp))
                        TextButton(onClick = { vm.removeCover() }, contentPadding = PaddingValues(horizontal = 0.dp)) {
                            Text("Remove")
                        }
                    }
                }
            }
        }

        // ---------- Tags ----------
        TagInput(st, TagKey.TITLE)
        TagInput(st, TagKey.ARTIST)
        TagInput(st, TagKey.ALBUM)
        TagInput(st, TagKey.ALBUM_ARTIST)
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            TagInput(st, TagKey.TRACK, Modifier.weight(1f))
            TagInput(st, TagKey.DISC, Modifier.weight(1f))
            TagInput(st, TagKey.YEAR, Modifier.weight(1f))
        }
        TagInput(st, TagKey.GENRE)

        Row(verticalAlignment = Alignment.CenterVertically) {
            Switch(checked = st.embedLyrics, onCheckedChange = { st.embedLyrics = it })
            Spacer(Modifier.width(12.dp))
            Text("Also embed the lyrics below in the tags", style = MaterialTheme.typography.bodyMedium)
        }
        Button(onClick = { vm.saveTags() }, enabled = !st.saving, modifier = Modifier.fillMaxWidth()) {
            Text(if (st.saving) "Saving…" else "Save tags")
        }

        HorizontalDivider(Modifier.padding(vertical = 8.dp))

        // ---------- Lyrics ----------
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text("Lyrics", style = MaterialTheme.typography.titleMedium, modifier = Modifier.weight(1f))
            Text(
                st.lyricsSource.label,
                style = MaterialTheme.typography.labelMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            OutlinedButton(onClick = { vm.openSearch() }, modifier = Modifier.weight(1f)) {
                Icon(Icons.Filled.Search, contentDescription = null, modifier = Modifier.size(18.dp))
                Spacer(Modifier.width(6.dp))
                Text("Find on LRCLIB")
            }
            OutlinedButton(onClick = { st.showRomaji = !st.showRomaji }, modifier = Modifier.weight(1f)) {
                Text(if (st.showRomaji) "Edit text" else "あ → Romaji")
            }
        }

        if (st.showRomaji) {
            Surface(shape = RoundedCornerShape(12.dp), color = MaterialTheme.colorScheme.surfaceVariant) {
                RomajiLyrics(st.lyrics)
            }
        } else {
            OutlinedTextField(
                value = st.lyrics,
                onValueChange = { st.lyrics = it },
                modifier = Modifier.fillMaxWidth(),
                minLines = 6,
                maxLines = 14,
                textStyle = TextStyle(fontFamily = FontFamily.Monospace, fontSize = 13.sp),
                placeholder = { Text("No lyrics yet. Find them on LRCLIB or paste an LRC here.") },
            )
        }

        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Button(
                onClick = { vm.saveLrc() },
                enabled = !st.saving && st.lyrics.isNotBlank(),
                modifier = Modifier.weight(1f),
            ) { Text("Save .lrc") }
            OutlinedButton(
                onClick = { vm.convertLyricsToRomaji() },
                enabled = !st.saving && st.lyrics.isNotBlank(),
                modifier = Modifier.weight(1f),
            ) { Text("Convert to romaji") }
        }
        Text(
            "Save .lrc writes ${st.file.baseName}.lrc next to the song, where the desktop player finds it. " +
                "Convert to romaji replaces the Japanese text in the editor with romaji and keeps " +
                "timestamps and translation lines. Nothing changes on disk until you save.",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(Modifier.height(24.dp))
    }
}

@Composable
private fun TagInput(st: EditorState, key: TagKey, modifier: Modifier = Modifier.fillMaxWidth()) {
    OutlinedTextField(
        value = st.fields[key].orEmpty(),
        onValueChange = { st.fields[key] = it },
        label = { Text(key.label) },
        singleLine = true,
        keyboardOptions = KeyboardOptions(
            keyboardType = if (key == TagKey.YEAR) KeyboardType.Number else KeyboardType.Text,
        ),
        modifier = modifier,
    )
}

@Composable
private fun RomajiLyrics(text: String) {
    val entries = remember(text) { Lrc.parseGrouped(text) }
    val texts = remember(entries) { entries.flatMap { listOf(it.main, it.sub) } }
    val hasJapanese = remember(texts) { texts.any { Romaji.hasJapanese(it) } }
    val romaji by rememberRomaji(texts, enabled = true)

    Column(Modifier.fillMaxWidth().padding(12.dp)) {
        when {
            entries.isEmpty() -> Text("No lyrics to show.", color = MaterialTheme.colorScheme.onSurfaceVariant)
            !hasJapanese -> Text(
                "These lyrics have no Japanese text to convert.",
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(bottom = 8.dp),
            )
            romaji.isEmpty() -> LinearProgressIndicator(Modifier.fillMaxWidth().padding(bottom = 8.dp))
        }
        entries.forEach { entry ->
            val (main, sub) = Lrc.display(entry, romajiMode = true, romaji = romaji)
            LyricLine(entry.timeMs, main, sub)
        }
    }
}
