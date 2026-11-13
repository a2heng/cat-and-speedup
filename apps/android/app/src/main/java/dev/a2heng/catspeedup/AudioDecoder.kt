package dev.a2heng.catspeedup

import android.content.Context
import android.media.AudioFormat
import android.media.MediaCodec
import android.media.MediaExtractor
import android.media.MediaFormat
import android.net.Uri
import android.util.Log

object AudioDecoder {
    /**
     * Decode the first audio track and stream mono float chunks to [onBuffer].
     * Chunks are at the track's native sample rate; nothing is kept in memory.
     */
    fun decodeMono(
        context: Context,
        uri: Uri,
        onBuffer: (samples: FloatArray, sampleRate: Int) -> Unit,
    ) {
        val extractor = MediaExtractor()
        var codec: MediaCodec? = null
        try {
            context.contentResolver.openFileDescriptor(uri, "r")!!.use { descriptor ->
                extractor.setDataSource(descriptor.fileDescriptor)
            }

            var trackIndex = -1
            var format: MediaFormat? = null
            for (index in 0 until extractor.trackCount) {
                val candidate = extractor.getTrackFormat(index)
                val mime = candidate.getString(MediaFormat.KEY_MIME).orEmpty()
                if (mime.startsWith("audio/")) {
                    trackIndex = index
                    format = candidate
                    break
                }
            }
            require(trackIndex >= 0 && format != null) { "视频没有音频轨道" }

            extractor.selectTrack(trackIndex)
            val mime = format!!.getString(MediaFormat.KEY_MIME)!!
            var sampleRate = format.getInteger(MediaFormat.KEY_SAMPLE_RATE)
            var channels = format.getInteger(MediaFormat.KEY_CHANNEL_COUNT).coerceAtLeast(1)
            var pcmEncoding = runCatching {
                format.getInteger(MediaFormat.KEY_PCM_ENCODING)
            }.getOrDefault(AudioFormat.ENCODING_PCM_16BIT)
            Log.i(TAG, "audio track: mime=$mime rate=$sampleRate channels=$channels")

            codec = MediaCodec.createDecoderByType(mime)
            codec.configure(format, null, null, 0)
            codec.start()

            val info = MediaCodec.BufferInfo()
            var inputDone = false
            var outputDone = false

            while (!outputDone) {
                if (!inputDone) {
                    val inputIndex = codec.dequeueInputBuffer(10_000)
                    if (inputIndex >= 0) {
                        val buffer = codec.getInputBuffer(inputIndex)!!
                        val size = extractor.readSampleData(buffer, 0)
                        if (size < 0) {
                            codec.queueInputBuffer(
                                inputIndex,
                                0,
                                0,
                                0,
                                MediaCodec.BUFFER_FLAG_END_OF_STREAM,
                            )
                            inputDone = true
                        } else {
                            codec.queueInputBuffer(inputIndex, 0, size, extractor.sampleTime, 0)
                            extractor.advance()
                        }
                    }
                }

                when (val outputIndex = codec.dequeueOutputBuffer(info, 10_000)) {
                    MediaCodec.INFO_TRY_AGAIN_LATER -> Unit
                    MediaCodec.INFO_OUTPUT_FORMAT_CHANGED -> {
                        val outputFormat = codec.outputFormat
                        sampleRate = outputFormat.getInteger(MediaFormat.KEY_SAMPLE_RATE)
                        channels = outputFormat.getInteger(MediaFormat.KEY_CHANNEL_COUNT)
                            .coerceAtLeast(1)
                        pcmEncoding = runCatching {
                            outputFormat.getInteger(MediaFormat.KEY_PCM_ENCODING)
                        }.getOrDefault(AudioFormat.ENCODING_PCM_16BIT)
                    }
                    else -> if (outputIndex >= 0) {
                        if (info.size > 0) {
                            val buffer = codec.getOutputBuffer(outputIndex)!!
                            buffer.position(info.offset)
                            buffer.limit(info.offset + info.size)
                            val mono = if (pcmEncoding == AudioFormat.ENCODING_PCM_FLOAT) {
                                monoFromFloats(buffer.asFloatBuffer().let { floats ->
                                    FloatArray(floats.remaining()).also { floats.get(it) }
                                }, channels)
                            } else {
                                monoFromShorts(buffer.asShortBuffer().let { shorts ->
                                    ShortArray(shorts.remaining()).also { shorts.get(it) }
                                }, channels)
                            }
                            onBuffer(mono, sampleRate)
                        }
                        codec.releaseOutputBuffer(outputIndex, false)
                        if (info.flags and MediaCodec.BUFFER_FLAG_END_OF_STREAM != 0) {
                            outputDone = true
                        }
                    }
                }
            }
        } finally {
            codec?.let {
                runCatching { it.stop() }
                it.release()
            }
            extractor.release()
        }
    }

