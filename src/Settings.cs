using System.Text.Json;
using System.Text.Json.Serialization;

namespace DontForget;

[JsonConverter(typeof(JsonStringEnumConverter))]
public enum AfterCapture { Follow, Drop, Stay }

sealed class Settings
{
    public AfterCapture AfterCapture { get; set; } = AfterCapture.Follow;
    public int HoldMs { get; set; } = 250;
    // Process names (no .exe) where the wheel button is left alone, e.g. CAD apps that orbit with it.
    public List<string> IgnoredApps { get; set; } = new() { "SLDWORKS", "blender", "FreeCAD", "acad" };
    public bool ShownIntro { get; set; }

    public static string FilePath => Path.Combine(
        Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData), "DontForget", "settings.json");

    public static Settings Load()
    {
        try { return JsonSerializer.Deserialize<Settings>(File.ReadAllText(FilePath)) ?? new Settings(); }
        catch { return new Settings(); }
    }

    public void Save()
    {
        try
        {
            Directory.CreateDirectory(Path.GetDirectoryName(FilePath));
            File.WriteAllText(FilePath, JsonSerializer.Serialize(this, new JsonSerializerOptions { WriteIndented = true }));
        }
        catch { }
    }
}
