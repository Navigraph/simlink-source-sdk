// Prints the SDK's logs, connection changes and anything Simlink rejects. Stop with Ctrl+C.

using System.Diagnostics;

using Navigraph.Simlink.SourceSdk;

SimlinkSourceSdk.SetLogger(new ConsoleLog(), LogLevel.Debug);

var options = new SourceOptions(
    Id: "diagnostics",
    DisplayName: "Diagnostics");
using var source = new Source(options, [Event.TelemetryTraffic]);
source.OnState(new State());
source.OnRejected(new Rejections());
source.Start();

while (true)
{
    // An empty traffic picture is valid; a real source sends the aircraft around the user.
    // Emitting fails for an undeclared event, before `Start`, or over Simlink's message size limit.
    try
    {
        source.EmitTelemetryTraffic([]);
    }
    catch (SimlinkException error)
    {
        Console.WriteLine($"Could not emit: {error.Message}");
    }
    Thread.Sleep(1000);
}

class ConsoleLog : LogListener
{
    public void Log(LogLevel level, string message) =>
        Console.Error.WriteLine($"[{level}] {message}");
}

class State : StateListener
{
    public void Changed(ConnectionState state) => Console.WriteLine(state switch
    {
        ConnectionState.Searching => "Searching for Simlink",
        ConnectionState.Connected connected => $"Connected to {connected.Address}",
        ConnectionState.Rejected rejected =>
            $"Refused ({rejected.Code}): {rejected.Message}"
                + (rejected.Retrying ? ", retrying" : ""),
        _ => throw new UnreachableException(),
    });
}

class Rejections : RejectionListener
{
    public void Rejected(Rejection rejection)
    {
        Console.WriteLine($"Simlink rejected {rejection.Channel} ({rejection.Code}): {rejection.Message}");
        foreach (var issue in rejection.Issues)
        {
            Console.WriteLine($"  {issue.Path}: {issue.Message}");
        }
    }
}
