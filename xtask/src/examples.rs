//! The Rust, C++ and C# examples, built against the generated bindings and run against a test Simlink.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Result;

use crate::{bindings, cargo, executable, run};

pub const EXAMPLES: [&str; 4] = ["quickstart", "traffic", "commands", "diagnostics"];

/// Builds every example, then tests them.
pub fn check() -> Result<()> {
    test(&build()?)
}

/// `crates/simlink-source-sdk/tests/examples.rs` checks that each example program joins a test Simlink and answers it.
pub fn test(programs: &[PathBuf]) -> Result<()> {
    let programs = programs
        .iter()
        .map(std::path::absolute)
        .collect::<std::io::Result<Vec<_>>>()?;

    run(cargo("test")
        .args([
            "-p",
            "simlink-source-sdk",
            "--test",
            "examples",
            "--",
            "--ignored",
        ])
        .env(
            "SIMLINK_SOURCE_SDK_EXAMPLES",
            std::env::join_paths(&programs)?,
        ))
}

/// Builds every example in release. Returns the programs.
fn build() -> Result<Vec<PathBuf>> {
    bindings::build()?;
    run(cargo("build").args(["--release", "-p", "simlink-source-sdk", "--examples"]))?;
    let cpp = cmake(Path::new("examples/cpp"), Path::new("target/examples/cpp"))?;

    let mut programs = Vec::new();
    for example in EXAMPLES {
        let name = pascal(example);
        let project = Path::new("examples/csharp")
            .join(&name)
            .join(format!("{name}.csproj"));
        let csharp = Path::new("target/examples/csharp").join(&name);
        dotnet(&project, &csharp)?;

        programs.push(Path::new("target/release/examples").join(executable(example)));
        programs.push(cpp.join(executable(example)));
        programs.push(csharp.join(executable(&name)));
    }

    Ok(programs)
}

/// Configures and builds a CMake project in release. Returns the directory holding its executables.
pub fn cmake(source: &Path, out: &Path) -> Result<PathBuf> {
    run(Command::new("cmake")
        .arg("-S")
        .arg(source)
        .arg("-B")
        .arg(out)
        // Single-config generators (Makefiles, Ninja) take the build type at configure time.
        .arg("-DCMAKE_BUILD_TYPE=Release"))?;
    run(Command::new("cmake")
        .arg("--build")
        .arg(out)
        .args(["--config", "Release"]))?;

    Ok(out.join("bin"))
}

/// Builds a .NET project in release into `out`.
pub fn dotnet(project: &Path, out: &Path) -> Result<()> {
    run(Command::new("dotnet")
        .arg("build")
        .arg(project)
        .args(["-c", "Release", "-o"])
        .arg(out))
}

/// `quickstart` → `Quickstart`, the C# project for a Rust and C++ example.
pub fn pascal(name: &str) -> String {
    name[..1].to_uppercase() + &name[1..]
}
