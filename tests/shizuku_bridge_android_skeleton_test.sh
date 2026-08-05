#!/usr/bin/env bash
set -Eeuo pipefail
IFS=$'\n\t'

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
cd "$REPO_ROOT"

fail() {
  printf 'Shizuku bridge Android skeleton contract failed: %s\n' "$1" >&2
  exit 1
}

validate_tree() {
  local root="$1"
  python3 - "$root" <<'PY'
import hashlib
import pathlib
import re
import stat
import sys
import xml.etree.ElementTree as ET

root = pathlib.Path(sys.argv[1]).resolve()
android = root / "android/shizuku-bridge"


def fail(message: str) -> None:
    raise SystemExit(message)


def text(relative: str) -> str:
    path = root / relative
    if not path.is_file():
        fail(f"missing {relative}")
    try:
        return path.read_text(encoding="utf-8")
    except UnicodeDecodeError as error:
        fail(f"non-UTF-8 contract input {relative}: {error}")


def require(condition: bool, message: str) -> None:
    if not condition:
        fail(message)


def require_fragment(haystack: str, needle: str, message: str) -> None:
    require(needle in haystack, message)


def local_text(relative: str) -> str:
    return text(f"android/shizuku-bridge/{relative}")


wrapper_jar = android / "gradle/wrapper/gradle-wrapper.jar"
require(wrapper_jar.is_file(), "Gradle wrapper JAR missing")
require(
    hashlib.sha256(wrapper_jar.read_bytes()).hexdigest()
    == "81a82aaea5abcc8ff68b3dfcb58b3c3c429378efd98e7433460610fecd7ae45f",
    "Gradle wrapper JAR digest changed",
)
gradlew = android / "gradlew"
require(gradlew.is_file(), "Gradle POSIX wrapper missing")
require(gradlew.stat().st_mode & stat.S_IXUSR != 0, "Gradle POSIX wrapper is not executable")
require(
    hashlib.sha256(gradlew.read_bytes()).hexdigest()
    == "2f1c4d0d61790cfa72630eb34a30eca29eda4fab0069d0e2716f4f4f5869e21c",
    "Gradle POSIX wrapper digest changed",
)

wrapper = local_text("gradle/wrapper/gradle-wrapper.properties")
expected_wrapper_properties = {
    "distributionBase": "GRADLE_USER_HOME",
    "distributionPath": "wrapper/dists",
    "distributionSha256Sum":
        "20f1b1176237254a6fc204d8434196fa11a4cfb387567519c61556e8710aed78",
    "distributionUrl": r"https\://services.gradle.org/distributions/gradle-8.13-bin.zip",
    "networkTimeout": "10000",
    "validateDistributionUrl": "true",
    "zipStoreBase": "GRADLE_USER_HOME",
    "zipStorePath": "wrapper/dists",
}
actual_wrapper_properties = {}
for line_number, raw_line in enumerate(wrapper.splitlines(), start=1):
    line = raw_line.strip()
    if not line or line.startswith(("#", "!")):
        continue
    separators = [index for index in (line.find("="), line.find(":")) if index >= 0]
    require(separators, f"invalid wrapper property at line {line_number}")
    separator = min(separators)
    key = line[:separator].strip()
    value = line[separator + 1:].strip()
    require(key not in actual_wrapper_properties, f"duplicate wrapper property: {key}")
    actual_wrapper_properties[key] = value
require(actual_wrapper_properties == expected_wrapper_properties, "Gradle wrapper properties changed")

expected_android_hashes = {
    "bridge-app/build.gradle.kts": "23e3596eeb4106ed57957dcb2401973f0d67b2b3d94f974917a59d808a1ef468",
    "bridge-app/gradle.lockfile": "71e38e1751aab7388f3e605d9a647c28f67be6d7ccd7ff9f933b662c76e072d2",
    "bridge-app/proguard-rules.pro": "3d1ec1661309bf212e4528d4403b31b41a87c7b42f5f7e09a483c7c1c1b8236d",
    "bridge-app/src/androidTest/java/io/github/cyberbasslord666/termuxmcpedge/bridge/BridgeManifestInstrumentationTest.java":
        "2abe988883c7832c66972e4e0bf39c5e63e6f5591a9e52e3b0d7b9017182695e",
    "bridge-app/src/androidTest/java/io/github/cyberbasslord666/termuxmcpedge/bridge/BridgeParcelInstrumentationTest.java":
        "29d3c74f229085514b1bba2099a7bb00875f7530686ccb01d6adc0314ec9f339",
    "bridge-app/src/androidTest/java/io/github/cyberbasslord666/termuxmcpedge/bridge/BridgeStage2Instrumentation.java":
        "6acd5e1649672e747923bb8efdabddceb101e394395e612f5b5856099a58b8bb",
    "bridge-app/src/androidTest/java/io/github/cyberbasslord666/termuxmcpedge/bridge/BridgeTestAssertions.java":
        "03fffb21e47e3c12a2ef9bb06979e5dea9efba0ceda471a1db3e607c8e58f4bb",
    "bridge-app/src/main/AndroidManifest.xml": "26039d157792851b1db77869e00baa35aebbbcfbe0fabbd87a3609316427316c",
    "bridge-app/src/main/java/io/github/cyberbasslord666/termuxmcpedge/bridge/BridgeSkeleton.java":
        "17e84e7617bb69f3e29576863d56d1ab77301cdb2f99f7bf3f9b491f6b1425ca",
    "bridge-app/src/main/res/drawable/bridge_inert_icon.xml":
        "4077a7e5b93b515bc526fbda215c2ac12fe27c3cb502e8982ef5a8b6aeaf91e1",
    "bridge-app/src/main/res/xml/data_extraction_rules.xml":
        "7a4e6defdcb7e21a8cdb4bb4cbdbbef15bcd4c6bae923372fbfa07f0642ac29a",
    "bridge-app/src/test/java/io/github/cyberbasslord666/termuxmcpedge/bridge/BridgeSkeletonTest.java":
        "99ef67a4fd3bfa3d2c7fc04bc552cde1f614d248af3969c7e18a41d6876c4ab7",
    "bridge-contract/build.gradle.kts": "741a6b35b003c8ee0ade326dd417a24ca559ef02e88f6b57c2934ba10194b92c",
    "bridge-contract/consumer-rules.pro": "452d9d12e3edd7ed1597714095d94a94f3ad25cc372613e9f68e48f6799ce27a",
    "bridge-contract/gradle.lockfile": "2179186d5e235cef4e223b2dba66fc10a63eb10217156dfbc0170c9c9db99cb9",
    "bridge-contract/src/main/AndroidManifest.xml":
        "c8fe08b06e479efc7e3d00476fffe693a3e943e2b874b3c6a1ba5156fc180751",
    "bridge-contract/src/main/aidl/io/github/cyberbasslord666/termuxmcpedge/bridge/BridgeCallContext.aidl":
        "2c5ca03e6cfbf4557f0517722aa7452502bb4bd8368adbc72095c9845e205014",
    "bridge-contract/src/main/aidl/io/github/cyberbasslord666/termuxmcpedge/bridge/BridgeFailure.aidl":
        "324102c8d4b58b354663a68c73f117646382933c6dfafaf84d98a8f20db5e164",
    "bridge-contract/src/main/aidl/io/github/cyberbasslord666/termuxmcpedge/bridge/BridgeIdentityObservation.aidl":
        "15021da5a3dd40e1514f27ac3ecdf82d1c255f4beff8143db31de41bc5eda556",
    "bridge-contract/src/main/aidl/io/github/cyberbasslord666/termuxmcpedge/bridge/IBridgeBootstrapCallback.aidl":
        "0f0c64b030174b52a9d18f49bdcb7327bf77bedae4f14de126cbfedaf2ec99dd",
    "bridge-contract/src/main/aidl/io/github/cyberbasslord666/termuxmcpedge/bridge/IBridgeBroker.aidl":
        "de46027147b5a02f1570f0b6d005aed21b5e41ec8387dad6b7dd97fce21e0745",
    "bridge-contract/src/main/aidl/io/github/cyberbasslord666/termuxmcpedge/bridge/IBridgeIdentityCallback.aidl":
        "a41e4160bbb3ebe635327d47c1f982e835f6dd511b345e23164ff06cb22b2893",
    "bridge-contract/src/main/aidl/io/github/cyberbasslord666/termuxmcpedge/bridge/IPrivilegedBridge.aidl":
        "16ca4eea3c4022fe51f6993b2acf1a25f95a6b8ea8fd7d4cd60e7cb34033b605",
    "bridge-contract/src/main/aidl/io/github/cyberbasslord666/termuxmcpedge/bridge/ISystemFeaturesCallback.aidl":
        "6bcd186c0fd8dcfb11477b5b41fa0a9bc4702df915ece15d0d5e5ced3fd8bb97",
    "bridge-contract/src/main/aidl/io/github/cyberbasslord666/termuxmcpedge/bridge/SystemFeaturesResult.aidl":
        "2e9aa61dee897b3a261f7b88cbd87d1a05b7ecef4c2db56e4998c680f441f61e",
    "bridge-contract/src/main/java/io/github/cyberbasslord666/termuxmcpedge/bridge/BridgeCallContext.java":
        "995df5c9746c50cd22e0d93c249c23a510c6f0dd5c6ebb1cbe8e84c86abb35c5",
    "bridge-contract/src/main/java/io/github/cyberbasslord666/termuxmcpedge/bridge/BridgeFailure.java":
        "5c5de8df6b93b7cc51f4d413daf01310736fae94630d0cda0e53e70451c6d5d4",
    "bridge-contract/src/main/java/io/github/cyberbasslord666/termuxmcpedge/bridge/BridgeIdentityObservation.java":
        "cfd85e8a57802ad8510e9b77da7075312dc4005b0e7314df8fa97e321607fef4",
    "bridge-contract/src/main/java/io/github/cyberbasslord666/termuxmcpedge/bridge/BridgeParcelCodec.java":
        "f5b077bda7af0e52951c80b89f0309293d4274d161942640c163eabd54b165e6",
    "bridge-contract/src/main/java/io/github/cyberbasslord666/termuxmcpedge/bridge/SystemFeaturesResult.java":
        "a7c2232c6e2415adbdd842c10a7801b5ba4265e87edff88fe53e7b116e59b243",
    "bridge-contract/src/test/java/io/github/cyberbasslord666/termuxmcpedge/bridge/BridgeParcelableContractTest.java":
        "ca0f1a7b65bfaf66c0c13a8b838b3807f72641dd85accfbbd5462c8d732f7765",
    "build.gradle.kts": "750c78e3a4bbd09f8b6485e76379a9f74395208e414a3cbfa2d4d00eaa3f69a6",
    "gradle.properties": "c0651164d8d678e58dfada42373a40ea37b5f6982c7841bcb79f7ddf4b3da842",
    "gradle/libs.versions.toml": "7c84fd7a5d63ca638d33dcbdd9b1cfe15e9d77ad59179f3dcdd86c51ab10cab0",
    "gradle/verification-metadata.xml": "4f002a6e10dbcb3ad3aa1ccb108e774e3ea05cef657a60fe8ca76080672195da",
    "gradle/wrapper/gradle-wrapper.jar": "81a82aaea5abcc8ff68b3dfcb58b3c3c429378efd98e7433460610fecd7ae45f",
    "gradle/wrapper/gradle-wrapper.properties":
        "c040b6ef2fb893ff5beea5a281614f2848f1c3996a7886f650b713f8240656a2",
    "gradlew": "2f1c4d0d61790cfa72630eb34a30eca29eda4fab0069d0e2716f4f4f5869e21c",
    "gradlew.bat": "7831af24ad4cd03fc71f21e74515656da3cb36fd15bfb7da370eb6df758a169f",
    "settings-gradle.lockfile": "e58bb2ed80db9f829601a658d7c33b9802eb7d2191c31e79c28e01f14c225926",
    "settings.gradle.kts": "d4529fb9c6de014344dcea86580068116280ad2cdd34608d3fc844b8850b0e66",
}
actual_android_hashes = {
    relative: hashlib.sha256((android / relative).read_bytes()).hexdigest()
    for relative in expected_android_hashes
}
require(
    actual_android_hashes == expected_android_hashes,
    "closed Android source/configuration bytes changed",
)
local_archives = {
    path.relative_to(android).as_posix()
    for path in android.rglob("*")
    if path.is_file()
    and path.suffix.lower() in {".aar", ".jar"}
    and not {"build", ".gradle"}.intersection(path.relative_to(android).parts)
}
require(
    local_archives == {"gradle/wrapper/gradle-wrapper.jar"},
    f"local Android archive inventory changed: {sorted(local_archives)}",
)
expected_android_files = set(expected_android_hashes)
def is_known_generated_path(relative: pathlib.PurePath) -> bool:
    parts = relative.parts
    return (
        parts[:1] in ((".gradle",), ("build",))
        or parts[:2] in (("bridge-app", "build"), ("bridge-contract", "build"))
    )


actual_android_files = set()
for path in android.rglob("*"):
    relative = path.relative_to(android)
    require(not path.is_symlink(), f"symlink added to Android subtree: {relative}")
    if is_known_generated_path(relative):
        continue
    if not path.is_file():
        continue
    relative_text = relative.as_posix()
    actual_android_files.add(relative_text)
    expected_mode = 0o755 if relative_text == "gradlew" else 0o644
    require(
        stat.S_IMODE(path.stat().st_mode) == expected_mode,
        f"unexpected Android file mode: {relative_text}",
    )
require(
    actual_android_files == expected_android_files,
    f"closed Android file inventory changed: {sorted(actual_android_files ^ expected_android_files)}",
)

settings = local_text("settings.gradle.kts")
include_modules = set(
    re.findall(r'["\'](:[a-z0-9-]+)["\']', "\n".join(
        match.group(1) for match in re.finditer(r"\binclude\((.*?)\)", settings, re.DOTALL)
    ))
)
require(
    include_modules == {":bridge-contract", ":bridge-app"},
    f"module inventory changed: {sorted(include_modules)}",
)
require_fragment(
    settings,
    "repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)",
    "project repositories are not rejected",
)
require(settings.count("google()") == 2, "Google repository declarations changed")
require(settings.count("mavenCentral()") == 2, "Maven Central declarations changed")
require("gradlePluginPortal" not in settings, "Gradle Plugin Portal was added")
require("jcenter" not in settings.lower(), "JCenter was added")

versions = local_text("gradle/libs.versions.toml")
require_fragment(versions, 'agp = "8.13.2"', "AGP pin changed")
require_fragment(versions, 'junit = "4.13.2"', "JUnit pin changed")
require("androidx" not in versions.lower(), "AndroidX dependency added to the inert skeleton")
require("+" not in versions and "latest" not in versions.lower(), "dynamic dependency version added")

root_build = local_text("build.gradle.kts")
for fragment in (
    "require(JavaVersion.current() == JavaVersion.VERSION_17)",
    "lockAllConfigurations()",
    "lockMode.set(LockMode.STRICT)",
    'tasks.register("stage2Check")',
    '":bridge-contract:testDebugUnitTest"',
    '":bridge-contract:lintDebug"',
    '":bridge-contract:lintRelease"',
    '":bridge-app:testDebugUnitTest"',
    '":bridge-app:lintDebug"',
    '":bridge-app:lintRelease"',
    '":bridge-app:assembleDebug"',
    '":bridge-app:assembleRelease"',
    '":bridge-app:assembleDebugAndroidTest"',
):
    require_fragment(root_build, fragment, f"Stage 2 Gradle contract missing {fragment}")

contract_build = local_text("bridge-contract/build.gradle.kts")
app_build = local_text("bridge-app/build.gradle.kts")
combined_build = "\n".join((root_build, contract_build, app_build))
require(combined_build.count("compileSdk = 36") == 2, "compile API must be exactly 36 in both modules")
require(combined_build.count('buildToolsVersion = "35.0.0"') == 2, "build-tools pin changed")
require(combined_build.count("minSdk = 30") == 2, "minimum API must be exactly 30 in both modules")
require_fragment(app_build, "targetSdk = 36", "target API must be exactly 36")
require_fragment(
    contract_build,
    'namespace = "io.github.cyberbasslord666.termuxmcpedge.bridge.contract"',
    "contract Android namespace changed",
)
require_fragment(
    app_build,
    'namespace = "io.github.cyberbasslord666.termuxmcpedge.bridge"',
    "application Android namespace changed",
)
require(combined_build.count("sourceCompatibility = JavaVersion.VERSION_11") == 2, "Java source level changed")
require(combined_build.count("targetCompatibility = JavaVersion.VERSION_11") == 2, "Java bytecode level changed")
for fragment in (
    'applicationId = "io.github.cyberbasslord666.termuxmcpedge.bridge"',
    "isDebuggable = false",
    "isJniDebuggable = false",
    "isMinifyEnabled = true",
    "isShrinkResources = true",
    "testInstrumentationRunner =",
    '"io.github.cyberbasslord666.termuxmcpedge.bridge.BridgeStage2Instrumentation"',
):
    require_fragment(app_build, fragment, f"hardened application build missing {fragment}")
require(app_build.count("isDebuggable = false") == 2, "both target variants must remain nondebuggable")
require(app_build.count("isJniDebuggable = false") == 2, "both target variants must disable JNI debugging")
require(
    app_build.count('disable += setOf("AndroidGradlePluginVersion", "HardcodedDebugMode")') == 1,
    "application lint exceptions changed",
)
require(
    contract_build.count('disable += "AndroidGradlePluginVersion"') == 1,
    "contract lint exception changed",
)
require(
    len(re.findall(r"\bdisable\b", combined_build)) == 2,
    "an additional lint disable or suppression was added",
)
for forbidden in (
    "baseline =",
    "baseline(",
    "abortOnError = false",
    "warningsAsErrors = false",
    "suppressLint",
    "@SuppressLint",
):
    require(forbidden not in combined_build, f"lint enforcement weakened by {forbidden}")
require(combined_build.count("abortOnError = true") == 2, "lint abort-on-error posture changed")
require(combined_build.count("warningsAsErrors = true") == 2, "lint warning posture changed")
require("signingConfig" not in app_build, "release signing configuration added to Stage 2")
require("androidTestImplementation" not in app_build, "instrumentation dependency added to Stage 2")

def dependency_body(source: str, label: str) -> list[str]:
    match = re.search(r"\bdependencies\s*\{(.*?)\}\s*$", source, re.DOTALL)
    require(match is not None, f"{label} dependency block missing or not terminal")
    return [
        line.strip()
        for line in match.group(1).splitlines()
        if line.strip() and not line.strip().startswith("//")
    ]


require(
    dependency_body(contract_build, "contract") == ["testImplementation(libs.junit)"],
    "contract dependency inventory changed",
)
require(
    dependency_body(app_build, "application")
    == [
        'implementation(project(":bridge-contract"))',
        "testImplementation(libs.junit)",
    ],
    "application dependency inventory changed",
)
for forbidden in (
    "com.android.dynamic-feature",
    "externalNativeBuild",
    "ndkVersion",
    "cmake",
    "System.loadLibrary",
    "Runtime.getRuntime().exec",
    "ProcessBuilder",
):
    require(forbidden not in combined_build, f"forbidden Android build authority added: {forbidden}")

proguard = local_text("bridge-app/proguard-rules.pro")
for forbidden in ("-dontshrink", "-dontobfuscate", "-ignorewarnings", "-dontoptimize"):
    require(forbidden not in proguard, f"release hardening disabled by {forbidden}")

verification_path = android / "gradle/verification-metadata.xml"
require(verification_path.is_file(), "dependency verification metadata missing")
verification_root = ET.parse(verification_path).getroot()
namespace = {"g": "https://schema.gradle.org/dependency-verification"}
metadata_flag = verification_root.find("g:configuration/g:verify-metadata", namespace)
signature_flag = verification_root.find("g:configuration/g:verify-signatures", namespace)
require(metadata_flag is not None and metadata_flag.text == "true", "metadata verification is not strict")
require(signature_flag is not None and signature_flag.text == "false", "unexpected signature-verification posture")
require(verification_root.find("g:configuration/g:trusted-artifacts", namespace) is None, "trusted-artifact wildcard added")
artifacts = verification_root.findall("g:components/g:component/g:artifact", namespace)
require(artifacts, "verified dependency artifact inventory is empty")
for artifact in artifacts:
    checksums = artifact.findall("g:sha256", namespace)
    require(len(checksums) == 1, f"artifact lacks exactly one SHA-256: {artifact.attrib.get('name')}")
    require(
        re.fullmatch(r"[0-9a-f]{64}", checksums[0].attrib.get("value", "")) is not None,
        f"invalid dependency digest: {artifact.attrib.get('name')}",
    )

lockfiles = {
    android / "settings-gradle.lockfile",
    android / "bridge-contract/gradle.lockfile",
    android / "bridge-app/gradle.lockfile",
}
require(all(path.is_file() for path in lockfiles), "module dependency lockfile missing")
for lockfile in lockfiles:
    lock = lockfile.read_text(encoding="utf-8")
    require(lock.strip(), f"empty dependency lockfile: {lockfile.relative_to(root)}")
    require("empty=" in lock, f"lock state sentinel missing: {lockfile.relative_to(root)}")
require(
    (android / "settings-gradle.lockfile").read_text(encoding="utf-8").splitlines()
    == [
        "# This is a Gradle generated file for dependency locking.",
        "# Manual edits can break the build and are not advised.",
        "# This file is expected to be part of source control.",
        "empty=incomingCatalogForLibs0",
    ],
    "settings dependency lock inventory changed",
)

android_text_files = [
    path for path in android.rglob("*")
    if path.is_file() and path.suffix in {".aidl", ".gradle", ".java", ".kt", ".kts", ".toml", ".xml"}
]
android_source = "\n".join(path.read_text(encoding="utf-8") for path in android_text_files)
for forbidden in (
    "dev.rikka.shizuku",
    "rikka.shizuku",
    "ShizukuProvider",
    "Shizuku.get",
    "Shizuku.bindUserService",
):
    require(forbidden not in android_source, f"Shizuku authority/dependency added at Stage 2: {forbidden}")
require(re.search(r"extends\s+[A-Za-z0-9_$.]+\.Stub\b", android_source) is None, "Binder Stub implementation added")
require(re.search(r"extends\s+(?:Activity|Service|BroadcastReceiver|ContentProvider|Application)\b", android_source) is None, "Android component implementation added")

aidl_root = android / "bridge-contract/src/main/aidl/io/github/cyberbasslord666/termuxmcpedge/bridge"
aidl_files = {path.name: path for path in aidl_root.glob("*.aidl")}
expected_aidl = {
    "IBridgeBootstrapCallback.aidl",
    "IBridgeBroker.aidl",
    "IBridgeIdentityCallback.aidl",
    "ISystemFeaturesCallback.aidl",
    "IPrivilegedBridge.aidl",
    "BridgeCallContext.aidl",
    "BridgeFailure.aidl",
    "BridgeIdentityObservation.aidl",
    "SystemFeaturesResult.aidl",
}
require(set(aidl_files) == expected_aidl, f"closed AIDL inventory changed: {sorted(aidl_files)}")
for name, path in aidl_files.items():
    source = path.read_text(encoding="utf-8")
    require_fragment(source, "package io.github.cyberbasslord666.termuxmcpedge.bridge;", f"wrong AIDL package: {name}")
    if name.startswith("I"):
        scrubbed = re.sub(r"/\*.*?\*/|//[^\n]*", "", source, flags=re.DOTALL)
        match = re.search(r"\binterface\s+\w+\s*\{(.*?)\}\s*$", scrubbed, re.DOTALL)
        require(match is not None and not match.group(1).strip(), f"callable AIDL method added: {name}")

android_ns = "{http://schemas.android.com/apk/res/android}"
for relative in (
    "bridge-contract/src/main/AndroidManifest.xml",
    "bridge-app/src/main/AndroidManifest.xml",
):
    manifest = ET.parse(android / relative).getroot()
    require(manifest.tag == "manifest", f"unexpected manifest root: {relative}")
    require(
        f"{android_ns}sharedUserId" not in manifest.attrib,
        f"shared Android UID added: {relative}",
    )
    require(
        f"{android_ns}sharedUserMaxSdkVersion" not in manifest.attrib,
        f"shared Android UID compatibility added: {relative}",
    )
    require(not manifest.attrib, f"source manifest root attributes changed: {relative}")
    require("split" not in manifest.attrib, f"dynamic-feature split added: {relative}")
    forbidden_tags = {
        "uses-permission", "uses-permission-sdk-23", "permission",
        "activity", "activity-alias", "service", "receiver", "provider",
    }
    for element in manifest.iter():
        tag = element.tag.rsplit("}", 1)[-1]
        require(tag not in forbidden_tags, f"forbidden manifest element {tag}: {relative}")

app_manifest = ET.parse(android / "bridge-app/src/main/AndroidManifest.xml").getroot()
applications = app_manifest.findall("application")
require(len(applications) == 1, "application manifest must contain one inert application")
application = applications[0]
require(len(application) == 0, "application manifest contains a component or metadata")
expected_application_attributes = {
    f"{android_ns}allowBackup": "false",
    f"{android_ns}dataExtractionRules": "@xml/data_extraction_rules",
    f"{android_ns}fullBackupContent": "false",
    f"{android_ns}hasFragileUserData": "false",
    f"{android_ns}icon": "@drawable/bridge_inert_icon",
    f"{android_ns}label": "Termux MCP Bridge (inert)",
    f"{android_ns}supportsRtl": "false",
    f"{android_ns}usesCleartextTraffic": "false",
}
require(application.attrib == expected_application_attributes, "inert application attributes changed")

extraction_rules = ET.parse(
    android / "bridge-app/src/main/res/xml/data_extraction_rules.xml"
).getroot()
require(extraction_rules.tag == "data-extraction-rules", "data extraction rule root changed")
expected_domains = {
    "root", "file", "database", "sharedpref", "external",
    "device_root", "device_file", "device_database", "device_sharedpref",
}
require(
    [child.tag for child in extraction_rules] == ["cloud-backup", "device-transfer"],
    "data extraction rule branches changed",
)
for branch in extraction_rules:
    require(all(child.tag == "exclude" for child in branch), "data extraction include/unknown rule added")
    require({child.attrib.get("domain") for child in branch} == expected_domains, "data extraction domains changed")
    require(all(child.attrib.get("path") == "." for child in branch), "data extraction exclusion narrowed")
require(
    extraction_rules[0].attrib == {"disableIfNoEncryptionCapabilities": "true"},
    "cloud backup encryption posture changed",
)
require(not extraction_rules[1].attrib, "device-transfer attributes added")

unit_tests = list(android.glob("*/src/test/**/*.java")) + list(android.glob("*/src/test/**/*.kt"))
instrumentation_sources = list(android.glob("bridge-app/src/androidTest/**/*.java")) + list(android.glob("bridge-app/src/androidTest/**/*.kt"))
expected_host_tests = {
    "bridge-app/src/test/java/io/github/cyberbasslord666/termuxmcpedge/bridge/BridgeSkeletonTest.java": {
        "stageTwoArtifactHasNoRuntimeAuthority",
    },
    "bridge-contract/src/test/java/io/github/cyberbasslord666/termuxmcpedge/bridge/BridgeParcelableContractTest.java": {
        "contextRejectsUnknownVersionAndNonPositiveRequestId",
        "contextRejectsEveryNonExactDigestOrNonceLength",
        "everyParcelableCreatorBoundsArrayAllocation",
        "mutableInputAndOutputArraysNeverAliasContextStorage",
        "failureCodeSetIsClosed",
        "failureAndIdentityCloneEveryMutableInputAndOutput",
        "identityIsClosedBoundedAndDefensivelyCopied",
        "identityRejectsUnboundedOrNonAsciiVersionNameAndInvalidUid",
        "systemFeaturesResultIsStructurallyInertUntilStageFive",
    },
}
actual_host_tests = {}
for test_path in unit_tests:
    relative = test_path.relative_to(android).as_posix()
    source = test_path.read_text(encoding="utf-8")
    actual_host_tests[relative] = set(
        re.findall(r"@Test\s+public\s+void\s+([A-Za-z][A-Za-z0-9_]*)\s*\(", source)
    )
require(actual_host_tests == expected_host_tests, "closed host-test inventory changed")
require(sum(map(len, actual_host_tests.values())) == 10, "host-test count changed")
skeleton_test = local_text(
    "bridge-app/src/test/java/io/github/cyberbasslord666/termuxmcpedge/bridge/BridgeSkeletonTest.java"
)
for fragment in (
    "assertEquals(2, BridgeSkeleton.SKELETON_STAGE);",
    "assertFalse(BridgeSkeleton.RUNTIME_AUTHORITY_ENABLED);",
    "assertFalse(BridgeSkeleton.SHIZUKU_LINKED);",
):
    require_fragment(skeleton_test, fragment, f"host authority assertion changed: {fragment}")
parcel_host_test = local_text(
    "bridge-contract/src/test/java/io/github/cyberbasslord666/termuxmcpedge/bridge/BridgeParcelableContractTest.java"
)
for fragment in (
    "assertThrows(",
    "BridgeCallContext.CREATOR.newArray(-1)",
    "BridgeFailure.CREATOR.newArray(17)",
    "BridgeIdentityObservation.CREATOR.newArray(Integer.MAX_VALUE)",
    "SystemFeaturesResult.CREATOR.newArray(Integer.MAX_VALUE)",
    "assertNotSame(observed, context.getRequestNonce());",
    'identityWithVersionName("v".repeat(65))',
):
    require_fragment(parcel_host_test, fragment, f"host parcel assertion changed: {fragment}")
require(
    {path.name for path in instrumentation_sources}
    == {
        "BridgeManifestInstrumentationTest.java",
        "BridgeParcelInstrumentationTest.java",
        "BridgeStage2Instrumentation.java",
        "BridgeTestAssertions.java",
    },
    "instrumentation source inventory changed",
)
expected_instrumentation_hashes = {
    "BridgeManifestInstrumentationTest.java":
        "2abe988883c7832c66972e4e0bf39c5e63e6f5591a9e52e3b0d7b9017182695e",
    "BridgeParcelInstrumentationTest.java":
        "29d3c74f229085514b1bba2099a7bb00875f7530686ccb01d6adc0314ec9f339",
    "BridgeStage2Instrumentation.java":
        "6acd5e1649672e747923bb8efdabddceb101e394395e612f5b5856099a58b8bb",
    "BridgeTestAssertions.java":
        "03fffb21e47e3c12a2ef9bb06979e5dea9efba0ceda471a1db3e607c8e58f4bb",
}
require(
    {
        path.name: hashlib.sha256(path.read_bytes()).hexdigest()
        for path in instrumentation_sources
    }
    == expected_instrumentation_hashes,
    "closed instrumentation implementation changed",
)
instrumentation_inventory = set()
for test_path in (
    path for path in instrumentation_sources if path.name.endswith("InstrumentationTest.java")
):
    test_source = test_path.read_text(encoding="utf-8")
    class_match = re.search(r"public\s+final\s+class\s+(\w+)", test_source)
    require(class_match is not None, f"instrumentation class is not final: {test_path.name}")
    methods = re.findall(r"public\s+void\s+(test\w+)\s*\(", test_source)
    require(methods, f"instrumentation class has no tests: {test_path.name}")
    instrumentation_inventory.update((class_match.group(1), method) for method in methods)
require(
    instrumentation_inventory == {
        ("BridgeManifestInstrumentationTest", "testTargetManifestHasNoPermissionOrComponent"),
        ("BridgeParcelInstrumentationTest", "testContextRoundTripPreservesOnlyFixedFields"),
        ("BridgeParcelInstrumentationTest", "testUnknownContextVersionFailsClosedDuringUnparcel"),
    },
    f"instrumentation test inventory changed: {sorted(instrumentation_inventory)}",
)
runner_source = next(
    path.read_text(encoding="utf-8")
    for path in instrumentation_sources
    if path.name == "BridgeStage2Instrumentation.java"
)
parcel_instrumentation_source = next(
    path.read_text(encoding="utf-8")
    for path in instrumentation_sources
    if path.name == "BridgeParcelInstrumentationTest.java"
)
expected_runtime_inventory = (
    "BridgeManifestInstrumentationTest#testTargetManifestHasNoPermissionOrComponent;"
    "BridgeParcelInstrumentationTest#testContextRoundTripPreservesOnlyFixedFields;"
    "BridgeParcelInstrumentationTest#testUnknownContextVersionFailsClosedDuringUnparcel"
)
for fragment in (
    "public final class BridgeStage2Instrumentation extends Instrumentation",
    "private static final int EXPECTED_TEST_COUNT = 3;",
    "manifestTest.testTargetManifestHasNoPermissionOrComponent(getTargetContext());",
    "parcelTest.testContextRoundTripPreservesOnlyFixedFields();",
    "parcelTest.testUnknownContextVersionFailsClosedDuringUnparcel();",
    'result.putInt("numtests", EXPECTED_TEST_COUNT);',
    'result.putString("tests", EXACT_TEST_INVENTORY);',
    'result.putString("stream", "\\nOK (3 tests)\\n");',
    'result.putString("failure", failureCheckpoint);',
    'result.putString("stream", "\\nFAILURES (checkpoint=" + failureCheckpoint + ")\\n");',
    'failureCheckpoint = "M00";',
    'failureCheckpoint = "R00";',
    'failureCheckpoint = "P00";',
    "finish(Activity.RESULT_OK, result);",
    "finish(Activity.RESULT_CANCELED, result);",
):
    require_fragment(runner_source, fragment, f"custom instrumentation runner missing {fragment}")
for fragment in (
    "static final class CheckpointFailure extends AssertionError",
    'super("closed instrumentation checkpoint failed");',
    "assertEveryAlignedTruncationRejected",
    "encoded.length % Integer.BYTES",
    "cut += Integer.BYTES",
):
    require_fragment(
        parcel_instrumentation_source,
        fragment,
        f"closed Parcel checkpoint/truncation contract missing {fragment}",
    )
checkpoint_codes = re.findall(
    r'runCheckpoint\(\s*"(P[0-9]{2})"', parcel_instrumentation_source
)
require(
    checkpoint_codes == [f"P{index:02d}" for index in range(1, 12)],
    f"closed Parcel checkpoint inventory changed: {checkpoint_codes}",
)
for forbidden in (
    "failure.getMessage()",
    "failure.getClass()",
    "failure.printStackTrace",
    "Log.",
    "cut++",
):
    require(
        forbidden not in runner_source + parcel_instrumentation_source,
        f"unbounded or byte-prefix instrumentation diagnostic added: {forbidden}",
    )
inventory_match = re.search(
    r"private\s+static\s+final\s+String\s+EXACT_TEST_INVENTORY\s*=\s*"
    r"(.*?)\n\s*\n\s*@Override",
    runner_source,
    re.DOTALL,
)
require(inventory_match is not None, "custom instrumentation result inventory constant missing")
inventory_declaration = inventory_match.group(1).strip()
require(inventory_declaration.endswith(";"), "custom instrumentation inventory declaration is unterminated")
inventory_literals = re.findall(r'"([^"\\]*)"', inventory_declaration[:-1])
require("".join(inventory_literals) == expected_runtime_inventory, "custom instrumentation result inventory changed")
for forbidden in ("Class.forName", "getDeclaredMethods", "androidx.", "org.junit"):
    require(forbidden not in runner_source, f"dynamic or external instrumentation authority added: {forbidden}")

workflow_path = root / ".github/workflows/shizuku-bridge-skeleton.yml"
workflow = text(".github/workflows/shizuku-bridge-skeleton.yml")
require(
    hashlib.sha256(workflow_path.read_bytes()).hexdigest()
    == "5b13d1a4c59819d50033e0af63ae47114b9d8841b1345c53dc48865ac94b452f",
    "closed Stage 2 workflow bytes changed",
)
for fragment in (
    "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1",
    "actions/setup-java@b6effb05e454b25005698d916606bdc6ffcbf961",
    "TEMURIN_JDK_URL: https://github.com/adoptium/temurin17-binaries/releases/download/jdk-17.0.20%2B8/OpenJDK17U-jdk_x64_linux_hotspot_17.0.20_8.tar.gz",
    "TEMURIN_JDK_SHA256: be7668bc030d578b83d6d5ef9221d6d6729bbbca8cf94a7d52e16ac68b5a5a35",
    'TEMURIN_JDK_SIZE: "193273593"',
    "distribution: jdkfile",
    'java-version: "17.0.20+8"',
    "jdk-file: ${{ runner.temp }}/temurin-17.0.20+8-linux-x64.tar.gz",
    'test ! -e "$RUNNER_TOOL_CACHE/Java_jdkfile_jdk"',
    'SETUP_JDK_DISTRIBUTION: ${{ steps.setup-jdk.outputs.distribution }}',
    'test "$SETUP_JDK_DISTRIBUTION" = jdkfile',
    "GRADLE_DISTRIBUTION_SHA256: 20f1b1176237254a6fc204d8434196fa11a4cfb387567519c61556e8710aed78",
    "GRADLE_WRAPPER_JAR_SHA256: 81a82aaea5abcc8ff68b3dfcb58b3c3c429378efd98e7433460610fecd7ae45f",
    "GRADLEW_SHA256: 2f1c4d0d61790cfa72630eb34a30eca29eda4fab0069d0e2716f4f4f5869e21c",
    'ANDROID_HOME: /tmp/termux-mcp-stage2-android-sdk',
    'ANDROID_CMDLINE_TOOLS_REVISION: "22.0"',
    "ANDROID_CMDLINE_TOOLS_URL: https://dl.google.com/android/repository/commandlinetools-linux-15859902_latest.zip",
    'ANDROID_PLATFORM_TOOLS_REVISION: "37.0.1"',
    "ANDROID_PLATFORM_TOOLS_URL: https://dl.google.com/android/repository/platform-tools_r37.0.1-linux.zip",
    'ANDROID_EMULATOR_REVISION: "37.1.11"',
    "ANDROID_EMULATOR_URL: https://dl.google.com/android/repository/emulator-linux_x64-15917651.zip",
    'ANDROID_PLATFORM_REVISION: "2"',
    "ANDROID_PLATFORM_URL: https://dl.google.com/android/repository/platform-36_r02.zip",
    'ANDROID_BUILD_TOOLS_REVISION: "35.0.0"',
    "ANDROID_BUILD_TOOLS_URL: https://dl.google.com/android/repository/build-tools_r35_linux.zip",
    "ANDROID_SYSTEM_IMAGE: system-images;android-30;google_apis;x86_64",
    'ANDROID_SYSTEM_IMAGE_REVISION: "16"',
    "ANDROID_SYSTEM_IMAGE_URL: https://dl.google.com/android/repository/sys-img/google_apis/x86_64-30_r16.zip",
    "ANDROID_CMDLINE_TOOLS_LINUX_SHA1: 040d3996a65543d22ec4bf73e4c37aa37a8d4af4",
    "ANDROID_CMDLINE_TOOLS_LINUX_SHA256: 4e4c464f145a7512b57d088ac6c278c03c9eea610886b35a5e0804e74eedf583",
    "ANDROID_PLATFORM_TOOLS_LINUX_SHA1: 477254aa5f903c15cf51001717bdf347fb6b53e0",
    "ANDROID_PLATFORM_TOOLS_LINUX_SHA256: d230f13842f60f782a8645f9c813f8f845bf36089ea7289f28c48f17979313f1",
    "ANDROID_EMULATOR_LINUX_SHA1: 1b1f78891abf8ec268264356e1365c25519e8379",
    "ANDROID_EMULATOR_LINUX_SHA256: 95771e0ae431897b2a4bd2d97fa095f29a8b0624a7b216baf529f9306161c266",
    "ANDROID_PLATFORM_LINUX_SHA1: 2c1a80dd4d9f7d0e6dd336ec603d9b5c55a6f576",
    "ANDROID_PLATFORM_LINUX_SHA256: 37607369a28c5b640b3a7998868d45898ebcb777565a0e85f9acf36f29631d2e",
    "ANDROID_BUILD_TOOLS_LINUX_SHA1: 2cfaa0bbb2336e9ec18ed3ecea84fa2e2af607bc",
    "ANDROID_BUILD_TOOLS_LINUX_SHA256: bd3a4966912eb8b30ed0d00b0cda6b6543b949d5ffe00bea54c04c81e1561d88",
    "ANDROID_SYSTEM_IMAGE_SHA1: 6ae21030eaadc041078444d3798e4b399f3e787d",
    "ANDROID_SYSTEM_IMAGE_SHA256: daae27654be74ae83a484daea4db2c0c77b4f4ad661a645bd5f36d96ce03e4d5",
    "https://dl.google.com/android/repository/repository2-3.xml",
    "https://dl.google.com/android/repository/sys-img/google_apis/sys-img2-3.xml",
    'repository_archive_base = "https://dl.google.com/android/repository/"',
    '"https://dl.google.com/android/repository/sys-img/google_apis/"',
    'actual_url = f"{archive_base}{relative_url}"',
    '"platforms;android-36"',
    'f"build-tools;{os.environ[\'ANDROID_BUILD_TOOLS_REVISION\']}"',
    'test ! -e "$ANDROID_SDK_ROOT"',
    "sha256sum --check --strict",
    "unsafe or unexpected archive entry",
    "SystemImage.TagId",
    "96f51acc01cabbcc32e158817daef7302add78b77365bf18b189cc3941ddea30",
    "emulator package registration template changed",
    'java.runtime.version[[:space:]]*$/',
    'GRADLE_USER_HOME: ${{ runner.temp }}/shizuku-bridge-gradle',
    'test ! -e "$GRADLE_USER_HOME"',
    "--dependency-verification=strict",
    "stage2Check",
    ":bridge-contract:testDebugUnitTest",
    ":bridge-app:testDebugUnitTest",
    ":bridge-app:lintDebug",
    ":bridge-app:lintRelease",
    ":bridge-app:assembleDebug",
    ":bridge-app:assembleRelease",
    ":bridge-app:assembleDebugAndroidTest",
    "Inspect exact host unit-test results",
    "BridgeSkeletonTest.xml",
    "BridgeParcelableContractTest.xml",
    "host unit-test report inventory changed",
    "host test failure or skip recorded",
    "merged_manifests",
    "aapt\" dump permissions",
    "apkanalyzer\" manifest debuggable",
    "expected_root_attributes = {",
    "merged-manifest root attributes changed",
    "APK unexpectedly declares a shared UID",
    "apksigner\" verify",
    "ERROR: Missing META-INF/MANIFEST.MF",
    "bridge-app-release-unsigned.apk",
    "bridge-app-debug-androidTest.apk",
    "BRIDGE_APPLICATION_ID: io.github.cyberbasslord666.termuxmcpedge.bridge",
    "BRIDGE_TEST_APPLICATION_ID: io.github.cyberbasslord666.termuxmcpedge.bridge.test",
    "BRIDGE_TEST_RUNNER: io.github.cyberbasslord666.termuxmcpedge.bridge.BridgeStage2Instrumentation",
    'bounded 10s "$emulator" -no-window -version',
    "validate_adb_install_transcript()",
    "validate_adb_uninstall_transcript()",
    "ulimit -f 8 || exit 70",
    'expected_stdout = b"Performing Push Install\\nSuccess\\n"',
    'expected_stdout = b"Success\\n"',
    'expected_stderr = b""',
    'megabytes_per_second = r"(?:0|[1-9][0-9]*)\\.[0-9]"',
    'seconds = r"(?:0|[1-9][0-9]*)\\.[0-9]{3}"',
    '+ r" MB/s \\("',
    "if ((install_status != 0)); then",
    "if ((uninstall_status != 0)); then",
    "if re.fullmatch(expected_stderr, stderr) is None:",
    "if stderr != expected_stderr:",
    'target_cleanup_required=true',
    'validate_adb_install_transcript target "$debug_apk"',
    'test_cleanup_required=true',
    'validate_adb_install_transcript test "$android_test_apk" -t',
    "-avd \"$avd_name\"",
    "ro.build.version.sdk",
    "OK (3 tests)",
    "INSTRUMENTATION_CODE: ",
    'expected_results = {',
    '"numtests": "3"',
    '"tests": expected_tests',
    '"stream": ""',
    "unexpected or duplicate instrumentation result key",
    "final instrumentation code changed or was not final",
    "adb_bounded()",
    "kill -TERM -- \"-$emulator_pgid\"",
    "kill -KILL -- \"-$emulator_pgid\"",
    "setsid --wait \"$emulator\"",
    "git diff --exit-code",
):
    require_fragment(workflow, fragment, f"Android workflow missing {fragment}")
headless_version_probe = 'bounded 10s "$emulator" -no-window -version'
require(workflow.count(headless_version_probe) == 1, "headless emulator version probe changed")
active_headless_version_probe = (
    '          bounded 10s "$emulator" -no-window -version \\\n'
    '            | grep -F "Android emulator version $ANDROID_EMULATOR_REVISION"'
)
require(
    workflow.count(active_headless_version_probe) == 1,
    "active headless emulator version probe changed",
)
require(
    'bounded 10s "$emulator" -version' not in workflow,
    "GUI-linked emulator version probe added",
)
require(workflow.count("install --no-streaming -r") == 1, "ADB install path changed")
require(workflow.count("ulimit -f 8 || exit 70") == 2, "ADB output ceilings changed")
require(
    workflow.count("<<'PY' || return 1") == 2,
    "ADB transcript validator failure propagation changed",
)
install_stdout_check = (
    '          expected_stdout = b"Performing Push Install\\nSuccess\\n"\n'
    "          stdout = stdout_path.read_bytes()\n"
    "          if stdout != expected_stdout:"
)
uninstall_stdout_check = (
    '          expected_stdout = b"Success\\n"\n'
    '          expected_stderr = b""\n'
    "          if stdout != expected_stdout:"
)
require(workflow.count("if stdout != expected_stdout:") == 2, "ADB stdout checks changed")
require(workflow.count(install_stdout_check) == 1, "ADB install stdout check changed")
require(workflow.count(uninstall_stdout_check) == 1, "ADB uninstall stdout check changed")
require_fragment(
    workflow,
    '"$(stat -c \'%s\' "$apk_path")" <<\'PY\' || return 1',
    "ADB install transcript validator can be ignored",
)
require_fragment(
    workflow,
    'python3 - "$stdout_path" "$stderr_path" <<\'PY\' || return 1',
    "ADB uninstall transcript validator can be ignored",
)
install_status_capture = (
    '            if (\n'
    '              ulimit -f 8 || exit 70\n'
    '              adb_bounded 2m -s "$EMULATOR_SERIAL" \\\n'
    '                install --no-streaming -r "$@" "$apk_path"\n'
    '            ) >"$stdout_path" 2>"$stderr_path"; then\n'
    '              install_status=0\n'
    '            else\n'
    '              install_status=$?\n'
    '            fi\n'
    '            if ((install_status != 0)); then\n'
    "              printf 'adb install failed for %s with status %d\\n' \\\n"
    '                "$label" "$install_status" >&2\n'
    '              cat "$stdout_path" >&2\n'
    '              cat "$stderr_path" >&2\n'
    '              return 1\n'
    '            fi'
)
uninstall_status_capture = (
    '            if (\n'
    '              ulimit -f 8 || exit 70\n'
    '              adb_bounded 20s -s "$EMULATOR_SERIAL" uninstall "$package_name"\n'
    '            ) >"$stdout_path" 2>"$stderr_path"; then\n'
    '              uninstall_status=0\n'
    '            else\n'
    '              uninstall_status=$?\n'
    '            fi\n'
    '            if ((uninstall_status != 0)); then\n'
    "              printf 'adb uninstall failed for %s with status %d\\n' \\\n"
    '                "$label" "$uninstall_status" >&2\n'
    '              cat "$stdout_path" >&2\n'
    '              cat "$stderr_path" >&2\n'
    '              return 1\n'
    '            fi'
)
require(workflow.count(install_status_capture) == 1, "ADB install status capture changed")
require(
    workflow.count(uninstall_status_capture) == 1,
    "ADB uninstall status capture changed",
)
require(
    workflow.count("              install_status=$?\n") == 1,
    "ADB install failure status capture changed",
)
require(
    workflow.count("              uninstall_status=$?\n") == 1,
    "ADB uninstall failure status capture changed",
)
require("target_installed" not in workflow, "stale post-validation cleanup flag returned")
require("test_installed" not in workflow, "stale test cleanup flag returned")
target_cleanup_arm = workflow.index("target_cleanup_required=true")
target_install_call = workflow.index('validate_adb_install_transcript target "$debug_apk"')
test_cleanup_arm = workflow.index("test_cleanup_required=true")
test_install_call = workflow.index(
    'validate_adb_install_transcript test "$android_test_apk" -t'
)
require(target_cleanup_arm < target_install_call, "target cleanup is not armed before install")
require(test_cleanup_arm < test_install_call, "test cleanup is not armed before install")
install_sequence = (
    "          target_cleanup_required=true\n"
    '          validate_adb_install_transcript target "$debug_apk"\n'
    "          test_cleanup_required=true\n"
    '          validate_adb_install_transcript test "$android_test_apk" -t'
)
require(workflow.count(install_sequence) == 1, "cleanup-armed APK install sequence changed")
require(
    workflow.count("target_cleanup_required=") == 3,
    "target cleanup assignment inventory changed",
)
require(
    workflow.count("test_cleanup_required=") == 3,
    "test cleanup assignment inventory changed",
)
require(
    workflow.count("validate_adb_uninstall_transcript") == 5,
    "ADB uninstall validator call inventory changed",
)
cleanup_uninstall_sequence = (
    '            if [[ "$test_cleanup_required" == true ]]; then\n'
    "              validate_adb_uninstall_transcript \\\n"
    '                cleanup-test "$BRIDGE_TEST_APPLICATION_ID" \\\n'
    "                || cleanup_failed=true\n"
    "            fi\n"
    '            if [[ "$target_cleanup_required" == true ]]; then\n'
    "              validate_adb_uninstall_transcript \\\n"
    '                cleanup-target "$BRIDGE_APPLICATION_ID" \\\n'
    "                || cleanup_failed=true\n"
    "            fi"
)
require(
    workflow.count(cleanup_uninstall_sequence) == 1,
    "trap cleanup package bindings changed",
)
normal_test_uninstall = workflow.rindex('            test "$BRIDGE_TEST_APPLICATION_ID"')
normal_target_uninstall = workflow.rindex('            target "$BRIDGE_APPLICATION_ID"')
test_absence_check = workflow.index('test -z "$test_path_after_uninstall"')
target_absence_check = workflow.index('test -z "$target_path_after_uninstall"')
test_cleanup_disarm = workflow.rindex("test_cleanup_required=false")
target_cleanup_disarm = workflow.rindex("target_cleanup_required=false")
require(
    normal_test_uninstall < normal_target_uninstall < test_absence_check,
    "normal uninstall ordering changed",
)
require(
    test_absence_check < target_absence_check < test_cleanup_disarm,
    "cleanup disarmed before package absence proof",
)
require(
    test_cleanup_disarm < target_cleanup_disarm,
    "cleanup disarm ordering changed",
)
require("uninstall_output" not in workflow, "pipeline-masked uninstall status returned")
for forbidden in (
    "actions/upload-artifact@",
    "actions/download-artifact@",
    "connectedDebugAndroidTest",
    "android-emulator-runner",
    "sdkmanager --install",
    "releaseEligible: true",
    "productionControlQualified: true",
):
    require(forbidden not in workflow, f"Android workflow added forbidden claim/action: {forbidden}")
require(workflow.count('bounded "$duration" "$adb" "$@"') == 1, "ADB timeout wrapper changed")
require(workflow.count('"$adb"') == 1, "an Android Debug Bridge call bypasses the timeout wrapper")
uses = re.findall(r"^\s*uses:\s*([^\s]+)", workflow, re.MULTILINE)
require(uses and all(re.search(r"@[0-9a-f]{40}$", use) for use in uses), "workflow action is not commit-pinned")
require(re.search(r"permissions:\s*\n\s+contents: read", workflow) is not None, "workflow permissions changed")

ci = text(".github/workflows/ci.yml")
security = text(".github/workflows/security.yml")
for source, label in ((ci, "CI"), (security, "Security")):
    require_fragment(source, '"android/shizuku-bridge/**"', f"{label} Android path filter missing")
    require_fragment(source, '".github/dependabot.yml"', f"{label} Dependabot path filter missing")
    require_fragment(source, '".gitattributes"', f"{label} attributes path filter missing")
    require_fragment(source, '".gitignore"', f"{label} ignore path filter missing")
require_fragment(ci, "bash tests/shizuku_bridge_android_skeleton_test.sh", "CI skeleton contract invocation missing")
for documentation_path in (
    '"docs/SHIZUKU_TYPED_BRIDGE_ARCHITECTURE.md"',
    '"docs/VALIDATION.md"',
):
    require_fragment(security, documentation_path, f"Security documentation path missing: {documentation_path}")
for language in ("rust", "actions", "java-kotlin"):
    require(security.count(f"          - {language}") == 1, f"CodeQL language missing or duplicated: {language}")
require_fragment(security, "build-mode: none", "buildless CodeQL contract changed")
require_fragment(security, "queries: security-extended", "CodeQL security-extended query suite missing")

dependabot = text(".github/dependabot.yml")
require(dependabot.count('package-ecosystem: "gradle"') == 1, "Gradle Dependabot entry missing or duplicated")
require_fragment(dependabot, 'directory: "/android/shizuku-bridge"', "Gradle Dependabot directory changed")

expected_attributes = b"""* text=auto eol=lf
*.sh text eol=lf
*.rs text eol=lf
*.toml text eol=lf
*.md text eol=lf
*.yml text eol=lf
*.yaml text eol=lf
*.aidl text eol=lf whitespace=-blank-at-eof
*.gradle text eol=lf
*.java text eol=lf
*.kt text eol=lf
*.kts text eol=lf
*.properties text eol=lf
*.xml text eol=lf
gradlew text eol=lf whitespace=-blank-at-eof
gradlew.bat text eol=lf whitespace=-blank-at-eof
gradle-wrapper.jar binary
"""
require(
    (root / ".gitattributes").read_bytes() == expected_attributes,
    "closed Git attribute bytes or ordered line inventory changed",
)
ignore = text(".gitignore")
for fragment in (
    "/android/shizuku-bridge/.gradle/",
    "/android/shizuku-bridge/**/build/",
    "/android/shizuku-bridge/local.properties",
):
    require_fragment(ignore, fragment, f"Android ignore rule missing: {fragment}")
require("gradle-wrapper.jar" not in ignore, "pinned Gradle wrapper JAR was ignored")

architecture = text("docs/SHIZUKU_TYPED_BRIDGE_ARCHITECTURE.md")
architecture_normalized = re.sub(r"\s+", " ", architecture)
inventory_phrase = (
    "exactly seven governed Android postures, exactly nine workflow artifacts, "
    "exactly twelve staged qualification members, and exactly sixteen public assets unchanged"
)
require(architecture_normalized.count(inventory_phrase) == 1, "architecture inventory preservation changed")
require_fragment(architecture_normalized, "2. **Android skeleton:** pinned Gradle wrapper/dependencies", "Stage 2 architecture step changed")
require_fragment(architecture_normalized, "inert AIDL, manifest/R8/unit/instrumentation checks;", "inert Stage 2 exit gate changed")
require_fragment(architecture_normalized, "no privileged method.", "Stage 2 authority boundary changed")

validation = text("docs/VALIDATION.md")
for fragment in (
    "### Stage 2 Android bridge skeleton",
    "It grants no Android or Shizuku authority",
    "Stable emulator | `37.1.11`",
    "`system-images;android-30;google_apis;x86_64` | `16`",
    "The mutable Google repository metadata must still name each",
    "the workflow never asks `sdkmanager` to refetch or reuse a package",
    "independently reviewed SHA-256",
    "passes only that file to the commit-pinned JDK installer in `jdkfile` mode",
    "be7668bc030d578b83d6d5ef9221d6d6729bbbca8cf94a7d52e16ac68b5a5a35",
    "Both target variants are explicitly",
    "nondebuggable",
    "`AndroidGradlePluginVersion`",
    "`HardcodedDebugMode`",
    "41-file Android subtree inventory",
    "exactly nine contract host-test methods",
    "and one application host-test",
    "positively parses the two JUnit XML reports",
    "nine-plus-one passing host tests",
    "`sharedUserId` or `sharedUserMaxSdkVersion`",
    "runs",
    "`io.github.cyberbasslord666.termuxmcpedge.bridge.test/io.github.cyberbasslord666.termuxmcpedge.bridge.BridgeStage2Instrumentation`",
    "Each no-streaming install arms cleanup before the attempt",
    "`Performing Push Install` followed by `Success`",
    "Unexpected framing cannot skip uninstall and emulator cleanup",
    "Every uninstall independently requires status zero, exact `Success` stdout, and empty stderr",
    "Both cleanup flags remain armed until `pm path` proves both package IDs absent",
    "`OK (3 tests)`",
    "final result code `-1`",
    "real API-30 `Parcel` test rejects truncation at every four-byte Parcel",
    "Android 11 `Parcel.unmarshall` aligns and zero-pads a non-word byte prefix",
    "Stage 2 therefore makes no impossible claim",
    "sender's pre-canonicalization byte count",
    "Later identity stages must compare",
    "every fixed commitment byte-exactly",
    "A runner-caught test failure emits only",
    "one closed checkpoint",
    "emits no exception text or",
    "stack trace",
    "Failures before the runner catch",
    "cannot pass the",
    "closed success parser",
    "This is emulator evidence for the inert Stage 2 skeleton only",
    "The workflow contains no artifact-upload step",
    "exactly seven governed Android postures, nine Android-workflow artifacts,",
    "twelve staged qualification members, and sixteen public assets",
):
    require_fragment(validation, fragment, f"Stage 2 validation boundary missing: {fragment}")

for relative in (
    ".github/workflows/android-cross-compile.yml",
    ".github/workflows/automated-release-qualification.yml",
    ".github/workflows/stage-release-assets.yml",
    ".github/workflows/publish-release.yml",
):
    require("android/shizuku-bridge" not in text(relative), f"Stage 2 entered governed inventory: {relative}")
PY
}

