package dev.a2heng.catspeedup

import android.content.Context
import android.util.Log
import com.k2fsa.sherpa.onnx.SileroVadModelConfig
import com.k2fsa.sherpa.onnx.Vad
import com.k2fsa.sherpa.onnx.VadModelConfig
import java.io.File

/**
 * Silero VAD from sherpa-onnx. Used to get exact speech boundaries; ASR then
 * only runs on these ranges instead of the whole soundtrack.
 *
 * Model is side-loaded at `<files>/models/silero_vad.onnx`.
 */
class VoiceActivityDetector(private val context: Context) {
    fun hasModel(): Boolean = modelFile() != null

    fun modelStatus(): String =
        modelFile()?.absolutePath ?: "未找到 VAD 模型（models/silero_vad.onnx）"

    fun create(): Vad {
        val model = modelFile() ?: error("未找到 VAD 模型")
        Log.i(TAG, "loading Silero VAD: ${model.absolutePath}")
        return Vad(
            config = VadModelConfig(
                sileroVadModelConfig = SileroVadModelConfig(
                    model = model.absolutePath,
                    threshold = 0.5f,
                    minSilenceDuration = 0.5f,
                    minSpeechDuration = 0.2f,
                    windowSize = 512,
                    maxSpeechDuration = 30f,
                ),
                sampleRate = 16_000,
                numThreads = 1,
                provider = "cpu",
            ),
        )
    }

    private fun modelFile(): File? {
        val roots = listOfNotNull(context.getExternalFilesDir(null), context.filesDir)
        return roots.map { File(it, "models/silero_vad.onnx") }.firstOrNull { it.isFile }
    }
}
