// Vendored from sherpa-onnx v2.0.1 kotlin-api (Apache-2.0).
// https://github.com/k2-fsa/sherpa-onnx

package com.k2fsa.sherpa.onnx

class VersionInfo {
    companion object {
        init {
            System.loadLibrary("sherpa-onnx-jni")
        }

        val version: String
            get() = getVersionStr2()

        val gitSha1: String
            get() = getGitSha12()

        val gitDate: String
            get() = getGitDate2()

        val onnxruntimeVersion: String
            get() = getOnnxruntimeVersionStr2()

        external fun getVersionStr2(): String
        external fun getGitSha12(): String
        external fun getGitDate2(): String
        external fun getOnnxruntimeVersionStr2(): String
    }
}
