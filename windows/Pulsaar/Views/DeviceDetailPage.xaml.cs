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
        App.Store.PropertyChanged += Store_PropertyChanged;
        UpdateUnpairEnabled();

        // Show cached settings immediately if available; lock them when the device is offline
        // so the user can see the last known state without a loading screen.
        // Only show the spinner for online devices where we're about to read fresh settings.
        if (App.Store.SettingsCache.TryGetValue(device.Id, out var cached))
        {
            SettingsPanel.ApplySettings(device, cached);
            SettingsPanel.SetLocked(!device.IsOnline);
        }
        else if (!device.IsOnline)
        {
            SettingsPanel.SetOffline();
        }
        else
        {
            SettingsPanel.SetLoading();
            _ = LoadSettingsAsync(device);
        }
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

        // Re-apply when prefetch completes or a settings-change event refreshes the cache.
        if (e.PropertyName == nameof(App.Store.SettingsCacheVersion))
            DispatcherQueue.TryEnqueue(RefreshFromCache);

        // Reload created new DeviceModel instances -- sync our reference and update header/lock.
        if (e.PropertyName == nameof(App.Store.Receivers))
            DispatcherQueue.TryEnqueue(SyncDeviceModel);
    }

    private void UpdateUnpairEnabled()
    {
        UnpairButton.IsEnabled = !App.Store.IsPrefetching;
    }

    // Called when the Receivers list changes (a Reload completed).
    // The Reload creates fresh DeviceModel instances, so our _device reference becomes stale.
    // Find the current instance by id and update header + lock state accordingly.
    private void SyncDeviceModel()
    {
        if (_device == null) return;

        var current = App.Store.Receivers
            .SelectMany(r => r.Devices)
            .FirstOrDefault(d => d.Id == _device.Id);

        if (current == null) return;

        bool wasOnline = _device.IsOnline;
        _device = current;
        DevHeader.Show(_device);

        bool nowOnline = _device.IsOnline;

        if (nowOnline && !wasOnline)
        {
            // Device just woke up. Show cached settings (if any) unlocked while the store
            // fetches fresh values; SettingsCacheVersion fires when the refresh is done.
            if (App.Store.SettingsCache.TryGetValue(_device.Id, out var cached))
            {
                SettingsPanel.ApplySettings(_device, cached);
                SettingsPanel.SetLocked(false);
            }
            else
            {
                SettingsPanel.SetLoading();
            }
        }
        else if (!nowOnline && wasOnline)
        {
            // Device just went offline. Lock cached settings, or show offline message.
            if (App.Store.SettingsCache.TryGetValue(_device.Id, out var cached))
            {
                SettingsPanel.ApplySettings(_device, cached);
                SettingsPanel.SetLocked(true);
            }
            else
            {
                SettingsPanel.SetOffline();
            }
        }
        // No online-state change: no settings action needed (battery update etc.)
    }

    private void RefreshFromCache()
    {
        if (_device == null) return;
        DevHeader.Show(_device);
        if (App.Store.SettingsCache.TryGetValue(_device.Id, out var settings))
        {
            SettingsPanel.ApplySettings(_device, settings);
            SettingsPanel.SetLocked(!_device.IsOnline);
        }
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
