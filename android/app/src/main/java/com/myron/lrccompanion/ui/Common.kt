package com.myron.lrccompanion.ui

import android.graphics.BitmapFactory
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.State
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import com.myron.lrccompanion.text.Lrc
import com.myron.lrccompanion.text.Romaji
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** Romaji for every Japanese line, computed off the main thread. */
@Composable
fun rememberRomaji(texts: List<String>, enabled: Boolean): State<Map<String, String>> =
    produceState<Map<String, String>>(initialValue = emptyMap(), texts, enabled) {
        value = if (!enabled) {
            emptyMap()
        } else {
            withContext(Dispatchers.Default) {
                texts.filter { Romaji.hasJapanese(it) }.distinct().associateWith {
                    runCatching { Romaji.convertLine(it) }.getOrDefault("")
                }
            }
        }
    }

/** One lyric card: main line and, below it, the paired line (translation) if there is one. */
@Composable
fun LyricLine(timeMs: Long?, main: String, sub: String) {
    Row(Modifier.fillMaxWidth().padding(vertical = 6.dp)) {
        if (timeMs != null) {
            Text(
                Lrc.formatTime(timeMs),
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.width(44.dp).padding(top = 4.dp),
            )
        }
        Column {
            Text(main.ifEmpty { "♪" }, style = MaterialTheme.typography.bodyLarge)
            if (sub.isNotEmpty()) {
                Text(
                    sub,
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.secondary,
                )
            }
        }
    }
}

@Composable
fun Pill(text: String, color: Color) {
    Surface(shape = RoundedCornerShape(50), color = color.copy(alpha = 0.16f)) {
        Text(
            text,
            color = color,
            style = MaterialTheme.typography.labelMedium,
            modifier = Modifier.padding(horizontal = 10.dp, vertical = 3.dp),
        )
    }
}

@Composable
fun CoverBox(bytes: ByteArray?, size: Dp) {
    val bitmap = remember(bytes) { bytes?.let { decodeThumbnail(it, 512) } }
    Box(
        Modifier.size(size).clip(RoundedCornerShape(10.dp)).background(MaterialTheme.colorScheme.surfaceVariant),
        contentAlignment = Alignment.Center,
    ) {
        if (bitmap != null) {
            Image(bitmap, contentDescription = "Cover", contentScale = ContentScale.Crop, modifier = Modifier.fillMaxSize())
        } else {
            Text("♪", style = MaterialTheme.typography.headlineMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
}

private fun decodeThumbnail(bytes: ByteArray, maxSide: Int): ImageBitmap? = try {
    val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
    BitmapFactory.decodeByteArray(bytes, 0, bytes.size, bounds)
    var sample = 1
    while (bounds.outWidth / (sample * 2) >= maxSide && bounds.outHeight / (sample * 2) >= maxSide) sample *= 2
    BitmapFactory.decodeByteArray(bytes, 0, bytes.size, BitmapFactory.Options().apply { inSampleSize = sample })
        ?.asImageBitmap()
} catch (t: Throwable) {
    null
}

fun formatSize(bytes: Long): String = when {
    bytes >= 1_000_000 -> "%.1f MB".format(bytes / 1_000_000.0)
    bytes >= 1_000 -> "%.0f KB".format(bytes / 1_000.0)
    else -> "$bytes B"
}
