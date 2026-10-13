# GMToolKit (`gmtoolkit`)

Cross-platform (Windows/Linux/macOS) Rust tool that sets up Garry's Mod: game content via steamcmd, VPhysics, GModPatchTool, x86-64 (Chromium) branch switch, and Proton Experimental on Linux.

## Commands
- `cargo run` (no args) — GUI (egui, `src/gui.rs`); each button re-runs this binary with CLI args in the background and streams stdout/stderr into the log
- `cargo test` — unit tests (inline `#[test]` in src/main.rs)
- `cargo test -- --include-ignored` — also round-trips this machine's real Steam config files
- `cargo run -- cstrike|tf|dod|hl2mp [off]` — install / uninstall content (downloads GBs, edits the real GMod install)
- `cargo run -- vphysics [off|<tag>]` / `chromium [off|<tag>]` — a release tag installs that version instead of the latest
- `cargo run -- vphysics [off]` — VPhysics-Box3D over every existing `bin/**/vphysics.{dll,so}`, `.orig` backup restored by `off`
- `cargo run -- chromium` — "Chromium patches": refuses unless `MountedConfig` BetaKey is `x86-64`; runs latest GModPatchTool headless (`FORCE_GUI=0`, `-s`), log relayed as `[patch]` lines, tag + patched files with mtimes saved to `<data>/versions/chromium` (badge = all unchanged)
- `cargo run -- chromium off` — opens `steam://validate/4000` (Steam must be running; also reverts Box3D)
- `cargo run -- switch <x86-64|default|prerelease|dev> [native|proton|<compat tool id>]` — branch (+ compat tool on Linux); refuses while Steam runs.

