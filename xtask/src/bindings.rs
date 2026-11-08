//! The libraries, and the C++ and C# bindings generated from them.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Result;

use crate::generators::{GENERATORS, Generator};
use crate::{cargo, executable, run};

/// Builds `simlink_source_sdk.lib`/`libsimlink_source_sdk.a` for C++, the dynamic library for C#, and both bindings
/// from it.
pub fn build() -> Result<()> {
    run(cargo("build").args(["--release", "-p", "simlink-source-sdk"]))?;
    let library = Path::new("target/release").join(dynamic_library());

    for generator in &GENERATORS {
        let out = Path::new("target/bindings").join(generator.out);
        // Stale files from a renamed or removed item would otherwise linger.
        if out.exists() {
            std::fs::remove_dir_all(&out)?;
        }
        std::fs::create_dir_all(&out)?;
        run(Command::new(install(generator)?)
            .arg("--library")
            .arg(&library)
            .args(generator.args)
            .arg("--out-dir")
            .arg(&out))?;
    }

    Ok(())
}

/// Installs a generator into `target/tools` on first use. Returns its executable.
fn install(generator: &Generator) -> Result<PathBuf> {
    let [_, pin] = generator.pin.args();
    let dir = Path::new("target/tools").join(format!("{}-{pin}", generator.name));
    let exe = dir.join("bin").join(executable(generator.name));
    if !exe.exists() {
        // Not `cargo()`: installs build in a throwaway directory, not alongside the libraries.
        run(Command::new("cargo")
            .args([
                "install",
                "--locked",
                generator.name,
                "--git",
                generator.repository,
            ])
            .args(generator.pin.args())
            .arg("--root")
            .arg(&dir)
            .env_remove("CARGO_TARGET_DIR"))?;
    }

    Ok(exe)
}

pub fn dynamic_library() -> String {
    format!(
        "{}simlink_source_sdk{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    )
}
