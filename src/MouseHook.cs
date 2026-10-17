using System.Diagnostics;
using System.Runtime.InteropServices;

namespace DontForget;

// Global low-level mouse hook on its own thread so a busy UI thread never stalls the system mouse.
// Wheel-button press: quick click -> replayed to the app, hold or drag -> screen selection.
sealed class MouseHook : IDisposable
{
    static readonly IntPtr Magic = new(0x44465247); // marks our own replayed clicks

    enum State { Idle, Pending, Selecting }

    readonly Action<Action> post;
    readonly Func<IntPtr, bool> isNote;
    readonly object gate = new();
    readonly System.Threading.Timer holdTimer;
    readonly Dictionary<uint, string> processNames = new();
    Native.HookProc proc;
    Thread thread;
    uint threadId;
    IntPtr hook;
    State state;
    Point downPt;
    bool eatMiddleUp, eatLeftUp;

    public int HoldMs { get; set; } = 250;
    public volatile bool DropArmed;
    public volatile bool FollowArmed; // a quick wheel click releases following notes instead of reaching the app
    public volatile bool Paused;
    public volatile HashSet<string> IgnoredApps = new(StringComparer.OrdinalIgnoreCase);

    public event Action<Point> SelectBegin, SelectEnd, DropClick;
    public event Action ReleaseFollowers;
    public event Action<IntPtr> DeleteRequested;

    public MouseHook(Action<Action> post, Func<IntPtr, bool> isNote)
    {
        this.post = post;
        this.isNote = isNote;
        holdTimer = new System.Threading.Timer(_ => { lock (gate) if (state == State.Pending) BeginSelect(); });
    }

    public void Start()
    {
        thread = new Thread(() =>
        {
            threadId = Native.GetCurrentThreadId();
            proc = Callback;
            hook = Native.SetWindowsHookEx(Native.WH_MOUSE_LL, proc, Native.GetModuleHandle(null), 0);
            while (Native.GetMessage(out _, IntPtr.Zero, 0, 0) > 0) { }
            Native.UnhookWindowsHookEx(hook);
        }) { IsBackground = true, Priority = ThreadPriority.AboveNormal, Name = "mouse hook" };
        thread.Start();
    }

    public void Dispose()
    {
        Native.PostThreadMessage(threadId, 0x12 /* WM_QUIT */, IntPtr.Zero, IntPtr.Zero);
        holdTimer.Dispose();
    }

    IntPtr Callback(int code, IntPtr wParam, IntPtr lParam)
    {
        if (code >= 0 && !Paused)
        {
            var m = Marshal.PtrToStructure<Native.MSLLHOOKSTRUCT>(lParam);
            if (m.dwExtraInfo != Magic && Handle((int)wParam, m.pt)) return 1;
        }
        return Native.CallNextHookEx(hook, code, wParam, lParam);
    }

    // Returns true to swallow the event.
    bool Handle(int msg, Native.POINT raw)
    {
        var pt = new Point(raw.X, raw.Y);
        lock (gate)
        {
            switch (msg)
            {
                case Native.WM_MBUTTONDOWN:
                    if (state != State.Idle) return true;
                    var root = RootAt(raw);
                    if (isNote(root)) { eatMiddleUp = true; post(() => DeleteRequested?.Invoke(root)); return true; }
                    if (IsIgnored(root)) return false;
                    state = State.Pending;
                    downPt = pt;
                    holdTimer.Change(HoldMs, Timeout.Infinite);
                    return true;

                case Native.WM_MOUSEMOVE:
                    if (state == State.Pending && (Math.Abs(pt.X - downPt.X) > 6 || Math.Abs(pt.Y - downPt.Y) > 6))
                        BeginSelect();
                    return false;

                case Native.WM_MBUTTONUP:
                    if (eatMiddleUp) { eatMiddleUp = false; return true; }
                    if (state == State.Pending)
                    {
                        state = State.Idle;
                        holdTimer.Change(Timeout.Infinite, Timeout.Infinite);
                        if (FollowArmed) post(() => ReleaseFollowers?.Invoke());
                        else ThreadPool.QueueUserWorkItem(_ => ReplayMiddleClick());
                        return true;
                    }
                    if (state == State.Selecting)
                    {
                        state = State.Idle;
                        post(() => SelectEnd?.Invoke(pt));
                        return true;
                    }
                    return false;

                case Native.WM_LBUTTONDOWN:
                    if (DropArmed && state == State.Idle && !isNote(RootAt(raw)))
                    {
                        eatLeftUp = true;
                        post(() => DropClick?.Invoke(pt));
                        return true;
                    }
                    return false;

                case Native.WM_LBUTTONUP:
                    if (eatLeftUp) { eatLeftUp = false; return true; }
                    return false;
            }
        }
        return false;
    }

    void BeginSelect()
    {
        state = State.Selecting;
        holdTimer.Change(Timeout.Infinite, Timeout.Infinite);
        var p = downPt;
        post(() => SelectBegin?.Invoke(p));
    }

    static IntPtr RootAt(Native.POINT p) => Native.GetAncestor(Native.WindowFromPoint(p), Native.GA_ROOT);

    bool IsIgnored(IntPtr hwnd)
    {
        var ignored = IgnoredApps;
        if (ignored.Count == 0) return false;
        Native.GetWindowThreadProcessId(hwnd, out uint pid);
        if (!processNames.TryGetValue(pid, out var name))
        {
            try { name = Process.GetProcessById((int)pid).ProcessName; } catch { name = ""; }
            processNames[pid] = name;
        }
        return ignored.Contains(name);
    }

    static void ReplayMiddleClick()
    {
        var inputs = new[] { Click(Native.MOUSEEVENTF_MIDDLEDOWN), Click(Native.MOUSEEVENTF_MIDDLEUP) };
        Native.SendInput((uint)inputs.Length, inputs, Marshal.SizeOf<Native.INPUT>());
    }

    static Native.INPUT Click(uint flags) => new() { type = 0, mi = new Native.MOUSEINPUT { dwFlags = flags, dwExtraInfo = Magic } };
}
