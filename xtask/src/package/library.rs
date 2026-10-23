//! This platform's Rust library, as both archives ship it.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Result, bail};

use crate::{bindings, cargo, run};

pub struct Library {
    /// The .NET runtime identifier, which also names the C++ archive.
    pub rid: &'static str,
    pub static_library: PathBuf,
    pub dynamic_library: PathBuf,
}

impl Library {
    /// Builds the library and both bindings. macOS gets one universal library for both architectures.
    pub fn build() -> Result<Library> {
        bindings::build()?;
        let static_name = format!(
            "{}simlink{}",
            std::env::consts::DLL_PREFIX,
            if cfg!(windows) { ".lib" } else { ".a" }
        );
        let dynamic_name = bindings::dynamic_library();
        let release = |rid| Library {
            rid,
            static_library: Path::new("target/release").join(&static_name),
            dynamic_library: Path::new("target/release").join(&dynamic_name),
        };

        Ok(match (std::env::consts::OS, std::env::consts::ARCH) {
            ("windows", "x86_64") => release("win-x64"),
            ("linux", "x86_64") => release("linux-x64"),
            ("macos", _) => Library {
                rid: "osx",
                static_library: universal(&static_name)?,
                dynamic_library: universal(&dynamic_name)?,
            },
            (os, arch) => bail!("no release archive for {os} {arch}"),
        })
    }
}

/// Builds `library` for both macOS architectures and joins them with `lipo`.
fn universal(library: &str) -> Result<PathBuf> {
    let mut slices = Vec::new();
    for target in ["aarch64-apple-darwin", "x86_64-apple-darwin"] {
        run(Command::new("rustup").args(["target", "add", target]))?;
        run(cargo("build").args(["--release", "-p", "simlink", "--target", target]))?;
        slices.push(
            Path::new("target")
                .join(target)
                .join("release")
                .join(library),
        );
    }

    let out = Path::new("target/universal").join(library);
    std::fs::create_dir_all("target/universal")?;
    run(Command::new("lipo")
        .arg("-create")
        .args(&slices)
        .arg("-output")
        .arg(&out))?;

    Ok(out)
}
