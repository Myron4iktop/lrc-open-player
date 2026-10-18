package com.myron.lrccompanion.text

import com.atilika.kuromoji.ipadic.Token
import com.atilika.kuromoji.ipadic.Tokenizer
import java.text.Normalizer
import java.util.concurrent.ConcurrentHashMap

/**
 * Japanese -> Hepburn-style romaji, modelled on the desktop player's output
 * ("Sou bokura wa ima", "Tada kimi to no mirai wo").
 *
 * Kuromoji splits the line into words and gives each one its katakana reading
 * (this is what resolves kanji); the readings are then romanized here.
 */
object Romaji {

    private val japaneseChar = Regex("[\\u3040-\\u30FF\\u3400-\\u4DBF\\u4E00-\\u9FFF\\uF900-\\uFAFF\\uFF66-\\uFF9F]")

    @Volatile
    private var tokenizer: Tokenizer? = null
    private val cache = ConcurrentHashMap<String, String>()

    fun hasJapanese(text: String): Boolean = japaneseChar.containsMatchIn(text)

    /** Loads the dictionary (takes a few seconds the first time). */
    fun warmUp() {
        tokenizer()
    }

    private fun tokenizer(): Tokenizer =
        tokenizer ?: synchronized(this) { tokenizer ?: Tokenizer().also { tokenizer = it } }

    fun convertLine(line: String): String {
        if (!hasJapanese(line)) return line
        cache[line]?.let { return it }
        val result = capitalizeFirst(buildLine(line))
        if (cache.size > 5000) cache.clear()
        cache[line] = result
        return result
    }

    // ---------------------------------------------------------------------
    // Tokens -> words
    // ---------------------------------------------------------------------

    private enum class Kind { WORD, LATIN, OPEN, CLOSE }

    private class Piece(val kind: Kind, val text: StringBuilder)

    private const val OPEN_CHARS = "([{「『【〈《“‘"
    private const val CLOSE_CHARS = ",.!?)]}:;~、。」』】〉》”’～〜‥…"

    private fun buildLine(line: String): String {
        val normalized = Normalizer.normalize(line, Normalizer.Form.NFKC)
        val pieces = ArrayList<Piece>()
        var pendingSpace = false
        var prev: Token? = null

        for (t in tokenizer().tokenize(normalized)) {
            val surface = t.surface
            if (surface.isBlank() || surface == "・") {
                pendingSpace = true
                prev = null
                continue
            }
            val last = pieces.lastOrNull()
            when {
                hasJapanese(surface) -> {
                    val kana = readingOf(t)
                    if (!pendingSpace && last != null && last.kind == Kind.WORD && prev != null && glues(t, prev)) {
                        last.text.append(kana)
                    } else {
                        pieces += Piece(Kind.WORD, StringBuilder(kana))
                    }
                }

                surface.all { it in OPEN_CHARS } -> pieces += Piece(Kind.OPEN, StringBuilder(mapPunct(surface)))
                surface.all { it in CLOSE_CHARS } -> pieces += Piece(Kind.CLOSE, StringBuilder(mapPunct(surface)))

                surface == "\"" || surface == "'" -> {
                    // Apostrophe inside an English word ("I'm") stays glued to it.
                    if (surface == "'" && !pendingSpace && last != null && last.kind == Kind.LATIN) {
                        last.text.append(surface)
                    } else {
                        val opening = pendingSpace || last == null || last.kind == Kind.OPEN
                        pieces += Piece(if (opening) Kind.OPEN else Kind.CLOSE, StringBuilder(surface))
                    }
                }

                else -> {
                    // Latin words, numbers, symbols like ♪
                    if (!pendingSpace && last != null && last.kind == Kind.LATIN) last.text.append(surface)
                    else pieces += Piece(Kind.LATIN, StringBuilder(surface))
                }
            }
            pendingSpace = false
            prev = t
        }

        val sb = StringBuilder()
        var prevKind: Kind? = null
        for (p in pieces) {
            val text = if (p.kind == Kind.WORD) kanaToRomaji(p.text.toString()) else p.text.toString()
            if (text.isEmpty()) continue
            if (sb.isNotEmpty() && p.kind != Kind.CLOSE && prevKind != Kind.OPEN) sb.append(' ')
            sb.append(text)
            prevKind = p.kind
        }
        return sb.toString().trim()
    }

    private val SPECIAL_READINGS = mapOf(
        "こんにちは" to "コンニチワ",
        "こんばんは" to "コンバンワ",
    )

    private fun readingOf(t: Token): String {
        val surface = t.surface
        if (t.partOfSpeechLevel1 == "助詞") {
            when (surface) {
                "は" -> return "ワ"
                "へ" -> return "エ"
            }
        }
        SPECIAL_READINGS[surface]?.let { return it }
        val reading = t.reading
        return if (reading.isNullOrEmpty() || reading == "*") toKatakana(surface) else reading
    }

    private val CONNECTIVE = setOf("て", "で", "ば", "ちゃ", "じゃ", "たり", "だり", "つつ")

    /** Decides whether a token is written together with the previous one (kiete, tomatta, bokura...). */
    private fun glues(t: Token, prev: Token): Boolean {
        val p1 = t.partOfSpeechLevel1
        val p2 = t.partOfSpeechLevel2
        val prev1 = prev.partOfSpeechLevel1
        val prevIsPredicate = prev1 == "動詞" || prev1 == "形容詞" || prev1 == "助動詞"
        return when {
            p1 == "助動詞" -> prevIsPredicate
            p1 == "助詞" && p2 == "接続助詞" && t.surface in CONNECTIVE -> prevIsPredicate
            p1 == "動詞" && p2 == "接尾" -> true
            p1 == "動詞" && p2 == "非自立" &&
                (t.surface.startsWith("て") || t.surface.startsWith("で") ||
                    t.surface.startsWith("ちゃ") || t.surface.startsWith("じゃ")) -> true
            p1 == "形容詞" && p2 == "接尾" -> true
            p1 == "名詞" && p2 == "接尾" && t.partOfSpeechLevel3 != "人名" -> true
            else -> false
        }
    }

