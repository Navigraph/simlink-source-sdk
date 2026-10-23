//! `simlink-cpp-<version>-<rid>.zip`: the headers and one static library with the C++ half of the bindings compiled
//! in, `bindings/cpp/prebuilt` as its CMake project, and the examples.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

use super::library::Library;
use super::{Archive, OUT, VERSION};
use crate::examples::{self, EXAMPLES, cmake};
use crate::{executable, run};

/// MSVC won't link Release C++ code into a Debug build, so Windows ships both.
const CONFIGS: &[&str] = if cfg!(windows) {
    &["Release", "Debug"]
} else {
    &["Release"]
};

/// Builds this platform's C++ archive and tests its examples.
pub fn package() -> Result<()> {
    let library = Library::build()?;
    let archive = Archive::new(format!("simlink-cpp-{VERSION}-{}", library.rid))?;

    archive.add("bindings/cpp/prebuilt/CMakeLists.txt", "")?;
    if cfg!(windows) {
        archive.add("bindings/cpp/prebuilt/simlink.props", "")?;
    }
    archive.add("bindings/cpp/cmake/system-libraries.cmake", "cmake")?;
    for header in ["simlink.hpp", "simlink_scaffolding.hpp"] {
        archive.add(Path::new("target/bindings/cpp").join(header), "include")?;
    }
    add_libraries(&archive, &library)?;
    archive.add_rewritten(
        "examples/cpp/CMakeLists.txt",
        "examples/CMakeLists.txt",
        &[("../../bindings/cpp", "..")],
    )?;
    for example in EXAMPLES {
        archive.add(format!("examples/cpp/{example}.cpp"), "examples")?;
    }
    archive.add("LICENSE", "")?;
    archive.zip()?;

    let bin = cmake(&archive.path("examples"), &archive.build_dir())?;
    examples::test(&EXAMPLES.map(|example| bin.join(executable(example))))?;
    // Running the Release examples covers what the Debug library does; this checks it links.
    if cfg!(windows) {
        run(Command::new("cmake")
            .arg("--build")
            .arg(archive.build_dir())
            .args(["--config", "Debug"]))?;
    }

    Ok(())
}

/// Compiles the bindings in each configuration and merges each into a copy of the Rust library in `lib`.
fn add_libraries(archive: &Archive, library: &Library) -> Result<()> {
    let build = Path::new(OUT).join("build/bindings-cpp");
    let mut configure = Command::new("cmake");
    configure
        .args(["-S", "bindings/cpp", "-B"])
        .arg(&build)
        .arg("-DCMAKE_BUILD_TYPE=Release");
    if cfg!(target_os = "macos") {
        configure.arg("-DCMAKE_OSX_ARCHITECTURES=arm64;x86_64");
    }
    run(&mut configure)?;

    let name = library.static_library.file_name().context("no file name")?;
    for config in CONFIGS {
        run(Command::new("cmake")
            .arg("--build")
            .arg(&build)
            .args(["--config", config]))?;
        // Multi-config generators (Visual Studio) put each configuration in its own folder.
        let (bindings, lib) = if cfg!(windows) {
            (
                build.join(config).join(name),
                archive.path(&format!("lib/{config}")),
            )
        } else {
            (build.join(name), archive.path("lib"))
        };
        std::fs::create_dir_all(&lib)?;
        merge(
            &[&library.static_library, &bindings],
            &lib.join(name),
            &build,
        )?;
    }

    Ok(())
}

/// Joins static libraries into `out` with the platform's archiver.
fn merge(libraries: &[&Path], out: &Path, cmake_build: &Path) -> Result<()> {
    if cfg!(windows) {
        // lib.exe is only on PATH in a developer prompt; CMake found it.
        let cache = std::fs::read_to_string(cmake_build.join("CMakeCache.txt"))?;
        let lib = cache
            .lines()
            .find_map(|line| line.strip_prefix("CMAKE_AR:FILEPATH="))
            .context("CMake found no archiver")?;
        run(Command::new(lib)
            .arg(format!("/OUT:{}", out.display()))
            // Rust's library repeats some Windows import stubs; lib.exe keeps the first.
            .arg("/IGNORE:4006")
            .args(libraries))
    } else if cfg!(target_os = "macos") {
        run(Command::new("libtool")
            .args(["-static", "-o"])
            .arg(out)
            .args(libraries))
    } else {
        // GNU ar can only merge archives through an MRI script.
        let mut script = format!("CREATE {}\n", out.display());
        for library in libraries {
            script += &format!("ADDLIB {}\n", library.display());
        }
        script += "SAVE\nEND\n";
        let mut ar = Command::new("ar").arg("-M").stdin(Stdio::piped()).spawn()?;
        ar.stdin
            .take()
            .context("ar stdin")?
            .write_all(script.as_bytes())?;
        let status = ar.wait()?;
        if !status.success() {
            bail!("ar -M failed with {status}");
        }

        Ok(())
    }
}
