use serde::{Deserialize, Serialize};
#[cfg(target_os = "windows")]
use std::env;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use rusty_leveldb::LdbIterator;
use tauri::{AppHandle, Manager};

const DEFAULT_AVATAR_COLOR: &str = "#4F7BFF";

// ── Data structures ──

// what gets persisted to accounts.json
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredProfile {
    id: String,
    #[serde(alias = "name")]
    nickname: String,
    #[serde(default = "default_avatar_color")]
    avatar_color: String,
    created_at_ms: u128,
}

// what the frontend actually sees (includes whether we have a token or not)
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Profile {
    id: String,
    nickname: String,
    avatar_color: String,
    created_at_ms: u128,
    has_token: bool,
}

impl StoredProfile {
    fn into_profile(self, has_token: bool) -> Profile {
        Profile {
            id: self.id,
            nickname: self.nickname,
            avatar_color: self.avatar_color,
            created_at_ms: self.created_at_ms,
            has_token,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum DiscordChannel {
    Auto,
    Stable,
    Ptb,
    Canary,
}

impl Default for DiscordChannel {
    fn default() -> Self {
        Self::Auto
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LauncherSettings {
    #[serde(default)]
    preferred_channel: DiscordChannel,
    custom_executable_path: Option<String>,
}

impl Default for LauncherSettings {
    fn default() -> Self {
        Self {
            preferred_channel: DiscordChannel::Auto,
            custom_executable_path: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DiscordInstallation {
    channel: DiscordChannel,
    label: String,
    executable_path: String,
}

fn default_avatar_color() -> String {
    DEFAULT_AVATAR_COLOR.to_string()
}

// ── Tauri commands: Profile CRUD ──

#[tauri::command]
fn list_profiles(app: AppHandle) -> Result<Vec<Profile>, String> {
    let file_path = profiles_file_path(&app)?;
    let stored = load_profiles(&file_path)?;
    let profiles = stored
        .into_iter()
        .map(|s| {
            let has = profile_has_token(&app, &s.id);
            s.into_profile(has)
        })
        .collect();
    Ok(profiles)
}

#[tauri::command]
fn add_profile(
    app: AppHandle,
    nickname: String,
    avatar_color: Option<String>,
) -> Result<Profile, String> {
    let clean_nickname = normalize_nickname(&nickname)?;
    let clean_avatar_color = normalize_avatar_color(avatar_color.as_deref())?;

    let file_path = profiles_file_path(&app)?;
    let mut profiles = load_profiles(&file_path)?;

    if profiles
        .iter()
        .any(|p| p.nickname.eq_ignore_ascii_case(&clean_nickname))
    {
        return Err("An account with this nickname already exists.".to_string());
    }

    let now_ms = now_ms();
    let stored = StoredProfile {
        id: format!("profile-{}", now_ms),
        nickname: clean_nickname,
        avatar_color: clean_avatar_color,
        created_at_ms: now_ms,
    };

    profiles.push(stored.clone());
    save_profiles(&file_path, &profiles)?;

    Ok(stored.into_profile(false))
}

#[tauri::command]
fn update_profile(
    app: AppHandle,
    profile_id: String,
    nickname: String,
    avatar_color: String,
) -> Result<Profile, String> {
    let clean_nickname = normalize_nickname(&nickname)?;
    let clean_avatar_color = normalize_avatar_color(Some(&avatar_color))?;

    let file_path = profiles_file_path(&app)?;
    let mut profiles = load_profiles(&file_path)?;

    if profiles
        .iter()
        .any(|p| p.id != profile_id && p.nickname.eq_ignore_ascii_case(&clean_nickname))
    {
        return Err("Another account already uses this nickname.".to_string());
    }

    let target = profiles
        .iter_mut()
        .find(|p| p.id == profile_id)
        .ok_or_else(|| "Account not found.".to_string())?;

    target.nickname = clean_nickname;
    target.avatar_color = clean_avatar_color;

    let updated = target.clone();
    save_profiles(&file_path, &profiles)?;

    let has = profile_has_token(&app, &updated.id);
    Ok(updated.into_profile(has))
}

#[tauri::command]
fn remove_profile(app: AppHandle, profile_id: String) -> Result<(), String> {
    let file_path = profiles_file_path(&app)?;
    let mut profiles = load_profiles(&file_path)?;

    let start_len = profiles.len();
    profiles.retain(|p| p.id != profile_id);

    if profiles.len() == start_len {
        return Err("Account not found.".to_string());
    }

    save_profiles(&file_path, &profiles)?;

    // Also delete the saved token file
    if let Ok(path) = token_file_path(&app, &profile_id) {
        let _ = fs::remove_file(path);
    }

    Ok(())
}

// ── Tauri commands: Launcher settings ──

#[tauri::command]
fn get_launcher_settings(app: AppHandle) -> Result<LauncherSettings, String> {
    let file_path = launcher_settings_file_path(&app)?;
    load_launcher_settings(&file_path)
}

#[tauri::command]
fn save_launcher_settings(
    app: AppHandle,
    settings: LauncherSettings,
) -> Result<LauncherSettings, String> {
    let cleaned = sanitize_launcher_settings(settings)?;
    let file_path = launcher_settings_file_path(&app)?;
    save_launcher_settings_to_file(&file_path, &cleaned)?;
    Ok(cleaned)
}

#[tauri::command]
fn detect_discord_installations() -> Vec<DiscordInstallation> {
    detect_installations_for_current_os()
}

// ── Tauri commands: Token management ──

// close Discord, wipe the stored token, and relaunch so the user
// lands on the login screen and can enter credentials
#[tauri::command]
fn prepare_login(app: AppHandle) -> Result<String, String> {
    terminate_discord();
    thread::sleep(Duration::from_millis(2000));

    // Clear the token from Discord's LevelDB so login screen appears
    if let Err(e) = delete_discord_token() {
        eprintln!("Warning: could not clear token: {e}");
    }

    let settings_path = launcher_settings_file_path(&app)?;
    let settings = load_launcher_settings(&settings_path)?;
    let target = resolve_launch_target(settings)?;
    launch_discord(&target)?;

    Ok("Discord launched. Log in with your account, then capture the token.".to_string())
}

// close Discord, pull the token out of its LevelDB, and stash it for this profile
#[tauri::command]
fn capture_token(app: AppHandle, profile_id: String) -> Result<Profile, String> {
    let file_path = profiles_file_path(&app)?;
    let profiles = load_profiles(&file_path)?;
    let stored = profiles
        .into_iter()
        .find(|p| p.id == profile_id)
        .ok_or_else(|| "Profile not found.".to_string())?;

    terminate_discord();
    thread::sleep(Duration::from_millis(2000));

    let token = read_discord_token()?;
    save_profile_token(&app, &profile_id, &token)?;

    Ok(stored.into_profile(true))
}

// inject this profile's saved token back into Discord's storage and launch it
#[tauri::command]
fn switch_to_profile(app: AppHandle, profile_id: String) -> Result<String, String> {
    let token = load_profile_token(&app, &profile_id)?;

    let file_path = profiles_file_path(&app)?;
    let profiles = load_profiles(&file_path)?;
    let profile = profiles
        .iter()
        .find(|p| p.id == profile_id)
        .ok_or_else(|| "Profile not found.".to_string())?;
    let nickname = profile.nickname.clone();

    terminate_discord();
    thread::sleep(Duration::from_millis(2000));

    write_discord_token(&token)?;

    let settings_path = launcher_settings_file_path(&app)?;
    let settings = load_launcher_settings(&settings_path)?;
    let target = resolve_launch_target(settings)?;
    launch_discord(&target)?;

    Ok(format!("Switched to '{nickname}'."))
}

// ── Helpers: time ──

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

// ── Helpers: validation ──

fn normalize_nickname(input: &str) -> Result<String, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("Nickname cannot be empty.".to_string());
    }
    if trimmed.chars().count() > 48 {
        return Err("Nickname must be at most 48 characters.".to_string());
    }
    Ok(trimmed.to_string())
}

fn normalize_avatar_color(input: Option<&str>) -> Result<String, String> {
    let source = input
        .map(|raw| raw.trim())
        .filter(|raw| !raw.is_empty())
        .unwrap_or(DEFAULT_AVATAR_COLOR);
    let normalized = source.to_ascii_uppercase();
    if !is_valid_hex_color(&normalized) {
        return Err("Avatar color must be a valid hex color like #4F7BFF.".to_string());
    }
    Ok(normalized)
}

fn sanitize_launcher_settings(settings: LauncherSettings) -> Result<LauncherSettings, String> {
    let clean_custom_path = settings
        .custom_executable_path
        .as_deref()
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(str::to_string);
    if let Some(path) = &clean_custom_path {
        if !PathBuf::from(path).exists() {
            return Err("Custom executable path does not exist.".to_string());
        }
    }
    Ok(LauncherSettings {
        preferred_channel: settings.preferred_channel,
        custom_executable_path: clean_custom_path,
    })
}

fn is_valid_hex_color(value: &str) -> bool {
    value.len() == 7
        && value.starts_with('#')
        && value.chars().skip(1).all(|c| c.is_ascii_hexdigit())
}

// ── Helpers: file paths ──

fn app_data_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Could not resolve app data directory: {e}"))?;
    fs::create_dir_all(&dir)
        .map_err(|e| format!("Could not create app data directory: {e}"))?;
    Ok(dir)
}

fn profiles_file_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app_data_dir(app)?.join("accounts.json"))
}

fn launcher_settings_file_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app_data_dir(app)?.join("launcher-settings.json"))
}

fn token_file_path(app: &AppHandle, profile_id: &str) -> Result<PathBuf, String> {
    let dir = app_data_dir(app)?.join("tokens");
    fs::create_dir_all(&dir)
        .map_err(|e| format!("Could not create tokens directory: {e}"))?;
    Ok(dir.join(format!("{profile_id}.token")))
}

// ── Helpers: profile persistence ──

fn load_profiles(file_path: &Path) -> Result<Vec<StoredProfile>, String> {
    if !file_path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(file_path)
        .map_err(|e| format!("Could not read account file: {e}"))?;
    if content.trim().is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str(&content)
        .map_err(|e| format!("Could not parse account file: {e}"))
}

fn save_profiles(file_path: &Path, profiles: &[StoredProfile]) -> Result<(), String> {
    let payload = serde_json::to_string_pretty(profiles)
        .map_err(|e| format!("Could not encode accounts: {e}"))?;
    fs::write(file_path, payload)
        .map_err(|e| format!("Could not save account file: {e}"))
}

// ── Helpers: token persistence ──

fn save_profile_token(app: &AppHandle, profile_id: &str, token: &str) -> Result<(), String> {
    let path = token_file_path(app, profile_id)?;
    fs::write(&path, token).map_err(|e| format!("Could not save token: {e}"))
}

fn load_profile_token(app: &AppHandle, profile_id: &str) -> Result<String, String> {
    let path = token_file_path(app, profile_id)?;
    if !path.exists() {
        return Err("No token saved for this profile. Log in first.".to_string());
    }
    fs::read_to_string(&path).map_err(|e| format!("Could not read token: {e}"))
}

fn profile_has_token(app: &AppHandle, profile_id: &str) -> bool {
    token_file_path(app, profile_id)
        .map(|p| p.exists())
        .unwrap_or(false)
}

// ── Helpers: launcher settings persistence ──

fn load_launcher_settings(file_path: &Path) -> Result<LauncherSettings, String> {
    if !file_path.exists() {
        return Ok(LauncherSettings::default());
    }
    let content = fs::read_to_string(file_path)
        .map_err(|e| format!("Could not read launcher settings: {e}"))?;
    if content.trim().is_empty() {
        return Ok(LauncherSettings::default());
    }
    let parsed: LauncherSettings = serde_json::from_str(&content)
        .map_err(|e| format!("Could not parse launcher settings: {e}"))?;
    sanitize_launcher_settings(parsed)
}

fn save_launcher_settings_to_file(
    file_path: &Path,
    settings: &LauncherSettings,
) -> Result<(), String> {
    let payload = serde_json::to_string_pretty(settings)
        .map_err(|e| format!("Could not encode launcher settings: {e}"))?;
    fs::write(file_path, payload)
        .map_err(|e| format!("Could not save launcher settings: {e}"))
}

// ── Discord token: LevelDB operations ──

// figure out where Discord keeps its localStorage LevelDB on this OS
fn discord_storage_dir() -> Result<PathBuf, String> {
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var("HOME").map_err(|_| "HOME not set.".to_string())?;
        for name in ["discord", "discordptb", "discordcanary"] {
            let path = PathBuf::from(&home)
                .join("Library/Application Support")
                .join(name)
                .join("Local Storage/leveldb");
            if path.exists() {
                return Ok(path);
            }
        }
        return Err(
            "Discord Local Storage not found. Is Discord installed?".to_string(),
        );
    }

