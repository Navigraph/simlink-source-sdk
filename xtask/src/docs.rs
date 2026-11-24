//! README snippets copied from the examples, so the docs can't drift from code that builds.

use anyhow::{Context, Result, bail};

const READMES: [&str; 3] = [
    "README.md",
    "docs/readme-csharp.md",
    "docs/readme-rust.md",
];

/// Replaces the fenced block after each `<!-- snippet: <path> -->` in each README with that file.
pub fn update(check: bool) -> Result<()> {
    let mut stale = Vec::new();
    for readme in READMES {
        let current = std::fs::read_to_string(readme)?.replace("\r\n", "\n");
        let fresh = with_snippets(&current).with_context(|| format!("updating {readme}"))?;
        if fresh == current {
            continue;
        }
        if check {
            stale.push(readme);
        } else {
            std::fs::write(readme, fresh)?;
            println!("Updated {readme}");
        }
    }

    if !stale.is_empty() {
        bail!("{} snippets are stale; run `cargo xtask docs`", stale.join(", "));
    }
    Ok(())
}

fn with_snippets(current: &str) -> Result<String> {
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
    Ok(fresh)
}
