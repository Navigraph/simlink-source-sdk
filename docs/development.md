# Development

How to build, test, and release the SDK. For how the code fits together, see [architecture.md](architecture.md).

## Prerequisites

- **Rust** via [rustup](https://rustup.rs) (`rust-toolchain.toml` pins the toolchain and installs automatically).
- **CMake** 3.20+ and a C++20 compiler, for the C++ examples (MSVC on Windows).
- **.NET SDK**, the exact version in `global.json`, for the C# examples.

`cargo test` only needs Rust. The first `cargo xtask build` also installs the UniFFI generators into `target/tools`, which takes a few minutes; subsequent builds reuse them.

### Docker

The `Dockerfile` includes the Linux toolchain at the versions CI uses. Prefix any command with `docker compose run --rm dev`:

```sh
docker compose run --rm dev cargo xtask examples
```

The repo is mounted into the container while `target/`, the Cargo registry, and NuGet packages live in Docker volumes so Linux builds stay isolated from the host (`docker compose down -v` deletes the volumes). Docker covers Linux only; Windows and macOS are built and tested in CI.

## Commands

```sh
cargo test --workspace      # unit, schema round-trip, and source tests
cargo xtask build           # libraries in target/release, C++/C# bindings in target/bindings
cargo xtask examples        # build all examples and run them against a test Simlink
cargo xtask docs [--check]  # sync example code into README snippets
cargo xtask package ...     # build release archives (see Releasing)
```

To run an example against a live Simlink (e.g. `quickstart`, or `diagnostics` to inspect logs, connection state, and rejections):

```sh
# Rust
cargo run --release -p simlink-source-sdk --example quickstart

# C++ (after `cargo xtask build`)
cmake -S examples/cpp -B target/examples/cpp -DCMAKE_BUILD_TYPE=Release
cmake --build target/examples/cpp --config Release
target/examples/cpp/bin/quickstart

# C# (after `cargo xtask build`)
dotnet run -c Release --project examples/csharp/Quickstart
```

CI (`.github/workflows/ci.yml`) runs tests and examples on Windows, Linux, and macOS, the README snippet check on Linux, and an [MSRV](#msrv) check. `.github/workflows/docker.yml` verifies the Docker image whenever its inputs change.

## Common changes

### Changing the API

Exported items in `crates/simlink-source-sdk/src` carry UniFFI attributes used to generate the bindings. After a change:

1. Run `cargo xtask build` to regenerate the bindings.
2. Update the examples in all three languages.
3. Run `cargo xtask docs` to update the README snippets.
4. Run `cargo xtask examples`.

Check generated names in `target/bindings` before updating examples, and watch for collisions (for example, `Error` is exported as `SimlinkError` because the C# generator would otherwise name it `Exception`).

### Protocol updates

`crates/simlink-source-sdk/protocol/source-schema.json` arrives via PRs from the Simlink repository — don't edit it by hand.

Types and `Source` methods update automatically on the next build, and the macro fails the build on any unrecognized key. See `crates/simlink-source-sdk/protocol/README.md` for the review checklist.

### Updating UniFFI or a generator

Each generator release targets one UniFFI minor version. Bump `uniffi` in the workspace `Cargo.toml` and both commit pins in `xtask/src/generators.rs` together (commits are pinned because tags can move; editing that file also invalidates CI's generator cache).

### Updating pinned tools

Release tools are pinned for reproducible builds. Renovate updates the first three; the rest are updated manually since they control minimum OS support or UniFFI compatibility.

| What | Where |
| --- | --- |
| GitHub Actions | Commit hashes in `.github/workflows/` |
| Rust | `rust-toolchain.toml` |
| .NET SDK | `global.json` (also used by `Dockerfile`) |
| CI runners & MSVC toolset | `runs-on` and `Pin the MSVC toolset` in `.github/workflows/` |
| Minimum macOS | `MACOSX_DEPLOYMENT_TARGET` in `.cargo/config.toml` |
| Minimum glibc | `GLIBC` in `xtask/src/package/library.rs` |
| Linux release tools (zig, cargo-zigbuild) | `.github/workflows/release.yml` |
| UniFFI generators | `xtask/src/generators.rs` |

Notes:
- The Linux release library is cross-linked against the pinned glibc via `cargo zigbuild`.
- Windows releases require the MSVC runtime matching the pinned toolset; runner image updates that bump the toolset will fail the build until the pin is updated.
- `Dockerfile` tracks the Linux runner's Ubuntu version.

## Testing

Tests verify the generated types and all hand-written runtime code:

- **Schema round-trip** (`crates/simlink-source-sdk/tests/schema.rs`): builds samples of every channel payload and the handshake descriptor from the manifest, round-trips them through the generated Rust types, and verifies the JSON matches and remains valid. New channels fail until covered.
- **Socket.IO client** (`crates/simlink-source-sdk/src/socket_io/tests/`): `protocol.rs` ports the official [Engine.IO](https://github.com/socketio/engine.io-protocol) and [Socket.IO](https://github.com/socketio/socket.io-protocol) server test suites to the client side; `client.rs` tests reconnection and namespace lifecycle against `socket.io-client` behavior.
- **Source behavior** (`crates/simlink-source-sdk/tests/source.rs`): runs the SDK against a fake Simlink (`tests/support`) covering handshakes, refusals, events, rate limiting, commands, rejections, heartbeats, and reconnects. Run individually with `cargo test -p simlink-source-sdk --test source`.
- **Bindings & examples** (`cargo xtask examples` / `tests/examples.rs`): builds all C++, C#, and Rust examples against the generated bindings and runs each against the fake Simlink on its default port (close any real Simlink first).

The fake Simlink validates received payloads against the manifest using an independent Socket.IO codec so protocol bugs can't pass on both sides. Connection tests point at it via `Source::set_address`, gated behind the `testing` Cargo feature.

## Supported platforms

The SDK targets the minimum OS versions supported by Rust at its [MSRV](#msrv):

| Platform | Minimum | Set by |
| --- | --- | --- |
| Windows | Windows 10 / Server 2016 (x64) | Rust |
| Linux | x64, glibc 2.17 | `GLIBC` in `xtask/src/package/library.rs` |
| macOS | 10.12 (Intel), 11 (Apple silicon) | `MACOSX_DEPLOYMENT_TARGET` in `.cargo/config.toml` |

Language minimums: **C++20** (generated by `uniffi-bindgen-cpp`; shipped as source), **.NET Standard 2.0** for C#, and the workspace **MSRV** for Rust.

When bumping the MSRV, check [Rust's platform support](https://doc.rust-lang.org/rustc/platform-support.html) and update this table, the pins above, and the platform line in `README.md`.

## MSRV

`rust-version` in the workspace `Cargo.toml` is the oldest supported Rust version (the newest release at least six months old, bumped manually). It is separate from `rust-toolchain.toml` and is verified by the `msrv` job in CI. Cargo respects it during `cargo update`, and any dependency requiring a newer Rust will fail CI.

## Releasing

`.github/workflows/release-plz.yml` (configured in `release-plz.toml`) keeps a Release PR open on `main` with the next version bump in `Cargo.toml`/`Cargo.lock` and updated `CHANGELOG.md`.

To cut a release, merge the Release PR. `release-plz` creates the `vX.Y.Z` tag and GitHub Release with the changelog, which triggers `.github/workflows/release.yml` (you can also bump `Cargo.toml` and push a `v*` tag manually for prereleases like `v1.0.0-rc.4`). Stages run sequentially so a failure publishes nothing:

1. **Per platform**: runs tests and examples, then `cargo xtask package cpp` and `cargo xtask package csharp-runtime` (each package task also builds and runs the examples from its own archive).
2. **Release**: `cargo xtask package csharp` combines all platform runtimes into one NuGet package and attaches all archives to the GitHub release.
3. **Publish**: publishes `Navigraph.Simlink.SourceSdk` to NuGet and `simlink-source-sdk` / `simlink-source-sdk-macros` to crates.io via trusted publishing (no stored API keys). Neither registry allows replacing a published version; rerunning skips NuGet duplicates but fails on already-published crates.
4. **vcpkg** (stable releases only): dispatches an update to [Navigraph/vcpkg-registry](https://github.com/Navigraph/vcpkg-registry) using `BOT_TOKEN`.

To build release archives locally, run the same `cargo xtask package` commands (`package csharp-runtime` before `package csharp`). Linux requires `zig` and `cargo-zigbuild` at the versions in `release.yml`.
