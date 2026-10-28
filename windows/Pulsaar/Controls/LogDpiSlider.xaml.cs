using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Shapes;
using Windows.Foundation;

namespace Pulsaar.Controls;

public sealed partial class LogDpiSlider : UserControl
{
    private IReadOnlyList<int>? _snapList;
    private bool _updating;
    private int _lastDpi = -1;
    private int _pendingDpi = -1;
    private bool _isDragging;

    public event EventHandler<int>? DpiChanged;

    public LogDpiSlider()
    {
        InitializeComponent();
        TickCanvas.SizeChanged += (_, _) => RebuildTicks();

        // Slider's Thumb handles PointerPressed internally, so handledEventsToo is required.
        DpiSliderControl.AddHandler(
            UIElement.PointerPressedEvent,
            new PointerEventHandler((_, _) => _isDragging = true),
            handledEventsToo: true);

        DpiSliderControl.PointerCaptureLost += (_, _) =>
        {
            _isDragging = false;
            if (_pendingDpi != -1 && _pendingDpi != _lastDpi)
            {
                _lastDpi = _pendingDpi;
                DpiChanged?.Invoke(this, _pendingDpi);
            }
        };
    }

    public void SetDpiList(IReadOnlyList<int> dpiList, int currentDpi)
    {
        _snapList = BuildSnapList(dpiList);

        _updating = true;
        DpiSliderControl.Minimum = 0;
        DpiSliderControl.Maximum = _snapList.Count - 1;
        DpiSliderControl.StepFrequency = 1;

        int idx = FindNearestIndex(_snapList, currentDpi);
        DpiSliderControl.Value = idx;
        _lastDpi = currentDpi;
        _pendingDpi = currentDpi;
        DpiReadout.Text = currentDpi.ToString();
        _updating = false;

        RebuildTicks();
    }

    // Curated snap list matching the macOS implementation exactly:
    //   50-1000  : 50-step
    //   1100-2000: 100-step
    //   2250-4000: 250-step
    //   4500-8000: 500-step
    // Each candidate is mapped to the nearest actual device DPI, deduped,
    // then device min and max are always included.
    // Falls back to the full list for devices with 20 or fewer DPI options.
    private static IReadOnlyList<int> BuildSnapList(IReadOnlyList<int> dpiList)
    {
        if (dpiList.Count <= 20) return dpiList;

        int lo = dpiList[0];
        int hi = dpiList[^1];

        var candidates = new List<int>();
        for (int v =   50; v <= 1000; v +=  50) candidates.Add(v);
        for (int v = 1100; v <= 2000; v += 100) candidates.Add(v);
        for (int v = 2250; v <= 4000; v += 250) candidates.Add(v);
        for (int v = 4500; v <= 8000; v += 500) candidates.Add(v);

        var mapped = new HashSet<int>(
            candidates
                .Where(c => c >= lo && c <= hi)
                .Select(c => dpiList[FindNearestIndex(dpiList, c)]));

        var sorted = mapped.Order().ToList();
        if (sorted.Count == 0) return dpiList;
        if (sorted[0] != lo) sorted.Insert(0, lo);
        if (sorted[^1] != hi) sorted.Add(hi);
        return sorted;
    }

    private static int FindNearestIndex(IReadOnlyList<int> list, int target)
    {
        int bestIdx = 0;
        int bestDiff = Math.Abs(list[0] - target);
        for (int i = 1; i < list.Count; i++)
        {
            int diff = Math.Abs(list[i] - target);
            if (diff < bestDiff) { bestDiff = diff; bestIdx = i; }
        }
        return bestIdx;
    }

    private void RebuildTicks()
    {
        TickCanvas.Children.Clear();
        var list = _snapList;
        if (list == null || list.Count < 2) return;

        double w = TickCanvas.ActualWidth;
        if (w < 10) return;

        Brush labelBrush, tickBrush;
        try
        {
            labelBrush = (Brush)Application.Current.Resources["TextFillColorSecondaryBrush"];
            tickBrush  = (Brush)Application.Current.Resources["TextFillColorTertiaryBrush"];
        }
        catch { return; }

        // Landmark DPIs: device min + max always present, plus any of the fixed set
        // that fall strictly between them. Matches the macOS implementation exactly.
        int lo = list[0];
        int hi = list[^1];
        var baseLandmarks = new[] { 200, 600, 1000, 2000, 4000, 8000 };
        var landmarks = new List<int> { lo };
        foreach (int d in baseLandmarks)
            if (d > lo && d < hi) landmarks.Add(d);
        landmarks.Add(hi);

        for (int li = 0; li < landmarks.Count; li++)
        {
            int dpi = landmarks[li];
            int nearestIdx = FindNearestIndex(list, dpi);
            double x = (double)nearestIdx / (list.Count - 1) * w;

            var tick = new Rectangle { Width = 1, Height = 5, Fill = tickBrush };
            Canvas.SetLeft(tick, x);
            TickCanvas.Children.Add(tick);

            string label = dpi >= 1000 ? $"{dpi / 1000}k" : dpi.ToString();
            var tb = new TextBlock { Text = label, FontSize = 10, Foreground = labelBrush };
            tb.Measure(new Size(200, 30));
            double tw = tb.DesiredSize.Width;

            // Left-align first label, right-align last, center-align the rest. Matches macOS.
            double drawX = li == 0 ? x : li == landmarks.Count - 1 ? x - tw : x - tw / 2;
            Canvas.SetLeft(tb, drawX);
            Canvas.SetTop(tb, 7);
            TickCanvas.Children.Add(tb);
        }
    }

    private void DpiSlider_ValueChanged(object sender, RangeBaseValueChangedEventArgs e)
    {
        if (_updating || _snapList == null) return;
        int idx = (int)Math.Round(e.NewValue);
        idx = Math.Clamp(idx, 0, _snapList.Count - 1);
        int dpi = _snapList[idx];
        _pendingDpi = dpi;
        DpiReadout.Text = dpi.ToString();

        // During drag, defer the write to PointerCaptureLost so we only write once on release.
        // For keyboard navigation there is no pointer capture, so fire immediately.
        if (!_isDragging && dpi != _lastDpi)
        {
            _lastDpi = dpi;
            DpiChanged?.Invoke(this, dpi);
        }
    }
}
