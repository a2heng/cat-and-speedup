package dev.a2heng.catspeedup

import android.content.Context
import android.net.Uri
import android.util.Log
import kotlin.math.log10

/** One contiguous slice of the source timeline, kept in original order. */
data class Segment(
    val startMs: Long,
    val endMs: Long,
    val speed: Float,
    val isVoice: Boolean,
) {
    val durationMs: Long get() = endMs - startMs
}

/**
 * Gap-free speed plan for the whole source timeline. Voice stays at
 * [PlanOptions.voiceSpeed]; everything else plays at [PlanOptions.silenceSpeed].
 * Segments are never reordered and always cover `0..sourceDurationMs`.
 */
data class PlaybackPlan(
    val sourceDurationMs: Long,
    val segments: List<Segment>,
) {
    fun segmentAt(positionMs: Long): Segment? {
        var low = 0
        var high = segments.lastIndex
        while (low <= high) {
            val mid = (low + high) ushr 1
            val segment = segments[mid]
            when {
                positionMs < segment.startMs -> high = mid - 1
                positionMs >= segment.endMs -> low = mid + 1
                else -> return segment
            }
        }
        return null
    }

    fun voiceDurationMs(): Long = segments.filter { it.isVoice }.sumOf { it.durationMs }

    fun silenceDurationMs(): Long = segments.filterNot { it.isVoice }.sumOf { it.durationMs }

    fun voiceSegmentCount(): Int = segments.count { it.isVoice }

    /** Estimated duration of the accelerated output. */
    fun outputDurationMs(): Long = segments.sumOf { segment ->
        if (segment.speed <= 0f) segment.durationMs
        else (segment.durationMs / segment.speed).toLong()
    }
}

data class PlanOptions(
    val voiceSpeed: Float = 1f,
    val silenceSpeed: Float = 4f,
    val marginMs: Long = 150,
    val noiseDb: Float = -30f,
    val minSilenceMs: Long = 500,
) {
    init {
        require(voiceSpeed > 0f) { "人声倍速必须大于 0" }
        require(silenceSpeed > 0f) { "静音倍速必须大于 0" }
    }
}

data class SilenceSpan(val startMs: Long, val endMs: Long) {
    val durationMs: Long get() = endMs - startMs
}

/**
 * Loudness-threshold silence detection, the same idea as
 * `ffmpeg -af silencedetect=noise=-30dB:d=0.5` used by the desktop core.
 *
 * Audio is streamed through [AudioDecoder] in 20 ms windows, so long videos are
 * never held in memory.
 */
object SilenceDetector {
    private const val WINDOW_MS = 20

    data class Result(
        val durationMs: Long,
        val silence: List<SilenceSpan>,
    )

    fun detect(
        context: Context,
        uri: Uri,
        noiseDb: Float,
        minSilenceMs: Long,
    ): Result {
        val spans = ArrayList<SilenceSpan>()
        var rate = 16_000
        var windowSamples = 0
        var sumSquares = 0.0
        var windowStartMs = 0L
        var silentRunStartMs = -1L
        var totalSamples = 0L

        AudioDecoder.decodeMono(context, uri) { samples, sampleRate ->
            rate = sampleRate
            windowSamples = (sampleRate * WINDOW_MS / 1000).coerceAtLeast(1)
            for (sample in samples) {
                sumSquares += sample.toDouble() * sample.toDouble()
                totalSamples++
                if (totalSamples % windowSamples != 0L) continue

                val isSilent = rmsDb(sumSquares / windowSamples) < noiseDb
                val windowEndMs = totalSamples * 1000L / sampleRate
                if (isSilent) {
                    if (silentRunStartMs < 0) silentRunStartMs = windowStartMs
                } else if (silentRunStartMs >= 0) {
                    spans += SilenceSpan(silentRunStartMs, windowStartMs)
                    silentRunStartMs = -1
                }
                windowStartMs = windowEndMs
                sumSquares = 0.0
            }
        }

        val durationMs = if (rate > 0) totalSamples * 1000L / rate else 0L
        if (silentRunStartMs >= 0 && durationMs > silentRunStartMs) {
            spans += SilenceSpan(silentRunStartMs, durationMs)
        }
        val longEnough = spans.filter { it.durationMs >= minSilenceMs }
        Log.i(
            TAG,
            "detect: noise=${noiseDb}dB minSilence=${minSilenceMs}ms " +
                "-> ${longEnough.size} spans, audio=${durationMs}ms",
        )
        return Result(durationMs = durationMs, silence = longEnough)
    }

