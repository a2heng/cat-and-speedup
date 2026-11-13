plugins {
    id("com.android.application")
}

android {
    namespace = "dev.a2heng.catspeedup"
    compileSdk = 37

    defaultConfig {
        applicationId = "dev.a2heng.catspeedup"
        minSdk = 26
        targetSdk = 37
        versionCode = 5
        versionName = "0.5.0"

        ndk {
            // sherpa-onnx ships prebuilt JNI libs; the model runs on 64-bit ARM only.
            abiFilters += "arm64-v8a"
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    kotlin {
        compilerOptions {
            jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17)
        }
    }

    buildFeatures {
        buildConfig = false
    }

    buildTypes {
        release {
            // Personal tool: sign releases with the local debug key so the
            // release build (non-debuggable, optimized) can be installed directly.
            signingConfig = signingConfigs.getByName("debug")
            isMinifyEnabled = false
            isShrinkResources = false
        }
    }

    androidResources {
        noCompress += listOf("onnx")
    }

    testOptions {
        unitTests.isReturnDefaultValues = true
    }
}

dependencies {
    implementation("androidx.appcompat:appcompat:1.8.0")
    implementation("com.google.android.material:material:1.14.0")
    implementation("androidx.recyclerview:recyclerview:1.4.0")
    implementation("androidx.media3:media3-exoplayer:1.11.1")
    implementation("androidx.media3:media3-ui:1.11.1")
    implementation("androidx.media3:media3-transformer:1.11.1")

    testImplementation("junit:junit:4.13.2")
}
