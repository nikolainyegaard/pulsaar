using Microsoft.UI;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Windows.Graphics;

namespace Pulsaar;

public sealed partial class MainWindow : Window
{
    public MainWindow()
    {
        InitializeComponent();

        Title = "Pulsaar";
        AppWindow.Resize(new SizeInt32(900, 620));

        ExtendsContentIntoTitleBar = true;
    }

    // Called from App.xaml.cs once ReceiverStore is ready.
    public void OnStoreReady()
    {
        var store = App.Store;
        SubText.Text = store.ErrorMessage ?? "Ready";

        // Stage 3: replace with NavigationView layout.
    }
}
