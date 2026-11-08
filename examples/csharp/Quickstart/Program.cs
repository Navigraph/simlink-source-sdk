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
