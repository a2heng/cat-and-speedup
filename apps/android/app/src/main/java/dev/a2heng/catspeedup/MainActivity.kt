package dev.a2heng.catspeedup

import android.content.Intent
import android.graphics.Bitmap
import android.graphics.Canvas
import android.net.Uri
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.util.Log
import android.view.LayoutInflater
import android.view.View
import android.view.ViewGroup
import android.widget.EditText
import android.widget.TextView
import androidx.activity.result.contract.ActivityResultContracts
import androidx.appcompat.app.AppCompatActivity
import androidx.core.content.ContextCompat
import androidx.media3.common.MediaItem
import androidx.media3.common.Player
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.ui.PlayerView
import androidx.recyclerview.widget.LinearLayoutManager
import androidx.recyclerview.widget.RecyclerView
import com.google.android.material.button.MaterialButton
import com.google.android.material.card.MaterialCardView
import android.widget.SeekBar
import java.io.File

class MainActivity : AppCompatActivity() {
    private lateinit var player: ExoPlayer
    private lateinit var playerView: PlayerView
    private lateinit var statusView: TextView
    private lateinit var modelView: TextView
    private lateinit var speedStateView: TextView
    private lateinit var currentSubtitleView: TextView
    private lateinit var currentSubtitleCard: View
    private lateinit var playToggle: MaterialButton
    private lateinit var seekBar: SeekBar
    private lateinit var positionText: TextView
    private lateinit var durationText: TextView
    private lateinit var recyclerView: RecyclerView
    private lateinit var subtitleCountView: TextView
    private lateinit var subtitleEmptyView: TextView
    private lateinit var paramsSummaryView: TextView
    private lateinit var settingsPanel: View
    private lateinit var adapter: SubtitleAdapter
    private lateinit var offlineAsr: OfflineAsr
    private lateinit var videoExporter: VideoExporter
    private lateinit var speedInput: EditText
    private lateinit var noiseInput: EditText
    private lateinit var minSilenceInput: EditText
    private lateinit var marginInput: EditText

    private var subtitles: List<Subtitle> = emptyList()
    private var currentIndex = -1
    private var videoUri: Uri? = null
    private var plan: PlaybackPlan? = null
    private var planOptions = PlanOptions()
    private var followPlan = true
    private var settingsExpanded = false
    private var userSeeking = false
    private var transcribing = false
    private var analyzing = false
    private var exporting = false

