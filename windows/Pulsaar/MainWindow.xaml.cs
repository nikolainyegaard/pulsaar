using System.Collections.ObjectModel;
using System.Runtime.InteropServices;
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

    // Sidebar collection: set once as ItemsSource, updated incrementally.
    private readonly ObservableCollection<object> _sidebarItems = new();

    // ---------------------------------------------------------------------------
    // Raw input: detect devices waking from inactivity via Col00 input reports.
    // Bolt receivers don't send HID++ notifications on Col02 for inactivity
    // wakeup -- the device just starts sending mouse/keyboard input through the
    // standard HID collection. We monitor that here and poke the store when
    // input arrives from a Logitech device while we have offline entries.
    // ---------------------------------------------------------------------------

    // P/Invoke -- kept local since nothing else uses these.
    [StructLayout(LayoutKind.Sequential)]
    private struct RAWINPUTDEVICE
    {
        public ushort usUsagePage;
        public ushort usUsage;
        public uint   dwFlags;
        public IntPtr hwndTarget;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct RAWINPUTHEADER
    {
        public uint   dwType;
        public uint   dwSize;
        public IntPtr hDevice;
        public IntPtr wParam;
    }

    private const uint RIDEV_INPUTSINK = 0x00000100;
    private const uint RID_INPUT       = 0x10000003;
    private const uint RIDI_DEVICENAME = 0x20000007;
    private const int  GWLP_WNDPROC    = -4;
    private const int  WM_INPUT        = 0x00FF;

    [DllImport("user32.dll")]
    private static extern bool RegisterRawInputDevices(
        [MarshalAs(UnmanagedType.LPArray, SizeParamIndex = 1)]
        RAWINPUTDEVICE[] devices, uint numDevices, uint cbSize);

    [DllImport("user32.dll")]
    private static extern unsafe uint GetRawInputData(
        IntPtr hRawInput, uint uiCommand, void* pData, ref uint pcbSize, uint cbSizeHeader);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern unsafe uint GetRawInputDeviceInfoW(
        IntPtr hDevice, uint uiCommand, void* pData, ref uint pcbSize);

    [DllImport("user32.dll", EntryPoint = "SetWindowLongPtrW")]
    private static extern IntPtr SetWindowLongPtr64(IntPtr hWnd, int nIndex, IntPtr newLong);

    [DllImport("user32.dll", EntryPoint = "SetWindowLongW")]
    private static extern int SetWindowLong32(IntPtr hWnd, int nIndex, int newLong);

    [DllImport("user32.dll", EntryPoint = "CallWindowProcW")]
    private static extern IntPtr CallWindowProc(
        IntPtr lpPrevWndFunc, IntPtr hWnd, uint msg, IntPtr wParam, IntPtr lParam);

    private delegate IntPtr WndProcDelegate(IntPtr hwnd, uint msg, IntPtr wParam, IntPtr lParam);
    private WndProcDelegate? _wndProcDelegate;   // must be rooted to prevent GC
    private IntPtr           _originalWndProc;
    private readonly Dictionary<IntPtr, bool> _rawInputDeviceCache = new();

    private void SetupRawInput()
    {
        var hwnd = WinRT.Interop.WindowNative.GetWindowHandle(this);

        // Receive mouse and keyboard raw input even when the window is not in the
        // foreground (RIDEV_INPUTSINK) so offline-device detection works while
        // Pulsaar is minimised or behind another window.
        var devices = new RAWINPUTDEVICE[]
        {
            new() { usUsagePage = 0x01, usUsage = 0x02, dwFlags = RIDEV_INPUTSINK, hwndTarget = hwnd },
            new() { usUsagePage = 0x01, usUsage = 0x06, dwFlags = RIDEV_INPUTSINK, hwndTarget = hwnd },
        };
        RegisterRawInputDevices(devices, (uint)devices.Length, (uint)Marshal.SizeOf<RAWINPUTDEVICE>());

        // Subclass the window proc. Keep the delegate alive in a field so it is
        // never collected while the subclass is in place.
        _wndProcDelegate = CustomWndProc;
        var newProc = Marshal.GetFunctionPointerForDelegate(_wndProcDelegate);
        _originalWndProc = IntPtr.Size == 8
            ? SetWindowLongPtr64(hwnd, GWLP_WNDPROC, newProc)
            : new IntPtr(SetWindowLong32(hwnd, GWLP_WNDPROC, newProc.ToInt32()));
    }

    private unsafe IntPtr CustomWndProc(IntPtr hwnd, uint msg, IntPtr wParam, IntPtr lParam)
    {
        if (msg == WM_INPUT)
        {
            uint cbSize = 0;
            GetRawInputData(lParam, RID_INPUT, null, ref cbSize, (uint)sizeof(RAWINPUTHEADER));

            if (cbSize > 0 && cbSize <= 512)
            {
                byte* buf = stackalloc byte[(int)cbSize];
                if (GetRawInputData(lParam, RID_INPUT, buf, ref cbSize, (uint)sizeof(RAWINPUTHEADER)) != uint.MaxValue)
                {
                    var hDevice = ((RAWINPUTHEADER*)buf)->hDevice;
                    if (IsLogiDevice(hDevice))
                        App.Store?.OnRawInputActivity();
                }
            }
        }
        return CallWindowProc(_originalWndProc, hwnd, msg, wParam, lParam);
    }

    private unsafe bool IsLogiDevice(IntPtr hDevice)
    {
        if (_rawInputDeviceCache.TryGetValue(hDevice, out bool cached)) return cached;

        uint nameLen = 0;
        GetRawInputDeviceInfoW(hDevice, RIDI_DEVICENAME, null, ref nameLen);
        if (nameLen == 0) { _rawInputDeviceCache[hDevice] = false; return false; }

        // nameLen is in WCHAR units including null terminator
        char* nameBuf = stackalloc char[(int)nameLen];
        GetRawInputDeviceInfoW(hDevice, RIDI_DEVICENAME, nameBuf, ref nameLen);
        var path = new string(nameBuf);

        bool isLogi = path.Contains("VID_046D", StringComparison.OrdinalIgnoreCase);
        _rawInputDeviceCache[hDevice] = isLogi;
        return isLogi;
    }

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
        // Bind the sidebar to the incremental collection once -- never reassigned.
        SidebarList.ItemsSource = _sidebarItems;

        var store = App.Store;
        store.PropertyChanged += (_, e) =>
        {
            if (e.PropertyName is nameof(ReceiverStore.Receivers)
                               or nameof(ReceiverStore.DirectDevices))
                BuildSidebar();
        };
        BuildSidebar();
        SetupRawInput();
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

        // Suppress navigation events while syncing the collection.
        _suppressNavigation = true;
        SyncSidebar(items);

        // Restore selection by key (the previous item reference may have been replaced).
        if (_selectedKey != null)
        {
            var match = _sidebarItems.FirstOrDefault(x => ItemKey(x) == _selectedKey);
            if (match != null && !ReferenceEquals(SidebarList.SelectedItem, match))
                SidebarList.SelectedItem = match;
        }
        _suppressNavigation = false;

        // First-ever selection: navigate to the top item.
        if (!_initialNavDone && _sidebarItems.Count > 0 && SidebarList.SelectedItem == null)
        {
            SidebarList.SelectedItem = _sidebarItems[0];
            _initialNavDone = true;
        }
    }

    // Sync _sidebarItems to the desired list with minimal churn.
    // When structure is unchanged (same keys in the same order), only items whose
    // visible state differs are replaced in-place -- the ListView updates only those
    // containers rather than tearing down and rebuilding the whole list.
    private void SyncSidebar(List<object> desired)
    {
        // Compare key sequences.
        bool sameStructure = _sidebarItems.Count == desired.Count;
        if (sameStructure)
        {
            for (int i = 0; i < desired.Count; i++)
            {
                if (ItemKey(_sidebarItems[i]) != ItemKey(desired[i]))
                {
                    sameStructure = false;
                    break;
                }
            }
        }

        if (sameStructure)
        {
            // Replace only items whose visible state actually changed.
            for (int i = 0; i < desired.Count; i++)
            {
                if (!SidebarItemsVisuallyEqual(_sidebarItems[i], desired[i]))
                    _sidebarItems[i] = desired[i];
            }
            return;
        }

        // Structure changed (receiver or device added/removed): full reset.
        _sidebarItems.Clear();
        foreach (var item in desired)
            _sidebarItems.Add(item);
    }

    private static bool SidebarItemsVisuallyEqual(object a, object b)
    {
        if (a is SidebarDeviceItem da && b is SidebarDeviceItem db)
            return da.Device.IsOnline         == db.Device.IsOnline
                && da.Device.Name             == db.Device.Name
                && da.Device.Battery?.Level   == db.Device.Battery?.Level
                && da.Device.Battery?.IsCharging == db.Device.Battery?.IsCharging
                && da.IsLast                  == db.IsLast;
        if (a is SidebarReceiverItem ra && b is SidebarReceiverItem rb)
            return ra.Receiver.Devices.Count  == rb.Receiver.Devices.Count;
        if (a is SidebarBluetoothItem bta && b is SidebarBluetoothItem btb)
            return bta.Count == btb.Count;
        return false;
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
