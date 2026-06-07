using Microsoft.UI;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Pulsaar.Core;

namespace Pulsaar.Controls;

public sealed partial class DeviceHeader : UserControl
{
    public DeviceHeader()
    {
        InitializeComponent();
    }

    public void Show(DeviceModel device)
    {
        KindIcon.Glyph = device.Kind.Glyph();
        NameText.Text = device.Name;

        if (device.IsOnline)
        {
            StatusText.Text = "Online";
            StatusText.Foreground = new SolidColorBrush(Colors.Green);
        }
        else
        {
            StatusText.Text = "Offline";
            StatusText.Foreground = (Brush)Application.Current.Resources["TextFillColorTertiaryBrush"];
        }

        if (device.Battery != null)
        {
            BatteryIcon.Glyph = device.Battery.BatteryGlyph;
            BatteryLevelText.Text = device.Battery.LevelText;
            BatteryPanel.Visibility = Visibility.Visible;
        }
        else
        {
            BatteryPanel.Visibility = Visibility.Collapsed;
        }
    }
}