    private val handler = Handler(Looper.getMainLooper())
    private val tick = object : Runnable {
        override fun run() {
            updateCurrentSubtitle()
            applyPlaybackSpeed()
            updateProgressUi()
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
        currentSubtitleCard.visibility = View.GONE
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
        currentSubtitleCard = findViewById(R.id.current_subtitle_card)
        playToggle = findViewById(R.id.play_toggle)
        seekBar = findViewById(R.id.seek_bar)
        positionText = findViewById(R.id.position_text)
        durationText = findViewById(R.id.duration_text)
        recyclerView = findViewById(R.id.subtitle_list)
        subtitleCountView = findViewById(R.id.subtitle_count)
        subtitleEmptyView = findViewById(R.id.subtitle_empty)
        paramsSummaryView = findViewById(R.id.params_summary)
        settingsPanel = findViewById(R.id.settings_panel)
        playerView = findViewById(R.id.player_view)
        speedInput = findViewById(R.id.speed_input)
        noiseInput = findViewById(R.id.noise_input)
        minSilenceInput = findViewById(R.id.min_silence_input)
        marginInput = findViewById(R.id.margin_input)

        player = ExoPlayer.Builder(this).build()
        playerView.player = player
        offlineAsr = OfflineAsr(applicationContext)
        videoExporter = VideoExporter(this)

        adapter = SubtitleAdapter()
        recyclerView.layoutManager = LinearLayoutManager(this)
        recyclerView.adapter = adapter

        findViewById<View>(R.id.open_video).setOnClickListener {
            openVideo.launch(arrayOf("video/*"))
        }
        findViewById<View>(R.id.analyze).setOnClickListener {
            analyze(null)
        }
        findViewById<View>(R.id.transcribe_local).setOnClickListener {
            transcribe()
        }
        findViewById<View>(R.id.export_video).setOnClickListener {
            exportVideo()
        }
        findViewById<View>(R.id.export_subtitles).setOnClickListener {
            exportSubtitles()
        }
        findViewById<View>(R.id.settings_toggle).setOnClickListener { toggleSettings() }
        paramsSummaryView.setOnClickListener { toggleSettings() }
        speedStateView.setOnClickListener {
            followPlan = !followPlan
            Log.i(TAG, "follow plan: $followPlan")
            applyPlaybackSpeed(force = true)
        }
        speedStateView.contentDescription = getString(R.string.follow_toggle_desc)

        playerView.setOnClickListener { togglePlayback() }
        playToggle.setOnClickListener { togglePlayback() }
        seekBar.setOnSeekBarChangeListener(
            object : SeekBar.OnSeekBarChangeListener {
                override fun onProgressChanged(bar: SeekBar, progress: Int, fromUser: Boolean) {
                    if (!fromUser) return
                    val duration = player.duration
                    if (duration > 0) player.seekTo(duration * progress / 1000)
                }

                override fun onStartTrackingTouch(bar: SeekBar) {
                    userSeeking = true
                }

                override fun onStopTrackingTouch(bar: SeekBar) {
                    userSeeking = false
                }
            },
        )
        player.addListener(
            object : Player.Listener {
                override fun onPlaybackStateChanged(playbackState: Int) = updateProgressUi()
                override fun onIsPlayingChanged(isPlaying: Boolean) = updateProgressUi()
            },
        )

        updateParamSummary()
        updateEmptyState()
        handler.post(tick)
        modelView.text = offlineAsr.modelStatus()
        handleViewIntent(intent)
    }

    private fun toggleSettings() {
        settingsExpanded = !settingsExpanded
        settingsPanel.visibility = if (settingsExpanded) View.VISIBLE else View.GONE
        updateParamSummary()
    }

    private fun updateParamSummary() {
        val speed = speedInput.text?.toString()?.takeIf { it.isNotBlank() } ?: "4"
        val noise = noiseInput.text?.toString()?.takeIf { it.isNotBlank() } ?: "-30"
        val minSilence = minSilenceInput.text?.toString()?.takeIf { it.isNotBlank() } ?: "500"
        val margin = marginInput.text?.toString()?.takeIf { it.isNotBlank() } ?: "150"
        paramsSummaryView.text = "${speed}x · ${noise}dB · ${minSilence}ms · 余量 ${margin}ms"
    }

    private fun updateEmptyState() {
        val empty = subtitles.isEmpty()
        subtitleEmptyView.visibility = if (empty) View.VISIBLE else View.GONE
        subtitleCountView.text = if (empty) "" else getString(R.string.subtitle_count, subtitles.size)
    }

    /** adb-only: render the current layout to a PNG even when the screen is off. */
    private fun dumpUiToFile() {
        val root = findViewById<View>(android.R.id.content)
        if (root.width <= 0 || root.height <= 0) {
            Log.w(TAG, "ui dump skipped: ${root.width}x${root.height}")
            return
        }
        val bitmap = Bitmap.createBitmap(root.width, root.height, Bitmap.Config.ARGB_8888)
        root.measure(
            View.MeasureSpec.makeMeasureSpec(root.width, View.MeasureSpec.EXACTLY),
            View.MeasureSpec.makeMeasureSpec(root.height, View.MeasureSpec.EXACTLY),
        )
        root.layout(0, 0, root.width, root.height)
        root.draw(Canvas(bitmap))
        val file = File(getExternalFilesDir(null), "ui-dump.png")
        file.outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
        Log.i(TAG, "ui dump: ${file.absolutePath} ${root.width}x${root.height}")
        Log.i(
            TAG,
            "current subtitle card: ${currentSubtitleCard.visibility} " +
                "h=${currentSubtitleCard.height} text=${currentSubtitleView.text}",
        )
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handleViewIntent(intent)
    }

    private fun handleViewIntent(intent: Intent?) {
        if (intent?.action != Intent.ACTION_VIEW) return
        intent.data?.let { openVideoUri(it) }
        if (intent.getBooleanExtra(EXTRA_DUMP_UI, false)) {
            handler.postDelayed({ dumpUiToFile() }, 12_000)
        }
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
                    followPlan = true
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
        const val EXTRA_DUMP_UI = "dump_ui"
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
        val segment = if (followPlan) plan?.segmentAt(player.currentPosition) else null
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
            !followPlan -> "1x · 跟随已关"
            segment == null -> "1x"
            segment.isVoice -> "${trimSpeed(segment.speed)}x · 人声"
            else -> "${trimSpeed(segment.speed)}x · 跳过静音"
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
        currentSubtitleCard.visibility = if (index >= 0) View.VISIBLE else View.GONE
        currentSubtitleView.text = subtitles.getOrNull(index)?.text.orEmpty()
        adapter.setActiveIndex(index)
        if (index >= 0) {
            recyclerView.smoothScrollToPosition(index)
        }
    }

    private fun togglePlayback() {
        if (player.isPlaying) {
            player.pause()
        } else {
            if (player.playbackState == Player.STATE_ENDED) player.seekTo(0)
            player.play()
        }
        updateProgressUi()
    }

    private fun updateProgressUi() {
        val label = getString(if (player.isPlaying) R.string.pause else R.string.play)
        if (playToggle.text.toString() != label) playToggle.text = label

        val duration = player.duration.takeIf { it > 0 } ?: 0L
        durationText.text = formatDuration(duration)
        positionText.text = formatDuration(player.currentPosition)
        seekBar.isEnabled = duration > 0
        if (!userSeeking && duration > 0) {
            val position = player.currentPosition.coerceIn(0, duration)
            seekBar.progress = (position * 1000 / duration).toInt()
        }
    }

    private inner class SubtitleAdapter : RecyclerView.Adapter<SubtitleHolder>() {
        private var items: List<Subtitle> = emptyList()
        private var activeIndex = -1

        fun setItems(value: List<Subtitle>) {
            items = value
            activeIndex = -1
            notifyDataSetChanged()
            updateEmptyState()
        }

        fun setActiveIndex(value: Int) {
            if (activeIndex == value) return
            val previous = activeIndex
            activeIndex = value
            if (previous >= 0) notifyItemChanged(previous)
            if (value >= 0) notifyItemChanged(value)
        }

        override fun getItemCount(): Int = items.size

        override fun onCreateViewHolder(parent: ViewGroup, viewType: Int): SubtitleHolder =
            SubtitleHolder(
                LayoutInflater.from(parent.context)
                    .inflate(R.layout.item_subtitle, parent, false),
            )

        override fun onBindViewHolder(holder: SubtitleHolder, position: Int) {
            val subtitle = items[position]
            holder.bind(subtitle, position == activeIndex) {
                Log.i(TAG, "seek via subtitle[$position] -> ${subtitle.startMs}ms")
                player.seekTo(subtitle.startMs)
                player.playWhenReady = true
                applyPlaybackSpeed(force = true)
            }
        }
    }

    private class SubtitleHolder(view: View) : RecyclerView.ViewHolder(view) {
        private val card = view as MaterialCardView
        private val timeView = view.findViewById<TextView>(R.id.row_time)
        private val textView = view.findViewById<TextView>(R.id.row_text)

        fun bind(subtitle: Subtitle, active: Boolean, onClick: () -> Unit) {
            timeView.text = formatTime(subtitle.startMs)
            textView.text = subtitle.text
            val context = itemView.context
            card.setCardBackgroundColor(
                ContextCompat.getColor(
                    context,
                    if (active) R.color.subtitle_active_bg else R.color.subtitle_bg,
                ),
            )
            timeView.setBackgroundResource(
                if (active) R.drawable.bg_time_pill_active else R.drawable.bg_time_pill,
            )
            timeView.setTextColor(
                ContextCompat.getColor(
                    context,
                    if (active) R.color.time_pill_active_text else R.color.subtitle_time,
                ),
            )
            textView.setTextColor(
                ContextCompat.getColor(
                    context,
                    if (active) R.color.subtitle_active_text else R.color.subtitle_text,
                ),
            )
            itemView.setOnClickListener { onClick() }
        }
    }
}
