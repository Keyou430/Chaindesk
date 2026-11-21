plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.compose.compiler)
}

val probeAbi = providers.gradleProperty("probeAbi").orElse("arm64-v8a").get()
require(probeAbi in setOf("arm64-v8a", "x86_64")) { "Unsupported A0 probe ABI" }
val developmentVersionCode = providers.gradleProperty("developmentVersionCode").orElse("4").get().toInt()
require(developmentVersionCode >= 4) { "Development versionCode must not precede version 4" }

android {
    namespace = "dev.chaindesk.android"
    compileSdk { version = release(37) }
    buildToolsVersion = "36.0.0"
    ndkVersion = "28.2.13676358"

    defaultConfig {
        applicationId = "dev.chaindesk.android"
        minSdk = 26
        targetSdk = 37
        versionCode = developmentVersionCode
        versionName = "0.1.0-dev.1"
        testInstrumentationRunner = "dev.chaindesk.android.A0Instrumentation"
        ndk { abiFilters += probeAbi }
    }

    buildTypes {
        debug { applicationIdSuffix = ".debug" }
    }

    buildFeatures { compose = true }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

dependencies {
    implementation(project(":coreBridge"))
    implementation(platform(libs.compose.bom))
    implementation(libs.compose.ui)
    implementation(libs.compose.foundation)
    implementation(libs.compose.material3)
    implementation(libs.activity.compose)
    implementation(libs.coroutines.android)
    testImplementation("junit:junit:4.13.2")
}
