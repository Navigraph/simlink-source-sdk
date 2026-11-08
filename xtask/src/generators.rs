//! The binding generators `build` runs. CI caches their installs on this file's hash.

pub struct Generator {
    pub name: &'static str,
    pub repository: &'static str,
    /// Each generator targets one UniFFI minor version, which the workspace pins to match.
    pub pin: Pin,
    pub args: &'static [&'static str],
    /// Under `target/bindings`.
    pub out: &'static str,
}

/// The git ref `cargo install` builds.
pub enum Pin {
    Tag(&'static str),
    Rev(&'static str),
}

impl Pin {
    pub fn args(&self) -> [&'static str; 2] {
        match self {
            Pin::Tag(tag) => ["--tag", tag],
            Pin::Rev(rev) => ["--rev", rev],
        }
    }
}

pub const GENERATORS: [Generator; 2] = [
    Generator {
        name: "uniffi-bindgen-cs",
        // Our fork's main: v0.11.1+v0.31.0 plus fixes for sibling variant collisions, doc comment warnings, and IL2CPP
        // callbacks.
        repository: "https://github.com/Navigraph/uniffi-bindgen-cs",
        pin: Pin::Rev("55b78d9df5ad36a9b50d4d1173a73b2248bf57e6"),
        // Without `--no-format` the output depends on whether CSharpier is installed.
        args: &[
            "--config",
            "crates/simlink-source-sdk/uniffi.toml",
            "--no-format",
        ],
        out: "csharp",
    },
    Generator {
        name: "uniffi-bindgen-cpp",
        repository: "https://github.com/NordSecurity/uniffi-bindgen-cpp",
        pin: Pin::Tag("v0.10.0+v0.31.0"),
        // Library mode rejects `--config`; the C++ namespace defaults to the crate name.
        args: &[],
        out: "cpp",
    },
];