validate_tracked_generated() {
  local root="$1"
  local tracked
  tracked="$(
    git -C "$root" ls-files -- \
      android/shizuku-bridge/.gradle \
      android/shizuku-bridge/build \
      android/shizuku-bridge/bridge-app/build \
      android/shizuku-bridge/bridge-contract/build \
      android/shizuku-bridge/local.properties
  )"
  [[ -z "$tracked" ]]
}

validate_repository_visibility() {
  local root="$1"
  diff -u \
    <(
      git -C "$root" ls-files --cached --others --exclude-standard -- \
        android/shizuku-bridge \
        | LC_ALL=C sort
    ) \
    <(
      find "$root/android/shizuku-bridge" \
        \( -path "$root/android/shizuku-bridge/.gradle" \
          -o -path "$root/android/shizuku-bridge/build" \
          -o -path "$root/android/shizuku-bridge/bridge-app/build" \
          -o -path "$root/android/shizuku-bridge/bridge-contract/build" \) \
        -prune \
        -o -type f -printf 'android/shizuku-bridge/%P\n' \
        | LC_ALL=C sort
    ) \
    >/dev/null
}

validate_tree "$REPO_ROOT" || fail baseline_validation
validate_tracked_generated "$REPO_ROOT" || fail tracked_generated_android_input
validate_repository_visibility "$REPO_ROOT" || fail Android_repository_visibility

