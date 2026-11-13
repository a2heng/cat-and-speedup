package dev.a2heng.catspeedup

import android.content.ContentValues
import android.content.Context
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.os.Handler
import android.os.Looper
import android.provider.MediaStore
import android.util.Log
import androidx.media3.common.C
import androidx.media3.common.MediaItem
import androidx.media3.common.MimeTypes
import androidx.media3.common.SpeedParameters
import androidx.media3.common.audio.SpeedProvider
import androidx.media3.transformer.Composition
import androidx.media3.transformer.EditedMediaItem
import androidx.media3.transformer.ExportException
import androidx.media3.transformer.ExportResult
import androidx.media3.transformer.ProgressHolder
import androidx.media3.transformer.Transformer
import java.io.File

/**
 * Renders the accelerated video on the phone.
 *
 * Every plan segment becomes one clipped, speed-adjusted item in a single
 * composition, so the order is preserved and audio/video stay in sync. Media3
 * re-encodes both tracks (speed changes require it) and concatenates them.
 */
class VideoExporter(private val context: Context) {
    private val handler = Handler(Looper.getMainLooper())
    private var transformer: Transformer? = null
    private var polling: Runnable? = null
    private val progressHolder = ProgressHolder()

    fun export(
        uri: Uri,
        plan: PlaybackPlan,
        frameRate: Int,
        onProgress: (Int) -> Unit,
        onFinished: (Result<String>) -> Unit,
    ) {
        if (transformer != null) {
            onFinished(Result.failure(IllegalStateException("正在导出上一个视频")))
            return
        }
        val temp = File(context.cacheDir, "export-${System.currentTimeMillis()}.mp4")

        // One item, time-varying speed: the provider is queried with source
        // time, so long videos do not need one clip+seek per segment.
        val speedProvider = object : SpeedProvider {
            override fun getSpeed(timeUs: Long): Float =
                plan.segmentAt(timeUs / 1_000)?.speed ?: 1f

            override fun getNextSpeedChangeTimeUs(timeUs: Long): Long {
                val segment = plan.segmentAt(timeUs / 1_000) ?: return C.TIME_UNSET
                return if (segment.endMs >= plan.sourceDurationMs) {
                    C.TIME_UNSET
                } else {
                    segment.endMs * 1_000
                }
            }
        }
        val itemBuilder = EditedMediaItem.Builder(MediaItem.fromUri(uri))
            .setSpeed(SpeedParameters(speedProvider, /* shouldMaintainPitch= */ true))
        if (frameRate > 0) itemBuilder.setFrameRate(frameRate)
        val item = itemBuilder.build()

        Log.i(
            TAG,
            "export: segments=${plan.segments.size} frameRate=$frameRate " +
                "output=${plan.outputDurationMs()}ms temp=${temp.name}",
        )

        val built = Transformer.Builder(context)
            .setAudioMimeType(MimeTypes.AUDIO_AAC)
            .setVideoMimeType(MimeTypes.VIDEO_H264)
            .addListener(object : Transformer.Listener {
                override fun onCompleted(composition: Composition, exportResult: ExportResult) {
                    Log.i(
                        TAG,
                        "export completed: duration=${exportResult.approximateDurationMs}ms " +
                            "size=${exportResult.fileSizeBytes}B " +
                            "video=${exportResult.videoEncoderName} " +
                            "audio=${exportResult.audioEncoderName} samples=${exportResult.sampleRate}Hz",
                    )
                    finish {
                        Thread {
                            val saved = runCatching {
                                ExportStore.saveVideo(context, temp, fileName("mp4"))
                            }
                            temp.delete()
                            handler.post { onFinished(saved) }
                        }.start()
                    }
                }

                override fun onError(
                    composition: Composition,
                    exportResult: ExportResult,
                    exportException: ExportException,
                ) {
                    Log.e(TAG, "export error: ${exportException.errorCodeName()}", exportException)
                    finish {
                        temp.delete()
                        onFinished(
                            Result.failure(
                                IllegalStateException(
                                    "导出失败（${exportException.errorCodeName}）：${exportException.message}",
                                ),
                            ),
                        )
                    }
                }
            })
            .build()
        transformer = built

        val startError = runCatching { built.start(item, temp.absolutePath) }.exceptionOrNull()
        if (startError != null) {
            transformer = null
            temp.delete()
            onFinished(Result.failure(startError))
            return
        }

        val poll = object : Runnable {
            override fun run() {
                val active = transformer ?: return
                when (active.getProgress(progressHolder)) {
                    Transformer.PROGRESS_STATE_AVAILABLE -> onProgress(progressHolder.progress)
                    Transformer.PROGRESS_STATE_UNAVAILABLE -> onProgress(-1)
                    else -> Unit
                }
                handler.postDelayed(this, 500)
            }
        }
        polling = poll
        handler.post(poll)
    }

