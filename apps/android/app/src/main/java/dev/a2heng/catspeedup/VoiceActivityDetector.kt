package dev.a2heng.catspeedup

import android.content.Context
import android.util.Log
import com.k2fsa.sherpa.onnx.SileroVadModelConfig
import com.k2fsa.sherpa.onnx.Vad
import com.k2fsa.sherpa.onnx.VadModelConfig

/**
 * Silero VAD from sherpa-onnx. Used to get exact speech boundaries; ASR then
 * only runs on these ranges instead of the whole soundtrack.
 *
 * The model is bundled in the APK at `assets/models/silero_vad.onnx`. A
 * side-loaded copy at `<files>/models/silero_vad.onnx` still wins, which is
 * handy for quick experiments without rebuilding.
 */
class VoiceActivityDetector(private val context: Context) {
    private val assetModel = "models/silero_vad.onnx"

    fun hasModel(): Boolean = sideLoadedFile() != null || bundled()

    fun modelStatus(): String = when {
        sideLoadedFile() != null -> sideLoadedFile()!!.absolutePath
        bundled() -> "assets/$assetModel"
        else -> "未找到 VAD 模型（assets 或 files/models/silero_vad.onnx）"
    }

    fun create(): Vad {
        val model = sideLoadedFile()
        val fromAssets = model == null
        if (fromAssets && !bundled()) error("未找到 VAD 模型")
        Log.i(TAG, "loading Silero VAD: ${if (fromAssets) "assets/$assetModel" else model!!.absolutePath}")
        val config = VadModelConfig(
            sileroVadModelConfig = SileroVadModelConfig(
                model = if (fromAssets) assetModel else model!!.absolutePath,
                threshold = 0.5f,
                minSilenceDuration = 0.5f,
                minSpeechDuration = 0.2f,
                windowSize = 512,
                maxSpeechDuration = 30f,
            ),
            sampleRate = 16_000,
            numThreads = 1,
            provider = "cpu",
        )
        return if (fromAssets) Vad(assetManager = context.assets, config = config)
        else Vad(config = config)
    }

    private fun bundled(): Boolean = runCatching {
        context.assets.list("models").orEmpty().contains("silero_vad.onnx")
    }.getOrDefault(false)

    private fun sideLoadedFile(): java.io.File? {
        val roots = listOfNotNull(context.getExternalFilesDir(null), context.filesDir)
        return roots.map { java.io.File(it, assetModel) }.firstOrNull { it.isFile }
    }
}
