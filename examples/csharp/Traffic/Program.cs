// Emits the traffic around the aircraft once a second and answers flight plan queries for it. Stop with Ctrl+C.

using Navigraph.Simlink.SourceSdk;

var options = new SourceOptions(
    Id: "traffic",
    DisplayName: "Traffic");
using var source = new Source(options, [Event.TelemetryTraffic]);
source.OnFlightplanGet(new FlightPlans());
source.Start();

while (true)
{
    // The whole picture each time: aircraft missing from it have left.
    source.EmitTelemetryTraffic(Traffic());
    Thread.Sleep(1000);
}

// In a real source these come from the simulator, each with a key that stays the same while it is in range.
static TrafficPosition[] Traffic() =>
[
    // Identified by ICAO type designator, with every optional field filled in.
    new TrafficPosition(
        Key: "EZY382",
        Aircraft: new TrafficAircraft.Type(
            TypeValue: "A320",
            Callsign: "EZY382",
            Position: new Coordinates(Latitude: 59.70, Longitude: 17.95),
            Altitude: new TrafficAltitude(True: 4500, Agl: 4300, Indicated: 4480, Pressure: 4520),
            Speed: new TrafficSpeed(Gs: 250, Ias: 220, Tas: 236),
            VerticalSpeed: -800,
            Heading: new TrafficHeading(True: 190, Magnetic: 184),
            Attitude: new TrafficAttitude(Bank: 0, Pitch: -1.5),
            Transponder: "4521")),
    // Identified by category when the exact type is unknown, with only the required fields.
    new TrafficPosition(
        Key: "SEJKL",
        Aircraft: new TrafficAircraft.Category(
            CategoryValue: new AircraftCategory(Type: AircraftCategoryType.Helicopter, OperatorType: null),
            TypeValue: null,
            Callsign: null,
            Position: new Coordinates(Latitude: 59.64, Longitude: 17.90),
            Altitude: new TrafficAltitude(True: 800, Agl: null, Indicated: null, Pressure: null),
            Speed: new TrafficSpeed(Gs: 90, Ias: null, Tas: null),
            VerticalSpeed: null,
            Heading: new TrafficHeading(True: 45, Magnetic: null),
            Attitude: new TrafficAttitude(Bank: null, Pitch: null),
            // Unset means "1200".
            Transponder: null)),
];

class FlightPlans : FlightplanGetHandler
{
    public void Handle(FlightplanGetCommandInput input, FlightplanGetReply reply)
    {
        using (reply)
        {
            // Every requested key gets an entry; null when this source has no plan for it.
            reply.Ok(input.Keys.ToDictionary(key => key, Plan));
        }
    }

    // The plan filed for a traffic key, if any.
    static FlightPlan? Plan(string key) => key != "EZY382" ? null : new FlightPlan(
        Key: key,
        Callsign: "EZY382",
        PilotName: "Jane Doe",
        Aircraft: "A320",
        Origin: new FlightPlanAirport(Ident: "EGKK", Coords: new AirportCoordinates(Lat: 51.148, Lon: -0.190)),
        Destination: new FlightPlanAirport(Ident: "ESSA", Coords: new AirportCoordinates(Lat: 59.652, Lon: 17.919)),
        Route: "EGKK DCT ESSA",
        Stats: new FlightPlanStats(
            DepDist: 712,
            ToGoDist: 18,
            Etd: "2026-10-19T09:05:00Z",
            Eta: "2026-10-19T11:40:00Z"),
        Status: FlightPlanStatus.Arriving);
}
