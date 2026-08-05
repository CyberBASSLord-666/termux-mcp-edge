plugins {
    alias(libs.plugins.android.library)
}

android {
    namespace = "io.github.cyberbasslord666.termuxmcpedge.bridge.contract"
    compileSdk = 36
    buildToolsVersion = "35.0.0"

    defaultConfig {
        minSdk = 30
        consumerProguardFiles("consumer-rules.pro")
    }

    buildFeatures {
        aidl = true
        buildConfig = false
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_11
        targetCompatibility = JavaVersion.VERSION_11
    }

    testOptions {
        unitTests.isReturnDefaultValues = false
    }

    lint {
        abortOnError = true
        checkDependencies = true
        checkReleaseBuilds = true
        // Toolchain versions and bytes are independently checksum-locked. Online update
        // availability is temporal advice and dependency updates remain separate changes.
        disable += "AndroidGradlePluginVersion"
        warningsAsErrors = true
    }
}

dependencies {
    testImplementation(libs.junit)
}