CASE_ROOT="$(mktemp -d)"
trap 'rm -rf -- "$CASE_ROOT"' EXIT

make_fixture() {
  rm -rf -- "$CASE_ROOT"
  mkdir -p "$CASE_ROOT"
  (
    cd "$REPO_ROOT"
    cp -a --parents \
      .gitattributes \
      .gitignore \
      .github/dependabot.yml \
      .github/workflows/ci.yml \
      .github/workflows/security.yml \
      .github/workflows/shizuku-bridge-skeleton.yml \
      .github/workflows/android-cross-compile.yml \
      .github/workflows/automated-release-qualification.yml \
      .github/workflows/stage-release-assets.yml \
      .github/workflows/publish-release.yml \
      docs/SHIZUKU_TYPED_BRIDGE_ARCHITECTURE.md \
      docs/VALIDATION.md \
      "$CASE_ROOT"
    cp -a --parents android/shizuku-bridge "$CASE_ROOT"
  )
}

replace_once() {
  local relative="$1"
  local old="$2"
  local new="$3"
  python3 - "$CASE_ROOT/$relative" "$old" "$new" <<'PY'
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
old = sys.argv[2]
new = sys.argv[3]
content = path.read_text(encoding="utf-8")
if old not in content:
    raise SystemExit(f"mutation source not found in {path}: {old}")
path.write_text(content.replace(old, new, 1), encoding="utf-8")
PY
}

