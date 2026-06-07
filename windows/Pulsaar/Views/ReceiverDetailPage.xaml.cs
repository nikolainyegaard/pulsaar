using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Navigation;
using Pulsaar.Core;

namespace Pulsaar.Views;

public sealed partial class ReceiverDetailPage : Page
{
    public ReceiverDetailPage()
    {
        InitializeComponent();
    }

    protected override void OnNavigatedTo(NavigationEventArgs e)
    {
        base.OnNavigatedTo(e);
        if (e.Parameter is ReceiverModel receiver)
            TitleText.Text = receiver.Name;
    }
}
