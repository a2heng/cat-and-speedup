package dev.a2heng.catspeedup

import android.content.Context
import android.util.Log
import com.k2fsa.sherpa.onnx.FeatureConfig
import com.k2fsa.sherpa.onnx.OfflineFunAsrNanoModelConfig
import com.k2fsa.sherpa.onnx.OfflineModelConfig
import com.k2fsa.sherpa.onnx.OfflineParaformerModelConfig
import com.k2fsa.sherpa.onnx.OfflineRecognizer
import com.k2fsa.sherpa.onnx.OfflineRecognizerConfig
import com.k2fsa.sherpa.onnx.OfflineRecognizerResult
import com.k2fsa.sherpa.onnx.OfflineTransducerModelConfig
import java.io.File

/**
 * Local, offline ASR via sherpa-onnx. Nothing is sent over the network.
 *
 * The phone uses the light X-ASR Zipformer-zh-en int8 model (~168 MB, token
 * timestamps); the desktop keeps FunASR-Nano for quality. Models are
 * side-loaded, never bundled:
 *
 *   1. `<external files dir>/models/zipformer-zh-en` (adb push / file copy)
 *   2. `<internal files dir>/models/zipformer-zh-en`
 *   3. assets `models/zipformer-zh-en` (only when someone bundles it manually)
 *
 * Paraformer-zh (`models/paraformer-zh`, no timestamps in sherpa) and
 * FunASR-Nano (`models/funasr-nano`) are still recognised as fallbacks.
 */
class OfflineAsr(private val context: Context) {
    class ModelMissing(message: String) : IllegalStateException(message)

    private enum class Kind(val label: String, val subdir: String) {
        XASR("X-ASR Zipformer zh-en int8", "models/zipformer-zh-en"),
        PARAFORMER("Paraformer-zh int8", "models/paraformer-zh"),
        FUNASR("FunASR-Nano int8", "models/funasr-nano"),
    }

    private data class ModelLocation(
        val kind: Kind,
        val dir: String,
        val fromAssets: Boolean,
    )

    private var recognizer: OfflineRecognizer? = null

    /** Human readable description of where the model was found, or how to install it. */
    fun modelStatus(): String {
        val location = locate() ?: return "未找到 ASR 模型\n请放入：${expectedDir().absolutePath}"
        val source = if (location.fromAssets) "APK 内置 assets" else location.dir
        return "模型：${location.kind.label}（$source）"
    }

    fun hasModel(): Boolean = locate() != null

    fun transcribe(samples: FloatArray): OfflineRecognizerResult {
        if (samples.isEmpty()) throw IllegalArgumentException("没有解码出音频")
        val engine = recognizer ?: createRecognizer().also { recognizer = it }
        val startedAt = System.currentTimeMillis()
        val stream = engine.createStream()
        return try {
            stream.acceptWaveform(samples, sampleRate = 16_000)
            engine.decode(stream)
            engine.getResult(stream).also { result ->
                Log.d(
                    TAG,
                    "asr: ${samples.size / 16.0f / 1000f}s audio -> " +
                        "${result.tokens.size} tokens, ${result.timestamps.size} stamps, " +
                        "${result.text.length} chars in ${System.currentTimeMillis() - startedAt}ms",
                )
            }
        } finally {
            stream.release()
        }
    }

    fun release() {
        recognizer?.release()
        recognizer = null
    }

    fun expectedDir(): File =
        File(context.getExternalFilesDir(null) ?: context.filesDir, Kind.XASR.subdir)

    private fun createRecognizer(): OfflineRecognizer {
        val location = locate()
            ?: throw ModelMissing("未找到 ASR 模型，请放入 ${expectedDir().absolutePath}")
        Log.i(TAG, "loading ${location.kind.label}: dir=${location.dir} assets=${location.fromAssets}")
        val startedAt = System.currentTimeMillis()
        val modelConfig = when (location.kind) {
            Kind.XASR -> OfflineModelConfig(
                transducer = OfflineTransducerModelConfig(
                    encoder = "${location.dir}/encoder-epoch-99-avg-1.int8.onnx",
                    decoder = "${location.dir}/decoder-epoch-99-avg-1.onnx",
                    joiner = "${location.dir}/joiner-epoch-99-avg-1.int8.onnx",
                ),
                tokens = "${location.dir}/tokens.txt",
                modelType = "transducer",
                numThreads = DEFAULT_THREADS,
                provider = "cpu",
            )
            Kind.PARAFORMER -> OfflineModelConfig(
                paraformer = OfflineParaformerModelConfig(
                    model = "${location.dir}/model.int8.onnx",
                ),
                tokens = "${location.dir}/tokens.txt",
                modelType = "paraformer",
                numThreads = DEFAULT_THREADS,
                provider = "cpu",
            )
            Kind.FUNASR -> OfflineModelConfig(
                funasrNano = OfflineFunAsrNanoModelConfig(
                    encoderAdaptor = "${location.dir}/encoder_adaptor.int8.onnx",
                    llm = "${location.dir}/llm.int8.onnx",
                    embedding = "${location.dir}/embedding.int8.onnx",
                    tokenizer = "${location.dir}/${TOKENIZER_DIR}",
                    language = "zh",
                    maxNewTokens = 512,
                    itn = true,
                ),
                numThreads = DEFAULT_THREADS,
                provider = "cpu",
            )
        }
        val config = OfflineRecognizerConfig(
            featConfig = FeatureConfig(sampleRate = 16_000, featureDim = 80),
            modelConfig = modelConfig,
        )
        val created = if (location.fromAssets) {
            OfflineRecognizer(assetManager = context.assets, config = config)
        } else {
            OfflineRecognizer(config = config)
        }
        Log.i(TAG, "${location.kind.label} ready in ${System.currentTimeMillis() - startedAt}ms")
        return created
    }

    private fun locate(): ModelLocation? {
        val roots = listOfNotNull(context.getExternalFilesDir(null), context.filesDir)
        for (kind in Kind.entries) {
            for (root in roots) {
                val dir = File(root, kind.subdir)
                if (dir.isCompleteModel(kind)) {
                    return ModelLocation(kind, dir.absolutePath, fromAssets = false)
                }
            }
        }
        for (kind in Kind.entries) {
            val bundled = runCatching {
                context.assets.list(kind.subdir).orEmpty().isNotEmpty()
            }.getOrDefault(false)
            if (bundled) {
                return ModelLocation(kind, kind.subdir, fromAssets = true)
            }
        }
        return null
    }

    private fun File.isCompleteModel(kind: Kind): Boolean = when (kind) {
        Kind.XASR ->
            File(this, "encoder-epoch-99-avg-1.int8.onnx").isFile &&
                File(this, "decoder-epoch-99-avg-1.onnx").isFile &&
                File(this, "joiner-epoch-99-avg-1.int8.onnx").isFile &&
                File(this, "tokens.txt").isFile
        Kind.PARAFORMER ->
            File(this, "model.int8.onnx").isFile && File(this, "tokens.txt").isFile
        Kind.FUNASR ->
            File(this, "encoder_adaptor.int8.onnx").isFile &&
                File(this, "llm.int8.onnx").isFile &&
                File(this, "embedding.int8.onnx").isFile &&
                File(this, TOKENIZER_DIR).isDirectory
    }

    private companion object {
        const val TOKENIZER_DIR = "Qwen3-0.6B"
        val DEFAULT_THREADS = Runtime.getRuntime().availableProcessors().coerceIn(2, 4)
    }
}