    #[cfg(target_os = "windows")]
    {
        let appdata = env::var("APPDATA").map_err(|_| "APPDATA not set.".to_string())?;
        for name in ["discord", "discordptb", "discordcanary"] {
            let path = PathBuf::from(&appdata)
                .join(name)
                .join("Local Storage\\leveldb");
            if path.exists() {
                return Ok(path);
            }
        }
        return Err(
            "Discord Local Storage not found. Is Discord installed?".to_string(),
        );
    }

    #[allow(unreachable_code)]
    Err("Unsupported platform.".to_string())
}

// all the LevelDB key variants Discord has used over the years
const TOKEN_KEYS: &[&[u8]] = &[
    b"_https://discord.com\x00\x01token",
    b"_https://discord.com/\x00\x01token",
    b"_https://discord.com\x00token",
    b"_https://discord.com/\x00token",
    b"_https://discordapp.com\x00\x01token",
    b"_https://discordapp.com/\x00\x01token",
];

// pull the token string out of a raw LevelDB value
// there's sometimes an encoding prefix byte (0x01 = Latin-1) we need to skip
fn extract_token_from_value(raw: &[u8]) -> Option<String> {
    if raw.is_empty() {
        return None;
    }

    // Skip encoding prefix if present
    let data = if raw[0] == 0x00 || raw[0] == 0x01 {
        &raw[1..]
    } else {
        raw
    };

    let s = std::str::from_utf8(data).ok()?;
    let s = s.trim_matches('"').trim();

    if s.is_empty() {
        return None;
    }

    Some(s.to_string())
}

