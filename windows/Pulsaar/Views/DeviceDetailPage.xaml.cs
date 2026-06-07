using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Navigation;
using Pulsaar.Core;

namespace Pulsaar.Views;

public sealed partial class DeviceDetailPage : Page
{
    private DeviceModel? _device;

    public DeviceDetailPage()
    {
        InitializeComponent();
    }

    protected override void OnNavigatedTo(NavigationEventArgs e)
    {
        base.OnNavigatedTo(e);
        if (e.Parameter is not DeviceModel device) return;

        _device = device;
        DevHeader.Show(device);
        SettingsPanel.SetLoading();

        App.Store.PropertyChanged += Store_PropertyChanged;
        UpdateUnpairEnabled();

        _ = LoadSettingsAsync(device);
    }

    protected override void OnNavigatedFrom(NavigationEventArgs e)
    {
        base.OnNavigatedFrom(e);
        App.Store.PropertyChanged -= Store_PropertyChanged;
    }

    private void Store_PropertyChanged(object? sender, System.ComponentModel.PropertyChangedEventArgs e)
    {
        if (e.PropertyName == nameof(App.Store.IsPrefetching))
            DispatcherQueue.TryEnqueue(UpdateUnpairEnabled);
    }

    private void UpdateUnpairEnabled()
    {
        UnpairButton.IsEnabled = !App.Store.IsPrefetching;
    }

    private async Task LoadSettingsAsync(DeviceModel device)
    {
        var settings = await App.Store.LoadSettings(device);
        SettingsPanel.ApplySettings(device, settings);
    }

    private async void UnpairButton_Click(object sender, RoutedEventArgs e)
    {
        if (_device == null) return;
        UnpairButton.IsEnabled = false;
        await App.Store.Unpair(_device);
    }
}