append_text() {
  local relative="$1"
  local addition="$2"
  python3 - "$CASE_ROOT/$relative" "$addition" <<'PY'
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
with path.open("a", encoding="utf-8") as handle:
    handle.write(sys.argv[2])
PY
}

expect_rejected_replace() {
  local label="$1"
  local relative="$2"
  local old="$3"
  local new="$4"
  make_fixture
  replace_once "$relative" "$old" "$new"
  if validate_tree "$CASE_ROOT" >/dev/null 2>&1; then
    fail "adversarial mutation accepted: $label"
  fi
}

expect_rejected_append() {
  local label="$1"
  local relative="$2"
  local addition="$3"
  make_fixture
  append_text "$relative" "$addition"
  if validate_tree "$CASE_ROOT" >/dev/null 2>&1; then
    fail "adversarial mutation accepted: $label"
  fi
}

expect_rejected_delete() {
  local label="$1"
  local relative="$2"
  make_fixture
  rm -- "$CASE_ROOT/$relative"
  if validate_tree "$CASE_ROOT" >/dev/null 2>&1; then
    fail "adversarial deletion accepted: $label"
  fi
}

expect_rejected_new_file() {
  local label="$1"
  local relative="$2"
  local content="$3"
  make_fixture
  mkdir -p -- "$(dirname "$CASE_ROOT/$relative")"
  python3 - "$CASE_ROOT/$relative" "$content" <<'PY'
import pathlib
import sys

pathlib.Path(sys.argv[1]).write_text(sys.argv[2], encoding="utf-8")
PY
  if validate_tree "$CASE_ROOT" >/dev/null 2>&1; then
    fail "adversarial new file accepted: $label"
  fi
}

