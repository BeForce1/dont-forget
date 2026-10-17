using System.Drawing.Drawing2D;
using System.Drawing.Imaging;

namespace DontForget;

// Full-screen frozen screenshot, dimmed, with the selected area shown bright.
sealed class SelectionOverlay : Form
{
    // Like CAD box selection: left-to-right is green, right-to-left (adds to followers) is blue.
    public static readonly Color Green = Color.FromArgb(46, 204, 113);
    public static readonly Color Blue = Color.FromArgb(52, 152, 255);
    readonly Bitmap shot, dim;
    readonly Rectangle vs;
    readonly Point start;
    readonly System.Windows.Forms.Timer timer = new() { Interval = 10 };
    readonly Font font = new("Segoe UI", 12f, FontStyle.Bold, GraphicsUnit.Pixel);
    Point cur;

    // additive = dragged right-to-left: the new note joins the ones already following.
    public event Action<Bitmap, Rectangle, bool> Captured;

    public SelectionOverlay(Point start)
    {
        this.start = start;
        cur = Cursor.Position;
        vs = SystemInformation.VirtualScreen;
        shot = new Bitmap(vs.Width, vs.Height, PixelFormat.Format32bppPArgb);
        using (var g = Graphics.FromImage(shot)) g.CopyFromScreen(vs.Location, Point.Empty, vs.Size);
        dim = new Bitmap(vs.Width, vs.Height, PixelFormat.Format32bppPArgb);
        using (var g = Graphics.FromImage(dim))
        using (var shade = new SolidBrush(Color.FromArgb(120, 10, 10, 20)))
        {
            g.DrawImageUnscaled(shot, 0, 0);
            g.FillRectangle(shade, 0, 0, vs.Width, vs.Height);
        }

        FormBorderStyle = FormBorderStyle.None;
        StartPosition = FormStartPosition.Manual;
        AutoScaleMode = AutoScaleMode.None;
        ShowInTaskbar = false;
        TopMost = true;
        DoubleBuffered = true;
        KeyPreview = true;
        Cursor = Cursors.Cross;
        Bounds = vs;
        timer.Tick += (_, _) => Track(Cursor.Position);
    }

    Rectangle Selection => Rectangle.FromLTRB(
        Math.Min(start.X, cur.X), Math.Min(start.Y, cur.Y), Math.Max(start.X, cur.X), Math.Max(start.Y, cur.Y));

    Rectangle ToClient(Rectangle r) => new(r.X - vs.X, r.Y - vs.Y, r.Width, r.Height);

    void Track(Point p)
    {
        if (p == cur) return;
        var before = ToClient(Selection);
        cur = p;
        var dirty = Rectangle.Union(before, ToClient(Selection));
        dirty.Inflate(120, 32);
        Invalidate(dirty);
    }

    public void Finish(Point end)
    {
        if (IsDisposed) return;
        cur = end;
        var r = Selection;
        Bitmap crop = null;
        var area = Rectangle.Intersect(ToClient(r), new Rectangle(Point.Empty, vs.Size));
        if (area.Width >= 4 && area.Height >= 4) crop = shot.Clone(area, PixelFormat.Format32bppArgb);
        Close();
        if (crop != null) Captured?.Invoke(crop, r, end.X < start.X);
    }

    protected override void OnShown(EventArgs e) { base.OnShown(e); timer.Start(); }
    protected override void OnKeyDown(KeyEventArgs e) { if (e.KeyCode == Keys.Escape) Close(); }
    protected override void OnMouseDown(MouseEventArgs e) { if (e.Button == MouseButtons.Right) Close(); }
    protected override void OnDpiChanged(DpiChangedEventArgs e) => e.Cancel = true;
    protected override void OnPaintBackground(PaintEventArgs e) { }

    protected override void OnPaint(PaintEventArgs e)
    {
        var g = e.Graphics;
        g.CompositingMode = CompositingMode.SourceCopy;
        g.InterpolationMode = InterpolationMode.NearestNeighbor;
        g.DrawImage(dim, e.ClipRectangle, e.ClipRectangle, GraphicsUnit.Pixel);
        var r = ToClient(Selection);
        if (r.Width <= 0 || r.Height <= 0) return;
        g.DrawImage(shot, r, r, GraphicsUnit.Pixel);
        g.CompositingMode = CompositingMode.SourceOver;
        g.SmoothingMode = SmoothingMode.AntiAlias;
        using (var pen = new Pen(cur.X < start.X ? Blue : Green, 2))
            g.DrawRoundedRectangle(pen, new Rectangle(r.X - 1, r.Y - 1, r.Width + 1, r.Height + 1), new Size(8, 8));
        g.SmoothingMode = SmoothingMode.None;

        var label = $"{r.Width} × {r.Height}";
        var size = TextRenderer.MeasureText(label, font);
        var box = new Rectangle(r.X, Math.Max(0, r.Y - size.Height - 8), size.Width + 8, size.Height + 4);
        using (var bg = new SolidBrush(Color.FromArgb(200, 20, 20, 24))) g.FillRectangle(bg, box);
        TextRenderer.DrawText(g, label, font, box, Color.White, TextFormatFlags.HorizontalCenter | TextFormatFlags.VerticalCenter);
    }

    protected override void OnFormClosed(FormClosedEventArgs e)
    {
        timer.Dispose();
        shot.Dispose();
        dim.Dispose();
        font.Dispose();
        base.OnFormClosed(e);
    }
}
