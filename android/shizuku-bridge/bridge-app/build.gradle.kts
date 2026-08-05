plugins {
    alias(libs.plugins.android.application)
}

android {
    namespace = "io.github.cyberbasslord666.termuxmcpedge.bridge"
    compileSdk = 36
    buildToolsVersion = "35.0.0"

    defaultConfig {
        applicationId = "io.github.cyberbasslord666.termuxmcpedge.bridge"
        minSdk = 30
        targetSdk = 36
        versionCode = 1
        versionName = "0.1.0-stage2-inert"
        testInstrumentationRunner =
            "io.github.cyberbasslord666.termuxmcpedge.bridge.BridgeStage2Instrumentation"
    }

    buildFeatures {
        aidl = false
        buildConfig = false
    }

    buildTypes {
        debug {
            isDebuggable = false
            isJniDebuggable = false
            isMinifyEnabled = false
            isPseudoLocalesEnabled = false
        }
        release {
            isDebuggable = false
            isJniDebuggable = false
            isMinifyEnabled = true
            isShrinkResources = true
            isPseudoLocalesEnabled = false
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro",
            )
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_11
        targetCompatibility = JavaVersion.VERSION_11
    }

    dependenciesInfo {
        includeInApk = false
        includeInBundle = false
    }

    packaging {
        jniLibs.useLegacyPackaging = false
        resources.excludes += setOf(
            "META-INF/DEPENDENCIES",
            "META-INF/LICENSE*",
            "META-INF/NOTICE*",
        )
    }

    testOptions {
        unitTests.isReturnDefaultValues = false
    }

    lint {
        abortOnError = true
        checkDependencies = true
        checkReleaseBuilds = true
        // Both Stage-2 target variants are intentionally nondebuggable. Runtime/APK contracts
        // independently assert the flag stays absent, so this advisory cannot mask a regression.
        disable += "HardcodedDebugMode"
        warningsAsErrors = true
    }
}

dependencies {
    implementation(project(":bridge-contract"))
    testImplementation(libs.junit)
}