    fun buildPlan(
        durationMs: Long,
        silence: List<SilenceSpan>,
        options: PlanOptions,
    ): PlaybackPlan {
        require(durationMs > 0) { "视频时长为 0" }
        val silences = mergeRanges(
            silence.mapNotNull { span ->
                val start = span.startMs.coerceIn(0, durationMs)
                val end = span.endMs.coerceIn(0, durationMs)
                if (end > start) Range(start, end) else null
            },
        )
        val voice = mergeRanges(
            complement(silences, durationMs).map { range ->
                Range(
                    (range.startMs - options.marginMs).coerceAtLeast(0),
                    (range.endMs + options.marginMs).coerceAtMost(durationMs),
                )
            },
        )

        val segments = ArrayList<Segment>(voice.size * 2 + 1)
        var cursor = 0L
        for (range in voice) {
            if (range.startMs > cursor) {
                addSegment(segments, cursor, range.startMs, options.silenceSpeed, false)
            }
            addSegment(segments, range.startMs, range.endMs, options.voiceSpeed, true)
            cursor = range.endMs
        }
        if (cursor < durationMs) {
            addSegment(segments, cursor, durationMs, options.silenceSpeed, false)
        }
        val plan = PlaybackPlan(durationMs, segments)
        Log.i(
            TAG,
            "plan: source=${durationMs}ms voice=${plan.voiceDurationMs()}ms " +
                "silence=${plan.silenceDurationMs()}ms output=${plan.outputDurationMs()}ms " +
                "segments=${segments.size}",
        )
        segments.take(40).forEachIndexed { index, segment ->
            Log.d(
                TAG,
                "  segment[$index] ${segment.startMs}-${segment.endMs}ms " +
                    "${segment.speed}x ${if (segment.isVoice) "voice" else "silence"}",
            )
        }
        return plan
    }

    private data class Range(val startMs: Long, val endMs: Long)

    private fun complement(silences: List<Range>, durationMs: Long): List<Range> {
        val ranges = ArrayList<Range>()
        var cursor = 0L
        for (silence in silences) {
            if (silence.startMs > cursor) ranges += Range(cursor, silence.startMs)
            cursor = cursor.coerceAtLeast(silence.endMs)
        }
        if (cursor < durationMs) ranges += Range(cursor, durationMs)
        return ranges
    }

    private fun mergeRanges(ranges: List<Range>): List<Range> {
        val sorted = ranges.sortedBy { it.startMs }
        val merged = ArrayList<Range>(sorted.size)
        for (range in sorted) {
            val last = merged.lastOrNull()
            if (last != null && range.startMs <= last.endMs) {
                merged[merged.lastIndex] = Range(last.startMs, last.endMs.coerceAtLeast(range.endMs))
            } else {
                merged += range
            }
        }
        return merged
    }

    private fun addSegment(
        segments: MutableList<Segment>,
        startMs: Long,
        endMs: Long,
        speed: Float,
        isVoice: Boolean,
    ) {
        if (endMs <= startMs) return
        val last = segments.lastOrNull()
        if (last != null && last.isVoice == isVoice && last.speed == speed) {
            segments[segments.lastIndex] = last.copy(endMs = endMs)
            return
        }
        segments += Segment(startMs, endMs, speed, isVoice)
    }

    private fun rmsDb(meanSquare: Double): Float {
        if (meanSquare <= 0.0) return -120f
        return (10.0 * log10(meanSquare)).toFloat()
    }
}
