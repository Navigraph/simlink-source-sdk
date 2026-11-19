# Protocol manifest

`source-schema.json` is Simlink's source manifest. Simlink generates it, and a workflow in the Simlink repository opens
a pull request here whenever it changes. Don't edit it by hand: the next update would overwrite the change.

## Reviewing an update

The build catches most of an update. The `protocol!` macro generates the API from the manifest and fails on a key the
SDK neither reads nor ignores on purpose, and on a missing field the SDK reads by name. The core builds the
capabilities and parses error frames through generated types, so a change to either fails to compile.
`crates/simlink-source-sdk/tests/schema.rs` fails until a new channel's types are listed, and CI runs every example
against the fake Simlink and checks the README's snippets.

What's left for the reviewer:

- [ ] **Build failures.** Each macro error names what the SDK doesn't handle. An unhandled key is read in the macro,
  or declared as an `Ignored` field with the reason. Anything else, such as a channel Simlink originates, needs the
  SDK extended before the update can merge.
- [ ] **New channels.** List their types in `tests/schema.rs`.
- [ ] **Generated names.** Run `cargo xtask build` and check new names in `target/bindings`. The C# generator renames
  some, such as `Error` to `Exception`, and a name can clash with one the SDK exports.
- [ ] **Examples.** A new event or command gets an example, or extends one, in Rust, C++ and C#. A changed or
  deprecated channel, field or code is updated in all three. Then run `cargo xtask docs` to copy them into the README.
- [ ] **Docs.** Update anything that names a channel, field or code, or describes what the manifest generates:
  - `README.md`: "What it does", "Using it" and "Examples".
  - `docs/architecture.md`: what the macro generates, and the worker's handling of handshake errors.
  - `docs/overview.md`: the SDK's scope, if the update changes what it covers.
- [ ] **New handshake errors.** Decide whether the source should keep retrying. Only `HandshakeIdentificationInvalid`
  stops it, in `crates/simlink-source-sdk/src/worker.rs`.
- [ ] **Breaking changes.** A renamed or removed channel, field or code, or a new required descriptor field, breaks
  integrators' code. Bump the version to match, and say so in the release notes.
- [ ] **A new `protocolVersion`.** Discovery then only finds Simlinks serving that major, so release it as a breaking
  change.
- [ ] **Changed behavior.** The macro reads settings by name, not their meaning. If the handshake, framing, port
  probing or mDNS TXT format changes, update `crates/simlink-source-sdk/src/discovery.rs` or the worker to match, and
  the fake Simlink in `crates/simlink-source-sdk/tests/support`, or the tests keep passing against the old behavior.
