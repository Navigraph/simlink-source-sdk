# Architecture

```
protocol/source-schema.json         Simlink's source manifest
  └─ protocol!()                    generates the protocol types and Source methods per channel
  └─ crates/simlink-source-sdk      connection core + the above, as UniFFI items
       ├─ uniffi-bindgen-cpp  →  target/bindings/cpp
       └─ uniffi-bindgen-cs   →  target/bindings/csharp
```

- **Bindings are generated, never checked in.** `xtask/src/generators.rs` pins both generators to UniFFI's minor
  version. C# uses [our fork](https://github.com/Navigraph/uniffi-bindgen-cs) until upstream releases our fixes.
- **The Socket.IO codec is our own** (`crates/simlink-source-sdk/src/wire.rs`): no Rust client answers server-initiated
  acks, which is how Simlink sends commands.
- **Each source runs one thread** (`crates/simlink-source-sdk/src/worker.rs`) that discovers Simlink, joins, sends each
  event's latest payload at its rate limit, and reconnects with backoff.
- **The socket is non-blocking after the upgrade**, since Windows sockets are unreliable once a read times out.
- **Foreign callbacks are shielded**: a C# or C++ exception becomes a log line instead of ending the thread.
