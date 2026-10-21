# Development

## Commands

```sh
cargo test --workspace      # unit, schema round-trip and session tests
cargo xtask build           # libraries into target/release, C++ and C# bindings into target/bindings
cargo xtask examples        # build every example and check each joins a test Simlink
cargo xtask docs [--check]  # copy example code into the README's snippets
```

CI (`.github/workflows/ci.yml`) runs the tests and the examples on Windows, Linux and macOS, and the snippet check on
Linux.

## Testing

The tests trust what is generated and check what is hand-written:

- **The generated types match the manifest.** `crates/simlink/tests/schema.rs` builds samples of every channel's
  payloads and the handshake descriptor from `protocol/source-schema.json`, round-trips them through the Rust types,
  and checks the JSON comes back the same and still valid. A channel added to the manifest fails the test until its
  types are listed.
- **The connection behaves.** `crates/simlink/tests/session.rs` runs the SDK against a minimal Simlink in
  `crates/simlink/tests/support`: handshakes and refusals, events and the rate limit, commands answered now or later,
  rejections, heartbeats, and reconnects.
- **The bindings work.** `cargo xtask examples` builds every Rust, C++ and C# example against the generated bindings,
  then `crates/simlink/tests/examples.rs` runs each one against the same minimal Simlink, listening on Simlink's
  default port so the unmodified examples find it by discovery. Each must join, emit valid payloads on the events it
  declares, answer the commands it handles, and survive a rejection, which the diagnostics examples must print with
  its issues. Close Simlink first: it would hold that port.

The minimal Simlink accepts any payload; the tests validate what it receives against the manifest. Its Socket.IO
codec is written separately from the SDK's, so one misreading of the protocol can't pass on both sides.

## Protocol updates

`protocol/source-schema.json` arrives as pull requests from the Simlink repository; see `protocol/README.md`.
