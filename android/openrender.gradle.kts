// Gradle integration for OpenRender.
//
// Option A — include from the app module:
//
//   // android/app/build.gradle.kts
//   apply(from = "../../openrender.gradle.kts")
//
//   android {
//       sourceSets["main"].jniLibs.srcDir(layout.projectDirectory.dir("src/main/jniLibs"))
//   }
//
// Option B — copy the task below into your module and adapt `repoRoot`.
//
// The task runs `cargo ndk` *before* mergeJniLibFolders, so the freshly built
// .so files are packaged without a manual copy step. Static archives (.a) are
// produced by scripts/build-android.ps1|sh for hosts that link statically.

import org.gradle.api.tasks.Exec

val repoRoot: File = rootProject.projectDir.parentFile
val rustOut: File = layout.projectDirectory.dir("src/main/jniLibs").asFile
val rustAbis = listOf("arm64-v8a", "armeabi-v7a", "x86_64")
val rustFeatures: List<String> =
    (project.findProperty("openrenderFeatures") as String?)
        ?.split(',')
        ?.map(String::trim)
        ?.filter(String::isNotEmpty)
        ?: emptyList()

val buildRust by tasks.registering(Exec::class) {
    group = "build"
    description = "Cross-compile openrender-core for Android with cargo-ndk"

    workingDir = repoRoot
    val args = mutableListOf("cargo", "ndk")
    rustAbis.forEach { abi ->
        args += listOf("-t", abi)
    }
    args += listOf("-o", rustOut.absolutePath)
    args += listOf("build", "--release", "-p", "openrender-core")
    if (rustFeatures.isNotEmpty()) {
        args += listOf("--features", rustFeatures.joinToString(","))
    }
    commandLine(args)

    inputs.dir(File(repoRoot, "crates"))
    inputs.file(File(repoRoot, "Cargo.toml"))
    inputs.file(File(repoRoot, "Cargo.lock"))
    outputs.dir(rustOut)
}

tasks.matching { it.name == "mergeJniLibFolders" }.configureEach { dependsOn(buildRust) }
tasks.matching { it.name == "preBuild" }.configureEach { dependsOn(buildRust) }
