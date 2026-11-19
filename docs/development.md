# Development

How to build, test and release the SDK. For how the code fits together, see [architecture.md](architecture.md).

## Prerequisites

- **Rust**, via [rustup](https://rustup.rs). `rust-toolchain.toml` pins the toolchain, and rustup installs it on first
  use.
- **CMake** 3.20+ and a C++20 compiler, for the C++ examples. On Windows, MSVC.
- **.NET SDK**, the exact version in `global.json`, for the C# examples.

`cargo test` only needs Rust. The first `cargo xtask build` also installs the UniFFI generators into
`target/tools`, which takes a few minutes; later builds reuse them.

### Docker

The `Dockerfile` has everything above for Linux, at the versions CI uses. Prefix any command with
`docker compose run --rm dev`:

```sh
docker compose run --rm dev cargo xtask examples
```

The repository is mounted into the container, while its `target` folder, the Cargo registry and NuGet packages live in
Docker volumes, so Linux builds don't mix with the host's and later runs reuse them. It builds and tests Linux only;
CI still builds and tests on Windows and macOS, and releases are built there. `docker compose down -v` deletes the
volumes.

## Commands

```sh
cargo test --workspace      # unit, schema round-trip and source tests
cargo xtask build           # libraries into target/release, C++ and C# bindings into target/bindings
cargo xtask examples        # build every example and check each joins a test Simlink
cargo xtask docs [--check]  # copy example code into the README's snippets
cargo xtask package ...     # release archives; see Releasing
```

Run one example against a real Simlink with `cargo run -p simlink-source-sdk --example quickstart`. The
`diagnostics` example prints the SDK's logs, connection changes and rejections, which helps when a source won't
connect.

CI (`.github/workflows/ci.yml`) runs the tests and the examples on Windows, Linux and macOS, the snippet check on
Linux, and a check on the oldest supported Rust (see [MSRV](#msrv)). `docker.yml` runs the tests and examples in the
Docker image whenever something it installs changes.

## Common changes

### Changing the API

Exported items in `crates/simlink-source-sdk/src` carry UniFFI attributes, which the bindings are generated from.
After a change:

1. Run `cargo xtask build` to regenerate the bindings.
2. Update the examples in all three languages, so each still shows the API it covers.
3. Run `cargo xtask docs` to copy the examples into the README.
4. Run `cargo xtask examples`.

Generated names follow each language's conventions, so check them in `target/bindings` before writing the examples.
Watch for collisions: `Error` is exported as `SimlinkError`, since the C# generator would otherwise name it
`Exception`.

### Protocol updates

`protocol/source-schema.json` arrives as pull requests from the Simlink repository. Don't edit it by hand.

The types and `Source` methods follow the manifest on the next build, and the macro fails the build on any key it
neither reads nor ignores on purpose. `protocol/README.md` has the checklist for reviewing an update: what the build
catches, and what it can't.

### Updating UniFFI or a generator

Each generator release targets one UniFFI minor version. Bump `uniffi` in the workspace `Cargo.toml` and both pins in
`xtask/src/generators.rs` together. Generators are pinned to commits, since tags can move. Changing that file also
invalidates CI's cache of the installed generators.

### Updating pinned tools

The tools a release is built with are pinned, so the same commit builds the same output. Renovate updates the
first three; the rest are bumped by hand, since each sets the oldest OS a release runs on or targets one UniFFI
version.

| What | Where |
| --- | --- |
| GitHub Actions | commit hashes in `.github/workflows` |
| Rust | `rust-toolchain.toml` |
| .NET SDK | `global.json`, which the `Dockerfile` installs too |
| CI runners, MSVC toolset | `runs-on` and the `Pin the MSVC toolset` step in `.github/workflows` |
| Oldest macOS supported | `MACOSX_DEPLOYMENT_TARGET` in `.cargo/config.toml` |
| Oldest glibc supported | `GLIBC` in `xtask/src/package/library.rs` |
| Linux release tools: zig, cargo-zigbuild | `.github/workflows/release.yml` |
| UniFFI generators | `xtask/src/generators.rs` |

The macOS target and glibc version follow [Supported platforms](#supported-platforms). The Linux library is built
against that glibc with `cargo zigbuild`. Windows releases need the MSVC runtime of the pinned
toolset. Windows runners update Visual Studio within a toolset, so its minor version isn't pinned; a new toolset fails
the build until the step is updated. The `Dockerfile` follows the Linux runner's Ubuntu version.

## Testing

The tests trust what is generated and check what is hand-written:

- **The generated types match the manifest.** `crates/simlink-source-sdk/tests/schema.rs` builds samples of every
  channel's payloads and the handshake descriptor from `protocol/source-schema.json`, round-trips them through the
  Rust types, and checks the JSON comes back the same and still valid. A channel added to the manifest fails the test
  until its types are listed.
- **The Socket.IO client follows the protocol and the reference client.** In `crates/socket-io`,
  `tests/protocol.rs` ports the official [Engine.IO](https://github.com/socketio/engine.io-protocol) and
  [Socket.IO](https://github.com/socketio/socket.io-protocol) test suites to the client's side: a scripted server
  sends the reference server's frames and checks the client's replies, and each test links the one it ports.
  `tests/client.rs` checks reconnection and the namespace's lifecycle against what socket.io-client does.
- **The connection behaves.** `crates/simlink-source-sdk/tests/source.rs` runs the SDK against a fake Simlink in
  `crates/simlink-source-sdk/tests/support`: handshakes and refusals, events and the rate limit, commands answered
  and refused, rejections, heartbeats, and reconnects.
- **The bindings work.** `cargo xtask examples` builds every Rust, C++ and C# example against the generated bindings,
  then `crates/simlink-source-sdk/tests/examples.rs` runs each one against the same fake Simlink, listening on
  Simlink's default port so the unmodified examples find it by discovery. Each must join, emit valid payloads on the
  events it declares, answer the commands it handles, and survive a rejection, which the diagnostics examples must
  print with its issues. Close Simlink first: it would hold that port.

The fake Simlink accepts any payload; the tests validate what it receives against the manifest. Its Socket.IO codec
is written separately from the SDK's, so one misreading of the protocol can't pass on both sides.

The connection tests point the SDK at the fake Simlink through `Source::set_address`, which only exists with the
crate's `testing` feature. Run one test file with `cargo test -p simlink-source-sdk --test source`.

## Supported platforms

The SDK supports every OS that Rust supports at the [MSRV](#msrv), and nothing older, since Rust's standard library
can't run there. A higher floor would only shut users out.

| Platform | Minimum | Set by |
| --- | --- | --- |
| Windows | Windows 10 or Server 2016 | Rust itself |
| Linux | glibc 2.17 | `GLIBC` in `xtask/src/package/library.rs` |
| macOS | 10.12 on Intel, 11 on Apple silicon | `MACOSX_DEPLOYMENT_TARGET` in `.cargo/config.toml` |

The language requirements come from elsewhere:

- **C++20**, because that's what `uniffi-bindgen-cpp` generates. The bindings ship as source, so users compile them
  with their own compiler.
- **C#**: the package targets .NET Standard 2.0.
- **Rust**: the MSRV.

When the MSRV moves, check [Rust's platform support](https://doc.rust-lang.org/rustc/platform-support.html) for the
new version, and update the table above, the pins it names, and the README's requirements.

## MSRV

`rust-version` in the workspace `Cargo.toml` is the oldest Rust the crates build on: the newest release that's at least
six months old, moved forward by hand. It's separate from the toolchain pinned in `rust-toolchain.toml`, and it sets
the oldest OS the SDK supports (see [Supported platforms](#supported-platforms)). CI checks the workspace on that
version. Cargo resolves dependencies to versions that support it, so `cargo update` keeps it where it is.
Bumping a dependency to a version that needs newer Rust fails the MSRV job.

## Releasing

1. Bump `version` in the workspace `Cargo.toml` and merge it.
2. Tag the merge commit with that version and push the tag:

   ```sh
   git tag v1.0.0
   git push origin v1.0.0
   ```

The tag starts `.github/workflows/release.yml`, which creates a GitHub release with the C++ and C# archives and
generated release notes. A version with a `-`, such as `v1.0.0-rc1`, is published as a prerelease. The workflow fails
if the tag doesn't match the version in `Cargo.toml`.

The workflow runs in two stages:

- **On each platform**, it runs the tests and examples, then `cargo xtask package cpp` and
  `cargo xtask package csharp-runtime`.
- **Once all platforms pass**, `cargo xtask package csharp` puts every platform's library into one NuGet
  package and attaches all of the archives to the release.

Each package task builds and runs the examples from its own archive, so a broken archive fails the release before it's
published.

To build a release archive locally, run the same commands. On Linux they need zig and `cargo-zigbuild`, at the versions
`release.yml` installs. `cargo xtask package csharp` packs the libraries `package csharp-runtime` left in
`target/package/runtimes`, so locally run that first.
