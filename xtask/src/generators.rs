//! The binding generators `build` runs. CI caches their installs on this file's hash.

pub struct Generator {
    pub name: &'static str,
    pub repository: &'static str,
    /// The commit `cargo install` builds. Each generator targets one UniFFI minor version, which the workspace pins
    /// to match.
    pub rev: &'static str,
    pub args: &'static [&'static str],
    /// Under `target/bindings`.
    pub out: &'static str,
}

pub const GENERATORS: [Generator; 2] = [
    Generator {
        name: "uniffi-bindgen-cs",
        // Our fork's main: v0.11.1+v0.31.0 plus fixes for sibling variant collisions, doc comment warnings, and IL2CPP
        // callbacks.
        repository: "https://github.com/Navigraph/uniffi-bindgen-cs",
        rev: "55b78d9df5ad36a9b50d4d1173a73b2248bf57e6",
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
        rev: "c592133cac069d4e870744c1cdf09558b148c9fc",
        // Library mode rejects `--config`; the C++ namespace defaults to the crate name.
        args: &[],
        out: "cpp",
    },
];
