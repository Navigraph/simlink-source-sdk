Connect a flight simulator, add-on, or tool to Navigraph Simlink v2 as a **source**: emit telemetry and answer commands without implementing the Simlink protocol.

## Installing

```sh
dotnet add package Navigraph.Simlink.SourceSdk
```

Targets .NET Standard 2.0 (.NET 6+ and .NET Framework 4.7.2+) and bundles native libraries for Windows (x64), Linux (x64, glibc 2.17+), and macOS (Intel & Apple silicon).

For .NET Framework or single-file publish, add to your project:

```xml
<PropertyGroup>
  <!-- .NET Framework: so NuGet copies the native library -->
  <PlatformTarget>x64</PlatformTarget>
  <!-- Single-file publish -->
  <IncludeNativeLibrariesForSelfExtract>true</IncludeNativeLibrariesForSelfExtract>
</PropertyGroup>
```

## Quickstart

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

## Learn more

- [Key concepts](https://github.com/Navigraph/simlink-source-sdk#key-concepts) and [errors](https://github.com/Navigraph/simlink-source-sdk#errors)
- [Examples](https://github.com/Navigraph/simlink-source-sdk/tree/main/examples/csharp)
