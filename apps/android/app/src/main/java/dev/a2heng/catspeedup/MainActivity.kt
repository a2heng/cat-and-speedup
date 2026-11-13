package dev.a2heng.catspeedup

import android.content.Intent
import android.graphics.Color
import android.net.Uri
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.util.Log
import android.view.View
import android.view.ViewGroup
import android.widget.BaseAdapter
import android.widget.Button
import android.widget.CheckBox
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.ListView
import android.widget.TextView
import androidx.activity.result.contract.ActivityResultContracts
import androidx.appcompat.app.AppCompatActivity
import androidx.media3.common.MediaItem
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.ui.PlayerView

class MainActivity : AppCompatActivity() {
    private lateinit var player: ExoPlayer
    private lateinit var playerView: PlayerView
    private lateinit var statusView: TextView
    private lateinit var modelView: TextView
    private lateinit var speedStateView: TextView
    private lateinit var currentSubtitleView: TextView
    private lateinit var listView: ListView
    private lateinit var adapter: SubtitleAdapter
    private lateinit var offlineAsr: OfflineAsr
    private lateinit var videoExporter: VideoExporter
    private lateinit var speedInput: EditText
    private lateinit var noiseInput: EditText
    private lateinit var minSilenceInput: EditText
    private lateinit var marginInput: EditText
    private lateinit var followPlanView: CheckBox

    private var subtitles: List<Subtitle> = emptyList()
    private var currentIndex = -1
    private var videoUri: Uri? = null
    private var plan: PlaybackPlan? = null
    private var planOptions = PlanOptions()
    private var transcribing = false
    private var analyzing = false
    private var exporting = false

    private val handler = Handler(Looper.getMainLooper())
    private val tick = object : Runnable {
        override fun run() {
            updateCurrentSubtitle()
            applyPlaybackSpeed()
            handler.postDelayed(this, 100)
        }
    }

    private val openVideo = registerForActivityResult(
        ActivityResultContracts.OpenDocument(),
    ) { uri: Uri? ->
        uri ?: return@registerForActivityResult
        openVideoUri(uri)
    }

    private fun openVideoUri(uri: Uri) {
        Log.i(TAG, "open video: $uri")
        runCatching {
            contentResolver.takePersistableUriPermission(
                uri,
                Intent.FLAG_GRANT_READ_URI_PERMISSION,
            )
        }
        player.setMediaItem(MediaItem.fromUri(uri))
        player.prepare()
        player.playWhenReady = true
        player.setPlaybackSpeed(1f)
        videoUri = uri
        plan = null
        subtitles = emptyList()
        currentIndex = -1
        adapter.setItems(subtitles)
        currentSubtitleView.text = ""
        speedStateView.text = getString(R.string.speed_state_idle)
        statusView.text = "已加入视频，点“切分”开始"
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_main)

        statusView = findViewById(R.id.status)
        modelView = findViewById(R.id.model)
        speedStateView = findViewById(R.id.speed_state)
        currentSubtitleView = findViewById(R.id.current_subtitle)
        listView = findViewById(R.id.subtitle_list)
        playerView = findViewById(R.id.player_view)
        speedInput = findViewById(R.id.speed_input)
        noiseInput = findViewById(R.id.noise_input)
        minSilenceInput = findViewById(R.id.min_silence_input)
        marginInput = findViewById(R.id.margin_input)
        followPlanView = findViewById(R.id.follow_plan)

        player = ExoPlayer.Builder(this).build()
        playerView.player = player
        offlineAsr = OfflineAsr(applicationContext)
        videoExporter = VideoExporter(this)

        adapter = SubtitleAdapter()
        listView.adapter = adapter
        listView.setOnItemClickListener { _, _, position, _ ->
            val subtitle = subtitles.getOrNull(position) ?: return@setOnItemClickListener
            Log.i(TAG, "seek via subtitle[$position] -> ${subtitle.startMs}ms")
            player.seekTo(subtitle.startMs)
            player.playWhenReady = true
            applyPlaybackSpeed(force = true)
        }

