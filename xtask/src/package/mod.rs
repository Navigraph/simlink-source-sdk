//! Release archives, one module per language. Each is staged in `target/package/<name>`, zipped next to it, and
//! tested by building its examples from the staged copy.

pub mod cpp;
pub mod csharp;
mod library;

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::run;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const OUT: &str = "target/package";

/// A release archive's staging folder.
struct Archive {
    name: String,
    root: PathBuf,
}

impl Archive {
    /// Starts `target/package/<name>` empty.
    fn new(name: String) -> Result<Archive> {
        let root = Path::new(OUT).join(&name);
        if root.exists() {
            std::fs::remove_dir_all(&root)?;
        }
        std::fs::create_dir_all(&root)?;

        Ok(Archive { name, root })
    }

    fn path(&self, path: &str) -> PathBuf {
        self.root.join(path)
    }

    /// Copies `file` into the archive's `dir`.
    fn add(&self, file: impl AsRef<Path>, dir: &str) -> Result<()> {
        let file = file.as_ref();
        let dir = self.path(dir);
        std::fs::create_dir_all(&dir)?;
        std::fs::copy(file, dir.join(file.file_name().context("no file name")?))
            .with_context(|| format!("copying {}", file.display()))?;

        Ok(())
    }

    /// Copies `file` to the archive's `to`, replacing each pattern, which must be present.
    fn add_rewritten(&self, file: &str, to: &str, replacements: &[(&str, &str)]) -> Result<()> {
        let mut text = std::fs::read_to_string(file)?;
        for (pattern, replacement) in replacements {
            if !text.contains(pattern) {
                bail!("{file} no longer contains {pattern:?}");
            }
            text = text.replace(pattern, replacement);
        }
        let to = self.path(to);
        std::fs::create_dir_all(to.parent().context("no parent")?)?;
        std::fs::write(to, text)?;

        Ok(())
    }

    /// Zips the archive into `target/package/<name>.zip`, with `<name>` as its top folder.
    fn zip(&self) -> Result<()> {
        let zip = format!("{}.zip", self.name);
        let _ = std::fs::remove_file(Path::new(OUT).join(&zip));
        run(Command::new("cmake")
            .args(["-E", "tar", "cf", &zip, "--format=zip", &self.name])
            .current_dir(OUT))
    }

    /// Where to build the archive's examples, outside it so the zip stays clean.
    fn build_dir(&self) -> PathBuf {
        Path::new(OUT).join("build").join(&self.name)
    }
}