## How it works
- steamcmd is downloaded on first run into `dirs::data_dir()/gmtoolkit/steamcmd` (unpacked with the system `tar` on Linux/macOS); content downloads anonymously via dedicated-server app IDs listed in `GAMES` (id = main content folder = mount.cfg entry name). Not possible anonymously: Alien Swarm (635 needs a subscription), Nuclear Dawn (server has no content), Contagion (no server app), Portal/Portal 2/HL2 episodes (no server).
- Content folders are detected after download by `content_folders` (top-level dirs with `*_dir.vpk` or maps/materials/models/sound, skipping bin/platform/hl2), so DLC/update folders (L4D2) get their own mount entries (`mount_name`); `remount` replaces all of a game's entries at once.
- Game content is mounted via `garrysmod/cfg/mount.cfg` (`remount`/`with_mount`: commented examples and CRLF preserved). Games the user owns on Steam (`Game.steam_appid`) get no mount.cfg entry: GMod mounts those itself with their own path ID; `remove_game` drops our copy. `sync_steam_copies` (GUI start, after tasks, `switch`, `cargo run -- sync`) does that cleanup automatically.
- mount.cfg content only gets GMod's shared `GAME` path ID (verified in-game with `path`), so Lua can't list a game's files. The app writes each game's model list to `garrysmod/data/gmtoolkit/<id>.txt` (`write_index`: first line name, then `models/....mdl` from VPKs + loose) and installs `src/gmtoolkit.lua` as `addons/gmtoolkit/lua/autorun/client/` while any list exists (`sync_addon`). The addon adds each game under the spawn menu's Browse → Games folder (found a frame after PopulateContent; own "GMToolKit" folder as fallback; icons created on click) shows a hint when a listed game is also mounted natively, and overrides `IsMounted` to return true for app-installed games so GMod's own `needsapp` spawnlists (settings/spawnlist: CS:S, DoD:S, TF2, L4D, CS:GO, …) appear. Maps-only games (CS:S) get a name-only list file for that. Games without a built-in list get one generated at `PopulatePropMenu` via `spawnmenu.AddPropCategory` (no files; IDs from 770000): a parent per game, a sub-list per top-level model folder, junk filtered by `useful()`.
- Under Proton (GMod has a compat tool forced) mount.cfg paths must be Wine paths, `Z:/home/...`; `mount_path`/`host_path` convert, and `sync_steam_copies` (also run by `switch` and `cargo run -- sync`) rewrites entries when the style is wrong.
- Otherwise the dedicated server is downloaded to `<GarrysMod>/gmtoolkit/<id>` (`content_dir`: same drive as GMod, visible to Flatpak/Snap Steam and Proton) and `prune` deletes everything GMod doesn't use: keeps `<folder>/*.vpk` (except CSS) and `maps/*.bsp|nav`, materials, models, sound, particles; keeps `steamapps/` for steamcmd. CSS keeps only maps (GMod ships CSS content).
- steamcmd tries the Windows depot first (`+@sSteamCmdForcePlatformType windows`; DoD:S's Linux depot fails with "Missing configuration"), then Linux (some servers only ship content there).
- GMod folder: `gmod_dir()` uses the folder saved in `<data>/gmod_path.txt` (first-launch screen / Settings tab, `save_gmod_dir`), else `detect_gmod_dir()`. GMod's manifest is derived from it (`<library>/steamapps/appmanifest_4000.acf`). Browse uses `rfd` (xdg-portal on Linux) on a background thread.
- Steam is found by `steam()`: checks native, Flatpak, Snap and Debian install paths and prefers the one that has GMod (steamlocate alone returns only the first install). Library folders on other drives come from that install's libraryfolders.vdf. Always use `steam()`, never `SteamDir::locate()` directly.

## Decisions
- Compat tools listed by `compat_tools()`: Proton Experimental always first/recommended (Steam fetches it on demand), Valve Protons from library `common/Proton *` (id via `valve_proton_id`), custom ones from `compatibilitytools.d` manifests.
- GUI theme: on Linux `watch_desktop_theme` polls the XDG settings portal (via `busctl`) every 3s for color-scheme + accent-color, so it follows KDE/GNOME incl. accent; elsewhere egui's OS detection. Colors in `pal()` / `style()`, accent in the `ACCENT` atomic.
- Versions: installs record a version in `<data>/versions/<id>` (GitHub tag, or the server's Steam buildid from its appmanifest). The GUI fetches latest GitHub releases (`releases`) and current server builds (`latest_builds`, one anonymous steamcmd app_info_print) in the background and shows "Update available"; GitHub items also get a version dropdown. Installs from before tracking show "version unknown".
- GUI fonts/shapes follow the desktop: `desktop_font` (KDE kdeglobals `font=`/`fixed=` or Plasma defaults Noto Sans/Hack 10pt; else gsettings; Windows Segoe UI/Consolas; macOS SF) → `font_file` (fc-match on Linux) → `load_system_fonts`; all text sizes scale from `base_px()`. Corner radii per desktop in `radii()` (Breeze small, Adwaita round, Win11/macOS between).
- Features a platform can't use are hidden, not disabled: compat tool dropdown Linux-only, VPhysics-Box3D (row, notes, release fetch) not on macOS.
- Cancel/Resume: tasks run in their own process group (Unix) and Cancel kills the group (`kill -TERM -<pgid>`) or tree (`taskkill /T`); closing the window cancels too (`on_exit`). A game dir with no mount entry is `is_partial` → row shows Resume (re-runs the install; steamcmd continues the partial download) and Discard (`<id> off`).
- GUI layout: tabs "Game content" / "Patches" (version card + Chromium + Box3D); each `item_row` shows one button — Install, or Uninstall when `is_installed` — and a spinner while its task runs; bottom status bar shows the last log line with a Show log toggle.
- Compat dropdown shows GMod's actual forced tool; a third-party one (e.g. Proton-GE) is left untouched unless the user picks Native / Proton Experimental.
- Default VPhysics: Box3D (active, has GMod release zips); Jolt optional. Neither has macOS builds — hide on Mac.
- x86-64 branch on macOS is unconfirmed; mark experimental.
- Branch switch: `betakey "x86-64"` in `appmanifest_4000.acf`; Proton: `CompatToolMapping` 4000 → `proton_experimental` in `config/config.vdf`. Steam must be closed.
- Steam VDF: parsed with `keyvalues-parser`, but written by our own `write_vdf` — Steam escapes only `\\` and `"` and keeps newlines literal, which neither of the crate's renderers matches.
- GitHub downloads (`github_asset`) are verified against the release asset's `digest` (sha256) from the API.
- Proton installs of GMod use the Windows dlls on Linux, so vphysics swaps whichever files exist rather than guessing by OS.
- macOS GModPatchTool ships as a .dmg inside the zip; mount/run path is untested.
- GModPatchTool: run its release headless; not reimplemented (~2k lines of binary patching + server-hosted patch files that track GMod updates).
- Releases (`.github/workflows/release.yml`): portable static-CRT `.exe`, `.AppImage` built on Ubuntu 22.04, universal macOS binary zipped.
- Keep it minimal: single binary; GUI drives the CLI by spawning itself, so features are written once as CLI commands.
- Windows builds use `windows_subsystem = "windows"`; every spawned console child goes through `hide_window` (CREATE_NO_WINDOW).
