using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Pulsaar.Core;
using Windows.Graphics;

namespace Pulsaar;

public sealed partial class MainWindow : Window
{
    private bool _suppressNavigation;
    private bool _initialNavDone;
    private string? _selectedKey;

    public MainWindow()
    {
        InitializeComponent();

        Title = "Pulsaar";
        AppWindow.Resize(new SizeInt32(900, 620));
        ExtendsContentIntoTitleBar = true;
        SetTitleBar(AppTitleBar);
    }

    public void OnStoreReady()
    {
        var store = App.Store;
        store.PropertyChanged += (_, e) =>
        {
            if (e.PropertyName is nameof(ReceiverStore.Receivers)
                               or nameof(ReceiverStore.DirectDevices))
                BuildSidebar();
        };
        BuildSidebar();
    }

    private void BuildSidebar()
    {
        var store = App.Store;
        var groups = new List<(string sortKey, object header, List<object> children)>();

        foreach (var receiver in store.Receivers)
        {
            var children = new List<object>();
            for (int i = 0; i < receiver.Devices.Count; i++)
                children.Add(new SidebarDeviceItem(receiver.Devices[i], isLast: i == receiver.Devices.Count - 1));
            groups.Add((receiver.Name, new SidebarReceiverItem(receiver), children));
        }

        if (store.DirectDevices.Count > 0)
            groups.Add(("Bluetooth", new SidebarBluetoothItem(store.DirectDevices), []));

        groups.Sort((a, b) => string.Compare(a.sortKey, b.sortKey, StringComparison.OrdinalIgnoreCase));

        var items = new List<object>();
        foreach (var (_, header, children) in groups)
        {
            items.Add(header);
            items.AddRange(children);
        }

        _suppressNavigation = true;
        SidebarList.ItemsSource = items;
        _suppressNavigation = false;

        // Restore selection after a rebuild (e.g. USB reconnect)
        bool restored = false;
        if (_selectedKey != null)
        {
            foreach (var item in items)
            {
                if (ItemKey(item) == _selectedKey)
                {
                    _suppressNavigation = true;
                    SidebarList.SelectedItem = item;
                    _suppressNavigation = false;
                    restored = true;
                    break;
                }
            }
        }

        if (!restored && !_initialNavDone && items.Count > 0)
        {
            SidebarList.SelectedItem = items[0];
            _initialNavDone = true;
        }
    }

    private static string? ItemKey(object item) => item switch
    {
        SidebarReceiverItem r => "r:" + r.Receiver.Id,
        SidebarDeviceItem d   => "d:" + d.Device.Id,
        SidebarBluetoothItem  => "bt",
        _                     => null,
    };

    private void SidebarList_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (_suppressNavigation) return;

        var item = SidebarList.SelectedItem;
        _selectedKey = ItemKey(item);

        switch (item)
        {
            case SidebarReceiverItem r:
                ContentFrame.Navigate(typeof(Views.ReceiverDetailPage), r.Receiver);
                break;
            case SidebarDeviceItem d:
                ContentFrame.Navigate(typeof(Views.DeviceDetailPage), d.Device);
                break;
            case SidebarBluetoothItem bt:
                ContentFrame.Navigate(typeof(Views.BluetoothDetailPage), bt.Devices);
                break;
        }
    }
}
