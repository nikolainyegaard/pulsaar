using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace Pulsaar.Controls;

public sealed class SidebarTemplateSelector : DataTemplateSelector
{
    public DataTemplate ReceiverTemplate { get; set; } = null!;
    public DataTemplate DeviceTemplate { get; set; } = null!;
    public DataTemplate BluetoothTemplate { get; set; } = null!;

    protected override DataTemplate SelectTemplateCore(object item) => item switch
    {
        SidebarReceiverItem  => ReceiverTemplate,
        SidebarDeviceItem    => DeviceTemplate,
        SidebarBluetoothItem => BluetoothTemplate,
        _                    => ReceiverTemplate,
    };
}
