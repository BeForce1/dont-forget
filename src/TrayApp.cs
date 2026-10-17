using System.Collections.Concurrent;
using System.Diagnostics;
using Microsoft.Win32;

namespace DontForget;

sealed class TrayApp : ApplicationContext
{
    const string RunKey = @"Software\Microsoft\Windows\CurrentVersion\Run";
    const string HelpText =
        "Hold the wheel and drag to snip (drag right-to-left to add to the notes already following).\n" +
        "Wheel-click anywhere to drop following notes; wheel-click a note to delete it.\n" +
        "Hold Shift to freeze following notes. Click a note for options.";

    Settings settings = Settings.Load();
    readonly Control invoker = new();
    readonly NotifyIcon tray;
    readonly MouseHook hook;
    readonly List<SnippetForm> notes = new();
    readonly List<SnippetForm> followers = new(); // newest first, stacked next to the cursor
    readonly System.Windows.Forms.Timer followTimer = new() { Interval = 15 };
    readonly ConcurrentDictionary<IntPtr, byte> noteHandles = new();
    SelectionOverlay overlay;

    public TrayApp()
    {
        _ = invoker.Handle; // bind to the UI thread so the hook thread can post to it
        hook = new MouseHook(a => invoker.BeginInvoke(a), h => noteHandles.ContainsKey(h));
        ApplySettings();
        hook.SelectBegin += BeginSelect;
        hook.SelectEnd += p => overlay?.Finish(p);
        hook.DeleteRequested += h => notes.FirstOrDefault(n => n.IsHandleCreated && n.Handle == h)?.Close();
        hook.DropClick += _ =>
        {
            foreach (var n in notes.Where(n => n.Mode == NoteMode.Drop).ToList()) n.SetMode(NoteMode.Pinned);
        };
        hook.ReleaseFollowers += PinFollowers;
        followTimer.Tick += (_, _) => LayoutFollowers();

        tray = new NotifyIcon { Icon = MakeIcon(), Text = "Don't Forget", Visible = true, ContextMenuStrip = BuildMenu() };
        tray.MouseClick += (_, e) => { if (e.Button == MouseButtons.Left) ShowHelp(); };
        hook.Start();

        if (!settings.ShownIntro)
        {
            ShowHelp();
            settings.ShownIntro = true;
            settings.Save();
        }
    }

    void ApplySettings()
    {
        hook.HoldMs = settings.HoldMs;
        hook.IgnoredApps = new HashSet<string>(
            settings.IgnoredApps.Select(a => a.EndsWith(".exe", StringComparison.OrdinalIgnoreCase) ? a[..^4] : a),
            StringComparer.OrdinalIgnoreCase);
    }

    void BeginSelect(Point start)
    {
        if (overlay != null) return;
        overlay = new SelectionOverlay(start);
        overlay.Captured += AddNote;
        overlay.FormClosed += (_, _) => overlay = null;
        overlay.Show();
        overlay.Activate();
    }

    void AddNote(Bitmap image, Rectangle area, bool additive)
    {
        if (!additive) PinFollowers();
        var edge = additive ? SelectionOverlay.Blue : SelectionOverlay.Green;
        var note = new SnippetForm(image, edge) { Location = new Point(area.X - SnippetForm.Border, area.Y - SnippetForm.Border) };
        note.ModeChanged += OnModeChanged;
        note.DeleteAllRequested += ClearAll;
        notes.Add(note);
        note.Show();
        var handle = note.Handle;
        noteHandles[handle] = 0;
        note.FormClosed += (_, _) =>
        {
            noteHandles.TryRemove(handle, out _);
            notes.Remove(note);
            followers.Remove(note);
            UpdateArmed();
        };
        note.SetMode(settings.AfterCapture switch
        {
            AfterCapture.Follow => NoteMode.Follow,
            AfterCapture.Drop => NoteMode.Drop,
            _ => NoteMode.Pinned,
        });
    }

    void OnModeChanged(SnippetForm changed)
    {
        followers.Remove(changed);
        if (changed.Mode != NoteMode.Pinned)
        {
            followers.Insert(0, changed);
            LayoutFollowers();
        }
        UpdateArmed();
    }

    void PinFollowers()
    {
        foreach (var n in followers.ToList()) n.SetMode(NoteMode.Pinned);
    }

    void UpdateArmed()
    {
        hook.DropArmed = followers.Any(n => n.Mode == NoteMode.Drop);
        hook.FollowArmed = followers.Count > 0;
        followTimer.Enabled = followers.Count > 0;
    }

