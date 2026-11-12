# Simlink Source SDK Overview

This doc explains what this repo is, why it exists, why it's built the way it is, and what it takes to maintain. For how to work in it, see [development.md](development.md). For how the code fits
together, see [architecture.md](architecture.md).

## What?

Simlink Source SDK gives third-party developers an easy way to add support for the Simlink v2 source protocol to
their apps. A "source" is anything that feeds Simlink data: a simulator, an add-on, or a tool.

The SDK covers the whole protocol, both the data and the connection:

- **The data.** Every event payload, command input, output and error is a typed struct or enum, generated from the
  latest schema on Simlink's main branch.
- **The connection.** The SDK finds Simlink (via mDNS on the network, or by probing ports on this machine), joins
  with the right handshake, reconnects whenever Simlink restarts, rate limits each event, answers Simlink's
  commands, and reports rejected payloads back.

Integrators just use the SDK: fill in the structs, emit events, and implement handlers for commands. They never touch
Socket.IO, JSON, or the schema.

## Why?

The protocol started out as a spec only. Early integrations of it had gaps, mainly around connecting to Simlink.
Every integration has to get the same details right (discovery, port probing, reconnects, acks), and
they are easy to get slightly wrong.

An SDK means those details are implemented once, tested in one place, and work the same for everyone. Integrators can
focus on their own data.

## Which languages?

C++ and C# are the languages integrators have asked for so far. Rust comes for free, since the core is Rust.

| Language | Shipped as                   |
|----------|------------------------------|
| C++      | vcpkg, from our own registry |
| C#       | NuGet                        |
| Rust     | crates.io                    |

