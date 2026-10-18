package com.myron.lrccompanion.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.BottomAppBar
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.myron.lrccompanion.AppViewModel
import com.myron.lrccompanion.data.LrcResult
import com.myron.lrccompanion.text.Lrc
import com.myron.lrccompanion.text.Romaji

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun PreviewScreen(vm: AppViewModel, r: LrcResult, snackbar: SnackbarHostState) {
    val hasSynced = r.syncedLyrics != null
    val hasPlain = r.plainLyrics != null
    var usePlain by remember(r.id) { mutableStateOf(!hasSynced) }
    var showRomaji by remember { mutableStateOf(true) }

    val text = (if (usePlain) r.plainLyrics else r.syncedLyrics).orEmpty()
    val entries = remember(text) { Lrc.parseGrouped(text) }
    val texts = remember(entries) { entries.flatMap { listOf(it.main, it.sub) } }
    val hasJapanese = remember(texts) { texts.any { Romaji.hasJapanese(it) } }
    val romaji by rememberRomaji(texts, showRomaji && hasJapanese)

    Scaffold(
        topBar = {
            TopAppBar(
                title = {
                    Column {
                        Text(r.trackName, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        Text(
                            r.artistName,
                            style = MaterialTheme.typography.labelMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
                },
                navigationIcon = {
                    IconButton(onClick = { vm.back() }) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                    }
                },
            )
        },
        bottomBar = {
            BottomAppBar {
                Button(
                    onClick = { vm.useLyrics(text) },
                    enabled = text.isNotBlank(),
                    modifier = Modifier.weight(1f).padding(horizontal = 16.dp),
                ) {
                    Text(if (usePlain) "Use plain lyrics" else "Use synced lyrics")
                }
            }
        },
        snackbarHost = { SnackbarHost(snackbar) },
    ) { padding ->
        LazyColumn(
            Modifier.fillMaxSize().padding(padding),
            contentPadding = PaddingValues(horizontal = 16.dp, vertical = 8.dp),
        ) {
            item {
                Column {
                if (r.albumName.isNotBlank()) {
                    Text(
                        r.albumName,
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    if (hasSynced) FilterChip(selected = !usePlain, onClick = { usePlain = false }, label = { Text("Synced") })
                    if (hasPlain) FilterChip(selected = usePlain, onClick = { usePlain = true }, label = { Text("Plain") })
                    if (hasJapanese) {
                        FilterChip(selected = showRomaji, onClick = { showRomaji = !showRomaji }, label = { Text("Romaji") })
                    }
                }
                if (showRomaji && hasJapanese && romaji.isEmpty()) {
                    LinearProgressIndicator(Modifier.fillMaxWidth().padding(vertical = 4.dp))
                }
                Spacer(Modifier.height(8.dp))
                }
            }
            if (text.isBlank()) {
                item {
                    Text(
                        if (r.instrumental) "LRCLIB lists this track as instrumental." else "This entry has no lyrics.",
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
            items(entries.size) { i ->
                val entry = entries[i]
                val (main, sub) = Lrc.display(entry, romajiMode = showRomaji && hasJapanese, romaji = romaji)
                LyricLine(entry.timeMs, main, sub)
            }
        }
    }
}
