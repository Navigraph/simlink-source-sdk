<div align="center">
  <a href="https://navigraph.com">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/Navigraph/simlink-source-sdk/main/assets/logo.svg">
      <img src="https://raw.githubusercontent.com/Navigraph/simlink-source-sdk/main/assets/logo-light.svg" alt="Navigraph Simlink Source SDK" width="560">
    </picture>
  </a>

  <br/>
  <br/>

  <p>Connect a flight simulator, add-on, or tool to Navigraph Simlink v2 as a <strong>source</strong>: emit telemetry and answer commands without implementing the Simlink protocol.</p>
</div>

## Installing

Prebuilt binaries support Windows (x64), Linux (x64, glibc 2.17+), and macOS (Intel & Apple silicon).

### C++

Requires C++20 (CMake 3.20+ if using CMake; MSVC `/MD` or `/MDd` on Windows).

**vcpkg**

1. Add [Navigraph's vcpkg registry](https://github.com/Navigraph/vcpkg-registry) to `vcpkg-configuration.json` (see its README).
2. Add `"simlink-source-sdk"` to `dependencies` in `vcpkg.json`.
3. Link in `CMakeLists.txt`:

   ```cmake
   find_package(simlink-source-sdk CONFIG REQUIRED)
   target_link_libraries(my_app PRIVATE simlink-source-sdk::simlink-source-sdk)
   ```

<details>
<summary>Without vcpkg (GitHub release archive)</summary>

1. Download and extract `simlink-source-sdk-cpp-<version>-<platform>.zip` from [Releases](https://github.com/Navigraph/simlink-source-sdk/releases).
2. Integrate it with your build system:
   - **CMake (`add_subdirectory`)**:

     ```cmake
     add_subdirectory(path/to/simlink-source-sdk-cpp-<version>-<platform> simlink-source-sdk)
     target_link_libraries(my_app PRIVATE simlink-source-sdk::simlink-source-sdk)
     ```

   - **CMake (installed package)** — install, then use `find_package(simlink-source-sdk CONFIG REQUIRED)` as above:

     ```sh
     cmake -S simlink-source-sdk-cpp-<version>-<platform> -B build -DCMAKE_BUILD_TYPE=Release
     cmake --build build --config Release
     cmake --install build --config Release --prefix <prefix>
     ```

   - **Visual Studio**: In Property Manager, right-click your project → *Add Existing Property Sheet* → select `simlink-source-sdk.props`.
   - **Other build systems**: Compile `src/simlink_source_sdk.cpp` as C++20, add `include` to the include path, and link the static library in `lib` along with `ntdll.lib` and `userenv.lib` (Windows) or `pthread`, `dl`, `m`, `rt`, and `util` (Linux).

</details>

### C#

```sh
dotnet add package Navigraph.Simlink.SourceSdk
```

Targets .NET Standard 2.0 (.NET 6+ and .NET Framework 4.7.2+) and bundles native libraries for all supported platforms.

- **.NET Framework**: set `<PlatformTarget>x64</PlatformTarget>` so NuGet copies the native library.
- **Single-file publish**: set `<IncludeNativeLibrariesForSelfExtract>true</IncludeNativeLibrariesForSelfExtract>`.

### Rust

```sh
cargo add simlink-source-sdk
```

## Using it

Each quickstart below emits the aircraft's position ten times a second:

<details>
<summary><strong>C++</strong></summary>

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

</details>

<details>
<summary><strong>C#</strong></summary>

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

</details>

<details>
<summary><strong>Rust</strong></summary>

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

</details>

### Key concepts

- **Lifecycle**: Create a `Source` with its ID, display name, and declared events. Register command handlers and listeners before calling `start()` (registering afterwards fails). Dropping the source (`Dispose` in C#, releasing the last `shared_ptr` in C++) disconnects. Don't hold a reference to `Source` inside handlers or listeners — the source owns them, so it would leak.
- **Events**: Call the emit method for each declared event (e.g. `emit_telemetry_ownship`) whenever new data arrives. The SDK rate-limits sends (10 Hz for ownship, 1 Hz for traffic) and keeps only the latest payload. Emitting before `start()`, emitting an undeclared event, or exceeding Simlink's message size limit fails.
- **Commands**: Handlers receive the command input and a reply handle. Answer once (with a result or error code) within the command's timeout; answering twice fails. In C#, dispose the reply after answering.
- **Diagnostics**:
  - *State listener* — reports `searching`, `connected`, or `refused`, whether the SDK is retrying, and refusal details (`HandshakeError`).
  - *Rejection listener* — reports payloads or replies rejected by Simlink, with channel, code, and per-field issues.
  - *`set_logger`* — receives SDK logs up to a level (only the first call per process takes effect).
- **Threading**: Each source runs one background thread for handlers and listeners — keep callbacks short and marshal to your UI thread if needed. Emitting and answering are thread-safe, so slow commands can store the reply and answer from another thread.
- **Naming**: Types and methods match across languages using native casing (`emit_telemetry_ownship` in C++/Rust, `EmitTelemetryOwnship` in C#). Keyword fields get a trailing underscore in C++ and Rust (e.g. `true_`).

### Errors

All languages expose the same error cases: `InvalidArgument`, `Undeclared`, `WrongState`, and `AlreadyAnswered`.

| Language | Type |
| --- | --- |
| C++ | `simlink_source_sdk::SimlinkError` (thrown; subclasses in `simlink_source_sdk::simlink_error`) |
| C# | `SimlinkException` (thrown; nested classes such as `SimlinkException.WrongState`) |
| Rust | `simlink_source_sdk::Error` (returned; enum variants) |

## Examples

Full examples for C++ ([`examples/cpp`](examples/cpp)), C# ([`examples/csharp`](examples/csharp)), and Rust ([`crates/simlink-source-sdk/examples`](crates/simlink-source-sdk/examples)):

- **quickstart** — emits ownship position (`telemetry.ownship`).
- **traffic** — emits surrounding aircraft (`telemetry.traffic`) and answers `flightplan.get`.
- **commands** — answers `atis.get`, refusing airports without an ATIS.
- **diagnostics** — prints SDK logs, connection changes, Simlink rejections, and failed emits.

## Contributing

See [docs/development.md](docs/development.md) for building from source, running examples and tests, and updating the protocol, and [docs/architecture.md](docs/architecture.md) for how the SDK is structured.