// wrap a token string in the format LevelDB expects
fn encode_token_value(token: &str) -> Vec<u8> {
    let mut value = Vec::new();
    value.push(0x01); // Latin-1 encoding prefix
    value.extend_from_slice(format!("\"{token}\"").as_bytes());
    value
}

// read the Discord auth token straight from the LevelDB database
fn read_discord_token() -> Result<String, String> {
    let storage_dir = discord_storage_dir()?;

    // Remove stale LOCK file (Discord should already be terminated)
    let _ = fs::remove_file(storage_dir.join("LOCK"));

    let opt = rusty_leveldb::Options::default();
    let mut db = rusty_leveldb::DB::open(&storage_dir, opt)
        .map_err(|e| format!("Failed to open Discord storage: {e}"))?;

    // Try known key patterns first
    for key in TOKEN_KEYS {
        if let Some(val) = db.get(key) {
            if let Some(token) = extract_token_from_value(&val) {
                if token.contains(':') || token.len() > 30 {
                    return Ok(token);
                }
            }
        }
    }

    // Fallback: iterate all entries looking for encrypted token marker
    let mut iter = db
        .new_iter()
        .map_err(|e| format!("Failed to iterate Discord storage: {e}"))?;

    let mut key_buf = Vec::new();
    let mut val_buf = Vec::new();

    iter.reset();
    while iter.advance() {
        if iter.current(&mut key_buf, &mut val_buf) {
            if let Some(token) = extract_token_from_value(&val_buf) {
                if token.starts_with("dQw4w9WgXcQ:") {
                    return Ok(token);
                }
            }
        }
    }

    Err("No Discord token found. Make sure you logged in to Discord first.".to_string())
}

