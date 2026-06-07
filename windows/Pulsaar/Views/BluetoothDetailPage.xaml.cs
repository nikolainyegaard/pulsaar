using System.Collections.Generic;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Navigation;
using Pulsaar.Core;

namespace Pulsaar.Views;

public sealed partial class BluetoothDetailPage : Page
{
    public BluetoothDetailPage()
    {
        InitializeComponent();
    }

    protected override void OnNavigatedTo(NavigationEventArgs e)
    {
        base.OnNavigatedTo(e);
        if (e.Parameter is IReadOnlyList<DirectDeviceModel> devices)
            SubtitleText.Text = $"{devices.Count} device{(devices.Count == 1 ? "" : "s")} paired";
    }
}
