//! Repository tasks: `cargo xtask <task>`.

mod bindings;
mod docs;
mod examples;
mod generators;
mod package;

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(bin_name = "cargo xtask", about = "Repository tasks")]
struct Cli {
    #[command(subcommand)]
    task: Task,
}

#[derive(Subcommand)]
enum Task {
    /// Build the libraries into target/release and the C++ and C# bindings into target/bindings
    Build,
    /// Build every example, then check that each joins a test Simlink and answers it
    Examples,
    /// Copy example code into the README's snippets
    Docs {
        /// Fail instead of writing when the snippets are stale
        #[arg(long)]
        check: bool,
    },
    /// Build a release archive into target/package
    #[command(subcommand)]
    Package(Package),
}

#[derive(Subcommand)]
enum Package {
    /// This platform's C++ archive
    Cpp,
    /// This platform's library, for `package csharp --runtimes` on one machine
    CsharpRuntime,
    /// The C# archive
    Csharp {
        /// Holds `<rid>/native/<library>` for each platform; without it, only this platform's is packed
        #[arg(long)]
        runtimes: Option<PathBuf>,
    },
}

fn main() -> Result<()> {
    let task = Cli::parse().task;
    std::env::set_current_dir(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .context("workspace root")?,
    )?;

    match task {
        Task::Build => bindings::build(),
        Task::Examples => examples::check(),
        Task::Docs { check } => docs::update(check),
        Task::Package(Package::Cpp) => package::cpp::package(),
        Task::Package(Package::CsharpRuntime) => package::csharp::runtime(),
        Task::Package(Package::Csharp { runtimes }) => {
            package::csharp::package(runtimes.as_deref())
        }
    }
}

/// `cargo <command> --locked`, building into `target`, where CMake and the C# project look for the libraries.
fn cargo(command: &str) -> Command {
    let mut cargo = Command::new("cargo");
    cargo
        .args([command, "--locked"])
        .env("CARGO_TARGET_DIR", "target");
    cargo
}

fn executable(name: &str) -> String {
    format!("{name}{}", std::env::consts::EXE_SUFFIX)
}

fn run(command: &mut Command) -> Result<()> {
    let status = command
        .status()
        .with_context(|| format!("running {command:?}"))?;
    if !status.success() {
        bail!("{command:?} failed with {status}");
    }

    Ok(())
}
