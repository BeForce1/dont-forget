// Recognizing games, so Don't Forget stays out of their way unless the user allows it per game.

/// Folders (lowercase, '/' separators) where game stores and launchers install games.
const GAME_FOLDERS: [&str; 13] = [
    "/steamapps/common/",
    "/epic games/",
    "/gog galaxy/games/",
    "/gog games/",
    "/xboxgames/",
    "/riot games/",
    "/ubisoft game launcher/games/",
    "/ea games/",
    "/origin games/",
    "/rockstar games/",
    "/itch/apps/",
    "/games/heroic/",
    "/lutris/",
];

/// Store apps that live inside those folders but are not games.
const LAUNCHERS: [&str; 2] = ["/epic games/launcher/", "/epic games/epiconlineservices/"];

/// Is this executable path (or command line) inside a game library?
pub fn is_game_path(path: &str) -> bool {
    let p = path.to_lowercase().replace('\\', "/");
    GAME_FOLDERS.iter().any(|f| p.contains(f)) && !LAUNCHERS.iter().any(|l| p.contains(l))
}

/// Process name (any case, with or without .exe) as stored in settings lists.
pub fn same_app(listed: &str, name: &str) -> bool {
    listed.trim().to_lowercase().trim_end_matches(".exe") == name.to_lowercase().trim_end_matches(".exe")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn game_paths() {
        assert!(is_game_path(r"D:\SteamLibrary\steamapps\common\Alice Madness Returns\Binaries\Win32\AliceMadness.exe"));
        assert!(is_game_path(r"C:\Program Files\Epic Games\Fortnite\FortniteClient.exe"));
        assert!(is_game_path("/home/a/.local/share/Steam/steamapps/common/Hades/Hades.exe"));
        assert!(is_game_path(r"C:\XboxGames\Forza Horizon 5\Content\ForzaHorizon5.exe"));
        assert!(!is_game_path(r"C:\Program Files\Google\Chrome\Application\chrome.exe"));
        assert!(!is_game_path(r"C:\Program Files (x86)\Steam\steam.exe"));
        assert!(!is_game_path(r"C:\Program Files (x86)\Epic Games\Launcher\Portal\Binaries\Win64\EpicGamesLauncher.exe"));
        assert!(same_app("AliceMadness.exe", "alicemadness"));
    }
}
