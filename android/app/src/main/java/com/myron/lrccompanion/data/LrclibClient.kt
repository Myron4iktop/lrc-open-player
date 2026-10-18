package com.myron.lrccompanion.data

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject
import java.io.IOException
import java.net.HttpURLConnection
import java.net.URL
import java.net.URLEncoder

data class LrcResult(
    val id: Long,
    val trackName: String,
    val artistName: String,
    val albumName: String,
    val duration: Double,
    val instrumental: Boolean,
    val plainLyrics: String?,
    val syncedLyrics: String?,
)

class LrclibClient {

    /**
     * Collects candidates from several queries (track+artist+album, track+artist, free text)
     * so the user can pick the right one instead of trusting the first hit.
     */
    suspend fun search(track: String, artist: String, album: String): List<LrcResult> =
        withContext(Dispatchers.IO) {
            val found = LinkedHashMap<Long, LrcResult>()
            var lastError: Exception? = null

            fun attempt(params: List<Pair<String, String>>) {
                try {
                    request(params).forEach { found.putIfAbsent(it.id, it) }
                } catch (e: Exception) {
                    lastError = e
                }
            }

            if (track.isNotBlank()) {
                val base = mutableListOf("track_name" to track)
                if (artist.isNotBlank()) base += "artist_name" to artist
                if (album.isNotBlank()) attempt(base + ("album_name" to album))
                attempt(base)
            }
            val freeText = listOf(track, artist).filter { it.isNotBlank() }.joinToString(" ")
            if (found.size < 10 && freeText.isNotBlank()) attempt(listOf("q" to freeText))

            if (found.isEmpty()) lastError?.let { throw it }
            found.values.toList()
        }

    private fun request(params: List<Pair<String, String>>): List<LrcResult> {
        val query = params.joinToString("&") { (k, v) -> "${enc(k)}=${enc(v)}" }
        val conn = URL("$BASE_URL/api/search?$query").openConnection() as HttpURLConnection
        conn.connectTimeout = 10_000
        conn.readTimeout = 15_000
        conn.setRequestProperty("User-Agent", USER_AGENT)
        try {
            val code = conn.responseCode
            if (code != HttpURLConnection.HTTP_OK) throw IOException("LRCLIB answered with HTTP $code")
            val body = conn.inputStream.use { it.readBytes().toString(Charsets.UTF_8) }
            val array = JSONArray(body)
            return (0 until array.length()).mapNotNull { i -> array.optJSONObject(i)?.let(::parse) }
        } finally {
            conn.disconnect()
        }
    }

    private fun parse(o: JSONObject) = LrcResult(
        id = o.optLong("id"),
        trackName = o.stringOrNull("trackName").orEmpty(),
        artistName = o.stringOrNull("artistName").orEmpty(),
        albumName = o.stringOrNull("albumName").orEmpty(),
        duration = o.optDouble("duration", 0.0).let { if (it.isNaN()) 0.0 else it },
        instrumental = o.optBoolean("instrumental", false),
        plainLyrics = o.stringOrNull("plainLyrics")?.takeIf { it.isNotBlank() },
        syncedLyrics = o.stringOrNull("syncedLyrics")?.takeIf { it.isNotBlank() },
    )

    // optString() turns JSON null into the text "null", so check explicitly.
    private fun JSONObject.stringOrNull(key: String): String? =
        if (!has(key) || isNull(key)) null else optString(key)

    private fun enc(s: String) = URLEncoder.encode(s, "UTF-8")

    companion object {
        private const val BASE_URL = "https://lrclib.net"
        private const val USER_AGENT = "LRCCompanion/1.0 (Android; companion to LRC Open Player)"
    }
}
