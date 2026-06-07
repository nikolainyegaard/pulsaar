using System.Text.Json;
using System.Text.Json.Serialization;
using Pulsaar.Core;

namespace Pulsaar.Core;

// ---------------------------------------------------------------------------
// Persisted types (same schema as macOS device-cache.json / device-names.json)
// ---------------------------------------------------------------------------

public class CachedBattery
{
    [JsonPropertyName("level")]     public int? Level { get; set; }
    [JsonPropertyName("statusByte")] public byte? StatusByte { get; set; }
    [JsonPropertyName("voltage")]   public ushort? Voltage { get; set; }
    [JsonPropertyName("seenAt")]    public long SeenAt { get; set; }
}

// ---------------------------------------------------------------------------
// DeviceCache
// ---------------------------------------------------------------------------

public class DeviceCache
{
    private static readonly string Dir =
        Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData), "Pulsaar");

    private static readonly string BatteryFile = Path.Combine(Dir, "device-cache.json");
    private static readonly string NamesFile   = Path.Combine(Dir, "device-names.json");

    private readonly JsonSerializerOptions _json = new() { WriteIndented = true };

    private Dictionary<string, CachedBattery> _battery = [];
    private Dictionary<string, string> _names = [];

    public DeviceCache()
    {
        Directory.CreateDirectory(Dir);
        _battery = Load<Dictionary<string, CachedBattery>>(BatteryFile) ?? [];
        _names   = Load<Dictionary<string, string>>(NamesFile) ?? [];
    }

    public void UpdateBattery(string serial, BatteryModel b)
    {
        if (string.IsNullOrEmpty(serial)) return;
        _battery[serial] = new CachedBattery
        {
            Level      = b.Level,
            StatusByte = b.Status.HasValue ? (byte?)b.Status.Value : null,
            Voltage    = b.Voltage,
            SeenAt     = DateTimeOffset.UtcNow.ToUnixTimeSeconds(),
        };
        Save(BatteryFile, _battery);
    }

    public BatteryModel? Battery(string serial)
    {
        if (string.IsNullOrEmpty(serial) || !_battery.TryGetValue(serial, out var c))
            return null;
        return BatteryModel.FromCache(c);
    }

    public void UpdateName(string serial, string name)
    {
        if (string.IsNullOrEmpty(serial) || string.IsNullOrEmpty(name)) return;
        _names[serial] = name;
        Save(NamesFile, _names);
    }

    public string? Name(string serial) =>
        string.IsNullOrEmpty(serial) ? null : _names.GetValueOrDefault(serial);

    private T? Load<T>(string path)
    {
        try
        {
            if (!File.Exists(path)) return default;
            return JsonSerializer.Deserialize<T>(File.ReadAllText(path), _json);
        }
        catch { return default; }
    }

    private void Save<T>(string path, T value)
    {
        try { File.WriteAllText(path, JsonSerializer.Serialize(value, _json)); }
        catch { }
    }
}
