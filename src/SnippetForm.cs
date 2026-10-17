using System.Drawing.Drawing2D;

namespace DontForget;

public enum NoteMode { Pinned, Follow, Drop }

// One floating note: a snipped piece of the screen that is pinned somewhere or rides next to the cursor.
sealed class SnippetForm : Form
{
    public const int Border = 3;
    static readonly int[] Opacities = { 100, 85, 70, 50, 30 };
    static readonly int[] Sizes = { 50, 75, 100, 150, 200 };

    readonly Bitmap image;
    readonly ContextMenuStrip menu;
    float scale = 1f;
    double opacity = 1.0;
    bool hover, closeHot, menuOpen, clickThrough, leftDown, dragged;
    Point downScreen, downLocation;

    public NoteMode Mode { get; private set; } = NoteMode.Pinned;
    public event Action<SnippetForm> ModeChanged;
    public event Action DeleteAllRequested;

    public SnippetForm(Bitmap image, Color edge)
    {
        this.image = image;
        FormBorderStyle = FormBorderStyle.None;
        StartPosition = FormStartPosition.Manual;
        AutoScaleMode = AutoScaleMode.None;
        ShowInTaskbar = false;
        TopMost = true;
        DoubleBuffered = true;
        BackColor = edge;
        Text = "Don't Forget note";
        ApplySize();
        menu = BuildMenu();
        menu.Opened += (_, _) => menuOpen = true;
        menu.Closed += (_, _) => menuOpen = false;
    }

    public bool MenuOpen => menuOpen;

    protected override bool ShowWithoutActivation => true;

    protected override CreateParams CreateParams
    {
        get
        {
            var cp = base.CreateParams;
            cp.ExStyle |= Native.WS_EX_TOOLWINDOW | Native.WS_EX_NOACTIVATE | Native.WS_EX_TOPMOST | Native.WS_EX_LAYERED;
            cp.ClassStyle |= Native.CS_DROPSHADOW;
            return cp;
        }
    }

    Rectangle CloseRect => new(ClientSize.Width - 24, 6, 18, 18);
    bool ShowsClose => hover && ClientSize.Width > 52 && ClientSize.Height > 32;

    public void SetMode(NoteMode mode)
    {
        Mode = mode;
        clickThrough = mode != NoteMode.Pinned;
        ApplyStyle();
        ModeChanged?.Invoke(this);
    }

    // Following notes let clicks pass through, except while frozen (Shift held or menu open).
    public void SetClickThrough(bool on)
    {
        if (clickThrough == on) return;
        clickThrough = on;
        ApplyStyle();
    }

    void ApplyStyle()
    {
        if (!IsHandleCreated) return;
        int ex = Native.GetWindowLong(Handle, Native.GWL_EXSTYLE) | Native.WS_EX_LAYERED;
        ex = clickThrough ? ex | Native.WS_EX_TRANSPARENT : ex & ~Native.WS_EX_TRANSPARENT;
        Native.SetWindowLong(Handle, Native.GWL_EXSTYLE, ex);
        Native.SetLayeredWindowAttributes(Handle, 0, (byte)Math.Round(opacity * 255), Native.LWA_ALPHA);
    }

    void ApplySize() => ClientSize = new Size(
        (int)Math.Round(image.Width * scale) + 2 * Border, (int)Math.Round(image.Height * scale) + 2 * Border);

    void SetScale(float s)
    {
        scale = Math.Clamp(s, 0.2f, 4f);
        ApplySize();
        Invalidate();
    }

    ContextMenuStrip BuildMenu()
    {
        var m = new ContextMenuStrip();
        var follow = new ToolStripMenuItem("Follow my mouse", null, (_, _) => SetMode(NoteMode.Follow));
        var pin = new ToolStripMenuItem("Pin here", null, (_, _) => SetMode(NoteMode.Pinned));
        var opacityItem = new ToolStripMenuItem("Opacity");
        foreach (var v in Opacities)
            opacityItem.DropDownItems.Add(new ToolStripMenuItem(v + "%", null, (_, _) => { opacity = v / 100.0; ApplyStyle(); }) { Tag = v });
        var sizeItem = new ToolStripMenuItem("Size  (or scroll on it)");
        foreach (var v in Sizes)
            sizeItem.DropDownItems.Add(new ToolStripMenuItem(v + "%", null, (_, _) => SetScale(v / 100f)) { Tag = v });

        m.Items.AddRange(new ToolStripItem[]
        {
            follow, pin, new ToolStripSeparator(),
            opacityItem, sizeItem,
            new ToolStripMenuItem("Copy image", null, (_, _) => Clipboard.SetImage(image)),
            new ToolStripSeparator(),
            new ToolStripMenuItem("Delete", null, (_, _) => Close()) { ShortcutKeyDisplayString = "Wheel click" },
            new ToolStripMenuItem("Delete all notes", null, (_, _) => DeleteAllRequested?.Invoke()),
        });
        m.Opening += (_, _) =>
        {
            follow.Checked = Mode != NoteMode.Pinned;
            pin.Checked = Mode == NoteMode.Pinned;
            foreach (ToolStripMenuItem it in opacityItem.DropDownItems) it.Checked = (int)it.Tag == (int)Math.Round(opacity * 100);
            foreach (ToolStripMenuItem it in sizeItem.DropDownItems) it.Checked = (int)it.Tag == (int)Math.Round(scale * 100);
        };
        return m;
    }

