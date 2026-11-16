<div align="center">
  <a href="https://navigraph.com">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="assets/logo.svg">
      <img src="assets/logo-light.svg" alt="Navigraph Simlink Source SDK" width="400">
    </picture>
  </a>

  <br/>
  <br/>

  <p>Connect a flight simulator, add-on or tool to Navigraph Simlink v2 as a <strong>source</strong>: emit telemetry and answer commands without implementing the Simlink protocol yourself.</p>
</div>

## What it does

A source tells Simlink about the simulator. It declares the events it emits, such as the aircraft's position and the
traffic around it, and the commands it answers, such as ATIS and flight plan lookups. The SDK finds Simlink on this
machine or the network, joins, reconnects whenever Simlink restarts, and sends each event no faster than its rate
limit, latest payload first.

## Requirements

| | Minimum |
| --- | --- |
| Windows | Windows 10 or Server 2016, x64 |
| Linux | x64 with glibc 2.17 or newer |
| macOS | 10.12 or newer on Intel, 11 or newer on Apple silicon |
| C++ | A C++20 compiler, and CMake 3.20 or newer for the CMake project |
| C# | .NET 6 or newer, or .NET Framework 4.7.2 or newer |
| Rust | 1.95 or newer |

## Installing

Download the SDK for your language from the [releases](https://github.com/Navigraph/simlink-source-sdk/releases).
Each archive also holds the [examples](#examples), ready to build against it.

### C++

`simlink-source-sdk-cpp-<version>-<platform>.zip` holds the headers in `include`, the C++ half of the bindings in
`src`, a static library in `lib`, a `CMakeLists.txt`, and on Windows a Visual Studio property sheet. Your build compiles
`src/simlink_source_sdk.cpp` with your own compiler and settings, so any C++20 compiler works; on Windows, MSVC with the
dynamic runtime library (`/MD` and `/MDd`, Visual Studio's default). The library is static, so nothing ships alongside
your program.

**CMake**

```cmake
add_subdirectory(path/to/simlink-source-sdk-cpp-<version>-<platform> simlink-source-sdk)
target_link_libraries(my_app PRIVATE simlink-source-sdk::simlink-source-sdk)
```

**Visual Studio project**

In the Property Manager, right-click your project, choose Add Existing Property Sheet, and pick
`simlink-source-sdk.props` from the SDK. It adds the headers, C++20, `src/simlink_source_sdk.cpp` and the library.

**Other build systems**: compile `src/simlink_source_sdk.cpp` as C++20, add `include` to the include path, and link
the library in `lib`, plus `ntdll.lib` and `userenv.lib` on Windows.

### C#

`simlink-source-sdk-csharp-<version>.zip` holds the `Navigraph.Simlink.SourceSdk` NuGet package. It targets .NET
Standard 2.0, for .NET 6+ and .NET Framework 4.7.2+.

1. Copy `Navigraph.Simlink.SourceSdk.<version>.nupkg` into a folder in your repository, such as `packages`.
2. Add that folder as a package source in a `nuget.config` next to your solution:

   ```xml
   <configuration>
     <packageSources>
       <add key="simlink-source-sdk" value="packages" />
     </packageSources>
   </configuration>
   ```

3. `dotnet add package Navigraph.Simlink.SourceSdk --version <version>`

The package carries the native library for every platform, and builds copy the right one. On .NET Framework, set
`<PlatformTarget>x64</PlatformTarget>`: NuGet only copies the library for an x64 build. A single-file publish needs
`<IncludeNativeLibrariesForSelfExtract>true</IncludeNativeLibrariesForSelfExtract>`.

### Rust

The crate isn't on crates.io yet:

```toml
simlink-source-sdk = { git = "https://github.com/Navigraph/simlink-source-sdk", tag = "v<version>" }
```

## Using it

Each quickstart emits the aircraft's position ten times a second.

**C++**

<!-- snippet: examples/cpp/quickstart.cpp -->
```cpp
// Emits the aircraft's position to Simlink ten times a second. Stop with Ctrl+C.
#include <chrono>
#include <thread>

#include <simlink_source_sdk.hpp>

// In a real source these values come from the simulator.
simlink_source_sdk::TelemetryOwnshipEvent ownship(double latitude, double longitude) {
    simlink_source_sdk::TelemetryOwnshipEvent ownship{};
    auto& aircraft = ownship.aircraft;
    aircraft.position = {latitude, longitude};
    aircraft.altitude.true_ = 1200;
    aircraft.altitude.agl = 900;
    aircraft.altitude.indicated = 1200;
    aircraft.speed = {.gs = 140, .ias = 135, .tas = 138};
    aircraft.heading = {.magnetic = 25, .true_ = 32};
    aircraft.track.true_ = 31;
    aircraft.attitude = {.bank = 0, .pitch = 2.5};
    aircraft.gforce = 1;
    aircraft.transponder = "7000";
    aircraft.category = {.type = simlink_source_sdk::AircraftCategoryType::kPiston};
    return ownship;
}

int main() {
    simlink_source_sdk::SourceOptions options{};
    options.id = "quickstart";
    options.display_name = "Quickstart";

    auto source = simlink_source_sdk::Source::init(options, {simlink_source_sdk::Event::kTelemetryOwnship});
    source->start();

    double latitude = 59.6519;
    for (;;) {
        latitude += 0.0001;
        source->emit_telemetry_ownship(ownship(latitude, 17.9186));
        std::this_thread::sleep_for(std::chrono::milliseconds(100));
    }
}
```

**C#**

<!-- snippet: examples/csharp/Quickstart/Program.cs -->
```csharp
// Emits the aircraft's position to Simlink ten times a second. Stop with Ctrl+C.

using Navigraph.Simlink.SourceSdk;

var options = new SourceOptions(
    Id: "quickstart",
    DisplayName: "Quickstart");
using var source = new Source(options, [Event.TelemetryOwnship]);
source.Start();

var latitude = 59.6519;
while (true)
{
    latitude += 0.0001;
    source.EmitTelemetryOwnship(Ownship(latitude, 17.9186));
    Thread.Sleep(100);
}

// In a real source these values come from the simulator.
static TelemetryOwnshipEvent Ownship(double latitude, double longitude)
{
    var aircraft = new OwnshipAircraft(
        Position: new Coordinates(Latitude: latitude, Longitude: longitude),
        Altitude: new OwnshipAltitude(True: 1200, Agl: 900, Indicated: 1200, Pressure: null),
        Speed: new OwnshipSpeed(Gs: 140, Ias: 135, Tas: 138),
        Heading: new OwnshipHeading(True: 32, Magnetic: 25),
        Track: new OwnshipTrack(True: 31),
        Attitude: new OwnshipAttitude(Bank: 0, Pitch: 2.5),
        VerticalSpeed: 0,
        TurnRate: 0,
        Gforce: 1,
        Transponder: "7000",
        Category: new AircraftCategory(Type: AircraftCategoryType.Piston, OperatorType: null));
    var simulation = new OwnshipSimulation(Paused: false, Slew: false, Time: null);

    return new TelemetryOwnshipEvent(Aircraft: aircraft, Simulation: simulation);
}
```

**Rust**

<!-- snippet: crates/simlink-source-sdk/examples/quickstart.rs -->
```rust
//! Emits the aircraft's position to Simlink ten times a second. Stop with Ctrl+C.

use std::time::Duration;

use simlink_source_sdk::*;

fn main() -> Result<(), Error> {
    let options = SourceOptions {
        id: "quickstart".into(),
        display_name: "Quickstart".into(),
    };
    let source = Source::new(options, vec![Event::TelemetryOwnship]);
    source.start()?;

    let mut latitude = 59.6519;
    loop {
        latitude += 0.0001;
        source.emit_telemetry_ownship(ownship(latitude, 17.9186))?;
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// In a real source these values come from the simulator.
fn ownship(latitude: f64, longitude: f64) -> TelemetryOwnshipEvent {
    TelemetryOwnshipEvent {
        aircraft: OwnshipAircraft {
            position: Coordinates {
                latitude,
                longitude,
            },
            altitude: OwnshipAltitude {
                true_: 1200.0,
                agl: 900.0,
                indicated: 1200.0,
                pressure: None,
            },
            speed: OwnshipSpeed {
                gs: 140.0,
                ias: 135.0,
                tas: 138.0,
            },
            heading: OwnshipHeading {
                true_: 32.0,
                magnetic: 25.0,
            },
            track: OwnshipTrack { true_: 31.0 },
            attitude: OwnshipAttitude {
                bank: 0.0,
                pitch: 2.5,
            },
            vertical_speed: 0.0,
            turn_rate: 0.0,
            gforce: 1.0,
            transponder: Some("7000".into()),
            category: Some(AircraftCategory {
                type_: AircraftCategoryType::Piston,
                operator_type: None,
            }),
        },
        simulation: OwnshipSimulation {
            paused: false,
            slew: false,
            time: None,
        },
    }
}
```

### Starting a source

Create a `Source` with its id, display name and the events it emits. Register a handler for each command it answers,
and any listeners, then start it. Registering after starting fails. Dropping the source (disposing it in C#,
releasing the last `shared_ptr` in C++) disconnects.

Handlers and listeners must not hold their `Source`: the source owns them, so it would never be released.

### Emitting events

Each declared event has an emit method, such as `emit_telemetry_ownship`. Emit whenever you have new data; the SDK
sends the latest payload no faster than the event's rate limit (10 Hz for ownship, 1 Hz for traffic) and drops the ones
in between. Emitting an undeclared event, before starting, or a payload larger than Simlink's message size limit fails.

### Answering commands

Each handler gets the command's input and a reply. Answer once, either with a result or by refusing with one of the
command's error codes, within the command's timeout; the method docs give each timeout. Answering twice fails. In C#,
dispose the reply once answered.

### Diagnostics

- **State listener**: searching, connected, or refused by Simlink, with whether the SDK is retrying. A refusal is a
  `HandshakeError` with its details, such as the descriptor's issues when it fails validation.
- **Rejection listener**: Simlink found a payload or answer invalid, with the channel, a `RejectionCode`, and the path
  and reason for each field.
- **`set_logger`**: the SDK's own logs, up to a level. Only the first call per process takes effect.

### Threading

Each source runs one background thread. Handlers and listeners run on it, so keep them short, and in a UI app hand
results to the UI thread yourself. Emitting and answering are thread-safe: to answer a slow command, keep its reply and
answer from any thread once the result is ready.

### Errors

| Language | Type | Cases |
| --- | --- | --- |
| C++ | `simlink_source_sdk::SimlinkError` (thrown) | `InvalidArgument`, `Undeclared`, `WrongState`, `AlreadyAnswered`, as subclasses in `simlink_source_sdk::simlink_error` |
| C# | `SimlinkException` (thrown) | The same, as nested classes such as `SimlinkException.WrongState` |
| Rust | `simlink_source_sdk::Error` (returned) | The same, as variants |

### Names in each language

The types and methods are the same in every language, in its own casing: `emit_telemetry_ownship` in C++ and Rust,
`EmitTelemetryOwnship` in C#. In C++ and Rust, a field named after a keyword gets a suffix, such as `true_`.

## Examples

The examples cover the whole SDK, in C++, C# and Rust:

- **quickstart** emits the aircraft's position, `telemetry.ownship`.
- **traffic** emits the aircraft around it, `telemetry.traffic`, identified by type or by category, and answers
  `flightplan.get` for them.
- **commands** answers `atis.get`, refusing airports without an ATIS.
- **diagnostics** prints the SDK's logs, connection changes, anything Simlink rejects, and failed emits.

Each one finds a running Simlink on this machine or the network and runs until you stop it with Ctrl+C. In the
unzipped C++ archive:

```sh
cmake -S examples -B build -DCMAKE_BUILD_TYPE=Release
cmake --build build --config Release
build/bin/quickstart
```

In the unzipped C# archive:

```sh
dotnet run -c Release --project examples/Quickstart
```

## Building from source

You need [Rust](https://rustup.rs) (the pinned toolchain installs itself), plus CMake 3.20 and a C++20 compiler for
C++, or the .NET 10 SDK for C#. Then:

```sh
cargo xtask build
```

This builds the library into `target/release` and generates the C++ and C# bindings into `target/bindings`. The first
run also installs the binding generators, which takes a few minutes. Link `bindings/cpp` with CMake as above, or
reference `bindings/csharp/Navigraph.Simlink.SourceSdk.csproj`.

The examples live in `examples/cpp`, `examples/csharp` and `crates/simlink-source-sdk/examples`. From the repository
root:

```sh
cargo run --release -p simlink-source-sdk --example quickstart

cmake -S examples/cpp -B target/examples/cpp -DCMAKE_BUILD_TYPE=Release
cmake --build target/examples/cpp --config Release
target/examples/cpp/bin/quickstart

dotnet run -c Release --project examples/csharp/Quickstart
```

## Contributing

See [docs/development.md](docs/development.md) for building, testing and protocol updates, and
[docs/architecture.md](docs/architecture.md) for how the SDK is put together.
