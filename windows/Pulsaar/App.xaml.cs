using Microsoft.UI.Xaml;
using Pulsaar.Core;

namespace Pulsaar;

public partial class App : Application
{
    public static ReceiverStore Store { get; private set; } = null!;

    public App()
    {
        InitializeComponent();
    }

    protected override void OnLaunched(LaunchActivatedEventArgs args)
    {
        var window = new MainWindow();
        Store = new ReceiverStore(window.DispatcherQueue);
        window.Activate();
        window.OnStoreReady();
    }
}
