//! README snippets copied from the examples, so the docs can't drift from code that builds.

use anyhow::{Context, Result, bail};

const README: &str = "README.md";

/// Replaces the fenced block after each `<!-- snippet: <path> -->` in the README with that file.
pub fn update(check: bool) -> Result<()> {
    let current = std::fs::read_to_string(README)?.replace("\r\n", "\n");

    let mut fresh = String::new();
    let mut lines = current.lines();
    while let Some(line) = lines.next() {
        fresh.push_str(line);
        fresh.push('\n');
        let Some(path) = line
            .strip_prefix("<!-- snippet: ")
            .and_then(|rest| rest.strip_suffix(" -->"))
        else {
            continue;
        };

        let fence = lines
            .next()
            .filter(|fence| fence.starts_with("```"))
            .context("a fenced block after the snippet")?;
        while lines.next().is_some_and(|line| line != "```") {}
        let code = std::fs::read_to_string(path).with_context(|| format!("reading {path}"))?;
        fresh.push_str(&format!(
            "{fence}\n{}\n```\n",
            code.replace("\r\n", "\n").trim_end()
        ));
    }

    if fresh == current {
        return Ok(());
    }
    if check {
        bail!("{README} snippets are stale; run `cargo xtask docs`");
    }
    std::fs::write(README, fresh)?;
    println!("Updated {README}");

    Ok(())
}
