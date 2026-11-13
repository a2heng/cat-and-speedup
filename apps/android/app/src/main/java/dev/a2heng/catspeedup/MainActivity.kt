package dev.a2heng.catspeedup

import android.content.Intent
import android.graphics.Color
import android.net.Uri
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.view.View
import android.view.ViewGroup
import android.widget.BaseAdapter
import android.widget.Button
import android.widget.ListView
import android.widget.TextView
import androidx.activity.result.contract.ActivityResultContracts
import androidx.appcompat.app.AppCompatActivity
import androidx.media3.common.MediaItem
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.ui.PlayerView
import org.json.JSONObject

class MainActivity : AppCompatActivity() {
    private lateinit var player: ExoPlayer
    private lateinit var playerView: PlayerView
    private lateinit var statusView: TextView
    private lateinit var currentSubtitleView: TextView
    private lateinit var listView: ListView
    private lateinit var adapter: SubtitleAdapter

    private var subtitles: List<Subtitle> = emptyList()
    private var currentIndex = -1

    private val handler = Handler(Looper.getMainLooper())
    private val tick = object : Runnable {
        override fun run() {
            updateCurrentSubtitle()
            handler.postDelayed(this, 120)
        }
    }

    private val openVideo = registerForActivityResult(
        ActivityResultContracts.OpenDocument(),
    ) { uri: Uri? ->
        uri ?: return@registerForActivityResult
        runCatching {
            contentResolver.takePersistableUriPermission(
                uri,
                Intent.FLAG_GRANT_READ_URI_PERMISSION,
            )
        }
        player.setMediaItem(MediaItem.fromUri(uri))
        player.prepare()
        player.playWhenReady = true
        statusView.text = "已打开视频"
    }

    private val openSubtitles = registerForActivityResult(
        ActivityResultContracts.OpenDocument(),
    ) { uri: Uri? ->
        uri ?: return@registerForActivityResult
        runCatching {
            contentResolver.openInputStream(uri)?.bufferedReader()?.use { reader ->
                subtitles = parseTranscript(reader.readText())
            } ?: error("无法读取文件")
            currentIndex = -1
            adapter.setItems(subtitles)
            statusView.text = "已导入 ${subtitles.size} 条字幕"
        }.onFailure {
            statusView.text = "字幕导入失败：${it.message}"
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_main)

        statusView = findViewById(R.id.status)
        currentSubtitleView = findViewById(R.id.current_subtitle)
        listView = findViewById(R.id.subtitle_list)
        playerView = findViewById(R.id.player_view)

        player = ExoPlayer.Builder(this).build()
        playerView.player = player

        adapter = SubtitleAdapter()
        listView.adapter = adapter
        listView.setOnItemClickListener { _, _, position, _ ->
            val subtitle = subtitles.getOrNull(position) ?: return@setOnItemClickListener
            player.seekTo(subtitle.startMs)
            player.playWhenReady = true
        }

        findViewById<Button>(R.id.open_video).setOnClickListener {
            openVideo.launch(arrayOf("video/*"))
        }
        findViewById<Button>(R.id.open_subtitles).setOnClickListener {
            openSubtitles.launch(arrayOf("application/json", "text/plain"))
        }

        handler.post(tick)
    }

    override fun onDestroy() {
        handler.removeCallbacks(tick)
        player.release()
        super.onDestroy()
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
            val view = (convertView as? TextView) ?: TextView(parent.context).apply {
                setPadding(16, 12, 16, 12)
                textSize = 14f
            }
            val subtitle = items[position]
            view.text = "[${formatTime(subtitle.startMs)}] ${subtitle.text}"
            if (position == activeIndex) {
                view.setBackgroundColor(Color.parseColor("#E3F2FD"))
                view.setTextColor(Color.parseColor("#1565C0"))
            } else {
                view.setBackgroundColor(Color.TRANSPARENT)
                view.setTextColor(Color.parseColor("#212121"))
            }
            return view
        }
    }
}

private data class Subtitle(
    val startMs: Long,
    val endMs: Long,
    val text: String,
)

private fun parseTranscript(json: String): List<Subtitle> {
    val root = JSONObject(json)
    val array = root.getJSONArray("segments")
    val result = ArrayList<Subtitle>(array.length())
    for (index in 0 until array.length()) {
        val item = array.getJSONObject(index)
        result += Subtitle(
            startMs = item.getLong("start_ms"),
            endMs = item.getLong("end_ms"),
            text = item.getString("text"),
        )
    }
    return result
}

private fun formatTime(ms: Long): String {
    val totalSeconds = ms / 1000
    val minutes = totalSeconds / 60
    val seconds = totalSeconds % 60
    val millis = ms % 1000
    return "%02d:%02d.%03d".format(minutes, seconds, millis)
}
