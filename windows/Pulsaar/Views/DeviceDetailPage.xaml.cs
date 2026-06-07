using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Navigation;
using Pulsaar.Core;

namespace Pulsaar.Views;

public sealed partial class DeviceDetailPage : Page
{
    public DeviceDetailPage()
    {
        InitializeComponent();
    }

    protected override void OnNavigatedTo(NavigationEventArgs e)
    {
        base.OnNavigatedTo(e);
        if (e.Parameter is DeviceModel device)
            TitleText.Text = device.Name;
    }
}