// write a token into Discord's LevelDB so it logs in as this account
fn write_discord_token(token: &str) -> Result<(), String> {
    let storage_dir = discord_storage_dir()?;
    let _ = fs::remove_file(storage_dir.join("LOCK"));

    let opt = rusty_leveldb::Options::default();
    let mut db = rusty_leveldb::DB::open(&storage_dir, opt)
        .map_err(|e| format!("Failed to open Discord storage: {e}"))?;

    // Find existing key or use default
    let key = TOKEN_KEYS
        .iter()
        .find(|k| db.get(*k).is_some())
        .copied()
        .unwrap_or(TOKEN_KEYS[0]);

    let value = encode_token_value(token);
    db.put(key, &value)
        .map_err(|e| format!("Failed to write token: {e}"))?;

    db.flush()
        .map_err(|e| format!("Failed to flush database: {e}"))?;

    Ok(())
}

// nuke the token from Discord's LevelDB so it shows the login screen
fn delete_discord_token() -> Result<(), String> {
    let storage_dir = discord_storage_dir()?;
    let _ = fs::remove_file(storage_dir.join("LOCK"));

    let opt = rusty_leveldb::Options::default();
    let mut db = rusty_leveldb::DB::open(&storage_dir, opt)
        .map_err(|e| format!("Failed to open Discord storage: {e}"))?;

    for key in TOKEN_KEYS {
        let _ = db.delete(key);
    }

    db.flush()
        .map_err(|e| format!("Failed to flush database: {e}"))?;

    Ok(())
}

