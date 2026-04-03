pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}

dependencyResolutionManagement {
    repositories {
        google()
        mavenCentral()
    }
}

rootProject.name = "barcode-scanner"
include(":app")

// KMP shared module (business logic shared between Android and iOS)
include(":shared")
project(":shared").projectDir = file("../shared")
