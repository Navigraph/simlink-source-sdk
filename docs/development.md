# Development

How to build, test and release the SDK. For how the code fits together, see [architecture.md](architecture.md).

## Prerequisites

- **Rust**, via [rustup](https://rustup.rs). `rust-toolchain.toml` pins the toolchain, and rustup installs it on first
  use.
- **CMake** 3.20+ and a C++20 compiler, for the C++ examples. On Windows, MSVC.
- **.NET SDK** 10, for the C# examples.

`cargo test` only needs Rust. The first `cargo xtask build` also installs the UniFFI generators into
`target/tools`, which takes a few minutes; later builds reuse them.

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
Linux, and a check on the oldest supported Rust (see [MSRV](#msrv)).

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

`protocol/source-schema.json` arrives as pull requests from the Simlink repository; see `protocol/README.md`. Don't
edit it by hand.

The types and `Source` methods follow the manifest on the next build. A new channel also needs its types listed in
`crates/simlink-source-sdk/tests/schema.rs`, which compares its list against the manifest's channels and fails until
they match. A new event or command should usually get an example too.

The macro fails the build on anything in the manifest the SDK doesn't support yet, with an error pointing at it.

### Updating UniFFI or a generator

Each generator release targets one UniFFI minor version. Bump `uniffi` in the workspace `Cargo.toml` and both pins in
`xtask/src/generators.rs` together. Changing that file also invalidates CI's cache of the installed generators.

## Testing

The tests trust what is generated and check what is hand-written:

- **The generated types match the manifest.** `crates/simlink-source-sdk/tests/schema.rs` builds samples of every
  channel's payloads and the handshake descriptor from `protocol/source-schema.json`, round-trips them through the
  Rust types, and checks the JSON comes back the same and still valid. A channel added to the manifest fails the test
  until its types are listed.
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

## MSRV

`rust-version` in the workspace `Cargo.toml` is the oldest Rust the crates build on. It's set by what the
dependencies need, and is separate from the toolchain pinned in `rust-toolchain.toml`. CI checks the workspace on
that version. Cargo resolves dependencies to versions that support it, so `cargo update` keeps it where it is.
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
- **Once all platforms pass**, `cargo xtask package csharp --runtimes` puts every platform's library into one NuGet
  package and attaches all of the archives to the release.

Each package task builds and runs the examples from its own archive, so a broken archive fails the release before it's
published.

To build a release archive locally, run the same commands. `cargo xtask package csharp` without `--runtimes` packages
only the current platform's library.
