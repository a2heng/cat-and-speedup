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

object PlanDefaults {
    const val VOICE_SPEED = 1f
    const val SILENCE_SPEED = 10f
    const val MARGIN_MS = 150L
    const val MIN_SPEEDUP_SILENCE_MS = 5_000L
    const val FALLBACK_NOISE_DB = -30f
    const val FALLBACK_MIN_SILENCE_MS = 500L
}

data class PlanOptions(
    val voiceSpeed: Float = PlanDefaults.VOICE_SPEED,
    /** Non-speech playback speed. ASR gives clean speech boundaries, so this can be aggressive. */
    val silenceSpeed: Float = PlanDefaults.SILENCE_SPEED,
    /** Padding kept around every speech range, in ms. */
    val marginMs: Long = PlanDefaults.MARGIN_MS,
    /** Only gaps at least this long are accelerated; shorter gaps stay 1x. */
    val minSpeedupSilenceMs: Long = PlanDefaults.MIN_SPEEDUP_SILENCE_MS,
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
        var samplesInWindow = 0
        var sumSquares = 0f
        var windowsDone = 0L
        var silentRunStartMs = -1L
        var totalSamples = 0L

        AudioDecoder.decodeMono(context, uri) { samples, sampleRate ->
            if (windowSamples == 0 || sampleRate != rate) {
                rate = sampleRate
                windowSamples = (sampleRate * WINDOW_MS / 1000).coerceAtLeast(1)
            }
            totalSamples += samples.size

            var index = 0
            while (index < samples.size) {
                val take = minOf(windowSamples - samplesInWindow, samples.size - index)
                var sum = sumSquares
                for (i in index until index + take) {
                    val value = samples[i]
                    sum += value * value
                }
                sumSquares = sum
                samplesInWindow += take
                index += take
                if (samplesInWindow < windowSamples) break

                val windowStartMs = windowsDone * windowSamples * 1000L / rate
                windowsDone++
                val isSilent = rmsDb(sumSquares / windowSamples) < noiseDb
                if (isSilent) {
                    if (silentRunStartMs < 0) silentRunStartMs = windowStartMs
                } else if (silentRunStartMs >= 0) {
                    spans += SilenceSpan(silentRunStartMs, windowStartMs)
                    silentRunStartMs = -1
                }
                samplesInWindow = 0
                sumSquares = 0f
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

    /**
     * Build the plan from ASR speech ranges instead of loudness detection.
     * Speech stays 1x; only gaps >= [PlanOptions.minSpeedupSilenceMs] are sped up,
     * so short sentence pauses never cause a speed change.
     */
    fun buildPlanFromSpeech(
        durationMs: Long,
        speechRanges: List<LongRange>,
        options: PlanOptions,
    ): PlaybackPlan {
        require(durationMs > 0) { "视频时长为 0" }
        val ranges = mergeRanges(
            speechRanges.mapNotNull { range ->
                val start = (range.first - options.marginMs).coerceIn(0, durationMs)
                val end = (range.last + options.marginMs).coerceIn(0, durationMs)
                if (end > start) Range(start, end) else null
            },
        )

        val segments = ArrayList<Segment>(ranges.size * 2 + 1)
        var cursor = 0L
        for (range in ranges) {
            val gap = range.startMs - cursor
            if (gap >= options.minSpeedupSilenceMs) {
                addSegment(segments, cursor, range.startMs, options.silenceSpeed, false)
                cursor = range.startMs
            }
            // Short gap: keep it at 1x by starting the voice segment at the cursor.
            val start = if (range.startMs > cursor) cursor else range.startMs
            addSegment(segments, start, range.endMs, options.voiceSpeed, true)
            cursor = range.endMs
        }

        if (cursor < durationMs) {
            val tailMs = durationMs - cursor
            if (tailMs >= options.minSpeedupSilenceMs) {
                addSegment(segments, cursor, durationMs, options.silenceSpeed, false)
            } else {
                addSegment(segments, cursor, durationMs, options.voiceSpeed, true)
            }
        }

        val plan = PlaybackPlan(durationMs, segments)
        Log.i(
            TAG,
            "plan(asr): speech=${ranges.size} ranges, segments=${segments.size}, " +
                "output=${plan.outputDurationMs()}ms",
        )
        return plan
    }

    fun buildPlan(
        durationMs: Long,
        silence: List<SilenceSpan>,
        options: PlanOptions,
    ): PlaybackPlan {
        require(durationMs > 0) { "视频时长为 0" }
        val silences = mergeRanges(
            silence
                .filter { it.durationMs >= options.minSpeedupSilenceMs }
                .mapNotNull { span ->
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

    private fun rmsDb(meanSquare: Float): Float {
        if (meanSquare <= 0f) return -120f
        return (10.0 * log10(meanSquare.toDouble())).toFloat()
    }
}