    // Stacks following notes below-right of the cursor (flipping at screen edges);
    // holding Shift or having a note's menu open freezes them so they can be reached.
    void LayoutFollowers()
    {
        if (followers.Count == 0) return;
        bool freeze = followers.Any(n => n.MenuOpen) || (Native.GetAsyncKeyState(Native.VK_SHIFT) & 0x8000) != 0;
        foreach (var n in followers) n.SetClickThrough(!freeze);
        if (freeze) return;

        const int gap = 6;
        var c = Cursor.Position;
        var wa = Screen.FromPoint(c).WorkingArea;
        int width = followers.Max(n => n.Width);
        int height = followers.Sum(n => n.Height) + gap * (followers.Count - 1);
        int x = c.X + 20, y = c.Y + 20;
        if (x + width > wa.Right) x = c.X - 12 - width;
        if (y + height > wa.Bottom) y = c.Y - 12 - height;
        x = Math.Max(wa.Left, x);
        y = Math.Max(wa.Top, y);
        foreach (var n in followers)
        {
            if (n.Left != x || n.Top != y) n.Location = new Point(x, y);
            y += n.Height + gap;
        }
    }

    void ClearAll()
    {
        foreach (var n in notes.ToList()) n.Close();
    }

    void ShowHelp() => tray.ShowBalloonTip(8000, "Don't Forget", HelpText, ToolTipIcon.None);

    ContextMenuStrip BuildMenu()
    {
        var m = new ContextMenuStrip();

        var after = new ToolStripMenuItem("After a snip");
        void AfterOption(string text, AfterCapture value)
        {
            var item = new ToolStripMenuItem(text, null, (_, _) => { settings.AfterCapture = value; settings.Save(); });
            after.DropDownItems.Add(item);
            after.DropDownOpening += (_, _) => item.Checked = settings.AfterCapture == value;
        }
        AfterOption("Follow my mouse", AfterCapture.Follow);
        AfterOption("Follow until I click to place it", AfterCapture.Drop);
        AfterOption("Stay where I snipped it", AfterCapture.Stay);

        var hold = new ToolStripMenuItem("Hold time before snipping");
        foreach (var ms in new[] { 150, 250, 400, 600 })
        {
            var item = new ToolStripMenuItem(ms + " ms", null, (_, _) => { settings.HoldMs = ms; settings.Save(); ApplySettings(); });
            hold.DropDownItems.Add(item);
            hold.DropDownOpening += (_, _) => item.Checked = settings.HoldMs == ms;
        }

        var pause = new ToolStripMenuItem("Pause", null, (s, _) => hook.Paused = ((ToolStripMenuItem)s).Checked = !hook.Paused);
        var startup = new ToolStripMenuItem("Start with Windows", null, (_, _) => StartsWithWindows = !StartsWithWindows);
        m.Opening += (_, _) => startup.Checked = StartsWithWindows;

        m.Items.AddRange(new ToolStripItem[]
        {
            after, hold,
            new ToolStripMenuItem("Ignored apps…", null, (_, _) => EditSettings()),
            new ToolStripSeparator(),
            new ToolStripMenuItem("Delete all notes", null, (_, _) => ClearAll()),
            pause, startup,
            new ToolStripSeparator(),
            new ToolStripMenuItem("How to use", null, (_, _) => ShowHelp()),
            new ToolStripMenuItem("Exit", null, (_, _) => ExitThread()),
        });
        return m;
    }

    // Opens settings.json in Notepad and reloads it when Notepad closes.
    void EditSettings()
    {
        settings.Save();
        var p = Process.Start(new ProcessStartInfo("notepad.exe", $"\"{Settings.FilePath}\"") { UseShellExecute = false });
        if (p == null) return;
        p.EnableRaisingEvents = true;
        p.Exited += (_, _) => invoker.BeginInvoke(() => { settings = Settings.Load(); ApplySettings(); });
    }

    static bool StartsWithWindows
    {
        get
        {
            using var key = Registry.CurrentUser.OpenSubKey(RunKey);
            return key?.GetValue("DontForget") != null;
        }
        set
        {
            using var key = Registry.CurrentUser.CreateSubKey(RunKey);
            if (value) key.SetValue("DontForget", $"\"{Environment.ProcessPath}\"");
            else key.DeleteValue("DontForget", false);
        }
    }

    static Icon MakeIcon()
    {
        using var bmp = new Bitmap(32, 32);
        using (var g = Graphics.FromImage(bmp))
        using (var fill = new SolidBrush(Color.FromArgb(255, 186, 0)))
        using (var font = new Font("Segoe UI", 22f, FontStyle.Bold, GraphicsUnit.Pixel))
        {
            g.SmoothingMode = System.Drawing.Drawing2D.SmoothingMode.AntiAlias;
            g.FillRoundedRectangle(fill, new Rectangle(2, 2, 28, 28), new Size(10, 10));
            TextRenderer.DrawText(g, "!", font, new Rectangle(0, 0, 32, 32), Color.FromArgb(30, 30, 30),
                TextFormatFlags.HorizontalCenter | TextFormatFlags.VerticalCenter);
        }
        return Icon.FromHandle(bmp.GetHicon());
    }

    protected override void ExitThreadCore()
    {
        hook.Dispose();
        followTimer.Dispose();
        ClearAll();
        tray.Visible = false;
        tray.Dispose();
        invoker.Dispose();
        base.ExitThreadCore();
    }
}
