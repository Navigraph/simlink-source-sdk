//! The Rust, C++ and C# examples, built against the generated bindings and run against a test Simlink.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Result;

use crate::{bindings, cargo, executable, run};

const EXAMPLES: [&str; 3] = ["quickstart", "commands", "diagnostics"];

/// Builds every example, then `crates/simlink/tests/examples.rs` checks that each joins a test Simlink and
/// answers it.
pub fn check() -> Result<()> {
    let programs = build()?;

    run(cargo("test")
        .args(["-p", "simlink", "--test", "examples", "--", "--ignored"])
        .env("SIMLINK_EXAMPLES", std::env::join_paths(&programs)?))
}

/// Builds every example in release. Returns the programs by absolute path.
fn build() -> Result<Vec<PathBuf>> {
    bindings::build()?;
    run(cargo("build").args(["--release", "-p", "simlink", "--examples"]))?;
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

    Ok(programs
        .into_iter()
        .map(std::path::absolute)
        .collect::<std::io::Result<_>>()?)
}

/// Configures and builds a CMake project in release. Returns the directory holding its executables.
fn cmake(source: &Path, out: &Path) -> Result<PathBuf> {
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
fn dotnet(project: &Path, out: &Path) -> Result<()> {
    run(Command::new("dotnet")
        .arg("build")
        .arg(project)
        .args(["-c", "Release", "-o"])
        .arg(out))
}

/// `quickstart` → `Quickstart`, the C# project for a Rust and C++ example.
fn pascal(name: &str) -> String {
    name[..1].to_uppercase() + &name[1..]
}
