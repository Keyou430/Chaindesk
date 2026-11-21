plugins {
    alias(libs.plugins.android.library)
}

val probeAbi = providers.gradleProperty("probeAbi").orElse("arm64-v8a").get()
require(probeAbi in setOf("arm64-v8a", "x86_64")) { "Unsupported A0 probe ABI" }
val nativeOutput = layout.buildDirectory.dir("generated/jniLibs/$probeAbi")
val buildGoCore by tasks.registering(Exec::class) {
    val apkRoot = rootProject.projectDir.parentFile
    inputs.dir(apkRoot.resolve("core"))
    inputs.property("probeAbi", probeAbi)
    inputs.file(apkRoot.resolve("toolchain.json"))
    inputs.file(apkRoot.resolve("scripts/buildCore.ps1"))
    inputs.file(apkRoot.resolve("scripts/preparePatchedCore.ps1"))
    outputs.files(nativeOutput.map { it.file("libchaindesk_core.so") }, nativeOutput.map { it.file("libchaindesk_core.h") })
    commandLine("pwsh", "-NoProfile", "-File", apkRoot.resolve("scripts/buildCore.ps1"), "-Abi", probeAbi)
}

android {
    namespace = "dev.chaindesk.core"
    compileSdk { version = release(37) }
    buildToolsVersion = "36.0.0"
    ndkVersion = "28.2.13676358"

    defaultConfig {
        minSdk = 26
        ndk { abiFilters += probeAbi }
    }

    externalNativeBuild {
        cmake {
            path = file("src/main/cpp/CMakeLists.txt")
            version = "3.31.6"
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

tasks.configureEach {
    if (name.startsWith("configureCMake") || name.startsWith("buildCMake")) {
        dependsOn(buildGoCore)
    }
}
