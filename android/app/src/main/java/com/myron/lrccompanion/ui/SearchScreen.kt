package com.myron.lrccompanion.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.myron.lrccompanion.AppViewModel
import com.myron.lrccompanion.SearchState
import com.myron.lrccompanion.data.LrcResult
import com.myron.lrccompanion.text.Lrc
import kotlin.math.abs
import kotlin.math.roundToInt

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SearchScreen(vm: AppViewModel, s: SearchState, snackbar: SnackbarHostState) {
    val searchAction = KeyboardActions(onSearch = { vm.runSearch() })
    val searchKeys = KeyboardOptions(imeAction = ImeAction.Search)

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Find lyrics on LRCLIB") },
                navigationIcon = {
                    IconButton(onClick = { vm.back() }) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                    }
                },
            )
        },
        snackbarHost = { SnackbarHost(snackbar) },
    ) { padding ->
        LazyColumn(
            Modifier.fillMaxSize().padding(padding).imePadding(),
            contentPadding = PaddingValues(16.dp),
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            item {
                OutlinedTextField(
                    value = s.track, onValueChange = { s.track = it },
                    label = { Text("Track") }, singleLine = true,
                    keyboardOptions = searchKeys, keyboardActions = searchAction,
                    modifier = Modifier.fillMaxWidth(),
                )
            }
            item {
                OutlinedTextField(
                    value = s.artist, onValueChange = { s.artist = it },
                    label = { Text("Artist") }, singleLine = true,
                    keyboardOptions = searchKeys, keyboardActions = searchAction,
                    modifier = Modifier.fillMaxWidth(),
                )
            }
            item {
                OutlinedTextField(
                    value = s.album, onValueChange = { s.album = it },
                    label = { Text("Album (optional)") }, singleLine = true,
                    keyboardOptions = searchKeys, keyboardActions = searchAction,
                    modifier = Modifier.fillMaxWidth(),
                )
            }
            item {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Button(onClick = { vm.runSearch() }, enabled = !s.loading) { Text("Search") }
                    if (s.fileDuration > 0) {
                        Text(
                            "Your file is ${Lrc.formatTime(s.fileDuration * 1000L)}",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            modifier = Modifier.padding(start = 16.dp),
                        )
                    }
                }
            }
            if (s.loading) item { LinearProgressIndicator(Modifier.fillMaxWidth()) }
            s.error?.let { err ->
                item { Text(err, color = MaterialTheme.colorScheme.error) }
            }
            if (s.searched && !s.loading && s.error == null && s.results.isEmpty()) {
                item {
                    Text(
                        "No matches. Try a shorter title, or remove the artist or album.",
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
            items(s.results, key = { it.id }) { r ->
                ResultCard(r, s.fileDuration) { vm.openResult(r) }
            }
        }
    }
}

@Composable
private fun ResultCard(r: LrcResult, fileDuration: Int, onClick: () -> Unit) {
    val shape = RoundedCornerShape(14.dp)
    val snippet = remember(r.id) {
        val lines = r.syncedLyrics?.let { synced -> Lrc.parse(synced).map { it.text } } ?: r.plainLyrics?.lines()
        lines.orEmpty().map { it.trim() }.filter { it.isNotEmpty() && it != "♪" }.take(2).joinToString(" / ")
    }

    Surface(
        shape = shape,
        color = MaterialTheme.colorScheme.surfaceVariant,
        modifier = Modifier.fillMaxWidth().clip(shape).clickable(onClick = onClick),
    ) {
        Column(Modifier.padding(14.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            Text(r.trackName, style = MaterialTheme.typography.titleMedium, maxLines = 2, overflow = TextOverflow.Ellipsis)
            val byline = listOf(r.artistName, r.albumName).filter { it.isNotBlank() }.joinToString(" / ")
            if (byline.isNotBlank()) {
                Text(
                    byline,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            Row(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalAlignment = Alignment.CenterVertically) {
                when {
                    r.syncedLyrics != null -> Pill("Synced", Coral)
                    r.plainLyrics != null -> Pill("Plain only", MaterialTheme.colorScheme.onSurfaceVariant)
                    r.instrumental -> Pill("Instrumental", MaterialTheme.colorScheme.onSurfaceVariant)
                    else -> Pill("No lyrics", MaterialTheme.colorScheme.onSurfaceVariant)
                }
                if (r.duration > 0) {
                    Text(Lrc.formatTime((r.duration * 1000).toLong()), style = MaterialTheme.typography.labelMedium)
                    if (fileDuration > 0) {
                        val diff = abs(r.duration - fileDuration).roundToInt()
                        val color = when {
                            diff <= 2 -> GoodGreen
                            diff <= 5 -> WarnAmber
                            else -> BadRed
                        }
                        Pill(if (diff == 0) "exact length" else "${diff}s off", color)
                    }
                }
            }
            if (snippet.isNotBlank()) {
                Text(
                    snippet,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
    }
}
