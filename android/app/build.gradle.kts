plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

// -PmarkiiVersion=0.7.0 (CI passes the version from crates/markii-gpui/Cargo.toml)
val markiiVersion = (project.findProperty("markiiVersion") as String?) ?: "0.7.0"

android {
    namespace = "com.akaineko.markii"
    compileSdk = 35

    defaultConfig {
        applicationId = "com.akaineko.markii"
        // Quest 2/3/3S/Pro and Pico 4 run Android 10+ (API 29)
        minSdk = 29
        targetSdk = 34
        versionName = markiiVersion
        versionCode = markiiVersion.split(".").map { it.toIntOrNull() ?: 0 }.let { (it.getOrElse(0) { 0 } * 10000) + (it.getOrElse(1) { 0 } * 100) + it.getOrElse(2) { 0 } }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            // installed by sideload (adb / SideQuest / Pico developer mode); not a store build
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
