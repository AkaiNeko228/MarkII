// Mark II sensors: reads hardware sensors with LibreHardwareMonitorLib (PawnIO driver)
// and streams them as JSON lines to the Mark II app over a named pipe.
// It only writes; it never accepts commands, so the elevated side has no input surface.
//
//   markii-sensors --pipe <name> [--interval <ms>]   stream to \.\pipe\<name>
//   markii-sensors --dump                            print one JSON sample to stdout
//   markii-sensors --openrgb --parent <pid>          run the bundled OpenRGB SDK server
//       (127.0.0.1 only) in a kill-on-close job; it ends when Mark II (<pid>) or this
//       helper ends, so OpenRGB never keeps running in the background. The OpenRGB
//       path and its config folder are fixed, not taken from the command line.
using System.Diagnostics;
using System.IO.Pipes;
using System.Runtime.InteropServices;
using System.Text;
using System.Text.Json;
using System.Text.RegularExpressions;
using LibreHardwareMonitor.Hardware;

string? pipe = null; int interval = 1000; bool dump = false; bool openrgb = false; int parent = 0;
for (int i = 0; i < args.Length; i++)
{
    switch (args[i])
    {
        case "--pipe" when i + 1 < args.Length: pipe = args[++i]; break;
        case "--interval" when i + 1 < args.Length: int.TryParse(args[++i], out interval); break;
        case "--dump": dump = true; break;
        case "--openrgb": openrgb = true; break;
        case "--parent" when i + 1 < args.Length: int.TryParse(args[++i], out parent); break;
    }
}
if (openrgb) return Watchdog.Run(parent);
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

static class Watchdog
{
    [StructLayout(LayoutKind.Sequential)]
    struct BasicLimit { public long PerProcessUserTimeLimit, PerJobUserTimeLimit; public uint LimitFlags; public UIntPtr MinimumWorkingSetSize, MaximumWorkingSetSize; public uint ActiveProcessLimit; public UIntPtr Affinity; public uint PriorityClass, SchedulingClass; }
    [StructLayout(LayoutKind.Sequential)]
    struct IoCounters { public ulong a, b, c, d, e, f; }
    [StructLayout(LayoutKind.Sequential)]
    struct ExtendedLimit { public BasicLimit Basic; public IoCounters Io; public UIntPtr ProcessMemoryLimit, JobMemoryLimit, PeakProcessMemoryUsed, PeakJobMemoryUsed; }
    [DllImport("kernel32.dll", SetLastError = true)] static extern IntPtr CreateJobObjectW(IntPtr attrs, string? name);
    [DllImport("kernel32.dll", SetLastError = true)] static extern bool SetInformationJobObject(IntPtr job, int cls, ref ExtendedLimit info, int len);
    [DllImport("kernel32.dll", SetLastError = true)] static extern bool AssignProcessToJobObject(IntPtr job, IntPtr process);
    const uint KILL_ON_JOB_CLOSE = 0x2000;

    static string? Find()
    {
        var dir = AppContext.BaseDirectory;
        foreach (var p in new[] {
            Path.Combine(dir, "openrgb", "OpenRGB.exe"),
            // dev tree: target/sensors/ -> vendor/openrgb/
            Path.Combine(dir, "..", "..", "vendor", "openrgb", "OpenRGB Windows 64-bit", "OpenRGB.exe"),
        })
            if (File.Exists(p)) return Path.GetFullPath(p);
        return null;
    }

    public static int Run(int parentPid)
    {
        Process parent;
        try { parent = Process.GetProcessById(parentPid); } catch { return 2; }
        var exe = Find();
        if (exe == null) return 4;
        var cfg = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData), "com.akaineko.markii", "openrgb");
        Directory.CreateDirectory(cfg);

        // the job dies with this process, and takes OpenRGB with it
        var job = CreateJobObjectW(IntPtr.Zero, null);
        var info = new ExtendedLimit();
        info.Basic.LimitFlags = KILL_ON_JOB_CLOSE;
        SetInformationJobObject(job, 9, ref info, Marshal.SizeOf<ExtendedLimit>());

        var psi = new ProcessStartInfo(exe) { UseShellExecute = false, CreateNoWindow = true, WorkingDirectory = Path.GetDirectoryName(exe)! };
        foreach (var a in new[] { "--server", "--server-host", "127.0.0.1", "--server-port", "6742", "--noautoconnect", "--config", cfg })
            psi.ArgumentList.Add(a);
        using var orgb = Process.Start(psi);
        if (orgb == null) return 5;
        AssignProcessToJobObject(job, orgb.Handle);

        // wait for Mark II or OpenRGB to end
        while (!parent.HasExited && !orgb.HasExited) Thread.Sleep(500);
        try { if (!orgb.HasExited) orgb.Kill(true); } catch { }
        return 0;
    }
}
