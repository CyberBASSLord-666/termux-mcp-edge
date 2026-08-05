import org.gradle.api.JavaVersion
import org.gradle.api.artifacts.dsl.LockMode

plugins {
    alias(libs.plugins.android.application) apply false
    alias(libs.plugins.android.library) apply false
}

require(JavaVersion.current() == JavaVersion.VERSION_17) {
    "The Stage-2 Android bridge build requires JDK 17 exactly"
}

allprojects {
    dependencyLocking {
        lockAllConfigurations()
        lockMode.set(LockMode.STRICT)
    }
}

tasks.register("stage2Check") {
    group = "verification"
    description = "Builds and verifies the inert Stage-2 Android bridge skeleton."
    dependsOn(
        ":bridge-contract:testDebugUnitTest",
        ":bridge-contract:lintDebug",
        ":bridge-contract:lintRelease",
        ":bridge-app:testDebugUnitTest",
        ":bridge-app:lintDebug",
        ":bridge-app:lintRelease",
        ":bridge-app:assembleDebug",
        ":bridge-app:assembleRelease",
        ":bridge-app:assembleDebugAndroidTest",
    )
}
