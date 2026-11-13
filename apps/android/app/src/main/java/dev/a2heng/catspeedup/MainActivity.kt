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
import android.widget.TextView
import androidx.activity.result.contract.ActivityResultContracts
import androidx.appcompat.app.AppCompatActivity
import androidx.core.content.ContextCompat
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import androidx.media3.common.MediaItem
import androidx.media3.common.Player
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.ui.PlayerView
import androidx.recyclerview.widget.LinearLayoutManager
import androidx.recyclerview.widget.RecyclerView
import com.google.android.material.button.MaterialButton
import com.google.android.material.card.MaterialCardView
import com.google.android.material.dialog.MaterialAlertDialogBuilder
import com.google.android.material.slider.Slider
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
    private lateinit var analyzeButton: MaterialButton
    private lateinit var timelineBar: TimelineBar
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
    private lateinit var silenceSpeedSlider: Slider
    private lateinit var minGapSlider: Slider
    private lateinit var marginSlider: Slider
    private lateinit var speedValueLabel: TextView
    private lateinit var gapValueLabel: TextView
    private lateinit var marginValueLabel: TextView

    private var subtitles: List<Subtitle> = emptyList()
    private var currentIndex = -1
    private var videoUri: Uri? = null
    private var plan: PlaybackPlan? = null
    private var planOptions = PlanOptions()
    private var cachedDurationMs = 0L
    private var transcriptReady = false
    private var followPlan = true
    private var settingsExpanded = false
    private var userSeeking = false
    private var suppressingSliderCallback = false
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
        transcriptReady = false
        currentIndex = -1
        adapter.setItems(subtitles)
        currentSubtitleView.text = ""
        currentSubtitleCard.visibility = View.GONE
        speedStateView.text = getString(R.string.speed_state_idle)

        cachedDurationMs = AudioDecoder.durationMs(this, uri)
        val cached = TranscriptStore.load(this, uri)
        if (cached != null) {
            cachedDurationMs = maxOf(cachedDurationMs, cached.durationMs)
            subtitles = cached.subtitles
            transcriptReady = true
            currentIndex = -1
            adapter.setItems(subtitles)
            replanFromTranscript()
            statusView.text = "已载入上次字幕 ${subtitles.size} 条（改参数不用重转）"
            Log.i(TAG, "loaded cached transcript: ${subtitles.size} lines")
        } else {
            statusView.text = "已加入视频，点“转写并切分”开始"
        }
        updateAnalyzeButton()
    }

    private fun updateAnalyzeButton() {
        analyzeButton.setText(if (transcriptReady) R.string.reanalyze else R.string.analyze)
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
        analyzeButton = findViewById(R.id.analyze)
        positionText = findViewById(R.id.position_text)
        durationText = findViewById(R.id.duration_text)
        timelineBar = findViewById(R.id.timeline_bar)
        recyclerView = findViewById(R.id.subtitle_list)
        subtitleCountView = findViewById(R.id.subtitle_count)
        subtitleEmptyView = findViewById(R.id.subtitle_empty)
        paramsSummaryView = findViewById(R.id.params_summary)
        settingsPanel = findViewById(R.id.settings_panel)
        playerView = findViewById(R.id.player_view)
        silenceSpeedSlider = findViewById(R.id.silence_speed_slider)
        minGapSlider = findViewById(R.id.min_gap_slider)
        marginSlider = findViewById(R.id.margin_slider)
        speedValueLabel = findViewById(R.id.speed_value_label)
        gapValueLabel = findViewById(R.id.gap_value_label)
        marginValueLabel = findViewById(R.id.margin_value_label)

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
        timelineBar.onSeek = { position ->
            player.seekTo(position)
            applyPlaybackSpeed(force = true)
        }
        timelineBar.onSeekingChanged = { seeking -> userSeeking = seeking }
        player.addListener(
            object : Player.Listener {
                override fun onPlaybackStateChanged(playbackState: Int) = updateProgressUi()
                override fun onIsPlayingChanged(isPlaying: Boolean) = updateProgressUi()
            },
        )

        setupSliders()
        updateParamSummary()
        updateEmptyState()
        updateAnalyzeButton()

        val root = findViewById<View>(R.id.root)
        ViewCompat.setOnApplyWindowInsetsListener(root) { view, insets ->
            val bars = insets.getInsets(WindowInsetsCompat.Type.systemBars())
            view.setPadding(bars.left, bars.top, bars.right, bars.bottom)
            insets
        }
        handler.post(tick)
        modelView.text = offlineAsr.modelStatus()
        handleViewIntent(intent)
    }

    private fun toggleSettings() {
        settingsExpanded = !settingsExpanded
        settingsPanel.visibility = if (settingsExpanded) View.VISIBLE else View.GONE
        updateParamSummary()
    }

    private fun setupSliders() {
        applyOptionsToSliders(AppPrefs.load(this))
        val listener = object : Slider.OnChangeListener {
            override fun onValueChange(slider: Slider, value: Float, fromUser: Boolean) {
                if (suppressingSliderCallback) return
                updateParamSummary()
                AppPrefs.save(this@MainActivity, currentOptions())
                scheduleReplan()
            }
        }
        silenceSpeedSlider.addOnChangeListener(listener)
        minGapSlider.addOnChangeListener(listener)
        marginSlider.addOnChangeListener(listener)
        findViewById<View>(R.id.reset_defaults).setOnClickListener {
            confirmResetDefaults()
        }
    }

    private fun applyOptionsToSliders(options: PlanOptions) {
        suppressingSliderCallback = true
        silenceSpeedSlider.value = options.silenceSpeed.coerceIn(2f, 20f)
        minGapSlider.value = (options.minSpeedupSilenceMs / 1000).coerceIn(1, 30).toFloat()
        marginSlider.value = options.marginMs.coerceIn(0, 400).toFloat()
        suppressingSliderCallback = false
    }

    private fun currentOptions(): PlanOptions = PlanOptions(
        silenceSpeed = silenceSpeedSlider.value,
        minSpeedupSilenceMs = minGapSlider.value.toLong() * 1000,
        marginMs = marginSlider.value.toLong(),
    )

    private fun confirmResetDefaults() {
        MaterialAlertDialogBuilder(this)
            .setTitle(R.string.reset_title)
            .setMessage(R.string.reset_message)
            .setNegativeButton(R.string.cancel, null)
            .setPositiveButton(R.string.reset_positive) { _, _ ->
                applyOptionsToSliders(PlanOptions())
                updateParamSummary()
                AppPrefs.save(this, currentOptions())
                scheduleReplan()
                Log.i(TAG, "parameters reset to defaults")
            }
            .show()
    }

    private val replanRunnable = Runnable { replanFromTranscript() }

    private fun scheduleReplan() {
        handler.removeCallbacks(replanRunnable)
        handler.postDelayed(replanRunnable, 250)
    }

    /** Rebuild the plan from the cached transcript; never decodes or transcribes again. */
    private fun replanFromTranscript() {
        val options = currentOptions()
        planOptions = options
        if (!transcriptReady) {
            updateParamSummary()
            return
        }
        val duration = durationForPlan()
        if (duration <= 0) return
        val result = SilenceDetector.buildPlanFromSpeech(
            duration,
            subtitles.map { it.startMs..it.endMs },
            options,
        )
        plan = result
        followPlan = true
        applyPlaybackSpeed(force = true)
        showPlanSummary(result)
        statusView.text = "参数已更新（没有重新转写）"
        Log.i(TAG, "replan from cached transcript: $options")
    }

    private fun durationForPlan(): Long =
        cachedDurationMs.takeIf { it > 0 }
            ?: player.duration.takeIf { it > 0 }
            ?: plan?.sourceDurationMs
            ?: 0L

    private fun updateParamSummary() {
        val options = currentOptions()
        paramsSummaryView.text =
            "${options.silenceSpeed.toInt()}× · 停顿≥${options.minSpeedupSilenceMs / 1000}秒" +
                " · 留白 ${options.marginMs}ms"
        speedValueLabel.text =
            getString(R.string.speed_value_format, options.silenceSpeed.toInt())
        gapValueLabel.text =
            getString(R.string.gap_value_format, (options.minSpeedupSilenceMs / 1000).toInt())
        marginValueLabel.text =
            getString(R.string.margin_value_format, options.marginMs.toInt())
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
        if (intent.getBooleanExtra(EXTRA_EXPAND_SETTINGS, false)) {
            if (!settingsExpanded) toggleSettings()
        }
        if (intent.getBooleanExtra(EXTRA_AUTO_ANALYZE, false)) {
            val autoExport = intent.getBooleanExtra(EXTRA_AUTO_EXPORT, false)
            Log.i(TAG, "auto_analyze=true auto_export=$autoExport")
            handler.postDelayed(
                {
                    analyze {
                        if (autoExport) handler.postDelayed({ exportVideo() }, 500)
                    }
                },
                500,
            )
        }
    }

    override fun onDestroy() {
        handler.removeCallbacks(tick)
        if (!exporting) {
            offlineAsr.release()
        }
        player.release()
        super.onDestroy()
    }

    /**
     * 切分 = 先拿到一次转写（会缓存），再用 token 时间轴生成计划。
     * 改参数时只重排计划，不再解码、也不再转写。
     */
    private fun analyze(next: (() -> Unit)?) {
        if (analyzing) {
            statusView.text = "正在处理上一组参数，请稍等…"
            return
        }
        val uri = videoUri
        if (uri == null) {
            statusView.text = "先加入一个视频"
            return
        }
        val options = currentOptions()
        if (transcriptReady) {
            // Transcript is already cached: this press only re-splits.
            planOptions = options
            replanFromTranscript()
            next?.invoke()
            return
        }

        analyzing = true
        val resumePlayback = player.playWhenReady
        if (resumePlayback) player.pause()
        val startedAt = System.currentTimeMillis()

        if (!offlineAsr.hasModel()) {
            modelView.text = offlineAsr.modelStatus()
            showProgress("切分中：没有 ASR 模型，用响度兜底…", startedAt)
            Log.w(TAG, "no ASR model, fallback to loudness detection")
            Thread {
                runCatching {
                    val detected = SilenceDetector.detect(
                        context = this,
                        uri = uri,
                        noiseDb = PlanDefaults.FALLBACK_NOISE_DB,
                        minSilenceMs = PlanDefaults.FALLBACK_MIN_SILENCE_MS,
                    )
                    val duration = maxOf(detected.durationMs, resolveDuration(uri))
                    cachedDurationMs = duration
                    SilenceDetector.buildPlan(duration, detected.silence, options)
                }.onSuccess { result ->
                    runOnUiThread {
                        analyzing = false
                        stopProgress()
                        plan = result
                        planOptions = options
                        followPlan = true
                        applyPlaybackSpeed(force = true)
                        showPlanSummary(result)
                        if (resumePlayback) player.playWhenReady = true
                        next?.invoke()
                    }
                }.onFailure { error ->
                    Log.e(TAG, "fallback analyze failed", error)
                    runOnUiThread {
                        analyzing = false
                        stopProgress()
                        statusView.text = "切分失败：${error.message}"
                        if (resumePlayback) player.playWhenReady = true
                    }
                }
            }.start()
            return
        }

        showProgress("切分中：转写一次（之后只调参）…", startedAt)
        Log.i(TAG, "analyze start (ASR-first): $options")
        Thread {
            runCatching {
                val duration = resolveDuration(uri)
                cachedDurationMs = duration
                val transcript = transcribeAll(uri, duration) { done, total ->
                    val seconds = (System.currentTimeMillis() - startedAt) / 1000
                    runOnUiThread {
                        statusView.text = "切分中：转写 $done/$total（${seconds}s）"
                    }
                }
                TranscriptStore.save(this, uri, duration, transcript)
                val result = SilenceDetector.buildPlanFromSpeech(
                    duration,
                    transcript.map { it.startMs..it.endMs },
                    options,
                )
                Pair(transcript, result)
            }.onSuccess { (transcript, result) ->
                Log.i(
                    TAG,
                    "analyze done: lines=${transcript.size} segments=${result.segments.size}",
                )
                runOnUiThread {
                    analyzing = false
                    stopProgress()
                    subtitles = transcript
                    transcriptReady = true
                    currentIndex = -1
                    adapter.setItems(transcript)
                    plan = result
                    planOptions = options
                    followPlan = true
                    applyPlaybackSpeed(force = true)
                    showPlanSummary(result)
                    statusView.text = "转写 ${transcript.size} 条（已缓存，改参数不会重转）"
                    updateAnalyzeButton()
                    if (resumePlayback) player.playWhenReady = true
                    next?.invoke()
                }
            }.onFailure { error ->
                Log.e(TAG, "analyze failed", error)
                runOnUiThread {
                    analyzing = false
                    stopProgress()
                    statusView.text = "切分失败：${error.message}"
                    if (resumePlayback) player.playWhenReady = true
                }
            }
        }.start()
    }

    private companion object {
        const val ASR_CHUNK_MS = 30_000L

        /** adb-only helpers: `am start ... --ez auto_analyze true`. */
        const val EXTRA_AUTO_ANALYZE = "auto_analyze"
        const val EXTRA_AUTO_EXPORT = "auto_export"
        const val EXTRA_DUMP_UI = "dump_ui"
        const val EXTRA_EXPAND_SETTINGS = "expand_settings"
    }

    private fun showPlanSummary(result: PlaybackPlan) {
        statusView.text = buildString {
            append(formatDuration(result.sourceDurationMs))
            append(" → ")
            append(formatDuration(result.outputDurationMs()))
            append(" · 人声 ")
            append(result.voiceSegmentCount())
            append(" 段 · ")
            append(trimSpeed(planOptions.silenceSpeed))
            append("×")
        }
    }

    private fun trimSpeed(speed: Float): String =
        if (speed == speed.toInt().toFloat()) speed.toInt().toString() else speed.toString()

    private fun resolveDuration(uri: Uri): Long = maxOf(
        cachedDurationMs,
        AudioDecoder.durationMs(this, uri),
    )

    /** Decode the whole audio once and transcribe 30 s chunks on the fly. */
    private fun transcribeAll(
        uri: Uri,
        durationMs: Long,
        onProgress: (done: Int, total: Int) -> Unit,
    ): List<Subtitle> {
        val ranges = ArrayList<LongRange>()
        var start = 0L
        while (start < durationMs) {
            val end = minOf(durationMs, start + ASR_CHUNK_MS)
            if (end > start) ranges += start..end
            start = end
        }
        val all = ArrayList<Subtitle>()
        var done = 0
        AudioDecoder.decodeRangesTo16kMono(this, uri, ranges) { index, samples ->
            if (samples.isNotEmpty() && isAudible(samples)) {
                val range = ranges[index]
                val result = offlineAsr.transcribe(samples)
                val added = buildSubtitles(result, range.first, range.last - range.first)
                all += added
                Log.d(
                    TAG,
                    "asr chunk[$index] ${range.first}-${range.last}ms " +
                        "samples=${samples.size} -> ${added.size} lines",
                )
            } else {
                Log.d(TAG, "asr chunk[$index] skipped (quiet)")
            }
            done++
            onProgress(done, ranges.size)
        }
        return all
    }

    /** Skip obviously silent 30 s chunks; speech detection itself comes from ASR tokens. */
    private fun isAudible(samples: FloatArray): Boolean {
        var sum = 0.0
        for (value in samples) sum += value * value
        return sum / samples.size > 1e-5
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
            !followPlan -> "1× · 跟随已关"
            segment == null -> "1×"
            segment.isVoice -> "${trimSpeed(segment.speed)}× · 人声"
            else -> "${trimSpeed(segment.speed)}× · 跳过静音"
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
        timelineBar.isEnabled = duration > 0
        timelineBar.durationMs = duration
        if (timelineBar.plan !== plan) timelineBar.plan = plan
        if (!userSeeking && duration > 0) {
            timelineBar.positionMs = player.currentPosition.coerceIn(0, duration)
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
