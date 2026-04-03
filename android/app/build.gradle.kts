plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.android)
}

android {
    namespace = "com.example.barcodescanner"
    compileSdk = 35

    defaultConfig {
        applicationId = "com.example.barcodescanner"
        minSdk = 26
        targetSdk = 34
        versionCode = 1
        versionName = "0.1.0"

        ndk {
            // Build the Rust cdylib for these ABIs.
            abiFilters += listOf("arm64-v8a")
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            // Use the debug signing key so the release APK can be installed
            // directly on device without setting up a keystore.
            signingConfig = signingConfigs.getByName("debug")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    kotlinOptions {
        jvmTarget = "17"
    }
}

dependencies {
    // KMP shared module (shop detection, card extraction, state enum)
    implementation(project(":shared"))

    implementation(libs.core.ktx)
    implementation(libs.appcompat)
    implementation(libs.activity.ktx)
    implementation(libs.lifecycle.runtime)
    implementation(libs.coroutines.android)
    implementation(libs.camerax.core)
    implementation(libs.camerax.camera2)
    implementation(libs.camerax.lifecycle)
    implementation(libs.camerax.view)
    implementation(libs.zxing.embedded)
}
