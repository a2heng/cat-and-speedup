// Vendored from sherpa-onnx v2.0.1 kotlin-api (Apache-2.0).
// https://github.com/k2-fsa/sherpa-onnx

package com.k2fsa.sherpa.onnx

data class QnnConfig(
    var backendLib: String = "",
    var contextBinary: String = "",
    var systemLib: String = "",
)
