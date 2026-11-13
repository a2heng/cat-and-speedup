package dev.a2heng.catspeedup

import android.content.Context
import android.net.Uri
import android.util.Log
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.security.MessageDigest

/** Persisted plan parameters, so reopening the app keeps the last tuning. */
object AppPrefs {
    private const val NAME = "cat-speedup"
    private const val KEY_SPEED = "silence_speed"
    private const val KEY_GAP = "min_gap_ms"
    private const val KEY_MARGIN = "margin_ms"

    fun load(context: Context): PlanOptions {
        val prefs = context.getSharedPreferences(NAME, Context.MODE_PRIVATE)
        return PlanOptions(
            silenceSpeed = prefs.getFloat(KEY_SPEED, PlanDefaults.SILENCE_SPEED),
            minSpeedupSilenceMs = prefs.getLong(
                KEY_GAP,
                PlanDefaults.MIN_SPEEDUP_SILENCE_MS,
            ),
            marginMs = prefs.getLong(KEY_MARGIN, PlanDefaults.MARGIN_MS),
        )
    }

    fun save(context: Context, options: PlanOptions) {
        context.getSharedPreferences(NAME, Context.MODE_PRIVATE).edit()
            .putFloat(KEY_SPEED, options.silenceSpeed)
            .putLong(KEY_GAP, options.minSpeedupSilenceMs)
            .putLong(KEY_MARGIN, options.marginMs)
            .apply()
    }
}

data class CachedTranscript(
    val durationMs: Long,
    val subtitles: List<Subtitle>,
    val speech: List<LongRange>,
)

/**
 * One transcript per video, keyed by URI. Tuning parameters afterwards only
 * re-plans; it never decodes or transcribes again.
 */
object TranscriptStore {
    private fun dir(context: Context): File =
        File(context.filesDir, "transcripts").apply { mkdirs() }

    private fun file(context: Context, uri: Uri): File {
        val digest = MessageDigest.getInstance("SHA-1")
            .digest(uri.toString().toByteArray())
            .joinToString("") { "%02x".format(it) }
        return File(dir(context), "$digest.json")
    }

    fun load(context: Context, uri: Uri): CachedTranscript? {
        val file = file(context, uri)
        if (!file.isFile) return null
        return runCatching {
            val root = JSONObject(file.readText())
            val duration = root.optLong("duration_ms", 0)
            val array = root.getJSONArray("segments")
            val subtitles = ArrayList<Subtitle>(array.length())
            for (index in 0 until array.length()) {
                val item = array.getJSONObject(index)
                subtitles += Subtitle(
                    startMs = item.getLong("start_ms"),
                    endMs = item.getLong("end_ms"),
                    text = item.getString("text"),
                )
            }
            val speech = ArrayList<LongRange>()
            val speechArray = root.optJSONArray("speech")
            if (speechArray != null) {
                for (index in 0 until speechArray.length()) {
                    val pair = speechArray.getJSONArray(index)
                    speech += pair.getLong(0)..pair.getLong(1)
                }
            } else {
                // Old cache from before token-range planning.
                speech += subtitles.map { it.startMs..it.endMs }
            }
            CachedTranscript(duration, subtitles, mergeSpeechRanges(speech))
        }.onFailure {
            Log.w(TAG, "transcript cache read failed: ${file.name}", it)
        }.getOrNull()
    }

    fun save(
        context: Context,
        uri: Uri,
        durationMs: Long,
        subtitles: List<Subtitle>,
        speech: List<LongRange>,
    ) {
        runCatching {
            val array = JSONArray()
            for (subtitle in subtitles) {
                array.put(
                    JSONObject()
                        .put("start_ms", subtitle.startMs)
                        .put("end_ms", subtitle.endMs)
                        .put("text", subtitle.text),
                )
            }
            val speechArray = JSONArray()
            for (range in speech) {
                speechArray.put(JSONArray().put(range.first).put(range.last))
            }
            val root = JSONObject()
                .put("duration_ms", durationMs)
                .put("segments", array)
                .put("speech", speechArray)
            file(context, uri).writeText(root.toString())
            Log.i(TAG, "transcript cached: ${subtitles.size} lines for $uri")
        }.onFailure {
            Log.w(TAG, "transcript cache write failed", it)
        }
    }
}
