using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Shapes;
using Windows.Foundation;

namespace Pulsaar.Controls;

public sealed partial class LogDpiSlider : UserControl
{
    private IReadOnlyList<int>? _dpiList;
    private bool _updating;
    private int _lastDpi = -1;

    public event EventHandler<int>? DpiChanged;

    public LogDpiSlider()
    {
        InitializeComponent();
        TickCanvas.SizeChanged += (_, _) => RebuildTicks();
    }

    public void SetDpiList(IReadOnlyList<int> dpiList, int currentDpi)
    {
        _dpiList = dpiList;

        _updating = true;
        DpiSliderControl.Minimum = 0;
        DpiSliderControl.Maximum = dpiList.Count - 1;
        DpiSliderControl.StepFrequency = 1;

        int idx = FindIndex(dpiList, currentDpi);
        DpiSliderControl.Value = idx;
        _lastDpi = currentDpi;
        _updating = false;

        RebuildTicks();
    }

    private static int FindIndex(IReadOnlyList<int> list, int value)
    {
        for (int i = 0; i < list.Count; i++)
            if (list[i] == value) return i;
        return 0;
    }

    private void RebuildTicks()
    {
        TickCanvas.Children.Clear();
        var list = _dpiList;
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

        // Show at most 9 labels spaced by index, always including the first and last.
        int maxLabels = 9;
        int step = Math.Max(1, (list.Count - 1) / (maxLabels - 1));
        var indices = new List<int>();
        for (int i = 0; i < list.Count - 1; i += step)
            indices.Add(i);
        if (indices.Count == 0 || indices[^1] != list.Count - 1)
            indices.Add(list.Count - 1);

        foreach (int i in indices)
        {
            double x = (double)i / (list.Count - 1) * w;

            var tick = new Rectangle { Width = 1, Height = 5, Fill = tickBrush };
            Canvas.SetLeft(tick, x);
            TickCanvas.Children.Add(tick);

            int dpi = list[i];
            string label = dpi >= 1000 ? $"{dpi / 1000}k" : dpi.ToString();
            var tb = new TextBlock { Text = label, FontSize = 10, Foreground = labelBrush };
            tb.Measure(new Size(200, 30));
            Canvas.SetLeft(tb, x - tb.DesiredSize.Width / 2);
            Canvas.SetTop(tb, 7);
            TickCanvas.Children.Add(tb);
        }
    }

    private void DpiSlider_ValueChanged(object sender, RangeBaseValueChangedEventArgs e)
    {
        if (_updating || _dpiList == null) return;
        int idx = (int)Math.Round(e.NewValue);
        idx = Math.Clamp(idx, 0, _dpiList.Count - 1);
        int dpi = _dpiList[idx];
        if (dpi != _lastDpi)
        {
            _lastDpi = dpi;
            DpiChanged?.Invoke(this, dpi);
        }
    }
}
