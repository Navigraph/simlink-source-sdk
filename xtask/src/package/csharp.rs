//! `simlink-csharp-<version>.zip`: the `Navigraph.Simlink` NuGet package, with every platform's library under
//! `runtimes/<rid>/native`, and the examples restoring it from the archive's folder.
//!
//! One package serves every platform, so each builds its library with `runtime` and one machine packs them all.

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};

use super::library::Library;
use super::{Archive, VERSION};
use crate::examples::{self, EXAMPLES, dotnet, pascal};
use crate::{executable, run};

/// Lets the examples restore the package from the archive's own folder.
const NUGET_CONFIG: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<configuration>
  <packageSources>
    <add key="simlink" value="." />
  </packageSources>
</configuration>
"#;

/// Where `runtime` leaves this platform's library.
const RUNTIMES: &str = "target/package/runtimes";

/// Builds this platform's library into `target/package/runtimes/<rid>/native`, as the package lays it out.
pub fn runtime() -> Result<()> {
    add_runtime(&Library::build()?)
}

fn add_runtime(library: &Library) -> Result<()> {
    let native = Path::new(RUNTIMES).join(library.rid).join("native");
    std::fs::create_dir_all(&native)?;
    let name = library
        .dynamic_library
        .file_name()
        .context("no file name")?;
    std::fs::copy(&library.dynamic_library, native.join(name))?;

    Ok(())
}

/// Packs the libraries in `runtimes`, or only this platform's without it, and tests the examples against the package.
pub fn package(runtimes: Option<&Path>) -> Result<()> {
    // Also generates the C# bindings the package compiles.
    let library = Library::build()?;
    let runtimes = match runtimes {
        Some(runtimes) => runtimes,
        None => {
            add_runtime(&library)?;
            Path::new(RUNTIMES)
        }
    };

    let archive = Archive::new(format!("simlink-csharp-{VERSION}"))?;
    run(Command::new("dotnet")
        .args([
            "pack",
            "bindings/csharp/Navigraph.Simlink.csproj",
            "-c",
            "Release",
            "-o",
        ])
        .arg(archive.path(""))
        .arg(format!("-p:Version={VERSION}"))
        .arg(format!(
            "-p:SimlinkRuntimes={}",
            std::path::absolute(runtimes)?.display()
        )))?;
    std::fs::write(archive.path("nuget.config"), NUGET_CONFIG)?;
    archive.add_rewritten(
        "examples/csharp/Directory.Build.props",
        "examples/Directory.Build.props",
        &[
            (
                "the bindings from `cargo xtask build`",
                "the Navigraph.Simlink package",
            ),
            (
                r#"<ProjectReference Include="$(MSBuildThisFileDirectory)../../bindings/csharp/Navigraph.Simlink.csproj" />"#,
                &format!(r#"<PackageReference Include="Navigraph.Simlink" Version="{VERSION}" />"#),
            ),
        ],
    )?;
    for example in EXAMPLES {
        let project = pascal(example);
        for file in ["Program.cs".to_owned(), format!("{project}.csproj")] {
            archive.add(
                format!("examples/csharp/{project}/{file}"),
                &format!("examples/{project}"),
            )?;
        }
    }
    archive.add("LICENSE", "")?;
    // Before building the examples, which leaves obj folders next to them.
    archive.zip()?;

    let mut programs = Vec::new();
    for example in EXAMPLES {
        let project = pascal(example);
        let out = archive.build_dir().join(&project);
        dotnet(
            &archive.path(&format!("examples/{project}/{project}.csproj")),
            &out,
        )?;
        programs.push(out.join(executable(&project)));
    }
    examples::test(&programs)
}
