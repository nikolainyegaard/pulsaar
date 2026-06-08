using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Pulsaar.Core;

namespace Pulsaar.Controls;

public sealed partial class DeviceSettingsPanel : UserControl
{
    private DeviceModel? _device;
    private DeviceSettingsModel? _settings;
    private bool _updating;
    private int _lastTorque = -1;
    private int _lastBrightness = -1;

    public DeviceSettingsPanel()
    {
        InitializeComponent();
        DpiSlider.DpiChanged += DpiSlider_DpiChanged;
    }

    public void SetLoading()
    {
        LoadingPanel.Visibility   = Visibility.Visible;
        EmptyText.Visibility      = Visibility.Collapsed;
        SettingsScroll.Visibility = Visibility.Collapsed;
        SettingsScroll.IsEnabled  = true;
    }

    // Device is offline and has no cached settings -- nothing to show yet.
    public void SetOffline()
    {
        LoadingPanel.Visibility   = Visibility.Collapsed;
        SettingsScroll.Visibility = Visibility.Collapsed;
        EmptyText.Text            = "Device is offline";
        EmptyText.Visibility      = Visibility.Visible;
    }

    // Lock or unlock all interactive controls.
    // Locked = showing cached settings while device is offline; writes are suppressed.
    public void SetLocked(bool locked)
    {
        SettingsScroll.IsEnabled = !locked;
    }

    public void ApplySettings(DeviceModel device, DeviceSettingsModel? settings)
    {
        _device   = device;
        _settings = settings;
        _updating = true;

        // Reset transient states; caller calls SetLocked() after if needed.
        SettingsScroll.IsEnabled = true;
        LoadingPanel.Visibility  = Visibility.Collapsed;

        if (settings == null || !settings.HasAnySettings)
        {
            EmptyText.Text            = "No configurable settings";
            EmptyText.Visibility      = Visibility.Visible;
            SettingsScroll.Visibility = Visibility.Collapsed;
            _updating = false;
            return;
        }

        EmptyText.Visibility      = Visibility.Collapsed;
        SettingsScroll.Visibility = Visibility.Visible;

        // DPI
        DpiSection.Visibility = settings.HasDpi ? Visibility.Visible : Visibility.Collapsed;
        if (settings.HasDpi)
            DpiSlider.SetDpiList(settings.DpiList, settings.CurrentDpi);

        // Scroll Wheel
        ScrollSection.Visibility = settings.HasScrollSettings ? Visibility.Visible : Visibility.Collapsed;
        if (settings.HasScrollSettings)
        {
            InvertToggle.Visibility = settings.HasInvert ? Visibility.Visible : Visibility.Collapsed;
            InvertToggle.IsOn = settings.ScrollInverted;
            HiresToggle.Visibility = settings.HasHires ? Visibility.Visible : Visibility.Collapsed;
            HiresToggle.IsOn = settings.HiresEnabled;
        }

        // SmartShift
        SmartShiftSection.Visibility = settings.HasSmartShift ? Visibility.Visible : Visibility.Collapsed;
        if (settings.HasSmartShift)
        {
            WheelModeCombo.Items.Clear();
            WheelModeCombo.Items.Add(WheelMode.Freespin.Label());
            WheelModeCombo.Items.Add(WheelMode.SmartShift.Label());
            WheelModeCombo.SelectedIndex = settings.WheelMode == WheelMode.Freespin ? 0 : 1;

            bool showTorque = settings.WheelMode == WheelMode.SmartShift && settings.HasTorque;
            TorquePanel.Visibility = showTorque ? Visibility.Visible : Visibility.Collapsed;
            _lastTorque = settings.SmartShiftTorque;
            TorqueSlider.Value = settings.SmartShiftTorque;
        }

        // Change Host
        HostsSection.Visibility = settings.HasHosts ? Visibility.Visible : Visibility.Collapsed;
        if (settings.HasHosts && settings.Hosts != null)
        {
            HostCombo.Items.Clear();
            foreach (var host in settings.Hosts)
                HostCombo.Items.Add(host.Name.Length > 0 ? host.Name : $"Host {host.Slot}");
            int activeIdx = settings.Hosts.FindIndex(h => h.IsActive);
            HostCombo.SelectedIndex = activeIdx >= 0 ? activeIdx : 0;
        }

        // FN Swap
        FnSwapSection.Visibility = settings.HasFnSwap ? Visibility.Visible : Visibility.Collapsed;
        if (settings.HasFnSwap)
            FnSwapToggle.IsOn = settings.FnSwapped == true;

        // Multiplatform
        PlatformSection.Visibility = settings.HasMultiplatform ? Visibility.Visible : Visibility.Collapsed;
        if (settings.HasMultiplatform && settings.Platforms != null)
        {
            PlatformCombo.Items.Clear();
            foreach (var p in settings.Platforms)
                PlatformCombo.Items.Add(p.Name);
            PlatformCombo.SelectedIndex = Math.Clamp(settings.CurrentOsIdx, 0, settings.Platforms.Count - 1);
        }

        // Backlight
        BacklightSection.Visibility = settings.HasBacklight ? Visibility.Visible : Visibility.Collapsed;
        if (settings.HasBacklight)
        {
            BacklightModeCombo.Items.Clear();
            BacklightModeCombo.Items.Add(BacklightMode.Disabled.Label());
            BacklightModeCombo.Items.Add(BacklightMode.Automatic.Label());
            BacklightModeCombo.Items.Add(BacklightMode.Manual.Label());
            BacklightModeCombo.SelectedIndex = settings.BacklightMode switch
            {
                BacklightMode.Automatic => 1,
                BacklightMode.Manual    => 2,
                _                       => 0,
            };

            bool showBrightness = settings.BacklightMode == BacklightMode.Manual;
            BrightnessPanel.Visibility = showBrightness ? Visibility.Visible : Visibility.Collapsed;
            _lastBrightness = settings.BacklightBrightness;
            BrightnessSlider.Value = settings.BacklightBrightness;
        }

        _updating = false;
    }

