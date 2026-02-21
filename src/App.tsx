import { FormEvent, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./App.css";

type DiscordChannel = "auto" | "stable" | "ptb" | "canary";

type Profile = {
  id: string;
  nickname: string;
  avatarColor: string;
  createdAtMs: number;
  hasToken: boolean;
};

type LauncherSettings = {
  preferredChannel: DiscordChannel;
  customExecutablePath: string | null;
};

type DiscordInstallation = {
  channel: DiscordChannel;
  label: string;
  executablePath: string;
};

type View = "empty" | "adding" | "profile";

const PALETTE = [
  "#4361ee",
  "#2ec4b6",
  "#e63946",
  "#f77f00",
  "#7209b7",
  "#06d6a0",
];

function formatDate(ms: number) {
  return new Date(ms).toLocaleDateString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function initialsOf(name: string) {
  const words = name.trim().split(/\s+/).filter(Boolean);
  if (words.length === 0) return "?";
  if (words.length === 1) return words[0].slice(0, 2).toUpperCase();
  return `${words[0][0]}${words[1][0]}`.toUpperCase();
}

function App() {
  const [profiles, setProfiles] = useState<Profile[]>([]);
  const [installations, setInstallations] = useState<DiscordInstallation[]>([]);
  const [, setSettings] = useState<LauncherSettings | null>(null);

  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [view, setView] = useState<View>("empty");

  const [nicknameInput, setNicknameInput] = useState("");
  const [colorInput, setColorInput] = useState(PALETTE[0]);

  const [editing, setEditing] = useState(false);
  const [editNickname, setEditNickname] = useState("");
  const [editColor, setEditColor] = useState(PALETTE[0]);

  const [settingsChannel, setSettingsChannel] =
    useState<DiscordChannel>("auto");
  const [settingsCustomPath, setSettingsCustomPath] = useState("");

  const [busy, setBusy] = useState(false);
  const [loading, setLoading] = useState(true);
  const [status, setStatus] = useState("");
  const [error, setError] = useState("");
  const [confirmRemove, setConfirmRemove] = useState<Profile | null>(null);
  const [waitingForLogin, setWaitingForLogin] = useState<string | null>(null);

  const sortedProfiles = useMemo(
    () => profiles.slice().sort((a, b) => b.createdAtMs - a.createdAtMs),
    [profiles],
  );

  const selectedProfile = useMemo(
    () => profiles.find((p) => p.id === selectedId) ?? null,
    [profiles, selectedId],
  );

  function showStatus(msg: string) {
    setStatus(msg);
    setError("");
    setTimeout(() => setStatus(""), 4000);
  }

  function showError(msg: string) {
    setError(String(msg));
    setStatus("");
    setTimeout(() => setError(""), 6000);
  }

  async function loadData() {
    try {
      const [loadedProfiles, loadedSettings, loadedInstallations] =
        await Promise.all([
          invoke<Profile[]>("list_profiles"),
          invoke<LauncherSettings>("get_launcher_settings"),
          invoke<DiscordInstallation[]>("detect_discord_installations"),
        ]);
      setProfiles(loadedProfiles);
      setSettings(loadedSettings);
      setInstallations(loadedInstallations);
      setSettingsChannel(loadedSettings.preferredChannel);
      setSettingsCustomPath(loadedSettings.customExecutablePath ?? "");
    } catch (err) {
      showError(String(err));
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    loadData();
  }, []);

  function selectProfile(profile: Profile) {
    setSelectedId(profile.id);
    setView("profile");
    setEditing(false);
    setWaitingForLogin(null);
  }

  function startAdding() {
    setView("adding");
    setSelectedId(null);
    setNicknameInput("");
    setColorInput(PALETTE[0]);
    setEditing(false);
    setWaitingForLogin(null);
  }

  async function addProfile(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    try {
      const created = await invoke<Profile>("add_profile", {
        nickname: nicknameInput,
        avatarColor: colorInput,
      });
      setProfiles((prev) => [...prev, created]);
      setSelectedId(created.id);
      setView("profile");
      showStatus(`Account "${created.nickname}" created. Click "Log In" to capture credentials.`);
    } catch (err) {
      showError(String(err));
    }
  }

  async function removeProfile(profile: Profile) {
    try {
      await invoke("remove_profile", { profileId: profile.id });
      setProfiles((prev) => prev.filter((p) => p.id !== profile.id));
      if (selectedId === profile.id) {
        setSelectedId(null);
        setView("empty");
      }
      showStatus(`Removed "${profile.nickname}".`);
    } catch (err) {
      showError(String(err));
    } finally {
      setConfirmRemove(null);
    }
  }

  function startEditing(profile: Profile) {
    setEditing(true);
    setEditNickname(profile.nickname);
    setEditColor(profile.avatarColor);
  }

  async function saveEdit() {
    if (!selectedId) return;
    try {
      const updated = await invoke<Profile>("update_profile", {
        profileId: selectedId,
        nickname: editNickname,
        avatarColor: editColor,
      });
      setProfiles((prev) =>
        prev.map((p) => (p.id === updated.id ? updated : p)),
      );
      setEditing(false);
      showStatus(`Updated "${updated.nickname}".`);
    } catch (err) {
      showError(String(err));
    }
  }

  // wipe the current token and open Discord so the user sees the login screen
  async function prepareLogin(profileId: string) {
    try {
      setBusy(true);
      await invoke<string>("prepare_login");
      setWaitingForLogin(profileId);
      showStatus("Discord launched. Log in with your account credentials.");
    } catch (err) {
      showError(String(err));
    } finally {
      setBusy(false);
    }
  }

  // close Discord, grab the token from its storage, and save it to this profile
  async function captureToken(profileId: string) {
    try {
      setBusy(true);
      const updated = await invoke<Profile>("capture_token", {
        profileId,
      });
      setProfiles((prev) =>
        prev.map((p) => (p.id === updated.id ? updated : p)),
      );
      setWaitingForLogin(null);
      showStatus(`Token captured for "${updated.nickname}".`);
    } catch (err) {
      showError(String(err));
    } finally {
      setBusy(false);
    }
  }

  // inject this profile's saved token into Discord and launch it
  async function switchToProfile(profile: Profile) {
    try {
      setBusy(true);
      const message = await invoke<string>("switch_to_profile", {
        profileId: profile.id,
      });
      showStatus(message);
    } catch (err) {
      showError(String(err));
    } finally {
      setBusy(false);
    }
  }

  async function saveLauncherSettings() {
    try {
      const saved = await invoke<LauncherSettings>("save_launcher_settings", {
        settings: {
          preferredChannel: settingsChannel,
          customExecutablePath: settingsCustomPath.trim() || null,
        },
      });
      const refreshed = await invoke<DiscordInstallation[]>(
        "detect_discord_installations",
      );
      setSettings(saved);
      setInstallations(refreshed);
      showStatus("Settings saved.");
    } catch (err) {
      showError(String(err));
    }
  }

  if (loading) {
    return (
      <main className="app-shell">
        <div className="sidebar" />
        <div className="main">
          <div className="loading-state">loading...</div>
        </div>
      </main>
    );
  }

  return (
    <main className="app-shell">
      {/* ── Sidebar ── */}
      <nav className="sidebar">
        <div className="sidebar-header">
          <h1>Accounts</h1>
        </div>

        <ul className="sidebar-list">
          {sortedProfiles.map((profile) => (
            <li key={profile.id}>
              <button
                className={`sidebar-item${selectedId === profile.id && view === "profile" ? " active" : ""}`}
                onClick={() => selectProfile(profile)}
              >
                <div
                  className="avatar"
                  style={{ backgroundColor: profile.avatarColor }}
                >
                  {initialsOf(profile.nickname)}
                </div>
                <div className="sidebar-item-info">
                  <span className="profile-name">{profile.nickname}</span>
                  {!profile.hasToken && (
                    <span className="token-badge no-token">no token</span>
                  )}
                </div>
              </button>
            </li>
          ))}
        </ul>

        <div className="sidebar-footer">
          <button className="add-btn" onClick={startAdding}>
            <span className="plus">+</span> Add Account
          </button>
        </div>
      </nav>

      {/* ── Main Area ── */}
      <div className="main">
        {view === "empty" && (
          <div className="empty-state">
            <div className="empty-icon">~</div>
            <p>
              {profiles.length === 0
                ? "No accounts yet"
                : "Select an account"}
            </p>
            <span className="hint">
              {profiles.length === 0
                ? "click + add account to get started"
                : `${profiles.length} account${profiles.length !== 1 ? "s" : ""} saved`}
            </span>
          </div>
        )}

        {view === "adding" && (
          <div className="add-form">
            <h2>New Account</h2>
            <p className="form-desc">
              Give this account a nickname and pick a color. After creating,
              you'll log into Discord to capture the token for this profile.
            </p>

            <form onSubmit={addProfile}>
              <div className="form-group">
                <label>Nickname</label>
                <input
                  className="field-input"
                  type="text"
                  placeholder="e.g. Main, Alt, Work"
                  value={nicknameInput}
                  onChange={(e) => setNicknameInput(e.currentTarget.value)}
                  maxLength={48}
                  autoFocus
                />
              </div>

              <div className="form-group">
                <label>Color</label>
                <div className="color-row">
                  {PALETTE.map((c) => (
                    <button
                      key={c}
                      type="button"
                      className={`color-swatch${colorInput === c ? " selected" : ""}`}
                      style={{ backgroundColor: c }}
                      onClick={() => setColorInput(c)}
                    />
                  ))}
                  <input
                    type="color"
                    className="color-picker-native"
                    value={colorInput}
                    onChange={(e) => setColorInput(e.currentTarget.value)}
                    title="Custom color"
                  />
                </div>
              </div>

              <div className="form-actions">
                <button type="submit" className="btn btn-primary">
                  Create Account
                </button>
                <button
                  type="button"
                  className="btn btn-secondary"
                  onClick={() => setView(selectedId ? "profile" : "empty")}
                >
                  Cancel
                </button>
              </div>
            </form>
          </div>
        )}

        {view === "profile" && selectedProfile && (
          <div className="profile-view">
            {/* Profile Header */}
            <div className="profile-header">
              <div
                className="avatar-large"
                style={{ backgroundColor: selectedProfile.avatarColor }}
              >
                {initialsOf(selectedProfile.nickname)}
              </div>
              <div className="profile-info">
                <h2>{selectedProfile.nickname}</h2>
                <span className="profile-meta">
                  created {formatDate(selectedProfile.createdAtMs)}
                </span>
              </div>
            </div>

            {/* Token status */}
            <div className={`token-status ${selectedProfile.hasToken ? "has-token" : "no-token-status"}`}>
              {selectedProfile.hasToken
                ? "Token captured"
                : "No token — log in to capture credentials"}
            </div>

            {/* Waiting for login message */}
            {waitingForLogin === selectedProfile.id && (
              <div className="login-instructions">
                <p>
                  Discord is open. Log in with this account's credentials, then
                  come back here and click <strong>Capture Token</strong>.
                </p>
                <button
                  className="btn btn-primary"
                  onClick={() => captureToken(selectedProfile.id)}
                  disabled={busy}
                >
                  {busy ? "Capturing..." : "Capture Token"}
                </button>
                <button
                  className="btn btn-secondary"
                  style={{ marginLeft: 8 }}
                  onClick={() => setWaitingForLogin(null)}
                  disabled={busy}
                >
                  Cancel
                </button>
              </div>
            )}

            {/* Actions */}
            {!editing && waitingForLogin !== selectedProfile.id && (
              <div className="profile-actions">
                {selectedProfile.hasToken ? (
                  <button
                    className="btn btn-primary"
                    onClick={() => switchToProfile(selectedProfile)}
                    disabled={busy}
                  >
                    {busy ? "Switching..." : "Switch to This Account"}
                  </button>
                ) : (
                  <button
                    className="btn btn-primary"
                    onClick={() => prepareLogin(selectedProfile.id)}
                    disabled={busy}
                  >
                    {busy ? "Launching..." : "Log In"}
                  </button>
                )}
                {selectedProfile.hasToken && (
                  <button
                    className="btn btn-secondary"
                    onClick={() => prepareLogin(selectedProfile.id)}
                    disabled={busy}
                    title="Re-capture token (log in again)"
                  >
                    Re-login
                  </button>
                )}
                <button
                  className="btn btn-secondary"
                  onClick={() => startEditing(selectedProfile)}
                  disabled={busy}
                >
                  Edit
                </button>
                <button
                  className="btn btn-danger"
                  onClick={() => setConfirmRemove(selectedProfile)}
                  disabled={busy}
                >
                  Remove
                </button>
              </div>
            )}

            {/* Edit Mode */}
            {editing && (
              <>
                <div className="edit-row">
                  <input
                    className="field-input"
                    type="text"
                    value={editNickname}
                    onChange={(e) => setEditNickname(e.currentTarget.value)}
                    maxLength={48}
                    autoFocus
                  />
                  <div className="color-row">
                    {PALETTE.map((c) => (
                      <button
                        key={c}
                        type="button"
                        className={`color-swatch${editColor === c ? " selected" : ""}`}
                        style={{ backgroundColor: c }}
                        onClick={() => setEditColor(c)}
                      />
                    ))}
                    <input
                      type="color"
                      className="color-picker-native"
                      value={editColor}
                      onChange={(e) => setEditColor(e.currentTarget.value)}
                    />
                  </div>
                </div>
                <div className="profile-actions">
                  <button className="btn btn-primary btn-sm" onClick={saveEdit}>
                    Save
                  </button>
                  <button
                    className="btn btn-secondary btn-sm"
                    onClick={() => setEditing(false)}
                  >
                    Cancel
                  </button>
                </div>
              </>
            )}

            <div className="divider" />

            {/* Settings */}
            <div className="section-label">Launch Settings</div>

            {installations.length > 0 && (
              <div className="install-chips">
                {installations.map((inst) => (
                  <span key={inst.channel} className="chip">
                    {inst.label}
                  </span>
                ))}
              </div>
            )}

            <div className="settings-row">
              <label>Channel</label>
              <select
                className="field-select"
                value={settingsChannel}
                onChange={(e) =>
                  setSettingsChannel(e.currentTarget.value as DiscordChannel)
                }
              >
                <option value="auto">Auto</option>
                <option value="stable">Stable</option>
                <option value="ptb">PTB</option>
                <option value="canary">Canary</option>
              </select>
            </div>

            <div className="settings-row">
              <label>Custom path</label>
              <input
                className="field-input"
                type="text"
                value={settingsCustomPath}
                onChange={(e) => setSettingsCustomPath(e.currentTarget.value)}
                placeholder="optional"
              />
            </div>

            <div style={{ marginTop: 8 }}>
              <button
                className="btn btn-secondary btn-sm"
                onClick={saveLauncherSettings}
              >
                Save Settings
              </button>
            </div>
          </div>
        )}
      </div>

      {/* ── Toasts ── */}
      {status && <div className="toast success">{status}</div>}
      {error && <div className="toast error">{error}</div>}

      {/* ── Confirm Remove Dialog ── */}
      {confirmRemove && (
        <div className="confirm-overlay" onClick={() => setConfirmRemove(null)}>
          <div className="confirm-box" onClick={(e) => e.stopPropagation()}>
            <h3>Remove account?</h3>
            <p>
              "{confirmRemove.nickname}" and its saved token will be permanently
              deleted.
            </p>
            <div className="form-actions">
              <button
                className="btn btn-danger btn-sm"
                onClick={() => removeProfile(confirmRemove)}
              >
                Remove
              </button>
              <button
                className="btn btn-secondary btn-sm"
                onClick={() => setConfirmRemove(null)}
              >
                Cancel
              </button>
            </div>
          </div>
        </div>
      )}
    </main>
  );
}

export default App;