    fun cancel() {
        transformer?.cancel()
        finish { }
    }

    private fun finish(block: () -> Unit) {
        polling?.let { handler.removeCallbacks(it) }
        polling = null
        transformer = null
        block()
    }

    private fun fileName(extension: String): String {
        val stamp = java.text.SimpleDateFormat("yyyyMMdd-HHmmss", java.util.Locale.US)
            .format(java.util.Date())
        return "cat-speedup-$stamp.$extension"
    }

    private fun ExportException.errorCodeName(): String =
        when (errorCode) {
            ExportException.ERROR_CODE_IO_FILE_NOT_FOUND -> "文件不存在"
            ExportException.ERROR_CODE_DECODING_FORMAT_UNSUPPORTED -> "解码格式不支持"
            ExportException.ERROR_CODE_ENCODING_FORMAT_UNSUPPORTED -> "编码格式不支持"
            ExportException.ERROR_CODE_MUXING_FAILED -> "封装失败"
            ExportException.ERROR_CODE_AUDIO_PROCESSING_FAILED -> "音频处理失败"
            else -> errorCode.toString()
        }
}

/** Writes finished files where the gallery / file manager can see them. */
object ExportStore {
    fun saveVideo(context: Context, temp: File, displayName: String): String {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            val values = ContentValues().apply {
                put(MediaStore.Video.Media.DISPLAY_NAME, displayName)
                put(MediaStore.Video.Media.MIME_TYPE, "video/mp4")
                put(
                    MediaStore.Video.Media.RELATIVE_PATH,
                    "${Environment.DIRECTORY_MOVIES}/CatSpeedup",
                )
                put(MediaStore.Video.Media.IS_PENDING, 1)
            }
            val resolver = context.contentResolver
            val uri = resolver.insert(MediaStore.Video.Media.EXTERNAL_CONTENT_URI, values)
                ?: error("无法创建视频文件")
            try {
                resolver.openOutputStream(uri)?.use { output ->
                    temp.inputStream().use { it.copyTo(output) }
                } ?: error("无法写入视频文件")
                values.clear()
                values.put(MediaStore.Video.Media.IS_PENDING, 0)
                resolver.update(uri, values, null, null)
            } catch (error: Throwable) {
                resolver.delete(uri, null, null)
                throw error
            }
            return "Movies/CatSpeedup/$displayName"
        }
        val dir = File(
            context.getExternalFilesDir(Environment.DIRECTORY_MOVIES) ?: context.filesDir,
            "CatSpeedup",
        ).apply { mkdirs() }
        val target = File(dir, displayName)
        temp.copyTo(target, overwrite = true)
        return target.absolutePath
    }

    fun saveSubtitle(context: Context, displayName: String, text: String): String {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            val values = ContentValues().apply {
                put(MediaStore.Downloads.DISPLAY_NAME, displayName)
                put(MediaStore.Downloads.MIME_TYPE, "application/x-subrip")
                put(MediaStore.Downloads.RELATIVE_PATH, "${Environment.DIRECTORY_DOWNLOADS}/CatSpeedup")
            }
            val resolver = context.contentResolver
            val uri = resolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values)
                ?: error("无法创建字幕文件")
            try {
                resolver.openOutputStream(uri)?.use { output ->
                    output.write(text.toByteArray(Charsets.UTF_8))
                } ?: error("无法写入字幕文件")
            } catch (error: Throwable) {
                resolver.delete(uri, null, null)
                throw error
            }
            return "Download/CatSpeedup/$displayName"
        }
        val dir = File(
            context.getExternalFilesDir(Environment.DIRECTORY_DOWNLOADS) ?: context.filesDir,
            "CatSpeedup",
        ).apply { mkdirs() }
        val target = File(dir, displayName)
        target.writeText(text)
        return target.absolutePath
    }
}
