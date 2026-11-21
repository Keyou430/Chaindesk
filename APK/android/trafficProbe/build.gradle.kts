plugins {
    id("com.android.application")
}

android {
    namespace = "dev.chaindesk.trafficprobe"
    compileSdk { version = release(37) }
    buildToolsVersion = "36.0.0"

    defaultConfig {
        applicationId = "dev.chaindesk.trafficprobe"
        minSdk = 26
        targetSdk = 37
        versionCode = 1
        versionName = "0.1.0-lab"
    }

    buildTypes {
        debug { applicationIdSuffix = ".debug" }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}
