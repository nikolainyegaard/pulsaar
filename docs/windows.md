# Windows app (windows/)

WinUI 3 app in C#. Deprioritized: macOS leads, and Windows work resumes when it resumes. Stages 1-4 and 7 are built; stages 5, 6, and 8 remain (`ReceiverDetailPage` and `BluetoothDetailPage` exist but are title-only placeholders). The app mirrors the macOS app file-for-file; when in doubt, the Swift version is the reference.

## Decisions

- Language: C#, not C++/WinRT. Same WinUI 3 feature set, far more productive; CommunityToolkit.Mvvm gives an exact `@Observable` equivalent.
- Packaging: MSIX. No traditional installer; side-loading works without a cert for dev builds. Distributing outside the Store requires a code signing certificate (same situation as macOS notarization); without one, SmartScreen blocks the installer. Options: buy a cert (~$70-400/yr), distribute unsigned with `Add-AppxPackage` + Developer Mode instructions, or fall back to an unpackaged .exe with Inno Setup. Decision deferred until first release.
- Linking: dynamic. macOS links the static `.a`; C# P/Invoke needs a DLL. Cargo.toml has `cdylib` in crate-type, so `pulsaar_core.dll` builds automatically and ships inside the MSIX next to the .exe.
- HID: no exclusive-access problem on Windows; multiple handles are allowed simultaneously. `DoSettingsRefresh` therefore does NOT stop event listeners (the key behavioral difference from macOS). The receiver HID collection split (Col01/Col02 dual handle) is handled in the core; see [core.md](core.md).
- USB monitoring: `DeviceWatcher` (Windows.Devices.Enumeration), filtered by HID class GUID + Logitech vendor ID; 3s debounce on connect, immediate on removal. `DeviceAdded` fires for already-connected devices at startup (same as IOKit on macOS); the debounce handles it.
- Hostname for set_host_name: `Environment.MachineName`.
- Icons: Segoe Fluent Icons via `FontIcon` replaces SF Symbols; custom Bolt/Unifying/Bluetooth assets converted from the macOS SVGs (stage 8, not done; Assets/ currently has placeholder PNGs only).

## API mapping, macOS to Windows

