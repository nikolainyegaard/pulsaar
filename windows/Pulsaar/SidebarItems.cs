using Microsoft.UI.Xaml;
using Pulsaar.Core;

namespace Pulsaar;

public sealed class SidebarReceiverItem(ReceiverModel receiver)
{
    public ReceiverModel Receiver { get; } = receiver;
    public string SortKey => Receiver.Name;
}

public sealed class SidebarDeviceItem(DeviceModel device, bool isLast)
{
    public DeviceModel Device { get; } = device;
    public bool IsLast { get; } = isLast;

    public string KindGlyph => Device.Kind.Glyph();
    public bool HasBattery => Device.Battery != null;
    public string BatteryGlyph => Device.Battery?.BatteryGlyph ?? "";
    public string BatteryText => Device.Battery?.LevelText ?? "";
    public bool IsOnline => Device.IsOnline;

    public Visibility ConnectorBottomVisible => IsLast ? Visibility.Collapsed : Visibility.Visible;
    public Visibility BatteryVisible => HasBattery ? Visibility.Visible : Visibility.Collapsed;
    public Visibility OfflineBadgeVisible => IsOnline ? Visibility.Collapsed : Visibility.Visible;
}

public sealed class SidebarBluetoothItem(IReadOnlyList<DirectDeviceModel> devices)
{
    public IReadOnlyList<DirectDeviceModel> Devices { get; } = devices;
    public int Count => Devices.Count;
    public string SortKey => "Bluetooth";
}