    protected override void OnHandleCreated(EventArgs e)
    {
        base.OnHandleCreated(e);
        ApplyStyle();
        // Windows 11 rounded corners with a matching border that follows the curve.
        int round = Native.DWMWCP_ROUND, color = ColorTranslator.ToWin32(BackColor);
        Native.DwmSetWindowAttribute(Handle, Native.DWMWA_WINDOW_CORNER_PREFERENCE, ref round, sizeof(int));
        Native.DwmSetWindowAttribute(Handle, Native.DWMWA_BORDER_COLOR, ref color, sizeof(int));
    }

    protected override void OnDpiChanged(DpiChangedEventArgs e) => e.Cancel = true;

    protected override void OnMouseEnter(EventArgs e) { hover = true; Invalidate(); base.OnMouseEnter(e); }
    protected override void OnMouseLeave(EventArgs e) { hover = closeHot = false; Invalidate(); base.OnMouseLeave(e); }

    protected override void OnMouseDown(MouseEventArgs e)
    {
        if (e.Button == MouseButtons.Left)
        {
            leftDown = true;
            dragged = false;
            downScreen = Cursor.Position;
            downLocation = Location;
        }
        base.OnMouseDown(e);
    }

    protected override void OnMouseMove(MouseEventArgs e)
    {
        if (leftDown)
        {
            var p = Cursor.Position;
            if (!dragged && (Math.Abs(p.X - downScreen.X) > 3 || Math.Abs(p.Y - downScreen.Y) > 3))
            {
                dragged = true;
                if (Mode != NoteMode.Pinned) SetMode(NoteMode.Pinned);
            }
            if (dragged) Location = new Point(downLocation.X + p.X - downScreen.X, downLocation.Y + p.Y - downScreen.Y);
        }
        bool hot = ShowsClose && CloseRect.Contains(e.Location);
        if (hot != closeHot) { closeHot = hot; Invalidate(); }
        base.OnMouseMove(e);
    }

    protected override void OnMouseUp(MouseEventArgs e)
    {
        if (e.Button == MouseButtons.Left && leftDown)
        {
            leftDown = false;
            if (!dragged)
            {
                if (ShowsClose && CloseRect.Contains(e.Location)) Close();
                else menu.Show(Cursor.Position);
            }
        }
        else if (e.Button == MouseButtons.Right) menu.Show(Cursor.Position);
        base.OnMouseUp(e);
    }

    protected override void OnMouseWheel(MouseEventArgs e) => SetScale(scale * (e.Delta > 0 ? 1.1f : 1 / 1.1f));

    protected override void OnPaint(PaintEventArgs e)
    {
        var g = e.Graphics;
        g.InterpolationMode = scale == 1f ? InterpolationMode.NearestNeighbor : InterpolationMode.HighQualityBicubic;
        g.PixelOffsetMode = PixelOffsetMode.Half;
        g.DrawImage(image, new Rectangle(Border, Border, ClientSize.Width - 2 * Border, ClientSize.Height - 2 * Border));
        if (!ShowsClose) return;

        var r = CloseRect;
        g.SmoothingMode = SmoothingMode.AntiAlias;
        using (var bg = new SolidBrush(closeHot ? Color.FromArgb(230, 210, 45, 45) : Color.FromArgb(170, 0, 0, 0)))
            g.FillEllipse(bg, r);
        using var pen = new Pen(Color.White, 2f);
        const int i = 5;
        g.DrawLine(pen, r.Left + i, r.Top + i, r.Right - i, r.Bottom - i);
        g.DrawLine(pen, r.Right - i, r.Top + i, r.Left + i, r.Bottom - i);
    }

    protected override void OnFormClosed(FormClosedEventArgs e)
    {
        menu.Dispose();
        image.Dispose();
        base.OnFormClosed(e);
    }
}