    /** Decode the first audio track to mono 16 kHz float samples. */
    fun decodeTo16kMono(context: Context, uri: Uri): FloatArray {
        val sink = FloatSink()
        var sampleRate = 16_000
        decodeMono(context, uri) { samples, rate ->
            sampleRate = rate
            sink.append(samples)
        }
        return resample(sink.toArray(), sampleRate, 16_000)
    }

    /**
     * Decode only [ranges] (source milliseconds) and hand each range to
     * [onRange] as soon as it is complete, then forget it.
     *
     * Used for ASR: long videos never hold the whole soundtrack in memory, and
     * silent stretches are skipped entirely.
     */
    fun decodeRangesTo16kMono(
        context: Context,
        uri: Uri,
        ranges: List<LongRange>,
        onRange: (index: Int, samples: FloatArray) -> Unit,
    ) {
        if (ranges.isEmpty()) return
        var sampleRate = 16_000
        var decodedSamples = 0L
        var index = 0
        var buffer = FloatSink()
        var bufferStart = -1L
        var bufferEnd = -1L

        decodeMono(context, uri) { samples, rate ->
            sampleRate = rate
            var cursor = 0
            while (cursor < samples.size && index < ranges.size) {
                val range = ranges[index]
                if (bufferStart < 0) {
                    bufferStart = range.first * rate / 1000
                    bufferEnd = range.last * rate / 1000
                }
                val absolute = decodedSamples + cursor
                when {
                    absolute >= bufferEnd -> {
                        emitRange(index, buffer, rate, onRange)
                        index++
                        buffer = FloatSink()
                        bufferStart = -1
                    }
                    absolute < bufferStart -> {
                        cursor += (bufferStart - absolute).toInt().coerceAtLeast(1)
                    }
                    else -> {
                        val take = minOf(samples.size - cursor, (bufferEnd - absolute).toInt())
                        buffer.append(samples, cursor, cursor + take)
                        cursor += take
                    }
                }
            }
            decodedSamples += samples.size
        }

        if (index < ranges.size) {
            emitRange(index, buffer, sampleRate, onRange)
        }
    }

    private fun emitRange(
        index: Int,
        sink: FloatSink,
        sampleRate: Int,
        onRange: (Int, FloatArray) -> Unit,
    ) {
        val raw = sink.toArray()
        onRange(index, if (raw.isEmpty()) raw else resample(raw, sampleRate, 16_000))
    }

    private fun monoFromShorts(samples: ShortArray, channels: Int): FloatArray {
        val output = FloatArray(samples.size / channels)
        for (index in output.indices) {
            // Take the loudest channel instead of averaging: a quiet or
            // phase-inverted channel must not make speech look like silence.
            var best = 0f
            for (channel in 0 until channels) {
                val value = samples[index * channels + channel] / 32768f
                if (kotlin.math.abs(value) > kotlin.math.abs(best)) best = value
            }
            output[index] = best
        }
        return output
    }

    private fun monoFromFloats(samples: FloatArray, channels: Int): FloatArray {
        val output = FloatArray(samples.size / channels)
        for (index in output.indices) {
            var best = 0f
            for (channel in 0 until channels) {
                val value = samples[index * channels + channel]
                if (kotlin.math.abs(value) > kotlin.math.abs(best)) best = value
            }
            output[index] = best
        }
        return output
    }

    private class FloatSink {
        private var data = FloatArray(16_000 * 30)
        private var size = 0

        fun append(samples: FloatArray, from: Int = 0, to: Int = samples.size) {
            val count = to - from
            if (size + count > data.size) {
                data = data.copyOf((size + count).coerceAtLeast(data.size * 2))
            }
            samples.copyInto(data, size, from, to)
            size += count
        }

        fun toArray(): FloatArray = data.copyOf(size)
    }

    private fun resample(samples: FloatArray, from: Int, to: Int): FloatArray {
        if (samples.isEmpty() || from == to || from <= 0 || to <= 0) return samples
        val ratio = from.toDouble() / to.toDouble()
        val outputLength = (samples.size / ratio).toInt().coerceAtLeast(0)
        val output = FloatArray(outputLength)
        for (index in output.indices) {
            val position = index * ratio
            val left = position.toInt().coerceIn(0, samples.lastIndex)
            val right = (left + 1).coerceAtMost(samples.lastIndex)
            val fraction = (position - left).toFloat()
            output[index] = samples[left] * (1f - fraction) + samples[right] * fraction
        }
        return output
    }
}