expect_rejected_symlink() {
  local label="$1"
  local relative="$2"
  local target="$3"
  make_fixture
  mkdir -p -- "$(dirname "$CASE_ROOT/$relative")"
  ln -s -- "$target" "$CASE_ROOT/$relative"
  if validate_tree "$CASE_ROOT" >/dev/null 2>&1; then
    fail "adversarial symlink accepted: $label"
  fi
}

expect_rejected_executable() {
  local label="$1"
  local relative="$2"
  make_fixture
  chmod +x -- "$CASE_ROOT/$relative"
  if validate_tree "$CASE_ROOT" >/dev/null 2>&1; then
    fail "adversarial executable mode accepted: $label"
  fi
}

expect_rejected_tracked_generated() {
  make_fixture
  mkdir -p -- "$CASE_ROOT/android/shizuku-bridge/.gradle"
  python3 - "$CASE_ROOT/android/shizuku-bridge/.gradle/payload.bin" <<'PY'
import pathlib
import sys

pathlib.Path(sys.argv[1]).write_bytes(b"tracked generated input")
PY
  git -C "$CASE_ROOT" init -q
  git -C "$CASE_ROOT" add -f android/shizuku-bridge/.gradle/payload.bin
  if validate_tracked_generated "$CASE_ROOT" \
    || validate_repository_visibility "$CASE_ROOT"; then
    fail "adversarial tracked generated input accepted"
  fi
}

