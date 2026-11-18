// Mark II sensors: reads hardware sensors with LibreHardwareMonitorLib (PawnIO driver)
// and streams them as JSON lines to the Mark II app over a named pipe.
// It only writes; it never accepts commands, so the elevated side has no input surface.
//
//   markii-sensors --pipe <name> [--interval <ms>]   stream to \.\pipe\<name>
//   markii-sensors --dump                            print one JSON sample to stdout
using System.IO.Pipes;
using System.Text;
using System.Text.Json;
using System.Text.RegularExpressions;
using LibreHardwareMonitor.Hardware;

string? pipe = null; int interval = 1000; bool dump = false;
for (int i = 0; i < args.Length; i++)
{
    switch (args[i])
    {
        case "--pipe" when i + 1 < args.Length: pipe = args[++i]; break;
        case "--interval" when i + 1 < args.Length: int.TryParse(args[++i], out interval); break;
        case "--dump": dump = true; break;
    }
}
interval = Math.Clamp(interval, 250, 10000);
if (pipe != null && !Regex.IsMatch(pipe, "^[A-Za-z0-9._-]{1,64}$")) return 2;
if (pipe == null && !dump) return 2;

var pc = new Computer
{
    IsCpuEnabled = true, IsGpuEnabled = true, IsMemoryEnabled = true, IsMotherboardEnabled = true,
    IsControllerEnabled = true, IsStorageEnabled = true, IsPsuEnabled = true,
};
pc.Open();

byte[] Sample()
{
    using var ms = new MemoryStream();
    using (var w = new Utf8JsonWriter(ms))
    {
        w.WriteStartObject();
        w.WriteNumber("v", 1);
        w.WriteBoolean("pawnio", File.Exists(Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.ProgramFiles), "PawnIO", "PawnIOLib.dll")));
        w.WriteStartArray("hw");
        foreach (var h in pc.Hardware) Write(w, h, null);
        w.WriteEndArray();
        w.WriteEndObject();
    }
    ms.WriteByte((byte)'\n');
    return ms.ToArray();
}

void Write(Utf8JsonWriter w, IHardware h, string? parent)
{
    try { h.Update(); } catch { }
    w.WriteStartObject();
    w.WriteString("t", h.HardwareType.ToString());
    w.WriteString("n", h.Name);
    w.WriteString("id", h.Identifier.ToString());
    if (parent != null) w.WriteString("p", parent);
    w.WriteStartArray("s");
    foreach (var s in h.Sensors)
    {
        w.WriteStartObject();
        w.WriteString("t", s.SensorType.ToString());
        w.WriteString("n", s.Name);
        if (s.Value is float v && float.IsFinite(v)) w.WriteNumber("v", v); else w.WriteNull("v");
        w.WriteEndObject();
    }
    w.WriteEndArray();
    w.WriteEndObject();
    foreach (var sub in h.SubHardware) Write(w, sub, h.Identifier.ToString());
}

if (dump)
{
    Sample();                       // first read primes load counters
    Thread.Sleep(500);
    using var o = Console.OpenStandardOutput();
    o.Write(Sample());
    pc.Close();
    return 0;
}

using var client = new NamedPipeClientStream(".", pipe!, PipeDirection.Out);
try { client.Connect(15000); } catch { pc.Close(); return 3; }
try
{
    while (client.IsConnected)
    {
        var started = Environment.TickCount64;
        client.Write(Sample());
        client.Flush();
        var left = interval - (int)(Environment.TickCount64 - started);
        if (left > 0) Thread.Sleep(left);
    }
}
catch (IOException) { }         // app closed the pipe
finally { pc.Close(); }
return 0;
