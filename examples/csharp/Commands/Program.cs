// Answers Simlink's ATIS command, refusing airports without one. Stop with Ctrl+C.

using Navigraph.Simlink.SourceSdk;

var options = new SourceOptions(
    Id: "commands",
    DisplayName: "Commands");
// Commands are declared by registering their handlers, so this source declares no events.
using var source = new Source(options, []);
source.OnAtisGet(new Atis());
source.Start();

Thread.Sleep(Timeout.Infinite);

class Atis : AtisGetHandler
{
    public void Handle(AtisGetInput input, AtisGetReply reply)
    {
        using (reply)
        {
            // Answering twice throws SimlinkException.
            try
            {
                if (Reports(input.Icao) is { } reports)
                {
                    reply.Ok(reports);
                }
                else
                {
                    reply.Refuse(AtisGetError.AtisNotAvailable);
                }
            }
            catch (SimlinkException error)
            {
                Console.Error.WriteLine($"Could not answer atis.get: {error.Message}");
            }
        }
    }

    // The airport's current ATIS broadcasts, if it has any.
    static AtisReport[]? Reports(string icao) => icao != "ESSA" ? null :
    [
        new AtisReport(
            Icao: icao,
            Type: AtisReportType.Arr,
            Letter: "A",
            Phonetic: "Alpha",
            Message: "Stockholm Arlanda arrival information Alpha",
            DateIssued: "2026-10-19T12:00:00Z",
            ArrivingRunways: ["19R"],
            DepartingRunways: []),
        new AtisReport(
            Icao: icao,
            Type: AtisReportType.Dep,
            Letter: "B",
            Phonetic: "Bravo",
            Message: "Stockholm Arlanda departure information Bravo",
            DateIssued: "2026-10-19T12:00:00Z",
            ArrivingRunways: [],
            DepartingRunways: ["19L"]),
    ];
}
