//! `simlink-source-sdk-cpp-<version>-<rid>.zip`: the headers, the C++ half of the bindings as source for the user's
//! build to compile, the Rust library, `bindings/cpp/prebuilt` as its CMake project, and the examples.

use std::path::Path;
use std::process::Command;

use anyhow::Result;

use super::library::Library;
use super::{Archive, VERSION};
use crate::examples::{self, EXAMPLES, cmake};
use crate::{executable, run};

/// Builds this platform's C++ archive and tests its examples.
pub fn package() -> Result<()> {
    let library = Library::build()?;
    let archive = Archive::new(format!("simlink-source-sdk-cpp-{VERSION}-{}", library.rid))?;

    archive.add("bindings/cpp/prebuilt/CMakeLists.txt", "")?;
    if cfg!(windows) {
        archive.add("bindings/cpp/prebuilt/simlink-source-sdk.props", "")?;
    }
    for header in [
        "simlink_source_sdk.hpp",
        "simlink_source_sdk_scaffolding.hpp",
    ] {
        archive.add(Path::new("target/bindings/cpp").join(header), "include")?;
    }
    archive.add("target/bindings/cpp/simlink_source_sdk.cpp", "src")?;
    archive.add(&library.static_library, "lib")?;
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
    // Running the Release examples covers the library; this checks the bindings also build and link in Debug.
    if cfg!(windows) {
        run(Command::new("cmake")
            .arg("--build")
            .arg(archive.build_dir())
            .args(["--config", "Debug"]))?;
    }

    Ok(())
}