expect_rejected_replace \
  wrapper_digest_drift \
  .github/workflows/shizuku-bridge-skeleton.yml \
  'GRADLE_WRAPPER_JAR_SHA256: 81a82aaea5abcc8ff68b3dfcb58b3c3c429378efd98e7433460610fecd7ae45f' \
  'GRADLE_WRAPPER_JAR_SHA256: 0000000000000000000000000000000000000000000000000000000000000000'
expect_rejected_replace \
  jdk_archive_checksum_drift \
  .github/workflows/shizuku-bridge-skeleton.yml \
  'TEMURIN_JDK_SHA256: be7668bc030d578b83d6d5ef9221d6d6729bbbca8cf94a7d52e16ac68b5a5a35' \
  'TEMURIN_JDK_SHA256: 0000000000000000000000000000000000000000000000000000000000000000'
expect_rejected_replace \
  jdk_archive_size_drift \
  .github/workflows/shizuku-bridge-skeleton.yml \
  'TEMURIN_JDK_SIZE: "193273593"' \
  'TEMURIN_JDK_SIZE: "193273594"'
expect_rejected_replace \
  jdk_local_file_mode_removed \
  .github/workflows/shizuku-bridge-skeleton.yml \
  'distribution: jdkfile' \
  'distribution: temurin'