    private fun mapPunct(s: String): String = buildString {
        for (c in s) {
            append(
                when (c) {
                    '、' -> ","
                    '。' -> "."
                    '「', '」', '『', '』', '“', '”' -> "\""
                    '‘', '’' -> "'"
                    '【', '〈', '《' -> "["
                    '】', '〉', '》' -> "]"
                    '～', '〜' -> "~"
                    '…' -> "..."
                    '‥' -> ".."
                    else -> c.toString()
                }
            )
        }
    }

    // ---------------------------------------------------------------------
    // Kana -> romaji
    // ---------------------------------------------------------------------

    private fun row(kana: String, vararg roma: String) = kana.toList().zip(roma.toList())

    private val MONO: Map<Char, String> = buildMap {
        listOf(
            row("アイウエオ", "a", "i", "u", "e", "o"),
            row("カキクケコ", "ka", "ki", "ku", "ke", "ko"),
            row("ガギグゲゴ", "ga", "gi", "gu", "ge", "go"),
            row("サシスセソ", "sa", "shi", "su", "se", "so"),
            row("ザジズゼゾ", "za", "ji", "zu", "ze", "zo"),
            row("タチツテト", "ta", "chi", "tsu", "te", "to"),
            row("ダヂヅデド", "da", "ji", "zu", "de", "do"),
            row("ナニヌネノ", "na", "ni", "nu", "ne", "no"),
            row("ハヒフヘホ", "ha", "hi", "fu", "he", "ho"),
            row("バビブベボ", "ba", "bi", "bu", "be", "bo"),
            row("パピプペポ", "pa", "pi", "pu", "pe", "po"),
            row("マミムメモ", "ma", "mi", "mu", "me", "mo"),
            row("ヤユヨ", "ya", "yu", "yo"),
            row("ラリルレロ", "ra", "ri", "ru", "re", "ro"),
            row("ワヰヱヲ", "wa", "i", "e", "wo"),
            row("ンヴ", "n", "vu"),
            row("ァィゥェォ", "a", "i", "u", "e", "o"),
            row("ャュョヮ", "ya", "yu", "yo", "wa"),
            row("ヵヶ", "ka", "ke"),
        ).forEach { pairs -> pairs.forEach { (k, r) -> put(k, r) } }
    }

    private val DIGRAPH: Map<String, String> = buildMap {
        for (base in "キギシジチヂニヒビピミリ") {
            val r = MONO.getValue(base)
            val stem = when (r) {
                "shi" -> "sh"
                "chi" -> "ch"
                "ji" -> "j"
                else -> r.dropLast(1) + "y"
            }
            put("${base}ャ", stem + "a")
            put("${base}ュ", stem + "u")
            put("${base}ョ", stem + "o")
        }
        putAll(
            mapOf(
                "シェ" to "she", "ジェ" to "je", "チェ" to "che",
                "ティ" to "ti", "ディ" to "di", "トゥ" to "tu", "ドゥ" to "du",
                "テュ" to "tyu", "デュ" to "dyu",
                "ファ" to "fa", "フィ" to "fi", "フェ" to "fe", "フォ" to "fo", "フュ" to "fyu",
                "ウィ" to "wi", "ウェ" to "we", "ウォ" to "wo",
                "ヴァ" to "va", "ヴィ" to "vi", "ヴェ" to "ve", "ヴォ" to "vo",
                "ツァ" to "tsa", "ツィ" to "tsi", "ツェ" to "tse", "ツォ" to "tso",
                "イェ" to "ye", "クァ" to "kwa", "クィ" to "kwi", "クェ" to "kwe", "クォ" to "kwo",
                "グァ" to "gwa",
            )
        )
    }

    fun kanaToRomaji(input: String): String {
        val s = toKatakana(input)
        val out = StringBuilder()
        var sokuon = false
        var i = 0
        while (i < s.length) {
            val c = s[i]
            if (c == 'ッ') {
                sokuon = true
                i++
                continue
            }
            if (c == 'ー') {
                out.lastOrNull()?.takeIf { it in "aeiou" }?.let { out.append(it) }
                i++
                continue
            }
            if (c == 'ン') {
                out.append('n')
                val next = s.getOrNull(i + 1)
                if (next != null && next in "アイウエオヤユヨ") out.append('\'')
                sokuon = false
                i++
                continue
            }

            val two = if (i + 1 < s.length) DIGRAPH[s.substring(i, i + 2)] else null
            val roma: String?
            if (two != null) {
                roma = two
                i += 2
            } else {
                roma = MONO[c]
                i += 1
            }

            if (roma == null) {
                out.append(c) // kanji without a known reading, latin, digits...
                sokuon = false
                continue
            }
            if (sokuon) {
                when {
                    roma.startsWith("ch") -> out.append('t')
                    roma[0] !in "aeiou" -> out.append(roma[0])
                }
                sokuon = false
            }
            out.append(roma)
        }
        return out.toString()
    }

    private fun toKatakana(s: String): String = buildString(s.length) {
        for (c in s) append(if (c in '\u3041'..'\u3096') c + 0x60 else c)
    }

    private fun capitalizeFirst(s: String): String {
        val idx = s.indexOfFirst { it.isLetter() }
        if (idx < 0) return s
        return s.substring(0, idx) + s[idx].uppercaseChar() + s.substring(idx + 1)
    }
}