| macOS | Windows |
|---|---|
| `@Observable` / `@Published` | `ObservableObject` + `[ObservableProperty]` |
| `NavigationSplitView` | `NavigationView` (left pane + `Frame` for detail) |
| `List` | `ListView` |
| `Toggle` | `ToggleSwitch` |
| `Picker` | `ComboBox` |
| `.sheet()` | `ContentDialog` |
| SF Symbols | Segoe Fluent Icons (`FontIcon`) |
| `Task {}` + `@MainActor` | `Task.Run()` + `DispatcherQueue.TryEnqueue()` |
| IOKit USB monitoring | `DeviceWatcher` |
| bridging header | P/Invoke in `NativeInterop.cs` |
| `Host.current().localizedName` | `Environment.MachineName` |
| `~/Library/Application Support/Pulsaar/` | `%APPDATA%\Pulsaar\` |

## Build

- `Pulsaar.csproj`: net9.0-windows10.0.19041.0, WinUI 3, MSIX packaging, `AllowUnsafeBlocks`, `WindowsAppSDKSelfContained=true` (avoids the runtime registration requirement). x64 + ARM64 only; x86 was dropped in the 2026-07 slim-down (taking `SetWindowLong32` and the `IntPtr.Size` branch with it).
- The `BuildRustCore` MSBuild target runs `cargo build` (release or debug per configuration) in `../core` before build and copies `pulsaar_core.dll` to the output directory.
- NuGet: `Microsoft.WindowsAppSDK`, `CommunityToolkit.Mvvm`, `CommunityToolkit.WinUI.Controls.Primitives` (optional helpers).
- Rust targets: `rustup target add x86_64-pc-windows-msvc`, plus `aarch64-pc-windows-msvc` for ARM builds. Multi-arch release means one build per target and one MSIX per architecture (no lipo equivalent); the CI matrix will handle both.

## P/Invoke (NativeInterop.cs)

Mirrors the bridging header one-for-one. Structs are `[StructLayout(LayoutKind.Sequential)]` with `unsafe fixed` fields for embedded arrays (`CMultiplatformSettings` uses `fixed byte platform_names[256]`); `[DllImport("pulsaar_core.dll", CallingConvention = CallingConvention.Cdecl)]`; `nint` for opaque context pointers. String extraction from fixed byte fields iterates to the null terminator and decodes UTF-8 (`CStringToString(byte* ptr, int maxLen)`, the equivalent of macOS `cBufToString`); `CHostList.GetHost` works via pointer arithmetic. Windows uses streaming enumeration, so the bulk `pulsaar_enumerate_devices`/`get_device_count`/`get_device_info` declarations went away with those exports in the 2026-07 slim-down.

## Threading

All HID work runs in `Task.Run()`; UI property updates go through `DispatcherQueue.TryEnqueue()`. Identical in intent to Swift's `await MainActor.run {}`. `App.xaml.cs` creates the ReceiverStore after MainWindow so a DispatcherQueue exists.

## What is built

Stage 1 (scaffold + P/Invoke), Stage 2 (ReceiverStore), Stage 3 (sidebar), Stage 4 (device detail), Stage 7 (DeviceCache):

- `Models.cs`: all enums with Label/Glyph extensions; BatteryModel, DeviceModel, DirectDeviceModel, ReceiverModel (factory methods take pointer parameters for correct fixed-array access); `DeviceSettingsModel.FromAllSettings` reads all 7 feature blocks via struct pointers. The pairing flow and the Stage 5/8 model surface (Labels, ProductId, Serial, MaxDevices, ConnectionLabel, IsCached) are deliberately kept for the upcoming stages.
- `ReceiverStore.cs`: ObservableObject with DeviceWatcher USB monitoring (3s debounce), 250ms event poll timer, 750ms settings refresh debounce, PrefetchSettings (writes the hostname to the active host slot), LoadSettings (cache-first with 6s prefetch wait), all Set* write methods with cache write-through + toast, and the pairing flow with poll timer and FinalizePairing.
- Streaming enumeration: Reload is two-phase. Phase 1 opens receivers and shows them in the sidebar immediately (~30ms). Phase 2 calls `pulsaar_start_enumerate` and loops `pulsaar_enumerate_next_device`, adding each device as found so they trickle in. PrefetchSettings raced with phase 2; fixed with a `.ToList()` snapshot in the device loop.
- Offline device guards: PrefetchSettings, DoSettingsRefresh, and LoadSettings all skip devices with `IsOnline == false`; `get_all_settings` on an offline device wastes a full feature-discovery timeout. LoadSettings still returns cached settings for offline devices.
- `DeviceCache.cs`: battery (`device-cache.json`) and device names (`device-names.json`) as JSON in `%APPDATA%\Pulsaar\`, same schema as macOS; `System.Text.Json` (built in, no dependency). Same UpdateBattery/Battery/UpdateName/Name contract, called from Reload and PrefetchSettings identically to macOS.
- Sidebar: `SidebarItems.cs` defines SidebarReceiverItem/SidebarDeviceItem/SidebarBluetoothItem with computed Visibility/glyph/battery properties for x:Bind; `SidebarTemplateSelector` picks the template by item type; MainWindow's ListView draws tree connector lines in a Canvas with battery indicators; BuildSidebar sorts receivers + the BT row alphabetically, restores selection across rebuilds, and navigates the content Frame on SelectionChanged.
- Device detail: `DeviceHeader` (kind glyph, name, online/offline text, battery glyph + level, set from code-behind via `Show(DeviceModel)`); `LogDpiSlider` (index-based Slider 0..N-1 with StepFrequency=1, Canvas tick labels at evenly spaced indices, DpiChanged only on actual change, SizeChanged rebuilds ticks); `DeviceSettingsPanel` with 7 conditionally visible sections (DPI, Scroll Wheel, Scroll Mode/SmartShift, Change Host, FN Swap, Set OS, Backlight), ProgressRing loading state, empty state, an `_updating` flag against recursive event handlers, torque slider snapping to 5%, all handlers calling `App.Store.Set*()`; `DeviceDetailPage` loads settings on navigation, gates the Unpair button on IsPrefetching, and unsubscribes PropertyChanged on NavigatedFrom.
- `MainWindow`: ExtendsContentIntoTitleBar, 900x620 initial size. `Package.appxmanifest`: runFullTrust, placeholder PNG assets in all required sizes.

Build fixes worth knowing: WinUI 3 has no `Window.Resources`, DataTemplates go in `Grid.Resources`; CS1686 means copy address-taken locals before lambda capture; `Microsoft.UI.Xaml` must be explicitly imported in UserControl code-behind (not in the global usings), which also resolves the `Visibility` type-vs-property ambiguity.

## Remaining stages

- Stage 5, receiver + Bluetooth detail views: `ReceiverDetailPage` (Bolt/Unifying icon, name, serial, product ID, X of Y devices, "Pair new device" button), `BluetoothDetailPage` (BT logo, machine name, device count, device list, "Open Bluetooth Settings" via the `ms-settings:bluetooth` URI).
- Stage 6, pairing: `PairingDialog` as a ContentDialog with per-state content (waiting ProgressRing, device found, numeric passkey, L/R button sequence, paired checkmark with 2s auto-close, failed with Retry), launched from ReceiverDetailPage.
- Stage 8, polish: toast overlay (Border at the bottom of the detail area, DispatcherTimer auto-hide after 2s, fade or slide), custom Bolt/Unifying/Bluetooth icons converted from the macOS SVGs (XAML PathGeometry or Image), app icon in Package.appxmanifest, a Force Refresh affordance (no menu bar on Windows; sidebar header button or receiver context menu), minimum window size in MainWindow.xaml.cs.