If someone wants support for another language, it should be easy to add the bindings generator for it (see
[Adding a language](#adding-a-language)).

## How?

We want to support many languages without duplicating code. The SDK has two parts:

- **The core**: connection, discovery, rate limiting and commands. It's written once, in Rust.
- **The protocol API**: the types and the typed method for each event and command. These are generated from the
  schema by the `protocol!` macro, so they always match it.

Both are exposed to each language through bindings. We could maintain the bindings by hand, but they would drift from
the Rust code and become a maintenance issue, so they're generated automatically too.

```
protocol/source-schema.json   synced from the Simlink repo by a PR whenever it changes
  └─ protocol!() macro        generates the protocol types and typed methods per channel
       └─ Rust core           connection, discovery, rate limiting, commands
            ├─ UniFFI + uniffi-bindgen-cpp  →  C++ bindings
            └─ UniFFI + uniffi-bindgen-cs   →  C# bindings
```

Nothing in the C++ or C# bindings is written by hand, and they're never checked in. A protocol update is: merge the
schema PR from Simlink, fix anything the tests flag, cut a release.

### Choosing a bindings generator

Within the Rust ecosystem, these are the main options:

| Tool                                                         | Target languages                                     | Approach                          | Best for                                                |
|--------------------------------------------------------------|------------------------------------------------------|-----------------------------------|---------------------------------------------------------|
| [UniFFI](https://github.com/mozilla/uniffi-rs)               | Swift, Kotlin, Python, Ruby; C#, C++, Go, Dart and more via third-party generators | Automatic high-level bindings | Cross-platform native libraries, especially iOS/Android |
| [Diplomat](https://github.com/rust-diplomat/diplomat)        | C, C++, JS/TS, Dart, C#, Kotlin, Python              | Automatic high-level bindings     | Exposing one Rust library to many languages             |
| [cbindgen](https://github.com/mozilla/cbindgen)              | C, C++                                               | Generates C/C++ headers from Rust | Universal native FFI interoperability                   |
| [wasm-bindgen](https://github.com/wasm-bindgen/wasm-bindgen) | JavaScript, TypeScript                               | WebAssembly bindings              | Browser and web applications                            |
| [CXX](https://github.com/dtolnay/cxx)                        | C++                                                  | Type-safe Rust/C++ bridge         | Native C++ integration                                  |

cbindgen and CXX only cover C/C++, so C# (and any later language) would be hand-written on top. wasm-bindgen is for
the browser. That leaves UniFFI and Diplomat.

Diplomat was prototyped as well. It produces nice header-only C++, but:

- **It can't cover C#.** Its .NET backend doesn't support callbacks, so no command handlers, listeners or logger.
- **It doesn't expand the `protocol!` macro**, so every channel's bindings would be written by hand, and those would
  drift from the Rust code.

UniFFI reads the compiled library, so the macro-generated API is picked up automatically, and its third-party
generators handle callbacks in both C++ and C#.

### The catch: per-language generators

UniFFI only gives you part of the solution. You still need a generator per language on top. From the UniFFI docs,
they recommend [NordSecurity's uniffi-bindgen-cs](https://github.com/NordSecurity/uniffi-bindgen-cs) for C# and
[uniffi-bindgen-cpp](https://github.com/NordSecurity/uniffi-bindgen-cpp) for C++. With those, we just annotate things
with `uniffi` in Rust, and the bindings "just work".

`xtask/src/generators.rs` pins both generators to a version matching our UniFFI version, so bindings don't change
under us.

### Why we have a fork of uniffi-bindgen-cs

The C# generator needed a few fixes for this SDK, so we use [our fork](https://github.com/Navigraph/uniffi-bindgen-cs)
with the fixes. Each one is also upstreamed:

| Fix                                                                  | Upstream PR |
|----------------------------------------------------------------------|-------------|
| Rename variant fields that collide with sibling variant records      | [#190](https://github.com/NordSecurity/uniffi-bindgen-cs/pull/190) |
| Emit doc comments C# accepts                                         | [#191](https://github.com/NordSecurity/uniffi-bindgen-cs/pull/191) |
| XML-escape docstrings in doc comments                                | [#192](https://github.com/NordSecurity/uniffi-bindgen-cs/pull/192) |
| Mark native callbacks for IL2CPP and Mono AOT (needed by Unity's IL2CPP) | [#193](https://github.com/NordSecurity/uniffi-bindgen-cs/pull/193) |

The plan is to switch back to upstream once these are merged. Until then, the fork is pinned to a commit, so it only
changes when we move it.

The alternatives to depending on `uniffi-bindgen-cs` are 1) going back to our own entire bindgen stack (which is very
complicated!) or 2) manually maintaining "vanilla" versions of the SDK for each language. Both are worse than a small
fork.

## Maintenance

### What we own

- The Rust core (`crates/simlink-source-sdk`): about 800 lines
- Our own Socket.IO client (`crates/socket-io`), since no Rust client answers server-initiated acks
- The `protocol!` macro (`crates/simlink-source-sdk-macros`): turns the schema into the Rust API
- `xtask`: builds, packages and tests everything
- The examples in each language, and the README
- The C# fork, until upstream catches up

### What we don't own

- The C++ and C# bindings: generated, never edited
- UniFFI and the generators: pinned versions

### Verifying every language

The SDK covers several languages and runtimes, so its correctness can't depend on someone reviewing C++ or C# by
hand:

- None of the C++ or C# is hand-written except the examples.
- Every example, in every language, is built and run against a test Simlink in CI on Windows, Linux and macOS. Each
  must join, emit valid payloads, answer its commands and survive a rejection.
- The release archives are tested the same way: each package task builds the archive's own examples from the
  packaged copy.

What this doesn't catch is idiomatic issues: an API that works but feels off to a C# or C++ developer. Issues like
that surface as integrators use the SDK and report them.

### Distribution

Each language ships through its usual package manager: vcpkg for C++, NuGet for C#, and crates.io for Rust.

All of them come out of `cargo xtask package` and the release workflow; see [development.md](development.md#releasing).

### Protocol updates

A workflow in the Simlink repository opens a PR here whenever the schema changes. The generated types and methods
update automatically. `crates/simlink-source-sdk/tests/schema.rs` lists the Rust type for every event payload and
command input, output and error, and round-trips a sample of each. It also compares that list against the channels in
the schema, so a new channel fails the test until its types are added. Merge, release, and integrators pick up the
new version.

### Adding a language

Find a UniFFI generator for it (the UniFFI docs list third-party ones for Go, Dart, Java, Node and more), pin it in
`xtask/src/generators.rs`, port the examples, and add them to `cargo xtask examples`. The core doesn't change.

## QA

Summarized from [development.md](development.md#testing):

- **The generated types match the schema.** Samples of every channel round-trip through the Rust types.
- **The connection behaves.** The core runs against a fake Simlink: handshakes and refusals, events and the rate
  limit, commands answered and refused, rejections, heartbeats and reconnects. The test Simlink's Socket.IO code is
  written separately from the SDK's, so one misreading of the protocol can't pass on both sides.
- **The bindings work.** Every example in every language runs against that test Simlink (see above).
- **CI** runs all of it on Windows, Linux and macOS for every PR, and again on release.

The tests use a fake Simlink. Running the real Simlink in CI would need changes to Simlink itself,
and would mean getting Simlink builds into this repo's CI on every platform.