expect_rejected_append \
  duplicate_wrapper_url \
  android/shizuku-bridge/gradle/wrapper/gradle-wrapper.properties \
  $'\ndistributionUrl=https\\://attacker.invalid/gradle.zip\n'
expect_rejected_replace \
  dependency_verification_disabled \
  .github/workflows/shizuku-bridge-skeleton.yml \
  '--dependency-verification=strict' \
  '--dependency-verification=off'
expect_rejected_replace \
  cold_gradle_home_guard_removed \
  .github/workflows/shizuku-bridge-skeleton.yml \
  'test ! -e "$GRADLE_USER_HOME"' \
  'test -d "$GRADLE_USER_HOME" || true'
expect_rejected_append \
  artifact_upload_added \
  .github/workflows/shizuku-bridge-skeleton.yml \
  $'\n# actions/upload-artifact@0000000000000000000000000000000000000000\n'
expect_rejected_append \
  broad_whitespace_attribute_relaxation \
  .gitattributes \
  $'* -whitespace\n'
expect_rejected_replace \
  rust_codeql_removed \
  .github/workflows/security.yml \
  '          - rust' \
  '          - java-kotlin'
expect_rejected_append \
  shizuku_dependency_added \
  android/shizuku-bridge/bridge-app/build.gradle.kts \
  $'\n// dev.rikka.shizuku:api:13.1.5\n'
expect_rejected_replace \
  top_level_dependency_api_added \
  android/shizuku-bridge/bridge-app/build.gradle.kts \
  'dependencies {' \
  $'dependencies.add("implementation", "junit:junit:4.13.2")\n\ndependencies {'
expect_rejected_replace \
  local_file_dependency_added \
  android/shizuku-bridge/bridge-app/build.gradle.kts \
  'dependencies {' \
  $'dependencies.add("implementation", files("libs/payload.jar"))\n\ndependencies {'
expect_rejected_new_file \
  local_jar_added \
  android/shizuku-bridge/bridge-app/libs/payload.jar \
  'not a reviewed dependency'
expect_rejected_new_file \
  build_src_added \
  android/shizuku-bridge/buildSrc/src/main/kotlin/Payload.kt \
  'class Payload'
expect_rejected_new_file \
  alternate_groovy_build_added \
  android/shizuku-bridge/bridge-app/build.gradle \
  'throw new GradleException("unreviewed build")'
expect_rejected_replace \
  included_build_added \
  android/shizuku-bridge/settings.gradle.kts \
  'rootProject.name = "termux-mcp-shizuku-bridge"' \
  $'rootProject.name = "termux-mcp-shizuku-bridge"\nincludeBuild("unreviewed")'
expect_rejected_replace \
  apply_from_added \
  android/shizuku-bridge/bridge-app/build.gradle.kts \
  'plugins {' \
  $'apply(from = "unreviewed.gradle.kts")\n\nplugins {'
expect_rejected_append \
  gradle_property_added \
  android/shizuku-bridge/gradle.properties \
  $'\norg.gradle.unsafe.configuration-cache=true\n'
expect_rejected_symlink \
  source_symlink_added \
  android/shizuku-bridge/bridge-app/src/main/java/Payload.java \
  /etc/passwd
expect_rejected_executable \
  source_executable_mode_added \
  android/shizuku-bridge/bridge-app/src/main/AndroidManifest.xml
expect_rejected_tracked_generated
expect_rejected_replace \
  extra_module_added \
  android/shizuku-bridge/settings.gradle.kts \
  'include(":bridge-contract", ":bridge-app")' \
  'include(":bridge-contract", ":bridge-app", ":dynamic-bridge")'
expect_rejected_replace \
  manifest_component_added \
  android/shizuku-bridge/bridge-app/src/main/AndroidManifest.xml \
  '</manifest>' \
  $'    <uses-permission android:name="android.permission.INTERNET" />\n</manifest>'
expect_rejected_replace \
  manifest_shared_user_id_added \
  android/shizuku-bridge/bridge-app/src/main/AndroidManifest.xml \
  '<manifest xmlns:android="http://schemas.android.com/apk/res/android">' \
  $'<manifest xmlns:android="http://schemas.android.com/apk/res/android"\n    android:sharedUserId="android.uid.system">'
expect_rejected_replace \
  manifest_shared_user_max_sdk_added \
  android/shizuku-bridge/bridge-app/src/main/AndroidManifest.xml \
  '<manifest xmlns:android="http://schemas.android.com/apk/res/android">' \
  $'<manifest xmlns:android="http://schemas.android.com/apk/res/android"\n    android:sharedUserMaxSdkVersion="32">'
expect_rejected_replace \
  bridge_skeleton_authority_enabled \
  android/shizuku-bridge/bridge-app/src/main/java/io/github/cyberbasslord666/termuxmcpedge/bridge/BridgeSkeleton.java \
  'RUNTIME_AUTHORITY_ENABLED = false' \
  'RUNTIME_AUTHORITY_ENABLED = true'