    // --- DPI ---

    private void DpiSlider_DpiChanged(object? sender, int dpi)
    {
        if (_device == null) return;
        _ = App.Store.SetDpi(_device, (ushort)dpi);
    }

    // --- Scroll Wheel ---

    private void InvertToggle_Toggled(object sender, RoutedEventArgs e)
    {
        if (_updating || _device == null) return;
        _ = App.Store.SetScrollSettings(_device, InvertToggle.IsOn, HiresToggle.IsOn);
    }

    private void HiresToggle_Toggled(object sender, RoutedEventArgs e)
    {
        if (_updating || _device == null) return;
        _ = App.Store.SetScrollSettings(_device, InvertToggle.IsOn, HiresToggle.IsOn);
    }

    // --- SmartShift ---

    private void WheelModeCombo_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (_updating || _device == null || _settings == null) return;
        var mode = WheelModeCombo.SelectedIndex == 0 ? WheelMode.Freespin : WheelMode.SmartShift;
        bool showTorque = mode == WheelMode.SmartShift && _settings.HasTorque;
        TorquePanel.Visibility = showTorque ? Visibility.Visible : Visibility.Collapsed;
        _ = App.Store.SetSmartShift(_device, mode, _settings.SmartShiftTorque);
    }

    private void TorqueSlider_ValueChanged(object sender, RangeBaseValueChangedEventArgs e)
    {
        if (_updating || _device == null || _settings == null) return;

        int snapped = (int)Math.Round(e.NewValue / 5.0) * 5;
        snapped = Math.Clamp(snapped, 5, 100);

        if ((int)TorqueSlider.Value != snapped)
        {
            _updating = true;
            TorqueSlider.Value = snapped;
            _updating = false;
        }

        if (snapped != _lastTorque)
        {
            _lastTorque = snapped;
            var mode = WheelModeCombo.SelectedIndex == 0 ? WheelMode.Freespin : WheelMode.SmartShift;
            _ = App.Store.SetSmartShift(_device, mode, snapped);
        }
    }

    // --- Change Host ---

    private void HostCombo_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (_updating || _device == null || _settings?.Hosts == null) return;
        int idx = HostCombo.SelectedIndex;
        if (idx < 0 || idx >= _settings.Hosts.Count) return;
        _ = App.Store.SetActiveHost(_device, _settings.Hosts[idx].Slot);
    }

    // --- FN Swap ---

    private void FnSwapToggle_Toggled(object sender, RoutedEventArgs e)
    {
        if (_updating || _device == null) return;
        _ = App.Store.SetFnSwap(_device, FnSwapToggle.IsOn);
    }

    // --- Set OS ---

    private void PlatformCombo_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (_updating || _device == null || _settings?.Platforms == null) return;
        int idx = PlatformCombo.SelectedIndex;
        if (idx < 0 || idx >= _settings.Platforms.Count) return;
        _ = App.Store.SetMultiplatform(_device, _settings.Platforms[idx].Id);
    }

    // --- Backlight ---

    private void BacklightModeCombo_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (_updating || _device == null || _settings == null) return;
        var mode = BacklightModeCombo.SelectedIndex switch
        {
            1 => BacklightMode.Automatic,
            2 => BacklightMode.Manual,
            _ => BacklightMode.Disabled,
        };
        bool showBrightness = mode == BacklightMode.Manual;
        BrightnessPanel.Visibility = showBrightness ? Visibility.Visible : Visibility.Collapsed;
        _ = App.Store.SetBacklight(_device, mode, _settings.BacklightBrightness);
    }

    private void BrightnessSlider_ValueChanged(object sender, RangeBaseValueChangedEventArgs e)
    {
        if (_updating || _device == null) return;
        int value = (int)e.NewValue;
        if (value != _lastBrightness)
        {
            _lastBrightness = value;
            var mode = BacklightModeCombo.SelectedIndex switch
            {
                1 => BacklightMode.Automatic,
                2 => BacklightMode.Manual,
                _ => BacklightMode.Disabled,
            };
            _ = App.Store.SetBacklight(_device, mode, value);
        }
    }
}