// ── Discord: launch target resolution ──

fn resolve_launch_target(settings: LauncherSettings) -> Result<DiscordInstallation, String> {
    if let Some(custom_path) = settings.custom_executable_path {
        return Ok(DiscordInstallation {
            channel: DiscordChannel::Auto,
            label: "Custom Discord executable".to_string(),
            executable_path: custom_path,
        });
    }

    let detected = detect_installations_for_current_os();

    if detected.is_empty() {
        return Err(
            "Discord was not auto-detected. Set a custom executable path in settings.".to_string(),
        );
    }

    if settings.preferred_channel == DiscordChannel::Auto {
        return detected
            .first()
            .cloned()
            .ok_or_else(|| "No Discord installations were detected.".to_string());
    }

    detected
        .into_iter()
        .find(|i| i.channel == settings.preferred_channel)
        .ok_or_else(|| {
            "Preferred Discord channel was not found. Use Auto or set a custom path.".to_string()
        })
}

// ── Discord: process control ──

fn terminate_discord() {
    #[cfg(target_os = "macos")]
    {
        for name in ["Discord", "Discord PTB", "Discord Canary"] {
            let _ = Command::new("pkill")
                .args(["-x", name])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }

    #[cfg(target_os = "windows")]
    {
        for name in ["Discord.exe", "DiscordPTB.exe", "DiscordCanary.exe"] {
            let _ = Command::new("taskkill")
                .args(["/IM", name, "/F"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
}

// launch Discord normally (we don't use --user-data-dir, tokens live in the default location)
fn launch_discord(installation: &DiscordInstallation) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let binary = if installation.executable_path.ends_with(".app") {
            let app_path = PathBuf::from(&installation.executable_path);
            let app_name = app_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("Discord")
                .to_string();
            let inner = app_path.join("Contents").join("MacOS").join(&app_name);
            if !inner.exists() {
                return Err(format!(
                    "Could not find binary inside {}: expected {}",
                    installation.executable_path,
                    inner.display()
                ));
            }
            inner.to_string_lossy().to_string()
        } else {
            installation.executable_path.clone()
        };

        Command::new(&binary)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("Failed to launch Discord: {e}"))?;

        return Ok(());
    }

    #[cfg(target_os = "windows")]
    {
        Command::new(&installation.executable_path)
            .spawn()
            .map_err(|e| format!("Failed to launch Discord: {e}"))?;

        return Ok(());
    }

    #[allow(unreachable_code)]
    Err("This app currently supports macOS and Windows only.".to_string())
}

// ── Discord: installation detection ──

fn detect_installations_for_current_os() -> Vec<DiscordInstallation> {
    #[cfg(target_os = "macos")]
    {
        return detect_macos_installations();
    }

    #[cfg(target_os = "windows")]
    {
        return detect_windows_installations();
    }

    #[allow(unreachable_code)]
    Vec::new()
}

#[cfg(target_os = "macos")]
fn detect_macos_installations() -> Vec<DiscordInstallation> {
    let home = std::env::var("HOME").unwrap_or_default();
    let home_apps = PathBuf::from(&home).join("Applications");
    let mut installations = Vec::new();

    let candidates = [
        (
            DiscordChannel::Stable,
            "Discord",
            [
                PathBuf::from("/Applications/Discord.app"),
                home_apps.join("Discord.app"),
            ],
        ),
        (
            DiscordChannel::Ptb,
            "Discord PTB",
            [
                PathBuf::from("/Applications/Discord PTB.app"),
                home_apps.join("Discord PTB.app"),
            ],
        ),
        (
            DiscordChannel::Canary,
            "Discord Canary",
            [
                PathBuf::from("/Applications/Discord Canary.app"),
                home_apps.join("Discord Canary.app"),
            ],
        ),
    ];

    for (channel, label, paths) in candidates {
        if let Some(found) = paths.into_iter().find(|p| p.exists()) {
            installations.push(DiscordInstallation {
                channel,
                label: label.to_string(),
                executable_path: found.to_string_lossy().to_string(),
            });
        }
    }

    installations
}

#[cfg(target_os = "windows")]
fn detect_windows_installations() -> Vec<DiscordInstallation> {
    let mut installations = Vec::new();

    if let Some(s) = detect_windows_channel_install(
        "Discord",
        DiscordChannel::Stable,
        "Discord",
        &["Discord.exe", "DiscordPTB.exe", "DiscordCanary.exe"],
    ) {
        installations.push(s);
    }

    if let Some(p) = detect_windows_channel_install(
        "DiscordPTB",
        DiscordChannel::Ptb,
        "Discord PTB",
        &["DiscordPTB.exe", "Discord.exe"],
    ) {
        installations.push(p);
    }

    if let Some(c) = detect_windows_channel_install(
        "DiscordCanary",
        DiscordChannel::Canary,
        "Discord Canary",
        &["DiscordCanary.exe", "Discord.exe"],
    ) {
        installations.push(c);
    }

    installations
}

#[cfg(target_os = "windows")]
fn detect_windows_channel_install(
    folder_name: &str,
    channel: DiscordChannel,
    label: &str,
    executable_names: &[&str],
) -> Option<DiscordInstallation> {
    let local_app_data = env::var("LOCALAPPDATA").ok()?;
    let root = PathBuf::from(local_app_data).join(folder_name);

    let mut app_dirs: Vec<PathBuf> = fs::read_dir(&root)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with("app-"))
                .unwrap_or(false)
        })
        .collect();

    app_dirs.sort();
    app_dirs.reverse();

    for dir in app_dirs {
        for exe in executable_names {
            let path = dir.join(exe);
            if path.exists() {
                return Some(DiscordInstallation {
                    channel,
                    label: label.to_string(),
                    executable_path: path.to_string_lossy().to_string(),
                });
            }
        }
    }

    None
}

// ── Entry point ──

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            list_profiles,
            add_profile,
            update_profile,
            remove_profile,
            get_launcher_settings,
            save_launcher_settings,
            detect_discord_installations,
            prepare_login,
            capture_token,
            switch_to_profile,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
