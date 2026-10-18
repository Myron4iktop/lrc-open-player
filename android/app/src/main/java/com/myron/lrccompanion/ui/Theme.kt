package com.myron.lrccompanion.ui

import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color

// Matches the desktop player: warm near-black, coral accent, soft pink italic romaji.
val Coral = Color(0xFFE0574F)
val RomajiPink = Color(0xFFF0A79C)
val GoodGreen = Color(0xFF7BC67E)
val WarnAmber = Color(0xFFE5B85C)
val BadRed = Color(0xFFE57373)

private val Scheme = darkColorScheme(
    primary = Coral,
    onPrimary = Color.White,
    primaryContainer = Color(0xFF5A2522),
    onPrimaryContainer = Color(0xFFFFDAD5),
    secondary = RomajiPink,
    onSecondary = Color(0xFF2A1513),
    secondaryContainer = Color(0xFF4A2A27),
    onSecondaryContainer = Color(0xFFFFDAD5),
    background = Color(0xFF161212),
    onBackground = Color(0xFFEDE6E3),
    surface = Color(0xFF161212),
    onSurface = Color(0xFFEDE6E3),
    surfaceVariant = Color(0xFF241D1D),
    onSurfaceVariant = Color(0xFFA89D9A),
    surfaceContainerLowest = Color(0xFF110E0E),
    surfaceContainerLow = Color(0xFF1B1616),
    surfaceContainer = Color(0xFF1F1919),
    surfaceContainerHigh = Color(0xFF261F1F),
    surfaceContainerHighest = Color(0xFF2E2626),
    outline = Color(0xFF5A4E4C),
    outlineVariant = Color(0xFF3A3130),
    error = Color(0xFFFF8A80),
)

@Composable
fun LrcTheme(content: @Composable () -> Unit) {
    MaterialTheme(colorScheme = Scheme, content = content)
}
