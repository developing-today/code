plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

val buildAndroidRemote by tasks.registering(Exec::class) {
    workingDir = rootProject.projectDir.resolve("..")
    commandLine(
        "cargo",
        "ndk",
        "-t", "arm64-v8a",
        "-t", "x86_64",
        "-p", "26",
        "-o", "android/app/src/main/jniLibs",
        "build",
        "--release",
        "-p", "id-android-remote",
    )
}

tasks.named("preBuild").configure {
    dependsOn(buildAndroidRemote)
}

android {
    namespace = "org.developingtoday.id"
    compileSdk = 35

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    defaultConfig {
        applicationId = "org.developingtoday.id"
        minSdk = 26
        targetSdk = 35
        versionCode = 1
        versionName = "0.1.0"
    }

    kotlinOptions {
        jvmTarget = "17"
    }
}
