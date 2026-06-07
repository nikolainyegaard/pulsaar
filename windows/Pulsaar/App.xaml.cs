using Microsoft.UI.Xaml;
using Pulsaar.Core;

namespace Pulsaar;

public partial class App : Application
{
    public static ReceiverStore Store { get; private set; } = null!;
    public static MainWindow MainWindow { get; private set; } = null!;

    public App()
    {
        InitializeComponent();
    }

    protected override void OnLaunched(LaunchActivatedEventArgs args)
    {
        MainWindow = new MainWindow();
        Store = new ReceiverStore(MainWindow.DispatcherQueue);
        MainWindow.Activate();
        MainWindow.OnStoreReady();
    }
}
