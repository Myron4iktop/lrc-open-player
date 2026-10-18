package com.myron.lrccompanion.text

import kotlin.math.abs

object Lrc {

    /** One or more [mm:ss.xx] stamps at the start of a line, then the lyric text. */
    private val leadingStamps = Regex("""^((?:\s*\[\d{1,3}:\d{1,2}(?:[.:]\d{1,3})?])+)(.*)$""")
    private val stamp = Regex("""\[(\d{1,3}):(\d{1,2})(?:[.:](\d{1,3}))?]""")
    private val metaTag = Regex("""^\[[A-Za-z#]+:.*]\s*$""")

    data class Line(val timeMs: Long?, val text: String)

    fun isSynced(text: String): Boolean =
        text.lineSequence().any { leadingStamps.matches(it.trim()) }

    /** Lines for display. Synced lyrics are expanded per timestamp and sorted, like the desktop player. */
    fun parse(text: String): List<Line> {
        val timed = ArrayList<Line>()
        val plain = ArrayList<Line>()
        for (raw in text.lines()) {
            val line = raw.trim()
            if (line.isEmpty()) continue
            val m = leadingStamps.matchEntire(line)
            if (m != null) {
                val body = m.groupValues[2].trim()
                stamp.findAll(m.groupValues[1]).forEach { timed += Line(toMs(it), body) }
            } else if (!metaTag.matches(line)) {
                plain += Line(null, line)
            }
        }
        return if (timed.isNotEmpty()) timed.sortedBy { it.timeMs } else plain
    }

    private fun toMs(m: MatchResult): Long {
        val minutes = m.groupValues[1].toLong()
        val seconds = m.groupValues[2].toLong()
        val frac = m.groupValues[3]
        val fracMs = when (frac.length) {
            0 -> 0L
            1 -> frac.toLong() * 100
            2 -> frac.toLong() * 10
            else -> frac.take(3).toLong()
        }
        return (minutes * 60 + seconds) * 1000 + fracMs
    }

    fun formatTime(ms: Long): String = "%d:%02d".format(ms / 60_000, (ms / 1000) % 60)

    /** A lyric card: the main line plus an optional paired line (usually a translation). */
    data class Entry(val timeMs: Long?, val main: String, val sub: String)

    /** Pairs lines whose timestamps are within 150 ms, exactly like the desktop player. */
    fun parseGrouped(text: String): List<Entry> {
        val lines = parse(text)
        if (lines.isEmpty() || lines.first().timeMs == null) return lines.map { Entry(null, it.text, "") }
        val out = ArrayList<Entry>()
        var i = 0
        while (i < lines.size) {
            val a = lines[i]
            val b = lines.getOrNull(i + 1)
            if (b != null && abs((b.timeMs ?: 0L) - (a.timeMs ?: 0L)) <= 150) {
                out += Entry(a.timeMs, a.text, b.text)
                i += 2
            } else {
                out += Entry(a.timeMs, a.text, "")
                i++
            }
        }
        return out
    }

    /**
     * What a card shows. In romaji mode Japanese is replaced by romaji and a translation stays below;
     * a paired line that is just the romaji of the main line is dropped so it isn't shown twice.
     */
    fun display(entry: Entry, romajiMode: Boolean, romaji: Map<String, String>): Pair<String, String> {
        if (!romajiMode) return entry.main to entry.sub
        val main = romaji[entry.main] ?: entry.main
        val sub = if (entry.sub.isEmpty()) "" else romaji[entry.sub] ?: entry.sub
        return if (sub.isNotEmpty() && sameText(sub, main)) main to "" else main to sub
    }

    private fun sameText(a: String, b: String): Boolean {
        fun norm(s: String) = s.lowercase().filter { it.isLetterOrDigit() }
        return norm(a) == norm(b)
    }

    class Converted(val text: String, val changed: Int)

    /**
     * Replaces every Japanese line with romaji, keeping timestamps, metadata tags and translation lines.
     * If the LRC already had romaji companion lines, the now-identical duplicates are removed.
     */
    fun toRomaji(text: String): Converted {
        val out = ArrayList<String>()
        var changed = 0
        var prevPrefix: String? = null
        var prevBody: String? = null
        for (raw in text.lines()) {
            val trimmed = raw.trim()
            if (metaTag.matches(trimmed)) {
                out += raw
                prevPrefix = null
                continue
            }
            val m = leadingStamps.matchEntire(trimmed)
            if (m == null) {
                if (Romaji.hasJapanese(raw)) {
                    out += Romaji.convertLine(trimmed)
                    changed++
                } else {
                    out += raw
                }
                prevPrefix = null
                continue
            }
            val prefix = m.groupValues[1].trim()
            var body = m.groupValues[2].trim()
            if (Romaji.hasJapanese(body)) {
                body = Romaji.convertLine(body)
                changed++
            }
            if (prefix == prevPrefix && prevBody != null && sameText(body, prevBody)) continue
            out += prefix + body
            prevPrefix = prefix
            prevBody = body
        }
        return Converted(out.joinToString("\n").trimEnd('\n') + "\n", changed)
    }
}