        findViewById<Button>(R.id.open_video).setOnClickListener {
            openVideo.launch(arrayOf("video/*"))
        }
        findViewById<Button>(R.id.analyze).setOnClickListener {
            analyze(null)
        }
        findViewById<Button>(R.id.transcribe_local).setOnClickListener {
            transcribe()
        }
        findViewById<Button>(R.id.export_video).setOnClickListener {
            exportVideo()
        }
        findViewById<Button>(R.id.export_subtitles).setOnClickListener {
            exportSubtitles()
        }
        followPlanView.setOnCheckedChangeListener { _, _ -> applyPlaybackSpeed(force = true) }

        handler.post(tick)
        modelView.text = offlineAsr.modelStatus()
        handleViewIntent(intent)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handleViewIntent(intent)
    }

    private fun handleViewIntent(intent: Intent?) {
        if (intent?.action != Intent.ACTION_VIEW) return
        intent.data?.let { openVideoUri(it) }
        if (intent.getBooleanExtra(EXTRA_AUTO_ANALYZE, false)) {
            val autoTranscribe = intent.getBooleanExtra(EXTRA_AUTO_TRANSCRIBE, false)
            val autoExport = intent.getBooleanExtra(EXTRA_AUTO_EXPORT, false)
            Log.i(
                TAG,
                "auto_analyze=true auto_transcribe=$autoTranscribe auto_export=$autoExport",
            )
            handler.postDelayed(
                {
                    analyze {
                        if (autoTranscribe) transcribe()
                        if (autoExport) handler.postDelayed({ exportVideo() }, 500)
                    }
                },
                500,
            )
        }
    }

    override fun onDestroy() {
        handler.removeCallbacks(tick)
        if (!transcribing && !exporting) {
            offlineAsr.release()
        }
        player.release()
        super.onDestroy()
    }

    /** Parse the four threshold fields; returns null (with a message) when invalid. */
    private fun readOptions(): PlanOptions? {
        val silenceSpeed = speedInput.text.toString().toFloatOrNull()
        val noiseDb = noiseInput.text.toString().toFloatOrNull()
        val minSilenceMs = minSilenceInput.text.toString().toLongOrNull()
        val marginMs = marginInput.text.toString().toLongOrNull()
        if (silenceSpeed == null || noiseDb == null || minSilenceMs == null || marginMs == null) {
            statusView.text = "参数填错了：倍速 / 阈值 / 最短静音 / 余量"
            return null
        }
        return PlanOptions(
            silenceSpeed = silenceSpeed.coerceIn(1f, 8f),
            noiseDb = noiseDb.coerceIn(-80f, -1f),
            marginMs = marginMs.coerceIn(0, 2_000),
            minSilenceMs = minSilenceMs.coerceIn(100, 10_000),
        )
    }

    /**
     * Analyze the audio, store the plan and then run [next] (on the UI thread).
     * If a plan already exists, [next] runs immediately.
     */
    private fun analyze(next: (() -> Unit)?) {
        if (analyzing) {
            statusView.text = "正在切分，请稍等…"
            return
        }
        val existing = plan
        val options = readOptions() ?: return
        if (existing != null && options == planOptions) {
            showPlanSummary(existing)
            next?.invoke()
            return
        }
        val uri = videoUri
        if (uri == null) {
            statusView.text = "先加入一个视频"
            return
        }

        analyzing = true
        showProgress("切分中：分析响度…", startedAt = System.currentTimeMillis())
        val videoDurationMs = player.duration.takeIf { it > 0 } ?: 0L
        Log.i(TAG, "analyze start: $options videoDuration=${videoDurationMs}ms")

        Thread {
            runCatching {
                val detected = SilenceDetector.detect(
                    context = this,
                    uri = uri,
                    noiseDb = options.noiseDb,
                    minSilenceMs = options.minSilenceMs,
                )
                val duration = maxOf(detected.durationMs, videoDurationMs)
                SilenceDetector.buildPlan(duration, detected.silence, options)
            }.onSuccess { result ->
                Log.i(
                    TAG,
                    "analyze done: segments=${result.segments.size} " +
                        "output=${result.outputDurationMs()}ms",
                )
                runOnUiThread {
                    analyzing = false
                    stopProgress()
                    plan = result
                    planOptions = options
                    followPlanView.isChecked = true
                    applyPlaybackSpeed(force = true)
                    showPlanSummary(result)
                    next?.invoke()
                }
            }.onFailure { error ->
                Log.e(TAG, "analyze failed", error)
                runOnUiThread {
                    analyzing = false
                    stopProgress()
                    statusView.text = "切分失败：${error.message}"
                }
            }
        }.start()
    }

    private companion object {
        const val ASR_CHUNK_MS = 30_000L

        /** adb-only helpers: `am start ... --ez auto_analyze true`. */
        const val EXTRA_AUTO_ANALYZE = "auto_analyze"
        const val EXTRA_AUTO_TRANSCRIBE = "auto_transcribe"
        const val EXTRA_AUTO_EXPORT = "auto_export"
    }

    private fun showPlanSummary(result: PlaybackPlan) {
        statusView.text = buildString {
            append("人声 ${result.voiceSegmentCount()} 段 ")
            append(formatDuration(result.voiceDurationMs()))
            append("；静音 ")
            append(formatDuration(result.silenceDurationMs()))
            append(" 按 ${trimSpeed(planOptions.silenceSpeed)}x")
            append("：")
            append(formatDuration(result.sourceDurationMs))
            append(" → 约 ")
            append(formatDuration(result.outputDurationMs()))
        }
    }

    private fun trimSpeed(speed: Float): String =
        if (speed == speed.toInt().toFloat()) speed.toInt().toString() else speed.toString()

    private fun transcribe() {
        if (transcribing) {
            statusView.text = "正在转写，请稍等…"
            return
        }
        analyze {
            val uri = videoUri ?: return@analyze
            if (transcribing) return@analyze
            if (!offlineAsr.hasModel()) {
                modelView.text = offlineAsr.modelStatus()
                statusView.text = "缺少 FunASR 模型，见上方路径"
                Log.w(TAG, "transcribe: model missing, expected ${offlineAsr.expectedDir()}")
                return@analyze
            }
            val currentPlan = plan ?: return@analyze
            val voiceRanges = currentPlan.segments
                .filter { it.isVoice }
                .map { it.startMs..it.endMs }
            if (voiceRanges.isEmpty()) {
                statusView.text = "没有检测到人声段，无法转写"
                Log.w(TAG, "transcribe: no voice segment in plan")
                return@analyze
            }

            transcribing = true
            val startedAt = System.currentTimeMillis()
            statusView.text = "转写中：准备人声 ${formatDuration(currentPlan.voiceDurationMs())}…"
            val chunks = splitForAsr(voiceRanges)
            Log.i(
                TAG,
                "transcribe start: model=${offlineAsr.modelStatus()} " +
                    "voiceRanges=${voiceRanges.size} chunks=${chunks.size} " +
                    "voice=${currentPlan.voiceDurationMs()}ms",
            )
            Thread {
                runCatching {
                    val all = ArrayList<Subtitle>()
                    var done = 0
                    AudioDecoder.decodeRangesTo16kMono(this, uri, chunks) { index, samples ->
                        if (samples.isNotEmpty()) {
                            val range = chunks[index]
                            val result = offlineAsr.transcribe(samples)
                            val added = buildSubtitles(result, range.first, range.last - range.first)
                            all += added
                            Log.d(
                                TAG,
                                "asr chunk[$index] ${range.first}-${range.last}ms " +
                                    "samples=${samples.size} -> ${added.size} lines",
                            )
                        } else {
                            Log.w(TAG, "asr chunk[$index] ${chunks[index]} decoded empty")
                        }
                        done++
                        if (done % 3 == 0 || done == chunks.size) {
                            val progress = done
                            val seconds = (System.currentTimeMillis() - startedAt) / 1000
                            runOnUiThread {
                                statusView.text = "转写中：$progress/${chunks.size} 段（${seconds}s）"
                            }
                        }
                    }
                    all
                }.onSuccess { result ->
                    Log.i(
                        TAG,
                        "transcribe done: ${result.size} lines, " +
                            "${System.currentTimeMillis() - startedAt}ms",
                    )
                    runOnUiThread {
                        transcribing = false
                        stopProgress()
                        subtitles = result
                        currentIndex = -1
                        adapter.setItems(result)
                        val seconds = (System.currentTimeMillis() - startedAt) / 1000
                        statusView.text = "字幕：${result.size} 条，用时 ${seconds}s"
                        modelView.text = offlineAsr.modelStatus()
                    }
                }.onFailure { error ->
                    Log.e(TAG, "transcribe failed", error)
                    runOnUiThread {
                        transcribing = false
                        stopProgress()
                        statusView.text = "转写失败：${error.message}"
                    }
                }
            }.start()
        }
    }

    private fun exportVideo() {
        if (exporting) {
            statusView.text = "正在导出，请稍等…"
            return
        }
        analyze {
            val uri = videoUri ?: return@analyze
            val currentPlan = plan ?: return@analyze
            if (exporting) return@analyze
            exporting = true
            val frameRate = player.videoFormat?.frameRate?.toInt() ?: 0
            statusView.text = "导出中：准备编码…"
            Log.i(
                TAG,
                "export start: segments=${currentPlan.segments.size} " +
                    "frameRate=$frameRate output=${currentPlan.outputDurationMs()}ms",
            )
            var lastPercent = -1
            videoExporter.export(
                uri = uri,
                plan = currentPlan,
                frameRate = frameRate.coerceIn(0, 120),
                onProgress = { percent ->
                    statusView.text = if (percent in 0..100) {
                        "导出中：$percent%（成片约 ${formatDuration(currentPlan.outputDurationMs())}）"
                    } else {
                        "导出中：编码…"
                    }
                    if (percent / 10 != lastPercent / 10) {
                        lastPercent = percent
                        Log.i(TAG, "export progress: $percent%")
                    }
                },
                onFinished = { result ->
                    exporting = false
                    result.onSuccess { Log.i(TAG, "export done: $it") }
                        .onFailure { Log.e(TAG, "export failed", it) }
                    statusView.text = result.fold(
                        onSuccess = { "已导出：$it" },
                        onFailure = { "导出失败：${it.message}" },
                    )
                },
            )
        }
    }

    private fun exportSubtitles() {
        if (subtitles.isEmpty()) {
            statusView.text = "先点“转写”，再导出字幕"
            return
        }
        val stamp = java.text.SimpleDateFormat("yyyyMMdd-HHmmss", java.util.Locale.US)
            .format(java.util.Date())
        val name = "cat-speedup-$stamp.srt"
        runCatching {
            ExportStore.saveSubtitle(this, name, toSrt(subtitles))
        }.onSuccess { location ->
            statusView.text = "已导出 ${subtitles.size} 条字幕：$location"
        }.onFailure { error ->
            statusView.text = "字幕导出失败：${error.message}"
        }
    }

    /** Split long voice ranges into ~30 s ASR chunks to keep memory bounded. */
    private fun splitForAsr(ranges: List<LongRange>): List<LongRange> {
        val output = ArrayList<LongRange>()
        for (range in ranges) {
            var start = range.first.coerceAtLeast(0)
            while (start < range.last) {
                val end = minOf(range.last, start + ASR_CHUNK_MS)
                if (end > start) output += start..end
                start = end
            }
        }
        return output
    }

    private fun applyPlaybackSpeed(force: Boolean = false) {
        val segment = if (followPlanView.isChecked) {
            plan?.segmentAt(player.currentPosition)
        } else {
            null
        }
        val desired = segment?.speed ?: 1f
        if (force || player.playbackParameters.speed != desired) {
            Log.d(
                TAG,
                "speed ${player.playbackParameters.speed}x -> ${desired}x " +
                    "at ${player.currentPosition}ms",
            )
            player.setPlaybackSpeed(desired)
        }

        val label = when {
            plan == null -> getString(R.string.speed_state_idle)
            !followPlanView.isChecked -> "播放：1x（跟随加速已关闭）"
            segment == null -> "播放：1x"
            segment.isVoice -> "播放：${trimSpeed(segment.speed)}x（人声段）"
            else -> "播放：${trimSpeed(segment.speed)}x（跳过静音）"
        }
        if (speedStateView.text != label) speedStateView.text = label
    }

    private var progressRunnable: Runnable? = null

    private fun showProgress(text: String, startedAt: Long) {
        stopProgress()
        statusView.text = text
        val runnable = object : Runnable {
            override fun run() {
                val seconds = (System.currentTimeMillis() - startedAt) / 1000
                statusView.text = "$text ${seconds}s"
                handler.postDelayed(this, 500)
            }
        }
        progressRunnable = runnable
        handler.postDelayed(runnable, 500)
    }

    private fun stopProgress() {
        progressRunnable?.let { handler.removeCallbacks(it) }
        progressRunnable = null
    }

    private fun updateCurrentSubtitle() {
        val position = player.currentPosition
        val index = subtitles.indexOfLast { position >= it.startMs && position < it.endMs }
        if (index == currentIndex) return

        currentIndex = index
        currentSubtitleView.text = subtitles.getOrNull(index)?.text.orEmpty()
        adapter.setActiveIndex(index)
        if (index >= 0) {
            listView.smoothScrollToPosition(index)
        }
    }

    private inner class SubtitleAdapter : BaseAdapter() {
        private var items: List<Subtitle> = emptyList()
        private var activeIndex = -1

        fun setItems(value: List<Subtitle>) {
            items = value
            activeIndex = -1
            notifyDataSetChanged()
        }

        fun setActiveIndex(value: Int) {
            if (activeIndex == value) return
            activeIndex = value
            notifyDataSetChanged()
        }

        override fun getCount(): Int = items.size

        override fun getItem(position: Int): Subtitle = items[position]

        override fun getItemId(position: Int): Long = position.toLong()

        override fun getView(position: Int, convertView: View?, parent: ViewGroup): View {
            val row = (convertView as? SubtitleRow) ?: SubtitleRow(parent.context)
            row.bind(items[position], position == activeIndex)
            return row
        }
    }

    private class SubtitleRow(context: android.content.Context) : LinearLayout(context) {
        private val timeView = TextView(context).apply {
            textSize = 11f
            setTextColor(Color.parseColor("#888888"))
        }
        private val textView = TextView(context).apply {
            textSize = 15f
            setTextColor(Color.parseColor("#212121"))
        }

        init {
            orientation = VERTICAL
            setPadding(16, 12, 16, 12)
            addView(timeView)
            addView(textView)
        }

        fun bind(subtitle: Subtitle, active: Boolean) {
            timeView.text = formatTime(subtitle.startMs)
            textView.text = subtitle.text
            if (active) {
                setBackgroundColor(Color.parseColor("#E3F2FD"))
                timeView.setTextColor(Color.parseColor("#1565C0"))
                textView.setTextColor(Color.parseColor("#0D47A1"))
            } else {
                setBackgroundColor(Color.TRANSPARENT)
                timeView.setTextColor(Color.parseColor("#888888"))
                textView.setTextColor(Color.parseColor("#212121"))
            }
        }
    }
}
