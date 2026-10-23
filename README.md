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

## Languages

| Language | Where |
| --- | --- |
| Rust | `crates/simlink` |
| C++20 | `bindings/cpp` (CMake target `simlink::simlink`) |
| C# (.NET 10) | `bindings/csharp` |

It runs on 64-bit Windows, Linux and macOS.

## Building

You need [Rust](https://rustup.rs) (the pinned toolchain installs itself), plus CMake 3.20 and a C++20 compiler for
C++, or the .NET 10 SDK for C#. Then:

```sh
cargo xtask build
```

This builds the library into `target/release` and generates the C++ and C# bindings into `target/bindings`. The first
run also installs the binding generators, which takes a few minutes.

## Using it

A source declares the events it emits and registers handlers for the commands it answers, then starts. The SDK finds
Simlink on this machine or the network, joins, reconnects whenever Simlink restarts, and sends each event no faster
than its rate limit, latest payload first. An emit larger than Simlink's message size limit fails with an error rather
than being sent, since Simlink would drop the connection.

Each source runs one background thread. Handlers and listeners run on it, so keep them short. To answer a slow
command, keep its reply and answer from any thread once the result is ready. Emitting is thread-safe too.

### Rust

<!-- snippet: crates/simlink/examples/quickstart.rs -->
```rust
//! Emits the aircraft's position to Simlink ten times a second. Stop with Ctrl+C.

use std::time::Duration;

use simlink::*;

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
fn ownship(latitude: f64, longitude: f64) -> TelemetryOwnship {
    TelemetryOwnship {
        aircraft: TelemetryOwnshipAircraft {
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
        },
        simulation: TelemetryOwnshipSimulation {
            paused: false,
            slew: false,
            time: None,
        },
    }
}
```

### C++

<!-- snippet: examples/cpp/quickstart.cpp -->
```cpp
// Emits the aircraft's position to Simlink ten times a second. Stop with Ctrl+C.
#include <chrono>
#include <thread>

#include <simlink.hpp>

// In a real source these values come from the simulator.
simlink::TelemetryOwnship ownship(double latitude, double longitude) {
    simlink::TelemetryOwnship ownship{};
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
    return ownship;
}

int main() {
    simlink::SourceOptions options{};
    options.id = "quickstart";
    options.display_name = "Quickstart";

    auto source = simlink::Source::init(options, {simlink::Event::kTelemetryOwnship});
    source->start();

    double latitude = 59.6519;
    for (;;) {
        latitude += 0.0001;
        source->emit_telemetry_ownship(ownship(latitude, 17.9186));
        std::this_thread::sleep_for(std::chrono::milliseconds(100));
    }
}
```

Add `add_subdirectory(path/to/simlink-source-sdk/bindings/cpp simlink)` to your CMake project and link
`simlink::simlink`. Every call throws `simlink::SimlinkError`.

### C#

<!-- snippet: examples/csharp/Quickstart/Program.cs -->
```csharp
// Emits the aircraft's position to Simlink ten times a second. Stop with Ctrl+C.

using Navigraph.Simlink;

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
static TelemetryOwnship Ownship(double latitude, double longitude)
{
    var aircraft = new TelemetryOwnshipAircraft(
        Position: new Coordinates(Latitude: latitude, Longitude: longitude),
        Altitude: new OwnshipAltitude(True: 1200, Agl: 900, Indicated: 1200, Pressure: null),
        Speed: new OwnshipSpeed(Gs: 140, Ias: 135, Tas: 138),
        Heading: new OwnshipHeading(True: 32, Magnetic: 25),
        Track: new OwnshipTrack(True: 31),
        Attitude: new OwnshipAttitude(Bank: 0, Pitch: 2.5),
        VerticalSpeed: 0,
        TurnRate: 0,
        Gforce: 1,
        Transponder: "7000");
    var simulation = new TelemetryOwnshipSimulation(Paused: false, Slew: false, Time: null);

    return new TelemetryOwnship(Aircraft: aircraft, Simulation: simulation);
}
```

Reference `bindings/csharp/Navigraph.Simlink.csproj` and ship `simlink.dll` (or `libsimlink.so`/`.dylib`) next to
your app. Every call throws `SimlinkException`.

## Examples

Each example exists in Rust (`crates/simlink/examples`), C++ (`examples/cpp`) and C# (`examples/csharp`):

- **quickstart** emits the aircraft's position.
- **commands** answers `atis.get` and `flightplan.get`.
- **diagnostics** prints the SDK's logs, connection changes and anything Simlink rejects.

Each one finds a running Simlink on this machine or the network and runs until you stop it with Ctrl+C. Run them
from the repository root.

**Rust**

```sh
cargo run --release -p simlink --example quickstart
```

**C++**, after `cargo xtask build`:

```sh
cmake -S examples/cpp -B target/examples/cpp -DCMAKE_BUILD_TYPE=Release
cmake --build target/examples/cpp --config Release
target/examples/cpp/bin/quickstart
```

**C#**, after `cargo xtask build`:

```sh
dotnet run -c Release --project examples/csharp/Quickstart
```

## Contributing

See [docs/development.md](docs/development.md) for building, testing and protocol updates, and
[docs/architecture.md](docs/architecture.md) for how the SDK is put together.
