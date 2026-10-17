namespace DontForget;

static class Program
{
    [STAThread]
    static void Main()
    {
        using var mutex = new Mutex(true, "DontForget.SingleInstance", out bool first);
        if (!first) return;
        ApplicationConfiguration.Initialize();
        Application.Run(new TrayApp());
    }
}
