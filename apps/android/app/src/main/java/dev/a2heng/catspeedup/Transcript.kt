package dev.a2heng.catspeedup

import com.k2fsa.sherpa.onnx.OfflineRecognizerResult

/** A subtitle span on the original source timeline, in milliseconds. */
data class Subtitle(
    val startMs: Long,
    val endMs: Long,
    val text: String,
)

/**
 * Turn sherpa token timestamps into sentence-ish subtitles.
 *
 * [offsetMs] is the start of the decoded chunk on the original timeline, so
 * chunked transcription still lands on source time.
 */
fun buildSubtitles(
    result: OfflineRecognizerResult,
    offsetMs: Long,
    durationMs: Long,
): List<Subtitle> {
    val tokens = result.tokens
    val timestamps = result.timestamps
    if (tokens.isEmpty() || timestamps.isEmpty()) {
        return splitWithoutTimestamps(result.text, offsetMs, durationMs)
    }

    val output = ArrayList<Subtitle>()
    val current = StringBuilder()
    var startMs = -1L
    var lastEndMs = offsetMs

    for (index in tokens.indices) {
        val token = tokens[index].trim()
        if (token.isEmpty() || token.startsWith("<0x")) continue

        val tokenStart = offsetMs + tokenTimeMs(timestamps.getOrNull(index), lastEndMs - offsetMs)
        val nextTime = timestamps.getOrNull(index + 1)
        val tokenEnd = if (nextTime != null) {
            offsetMs + tokenTimeMs(nextTime, tokenStart - offsetMs)
        } else {
            (offsetMs + durationMs).coerceAtLeast(tokenStart)
        }

        if (startMs < 0) startMs = tokenStart
        val left = if (current.isEmpty()) null else current[current.length - 1]
        if (needsSpace(left, token.firstOrNull())) current.append(' ')
        current.append(token)
        lastEndMs = tokenEnd

        val sentenceEnd = token.any { it in "。！？!?;；" }
        val tooLong = tokenEnd - startMs >= MAX_LINE_MS || current.length >= MAX_LINE_CHARS
        if (sentenceEnd || tooLong) {
            output += Subtitle(startMs, tokenEnd.coerceAtLeast(startMs), current.toString().trim())
            current.clear()
            startMs = -1
        }
    }

    if (current.isNotBlank()) {
        output += Subtitle(
            startMs.coerceAtLeast(offsetMs),
            lastEndMs.coerceAtLeast(startMs),
            current.toString().trim(),
        )
    }

    return output
}

private fun tokenTimeMs(value: Float?, fallbackMs: Long): Long =
    if (value == null || value.isNaN()) fallbackMs
    else (value.coerceAtLeast(0f) * 1000f).toLong()

private const val MAX_LINE_MS = 8_000L
private const val MAX_LINE_CHARS = 24

/**
 * Some models return text without token timestamps. Split by punctuation and
 * length, then spread the chunk duration over the pieces so the subtitle list
 * still has usable jump points.
 */
private fun splitWithoutTimestamps(
    text: String,
    offsetMs: Long,
    durationMs: Long,
): List<Subtitle> {
    val cleaned = text.trim()
    if (cleaned.isEmpty()) return emptyList()

    val pieces = ArrayList<String>()
    val buffer = StringBuilder()
    for (char in cleaned) {
        buffer.append(char)
        val boundary = char in "。！？!?；;，,、"
        if ((boundary && buffer.trim().length >= 8) || buffer.length >= MAX_LINE_CHARS) {
            pieces += buffer.toString()
            buffer.clear()
        }
    }
    if (buffer.isNotBlank()) pieces += buffer.toString()
    if (pieces.isEmpty()) return emptyList()

    val totalChars = pieces.sumOf { it.trim().length }.coerceAtLeast(1)
    var cursor = offsetMs
    return pieces.mapIndexed { index, piece ->
        val start = cursor
        val span = if (index == pieces.lastIndex) {
            offsetMs + durationMs - cursor
        } else {
            durationMs * piece.trim().length / totalChars
        }
        cursor += span.coerceAtLeast(0)
        Subtitle(start, cursor.coerceAtLeast(start), piece.trim())
    }
}

private fun needsSpace(left: Char?, right: Char?): Boolean =
    left != null && right != null && left.isLetterOrDigit() && right.isLetterOrDigit() &&
        left.code < 128 && right.code < 128

/** Standard SRT (HH:MM:SS,mmm), so players and editors accept it as-is. */
fun toSrt(subtitles: List<Subtitle>): String {
    val builder = StringBuilder()
    subtitles.forEachIndexed { index, subtitle ->
        builder.append(index + 1).append('\n')
        builder.append(srtTime(subtitle.startMs)).append(" --> ")
            .append(srtTime(subtitle.endMs)).append('\n')
        builder.append(subtitle.text.trim()).append('\n')
        builder.append('\n')
    }
    return builder.toString()
}

private fun srtTime(ms: Long): String {
    val value = ms.coerceAtLeast(0)
    val hours = value / 3_600_000
    val minutes = value / 60_000 % 60
    val seconds = value / 1_000 % 60
    val millis = value % 1_000
    return "%02d:%02d:%02d,%03d".format(hours, minutes, seconds, millis)
}

fun formatTime(ms: Long): String {
    val totalSeconds = ms / 1000
    val minutes = totalSeconds / 60
    val seconds = totalSeconds % 60
    val millis = ms % 1000
    return "%02d:%02d.%03d".format(minutes, seconds, millis)
}

fun formatDuration(ms: Long): String {
    val totalSeconds = ms.coerceAtLeast(0) / 1000
    val hours = totalSeconds / 3600
    val minutes = totalSeconds / 60 % 60
    val seconds = totalSeconds % 60
    return if (hours > 0) "%d:%02d:%02d".format(hours, minutes, seconds)
    else "%02d:%02d".format(minutes, seconds)
}