expect_rejected_replace \
  parcel_truncation_guard_removed \
  android/shizuku-bridge/bridge-contract/src/main/java/io/github/cyberbasslord666/termuxmcpedge/bridge/BridgeParcelCodec.java \
  'requireRemaining(source, outerEndPosition, INT_BYTES);' \
  '// requireRemaining(source, outerEndPosition, INT_BYTES);'
expect_rejected_replace \
  data_extraction_resource_weakened \
  android/shizuku-bridge/bridge-app/src/main/res/xml/data_extraction_rules.xml \
  '<exclude domain="root" path="." />' \
  '<exclude domain="root" path="files" />'
expect_rejected_replace \
  host_authority_assertion_weakened \
  android/shizuku-bridge/bridge-app/src/test/java/io/github/cyberbasslord666/termuxmcpedge/bridge/BridgeSkeletonTest.java \
  'assertFalse(BridgeSkeleton.RUNTIME_AUTHORITY_ENABLED);' \
  'assertTrue(BridgeSkeleton.RUNTIME_AUTHORITY_ENABLED);'
expect_rejected_append \
  release_signing_added \
  android/shizuku-bridge/bridge-app/build.gradle.kts \
  $'\n// signingConfig = signingConfigs.getByName("debug")\n'
expect_rejected_replace \
  debug_target_enabled \
  android/shizuku-bridge/bridge-app/build.gradle.kts \
  'isDebuggable = false' \
  'isDebuggable = true'
expect_rejected_replace \
  lint_exception_broadened \
  android/shizuku-bridge/bridge-app/build.gradle.kts \
  'disable += setOf("AndroidGradlePluginVersion", "HardcodedDebugMode")' \
  'disable += setOf("AndroidGradlePluginVersion", "HardcodedDebugMode", "UnsafeOptInUsageError")'
expect_rejected_replace \
  contract_lint_exception_broadened \
  android/shizuku-bridge/bridge-contract/build.gradle.kts \
  'disable += "AndroidGradlePluginVersion"' \
  'disable += setOf("AndroidGradlePluginVersion", "UnsafeOptInUsageError")'
expect_rejected_append \
  lint_baseline_added \
  android/shizuku-bridge/bridge-app/build.gradle.kts \
  $'\n// baseline = file("lint-baseline.xml")\n'
expect_rejected_replace \
  android_test_assembly_removed \
  .github/workflows/shizuku-bridge-skeleton.yml \
  ':bridge-app:assembleDebugAndroidTest' \
  ':bridge-app:assembleDebug'
expect_rejected_replace \
  historical_inventory_changed \
  docs/SHIZUKU_TYPED_BRIDGE_ARCHITECTURE.md \
  'exactly seven governed Android postures' \
  'exactly eight governed Android postures'
expect_rejected_delete \
  contract_lock_deleted \
  android/shizuku-bridge/bridge-contract/gradle.lockfile
expect_rejected_delete \
  settings_lock_deleted \
  android/shizuku-bridge/settings-gradle.lockfile
expect_rejected_replace \
  emulator_revision_drift \
  .github/workflows/shizuku-bridge-skeleton.yml \
  'ANDROID_EMULATOR_REVISION: "37.1.11"' \
  'ANDROID_EMULATOR_REVISION: "37.1.12"'
expect_rejected_replace \
  system_image_checksum_drift \
  .github/workflows/shizuku-bridge-skeleton.yml \
  'ANDROID_SYSTEM_IMAGE_SHA1: 6ae21030eaadc041078444d3798e4b399f3e787d' \
  'ANDROID_SYSTEM_IMAGE_SHA1: 0000000000000000000000000000000000000000'
expect_rejected_replace \
  system_image_archive_base_drift \
  .github/workflows/shizuku-bridge-skeleton.yml \
  'ANDROID_SYSTEM_IMAGE_URL: https://dl.google.com/android/repository/sys-img/google_apis/x86_64-30_r16.zip' \
  'ANDROID_SYSTEM_IMAGE_URL: https://dl.google.com/android/repository/x86_64-30_r16.zip'
expect_rejected_replace \
  emulator_archive_sha256_drift \
  .github/workflows/shizuku-bridge-skeleton.yml \
  'ANDROID_EMULATOR_LINUX_SHA256: 95771e0ae431897b2a4bd2d97fa095f29a8b0624a7b216baf529f9306161c266' \
  'ANDROID_EMULATOR_LINUX_SHA256: 0000000000000000000000000000000000000000000000000000000000000000'
expect_rejected_replace \
  headless_emulator_version_probe_removed \
  .github/workflows/shizuku-bridge-skeleton.yml \
  'bounded 10s "$emulator" -no-window -version' \
  'bounded 10s "$emulator" -version'
expect_rejected_append \
  bare_emulator_version_probe_added \
  .github/workflows/shizuku-bridge-skeleton.yml \
  $'\n# bounded 10s "$emulator" -version\n'
expect_rejected_append \
  commented_headless_emulator_probe_added \
  .github/workflows/shizuku-bridge-skeleton.yml \
  $'\n# bounded 10s "$emulator" -no-window -version\n'
expect_rejected_replace \
  adb_install_stdout_contract_loosened \
  .github/workflows/shizuku-bridge-skeleton.yml \
  'expected_stdout = b"Performing Push Install\nSuccess\n"' \
  'expected_stdout = b"Success\n"'
expect_rejected_replace \
  adb_install_stderr_contract_loosened \
  .github/workflows/shizuku-bridge-skeleton.yml \
  'if re.fullmatch(expected_stderr, stderr) is None:' \
  'if re.search(expected_stderr, stderr) is None:'
expect_rejected_replace \
  adb_install_output_limit_removed \
  .github/workflows/shizuku-bridge-skeleton.yml \
  'ulimit -f 8' \
  'ulimit -f unlimited'
expect_rejected_replace \
  target_cleanup_arm_removed \
  .github/workflows/shizuku-bridge-skeleton.yml \
  'target_cleanup_required=true' \
  'target_cleanup_required=false'
expect_rejected_replace \
  adb_uninstall_status_ignored \
  .github/workflows/shizuku-bridge-skeleton.yml \
  'if ((uninstall_status != 0)); then' \
  'if false; then'
expect_rejected_replace \
  adb_install_failure_status_zeroed \
  .github/workflows/shizuku-bridge-skeleton.yml \
  'install_status=$?' \
  'install_status=0'
expect_rejected_replace \
  adb_uninstall_failure_status_zeroed \
  .github/workflows/shizuku-bridge-skeleton.yml \
  'uninstall_status=$?' \
  'uninstall_status=0'
expect_rejected_replace \
  adb_install_failure_return_zeroed \
  .github/workflows/shizuku-bridge-skeleton.yml \
  $'adb install failed for %s with status %d\\n\' \\\n                "$label" "$install_status" >&2\n              cat "$stdout_path" >&2\n              cat "$stderr_path" >&2\n              return 1' \
  $'adb install failed for %s with status %d\\n\' \\\n                "$label" "$install_status" >&2\n              cat "$stdout_path" >&2\n              cat "$stderr_path" >&2\n              return 0'
expect_rejected_replace \
  adb_uninstall_failure_return_zeroed \
  .github/workflows/shizuku-bridge-skeleton.yml \
  $'adb uninstall failed for %s with status %d\\n\' \\\n                "$label" "$uninstall_status" >&2\n              cat "$stdout_path" >&2\n              cat "$stderr_path" >&2\n              return 1' \
  $'adb uninstall failed for %s with status %d\\n\' \\\n                "$label" "$uninstall_status" >&2\n              cat "$stdout_path" >&2\n              cat "$stderr_path" >&2\n              return 0'
expect_rejected_replace \
  adb_install_transcript_failure_ignored \
  .github/workflows/shizuku-bridge-skeleton.yml \
  $'"$(stat -c \'%s\' "$apk_path")" <<\'PY\' || return 1' \
  $'"$(stat -c \'%s\' "$apk_path")" <<\'PY\' || true'
expect_rejected_replace \
  adb_uninstall_transcript_failure_ignored \
  .github/workflows/shizuku-bridge-skeleton.yml \
  $'python3 - "$stdout_path" "$stderr_path" <<\'PY\' || return 1' \
  $'python3 - "$stdout_path" "$stderr_path" <<\'PY\' || true'
expect_rejected_replace \
  adb_install_stdout_check_ignored \
  .github/workflows/shizuku-bridge-skeleton.yml \
  $'expected_stdout = b"Performing Push Install\\nSuccess\\n"\n          stdout = stdout_path.read_bytes()\n          if stdout != expected_stdout:' \
  $'expected_stdout = b"Performing Push Install\\nSuccess\\n"\n          stdout = stdout_path.read_bytes()\n          if False and stdout != expected_stdout:'
expect_rejected_replace \
  adb_uninstall_stdout_check_ignored \
  .github/workflows/shizuku-bridge-skeleton.yml \
  $'expected_stdout = b"Success\\n"\n          expected_stderr = b""\n          if stdout != expected_stdout:' \
  $'expected_stdout = b"Success\\n"\n          expected_stderr = b""\n          if False and stdout != expected_stdout:'
expect_rejected_replace \
  adb_uninstall_stderr_ignored \
  .github/workflows/shizuku-bridge-skeleton.yml \
  'if stderr != expected_stderr:' \
  'if False and stderr != expected_stderr:'
expect_rejected_replace \
  cleanup_disarmed_before_absence \
  .github/workflows/shizuku-bridge-skeleton.yml \
  $'          test -z "$target_path_after_uninstall"\n          test_cleanup_required=false' \
  $'          test_cleanup_required=false\n          test -z "$target_path_after_uninstall"'
expect_rejected_replace \
  trap_test_package_binding_changed \
  .github/workflows/shizuku-bridge-skeleton.yml \
  'cleanup-test "$BRIDGE_TEST_APPLICATION_ID"' \
  'cleanup-test "$BRIDGE_APPLICATION_ID"'
expect_rejected_append \
  extra_instrumentation_test_added \
  android/shizuku-bridge/bridge-app/src/androidTest/java/io/github/cyberbasslord666/termuxmcpedge/bridge/BridgeParcelInstrumentationTest.java \
  $'\npublic void testUnexpectedFourthOperation() {}\n'

printf '%s\n' 'Shizuku bridge Android skeleton contract passed'
