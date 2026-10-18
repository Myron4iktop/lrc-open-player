package com.myron.lrccompanion.data

import org.jaudiotagger.audio.AudioFileIO
import org.jaudiotagger.audio.flac.metadatablock.MetadataBlockDataPicture
import org.jaudiotagger.tag.FieldKey
import org.jaudiotagger.tag.Tag
import org.jaudiotagger.tag.TagOptionSingleton
import org.jaudiotagger.tag.flac.FlacTag
import org.jaudiotagger.tag.images.ArtworkFactory
import java.io.File
import java.util.logging.Level
import java.util.logging.Logger

enum class TagKey(val label: String, val fieldKey: FieldKey) {
    TITLE("Title", FieldKey.TITLE),
    ARTIST("Artist", FieldKey.ARTIST),
    ALBUM("Album", FieldKey.ALBUM),
    ALBUM_ARTIST("Album artist", FieldKey.ALBUM_ARTIST),
    TRACK("Track", FieldKey.TRACK),
    DISC("Disc", FieldKey.DISC_NO),
    YEAR("Year", FieldKey.YEAR),
    GENRE("Genre", FieldKey.GENRE),
}

class CoverImage(val bytes: ByteArray, val mime: String, val width: Int, val height: Int)

sealed interface CoverChange {
    data object Keep : CoverChange
    data object Remove : CoverChange
    class Replace(val image: CoverImage) : CoverChange
}

class TagData(
    val fields: Map<TagKey, String>,
    val lyrics: String,
    val cover: ByteArray?,
    val durationSec: Int,
    val formatInfo: String,
)

object TagRepo {

    // Keep a strong reference, otherwise the logger (and its level) can be garbage collected.
    private val jatLogger: Logger = Logger.getLogger("org.jaudiotagger")

    init {
        jatLogger.level = Level.OFF
        // Makes jaudiotagger use Android-friendly artwork classes instead of java.awt / ImageIO.
        TagOptionSingleton.getInstance().setAndroid(true)
    }

    fun read(file: File): TagData {
        val audioFile = AudioFileIO.read(file)
        val tag: Tag? = audioFile.tag

        val fields = TagKey.entries.associateWith { key -> tag?.let { safeGet(it, key.fieldKey) }.orEmpty() }
        val lyrics = tag?.let { safeGet(it, FieldKey.LYRICS) }.orEmpty().replace("\r\n", "\n")
        val cover = try {
            tag?.firstArtwork?.binaryData
        } catch (t: Throwable) {
            null
        }

        val header = audioFile.audioHeader
        val parts = mutableListOf<String>()
        header.encodingType
            ?.replace(Regex("\\s*\\d+\\s*bits?", RegexOption.IGNORE_CASE), "")
            ?.trim()
            ?.takeIf { it.isNotBlank() }
            ?.let { parts += it }
        try {
            val bits = header.bitsPerSample
            if (bits > 0) parts += "${bits}bit"
        } catch (_: Throwable) {
        }
        header.sampleRate?.toIntOrNull()?.let { parts += "%.1fkHz".format(it / 1000.0) }
        header.bitRate?.takeIf { it.isNotBlank() }?.let { parts += "${it.trim('~')}kbps" }

        return TagData(
            fields = fields,
            lyrics = lyrics,
            cover = cover,
            durationSec = header.trackLength,
            formatInfo = parts.joinToString(" / "),
        )
    }

    /**
     * Writes only what changed. [lyrics] == null means "leave the lyrics tag alone",
     * a blank string removes it.
     */
    fun write(file: File, changed: Map<TagKey, String>, lyrics: String?, cover: CoverChange) {
        val audioFile = AudioFileIO.read(file)
        val tag = audioFile.tagOrCreateAndSetDefault

        for ((key, raw) in changed) {
            val value = raw.trim()
            if (value.isEmpty()) {
                runCatching { tag.deleteField(key.fieldKey) }
            } else {
                try {
                    tag.setField(key.fieldKey, value)
                } catch (e: Exception) {
                    throw IllegalArgumentException("${key.label}: ${e.message ?: "invalid value"}")
                }
            }
        }

        if (lyrics != null) {
            if (lyrics.isBlank()) runCatching { tag.deleteField(FieldKey.LYRICS) }
            else tag.setField(FieldKey.LYRICS, lyrics)
        }

        when (cover) {
            CoverChange.Keep -> Unit
            CoverChange.Remove -> runCatching { tag.deleteArtworkField() }
            is CoverChange.Replace -> setCover(tag, cover.image)
        }

        audioFile.commit()
    }

    private fun setCover(tag: Tag, image: CoverImage) {
        try {
            runCatching { tag.deleteArtworkField() }
            if (tag is FlacTag) {
                // Built directly so jaudiotagger doesn't try to decode the image with desktop-Java APIs.
                tag.setField(
                    MetadataBlockDataPicture(image.bytes, 3, image.mime, "", image.width, image.height, 24, 0)
                )
            } else {
                val artwork = ArtworkFactory.getNew()
                artwork.binaryData = image.bytes
                artwork.mimeType = image.mime
                artwork.pictureType = 3 // front cover
                tag.setField(artwork)
            }
        } catch (t: Throwable) {
            throw UnsupportedOperationException(
                "Cover editing isn't supported for this format (${t.javaClass.simpleName})"
            )
        }
    }

    private fun safeGet(tag: Tag, key: FieldKey): String = try {
        tag.getFirst(key) ?: ""
    } catch (t: Throwable) {
        ""
    }
}
