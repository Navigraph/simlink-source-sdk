// Answers Simlink's ATIS and flight plan commands. Stop with Ctrl+C.

using Navigraph.Simlink;

var options = new SourceOptions(
    Id: "commands",
    DisplayName: "Commands",
    IsInjectingSimTraffic: false);
using var source = new Source(options, []);
source.OnAtisGet(new Atis());
source.OnFlightplanGet(new FlightPlans());
source.Start();

Thread.Sleep(Timeout.Infinite);

class Atis : AtisGetHandler
{
    public void Handle(AtisGetInput input, AtisGetReply reply)
    {
        using (reply)
        {
            if (input.Icao != "ESSA")
            {
                reply.Refuse(AtisGetError.AtisNotAvailable);
                return;
            }

            reply.Ok([
                new AtisReport(
                    Icao: input.Icao,
                    Type: AtisReportType.Arr,
                    Letter: "A",
                    Phonetic: "Alpha",
                    Message: "Stockholm Arlanda information Alpha",
                    DateIssued: "2026-10-19T12:00:00Z",
                    ArrivingRunways: ["19R"],
                    DepartingRunways: ["19L"]),
            ]);
        }
    }
}

class FlightPlans : FlightplanGetHandler
{
    public void Handle(FlightplanGetInput input, FlightplanGetReply reply)
    {
        using (reply)
        {
            // No flight plans here; a real source looks each key up.
            reply.Ok(input.Keys.ToDictionary(key => key, FlightPlan? (_) => null));
        }
    }
}
