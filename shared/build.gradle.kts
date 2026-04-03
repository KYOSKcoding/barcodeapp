import org.jetbrains.kotlin.gradle.plugin.mpp.apple.XCFramework

plugins {
    kotlin("multiplatform") version "2.1.0"
    id("com.android.library") version "8.7.3"
}

kotlin {
    // Android target — used when the Android app pulls in the shared module as a project dep
    androidTarget {
        compilations.all {
            kotlinOptions {
                jvmTarget = "17"
            }
        }
    }

    // iOS targets — all three slices contribute to the XCFramework
    val xcf = XCFramework("shared")
    val iosTargets = listOf(iosArm64(), iosSimulatorArm64(), iosX64())
    iosTargets.forEach { target ->
        target.binaries.framework {
            baseName = "shared"
            xcf.add(this)
        }
    }

    sourceSets {
        commonMain {
            // Pure Kotlin business logic; no platform APIs here.
            // Source files live in src/commonMain/kotlin/
        }
    }
}

android {
    namespace = "com.example.barcodescanner.shared"
    compileSdk = 35
    defaultConfig {
        minSdk = 26
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}
