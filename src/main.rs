#![cfg_attr(windows, windows_subsystem = "windows")] // no console window for the GUI

mod gui;

use anyhow::{Context, Result, bail};
use keyvalues_parser::{Obj, Value};
use sha2::Digest;
use std::borrow::Cow;
use std::{fs, path::{Path, PathBuf}, process::Command};

const GMOD_APPID: u32 = 4000;
/// GMod's Steam branches as (betakey, label); "" is the default branch.
pub const BRANCHES: &[(&str, &str)] = &[
    ("x86-64", "64-bit + Chromium (recommended)"),
    ("", "Default (32-bit)"),
    ("prerelease", "Prerelease"),
    ("dev", "Dev"),
];
/// A Source game whose content GMod can mount.
pub struct Game {
    /// Also the name of its main content folder and of its mount.cfg entry.
    pub id: &'static str,
    pub name: &'static str,
    /// Dedicated server app: anonymous-downloadable, with the same content as the game.
    pub server_appid: u32,
    /// The regular Steam game, used instead when it's installed.
    pub steam_appid: u32,
    /// Listed in GMod's own Games menu (so its content is known to work well); the rest are community Source games.
    pub gmod_supported: bool,
    /// Loose top-level folders kept after download (maps keep only .bsp/.nav).
    keep: &'static [&'static str],
    /// Whether the game's VPK archives are kept.
    vpks: bool,
}

const fn game(id: &'static str, name: &'static str, server_appid: u32, steam_appid: u32, gmod_supported: bool) -> Game {
    Game { id, name, server_appid, steam_appid, gmod_supported, keep: CONTENT_DIRS, vpks: true }
}

pub const GAMES: &[Game] = &[
    // GMod already ships CSS content; only the maps are missing.
    Game { keep: &["maps"], vpks: false, ..game("cstrike", "Counter-Strike: Source", 232330, 240, true) },
    game("tf", "Team Fortress 2", 232250, 440, true),
    game("dod", "Day of Defeat: Source", 232290, 300, true),
    game("hl2mp", "Half-Life 2: Deathmatch", 232370, 320, true),
    game("left4dead2", "Left 4 Dead 2", 222860, 550, true),
    game("left4dead", "Left 4 Dead", 222840, 500, true),
    game("csgo", "Counter-Strike: Global Offensive", 740, 730, true),
    game("insurgency", "Insurgency", 237410, 222880, true),
    game("bms", "Black Mesa", 346680, 362890, true),
    game("nmrih", "No More Room in Hell", 317670, 224260, false),
    game("doi", "Day of Infamy", 462310, 447820, false),
    game("fof", "Fistful of Frags", 295230, 265630, false),
    game("zps", "Zombie Panic! Source", 17505, 17500, false),
    game("pvkii", "Pirates, Vikings & Knights II", 17575, 17570, false),
];
const CONTENT_DIRS: &[&str] = &["maps", "materials", "models", "sound", "particles"];

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() { return gui::run().map_err(|e| anyhow::anyhow!("{e}")); }
    let on = args.get(1).map_or(true, |a| a != "off");
    // `vphysics <tag>` / `chromium <tag>` install a specific release instead of the latest.
    let tag = args.get(1).map(String::as_str).filter(|a| *a != "off");
    match args.first().map(String::as_str) {
        Some("switch") => {
            // switch <branch|default> [native|proton]: one Steam-closed check covers both edits.
            let branch = args.get(1).map(String::as_str).filter(|b| *b != "default").unwrap_or("");
            set_branch(branch)?;
            if let Some(compat) = args.get(2) { set_compat(compat)?; }
            // Native and Proton need different mount.cfg path styles.
            for line in sync_steam_copies() { println!("{line}"); }
            return Ok(());
        }
        Some("sync") => { for line in sync_steam_copies() { println!("{line}"); } return Ok(()); }
        Some(engine @ ("vphysics" | "jolt")) => return set_vphysics(engine, on, tag),
        Some("chromium") => return if on { run_patchtool(tag) } else { unpatch_chromium() },
        // skategm [<tag>] [<disc .iso or default.xex>]
        Some("skategm") => {
            let disc = args[1..].iter().find(|a| Path::new(a).is_file()).map(String::as_str);
            let tag = args[1..].iter().map(String::as_str).find(|a| Some(*a) != disc && *a != "off");
            return if on { install_skategm(tag, disc) } else { remove_skategm() };
        }
        Some("gwater2") => return if on { install_gwater2(tag) } else { remove_gwater2() },
        _ => {}
    }
    let name = args[0].clone();
    let game = GAMES.iter().find(|g| g.id == name)
        .with_context(|| {
            let games: Vec<_> = GAMES.iter().map(|g| g.id).collect();
            format!("unknown command {name}, pick one of: {} switch sync vphysics jolt chromium skategm gwater2", games.join(" "))
        })?;
    install_game(game, on)
}

/// Where a downloaded game lives: inside GMod's own folder, so it's on the same drive and visible to
/// sandboxed Steam (Flatpak/Snap) and Proton alike.
fn content_dir(id: &str) -> Result<PathBuf> {
    Ok(gmod_dir()?.join("gmtoolkit").join(id))
}

fn mount_cfg() -> Result<PathBuf> {
    Ok(gmod_dir()?.join("garrysmod/cfg/mount.cfg"))
}

/// The install folder of the user's own Steam copy of the game, if installed.
fn steam_copy(game: &Game) -> Option<PathBuf> {
    let steam = steam().ok()?;
    let (app, lib) = steam.find_app(game.steam_appid).ok()??;
    Some(lib.resolve_app_dir(&app)).filter(|p| p.join(game.id).is_dir())
}

/// A download that was cancelled or cut off (app closed): files on disk but never mounted. Re-running the
/// install resumes it, since steamcmd keeps partial downloads in the same folder.
pub fn is_partial(id: &str) -> bool {
    let Some(game) = GAMES.iter().find(|g| g.id == id) else { return false };
    content_dir(id).is_ok_and(|d| d.exists())
        && steam_copy(game).is_none()
        && !mount_cfg().and_then(|c| Ok(fs::read_to_string(c)?)).is_ok_and(|c| mount_entry(&c, id).is_some())
}

pub fn uses_steam_copy(id: &str) -> bool {
    GAMES.iter().find(|g| g.id == id).is_some_and(|g| steam_copy(g).is_some())
}

/// Top-level folders of an install that hold content, e.g. `left4dead2` plus `left4dead2_dlc1..3` and `update`.
/// Engine folders are skipped (`hl2`/`platform` content already ships with GMod).
fn content_folders(root: &Path) -> Vec<PathBuf> {
    let has_content = |d: &Path| fs::read_dir(d).into_iter().flatten().flatten().any(|e| {
        let n = e.file_name().to_string_lossy().to_lowercase();
        n.ends_with("_dir.vpk") || ["maps", "materials", "models", "sound"].contains(&n.as_str())
    });
    let mut dirs: Vec<PathBuf> = fs::read_dir(root).into_iter().flatten().flatten().map(|e| e.path())
        .filter(|p| p.is_dir())
        .filter(|p| !["bin", "platform", "hl2", "steamapps"].contains(&&*p.file_name().unwrap().to_string_lossy().to_lowercase()))
        .filter(|p| has_content(p))
        .collect();
    dirs.sort();
    dirs
}

/// mount.cfg entry name for a content folder: the folder itself if it starts with the game id, else prefixed
/// (two games can both have an `update` folder).
fn mount_name(game: &Game, folder: &Path) -> String {
    let f = folder.file_name().unwrap().to_string_lossy().into_owned();
    if f.starts_with(game.id) { f } else { format!("{}_{f}", game.id) }
}

/// Replaces all of a game's mount.cfg entries with one per content folder under `root` (or none).
fn remount(game: &Game, root: Option<&Path>) -> Result<()> {
    let cfg = mount_cfg()?;
    let mut text = fs::read_to_string(&cfg).unwrap_or_else(|_| "\"mountcfg\"\n{\n}\n".into());
    let ours = content_dir(game.id)?;
    let steam_root = steam_copy(game);
    for (name, path) in mount_entries(&text) {
        let host = host_path(&path);
        let belongs = name == game.id || name.starts_with(&format!("{}_", game.id))
            || host.starts_with(&ours) || steam_root.as_ref().is_some_and(|r| host.starts_with(r));
        if belongs { text = with_mount(&text, &name, None)?; }
    }
    for folder in root.map(content_folders).unwrap_or_default() {
        text = with_mount(&text, &mount_name(game, &folder), Some(&mount_path(&folder)))?;
    }
    if fs::read_to_string(&cfg).ok().as_deref() != Some(&*text) { fs::write(&cfg, text)?; }
    Ok(())
}

/// Installs (or removes) a game's content and points mount.cfg at it. Uses the user's Steam copy when there is one;
/// otherwise downloads the dedicated server and deletes everything GMod doesn't need from it.
fn install_game(game: &Game, on: bool) -> Result<()> {
    let id = game.id;
    let dir = content_dir(id)?;
    // Uninstalling, or the user owns it on Steam: GMod mounts Steam copies itself (Games menu), so drop ours.
    if !on || steam_copy(game).is_some() {
        remove_game(game)?;
        println!("{}", if on { format!("{} is installed through Steam; GMod mounts it itself (tick it in GMod's Games menu)", game.name) }
            else { format!("{} uninstalled", game.name) });
        return Ok(());
    }
    let steamcmd = ensure_steamcmd(&data_dir()?.join("steamcmd"))?;
    fs::create_dir_all(&dir)?;
    let appid = game.server_appid;
    // Windows depots first: same content, and some Linux ones (DoD:S) fail with "Missing configuration".
    // Some servers (Nuclear Dawn) only ship content in their Linux depot, so fall back to that.
    let mut ok = false;
    for platform in ["windows", "linux"] {
        let mut cmd = Command::new(&steamcmd);
        cmd.args(["+@sSteamCmdForcePlatformType", platform, "+force_install_dir"]).arg(&dir)
            .args(["+login", "anonymous", "+app_update", &appid.to_string(), "validate", "+quit"]);
        hide_window(&mut cmd);
        // steamcmd can exit 0 on failure, so also check that the game folder arrived.
        if cmd.status()?.success() && dir.join(id).exists() { ok = true; break; }
        println!("no {} content in the {platform} download, trying the next platform", game.name);
    }
    if !ok {
        let _ = fs::remove_dir_all(&dir);
        bail!("steamcmd failed to download {}", game.name);
    }
    // Remember the server build we got, to compare against Steam's current one for updates.
    let manifest = fs::read_to_string(dir.join(format!("steamapps/appmanifest_{appid}.acf"))).unwrap_or_default();
    set_installed_version(id, vdf_get(&manifest, &["buildid"]).as_deref())?;
    let folders = content_folders(&dir);
    if folders.is_empty() {
        let _ = fs::remove_dir_all(&dir);
        set_installed_version(id, None)?;
        bail!("the {} server download has no game content GMod can use", game.name);
    }
    let freed = prune(&dir, &folders, game)?;
    remount(game, Some(&dir))?;
    write_index(game, &dir)?;
    sync_addon()?;
    let names: Vec<_> = folders.iter().map(|f| f.file_name().unwrap().to_string_lossy().into_owned()).collect();
    println!("{} installed and mounted ({}); removed {} MB of files GMod doesn't use", game.name, names.join(", "), freed / 1_000_000);
    Ok(())
}

/// Removes our download, mount.cfg entries and model list for a game.
fn remove_game(game: &Game) -> Result<()> {
    let dir = content_dir(game.id)?;
    remount(game, None)?;
    if dir.exists() { fs::remove_dir_all(&dir)?; }
    let _ = fs::remove_file(index_path(game.id)?);
    set_installed_version(game.id, None)?;
    sync_addon()
}

/// The game's model list for the spawn menu addon (mount.cfg content has no path ID of its own in GMod,
/// so Lua can't list a game's files by itself).
fn index_path(id: &str) -> Result<PathBuf> {
    Ok(gmod_dir()?.join("garrysmod/data/gmtoolkit").join(format!("{id}.txt")))
}

/// Writes `<name>` then every model path (`models/....mdl`) in the game's content folders, VPKs included.
fn write_index(game: &Game, root: &Path) -> Result<usize> {
    let mut models = std::collections::BTreeSet::new();
    for folder in content_folders(root) {
        for e in fs::read_dir(&folder)? {
            let p = e?.path();
            if !p.to_string_lossy().ends_with("_dir.vpk") { continue; }
            let pak = vpk::from_path(&p).with_context(|| format!("reading {}", p.display()))?;
            models.extend(pak.tree.keys().map(|k| k.replace('\\', "/").to_lowercase()));
        }
        let mut stack = vec![folder.join("models")];
        while let Some(d) = stack.pop() {
            for e in fs::read_dir(&d).into_iter().flatten().flatten() {
                let p = e.path();
                if p.is_dir() { stack.push(p); continue; }
                models.insert(p.strip_prefix(&folder)?.to_string_lossy().replace('\\', "/").to_lowercase());
            }
        }
    }
    models.retain(|m| m.starts_with("models/") && m.ends_with(".mdl"));
    // Written even without models (CS:S is maps only): the addon also uses it to know the game is installed.
    let path = index_path(game.id)?;
    fs::create_dir_all(path.parent().unwrap())?;
    let list: Vec<&str> = models.iter().map(String::as_str).collect();
    fs::write(&path, format!("{}\n{}\n", game.name, list.join("\n")))?;
    Ok(models.len())
}

/// Installs the spawn menu addon while any model list exists, and removes it once none do.
fn sync_addon() -> Result<()> {
    let gmod = gmod_dir()?;
    let addon = gmod.join("garrysmod/addons/gmtoolkit");
    let lists = fs::read_dir(gmod.join("garrysmod/data/gmtoolkit")).into_iter().flatten().flatten()
        .any(|e| e.path().extension().is_some_and(|x| x == "txt"));
    if !lists {
        if addon.exists() { fs::remove_dir_all(&addon)?; }
        return Ok(());
    }
    let lua = addon.join("lua/autorun/client/gmtoolkit.lua");
    let code = include_str!("gmtoolkit.lua");
    if fs::read_to_string(&lua).ok().as_deref() != Some(code) {
        fs::create_dir_all(lua.parent().unwrap())?;
        fs::write(&lua, code)?;
    }
    Ok(())
}

/// Whether a file (relative to a content folder) is worth keeping for GMod.
fn wanted(path: &str, game: &Game) -> bool {
    let path = path.replace('\\', "/").to_lowercase();
    let Some((top, _)) = path.split_once('/') else {
        return game.vpks && path.ends_with(".vpk");
    };
    game.keep.contains(&top) && (top != "maps" || path.ends_with(".bsp") || path.ends_with(".nav"))
}

/// Deletes everything in a server download that GMod doesn't use, keeping wanted files in the content folders;
/// `steamapps/` stays so updates are incremental. Returns the bytes freed.
fn prune(dir: &Path, folders: &[PathBuf], game: &Game) -> Result<u64> {
    let mut freed = 0;
    let mut stack = vec![dir.to_path_buf()];
    let mut dirs = Vec::new();
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d)? {
            let p = e?.path();
            if p == dir.join("steamapps") { continue; }
            if p.is_dir() { stack.push(p.clone()); dirs.push(p); continue; }
            let keep = folders.iter().find_map(|f| p.strip_prefix(f).ok())
                .is_some_and(|r| wanted(&r.to_string_lossy(), game));
            if !keep { freed += fs::metadata(&p)?.len(); fs::remove_file(&p)?; }
        }
    }
    // Deepest first, so emptied parents can go too; non-empty ones just fail.
    dirs.sort_by_key(|d| std::cmp::Reverse(d.components().count()));
    for d in dirs { let _ = fs::remove_dir(d); }
    Ok(freed)
}

/// Keeps mount.cfg in step with what's on disk: drops our copy of a game once it's installed through Steam
/// (GMod mounts that itself), rewrites paths when GMod moves between native and Proton, drops entries whose
/// folder is gone, writes missing model lists, and installs/removes the spawn menu addon. Returns a line per change.
pub fn sync_steam_copies() -> Vec<String> {
    let mut changes = Vec::new();
    let Ok(cfg) = mount_cfg() else { return changes };
    for game in GAMES {
        let Ok(dir) = content_dir(game.id) else { continue };
        let text = fs::read_to_string(&cfg).unwrap_or_default();
        let current = mount_entry(&text, game.id);
        let ours = dir.join(game.id).is_dir();
        let result = if steam_copy(game).is_some() {
            if !ours && current.is_none() { continue; }
            remove_game(game).map(|_| format!("{}: now installed through Steam, removed our copy", game.name))
        } else if ours && current.as_ref().is_some_and(|c| *c != mount_path(&dir.join(game.id))) {
            remount(game, Some(&dir)).map(|_| format!("{}: updated the mount paths", game.name))
        } else if ours && current.is_some() && !index_path(game.id).is_ok_and(|p| p.exists()) {
            write_index(game, &dir).map(|n| format!("{}: listed {n} models for the spawn menu", game.name))
        } else if !ours && current.as_ref().is_some_and(|c| !host_path(c).exists()) {
            remount(game, None).map(|_| format!("{}: removed mounts for missing folders", game.name))
        } else {
            continue;
        };
        changes.push(result.unwrap_or_else(|e| format!("{}: {e}", game.name)));
    }
    if let Err(e) = sync_addon() { changes.push(format!("spawn menu addon: {e}")); }
    changes
}

/// Whether GMod runs the Windows build under Proton/Wine, where Linux paths must be written as `Z:/...`.
fn gmod_uses_wine() -> bool {
    cfg!(target_os = "linux") && !compat_tool().is_empty()
}

/// A path as GMod sees it: forward slashes, and on the `Z:` drive under Proton (Wine maps `/` to `Z:`).
fn mount_path(p: &Path) -> String {
    let s = p.to_string_lossy().replace('\\', "/");
    if gmod_uses_wine() { format!("Z:{s}") } else { s }
}

/// The reverse of `mount_path`: a mount.cfg path as a path on this machine.
fn host_path(entry: &str) -> PathBuf {
    let unix = cfg!(target_os = "linux") && entry.len() > 2 && entry[..2].eq_ignore_ascii_case("z:");
    PathBuf::from(if unix { &entry[2..] } else { entry })
}

/// All live `"name" "path"` entries.
fn mount_entries(cfg: &str) -> Vec<(String, String)> {
    cfg.lines().filter_map(|l| {
        let mut parts = l.trim().split('"').filter(|p| !p.trim().is_empty());
        let (name, path) = (parts.next()?, parts.next()?);
        (l.trim_start().starts_with('"') && name != "mountcfg").then(|| (name.to_owned(), path.to_owned()))
    }).collect()
}

/// The path mounted for `name`, ignoring commented-out lines.
fn mount_entry(cfg: &str, name: &str) -> Option<String> {
    mount_entries(cfg).into_iter().find(|(n, _)| n == name).map(|(_, p)| p)
}

/// Removes any live entry for `name`, then adds `path` (if given) before the closing brace.
fn with_mount(cfg: &str, name: &str, path: Option<&str>) -> Result<String> {
    let quoted = format!("\"{name}\"");
    // Commented examples stay; only live entries for this name are replaced.
    let mut lines: Vec<&str> = cfg.lines().filter(|l| !l.trim_start().starts_with(&quoted)).collect();
    let close = lines.iter().rposition(|l| l.trim() == "}").context("mount.cfg has no closing brace")?;
    let entry = path.map(|p| format!("\t{quoted}\t\"{p}\""));
    if let Some(e) = &entry { lines.insert(close, e); }
    let nl = if cfg.contains("\r\n") { "\r\n" } else { "\n" }; // GMod ships it with CRLF
    Ok(lines.join(nl) + nl)
}

/// The Steam install to use. Several can coexist (native, Flatpak, Snap, Debian's), so prefer the one that has
/// GMod; extra library folders on other drives come from its libraryfolders.vdf.
fn steam() -> Result<steamlocate::SteamDir> {
    static CHOSEN: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    let chosen = CHOSEN.get_or_init(|| {
        let home = dirs::home_dir()?;
        let data = dirs::data_dir()?;
        // Same places steamlocate checks on Linux; it only returns the first one found.
        let candidates = [
            home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"),
            home.join(".var/app/com.valvesoftware.Steam/.steam/steam"),
            data.join("Steam"),
            home.join(".steam/steam"),
            home.join(".steam/debian-installation"),
            home.join("snap/steam/common/.local/share/Steam"),
            home.join("snap/steam/common/.steam/steam"),
        ];
        candidates.into_iter().filter(|_| cfg!(target_os = "linux"))
            .find(|p| steamlocate::SteamDir::from_dir(p).is_ok_and(|s| s.find_app(GMOD_APPID).ok().flatten().is_some()))
    });
    Ok(match chosen {
        Some(p) => steamlocate::SteamDir::from_dir(p)?,
        None => steamlocate::SteamDir::locate()?,
    })
}

/// The GMod folder: the one picked in the app if set, otherwise auto-detected.
fn gmod_dir() -> Result<PathBuf> {
    match saved_gmod_dir() {
        Some(p) => Ok(p),
        None => detect_gmod_dir(),
    }
}

/// Finds GMod through Steam's libraries.
pub fn detect_gmod_dir() -> Result<PathBuf> {
    let steam = steam()?;
    let (app, lib) = steam.find_app(GMOD_APPID)?.context("Garry's Mod is not installed")?;
    Ok(lib.resolve_app_dir(&app))
}

/// A GMod install has a `garrysmod` content folder.
pub fn is_gmod_dir(p: &Path) -> bool {
    p.join("garrysmod").is_dir()
}

/// The folder chosen in the app (first launch or Settings), stored in `<data>/gmod_path.txt`.
pub fn saved_gmod_dir() -> Option<PathBuf> {
    let p = PathBuf::from(fs::read_to_string(data_dir().ok()?.join("gmod_path.txt")).ok()?.trim());
    is_gmod_dir(&p).then_some(p)
}

pub fn save_gmod_dir(p: &Path) -> Result<()> {
    if !is_gmod_dir(p) { bail!("{} doesn't look like a Garry's Mod folder (no garrysmod folder inside)", p.display()); }
    fs::create_dir_all(data_dir()?)?;
    Ok(fs::write(data_dir()?.join("gmod_path.txt"), p.to_string_lossy().as_bytes())?)
}

fn steam_running() -> bool {
    let (cmd, args): (&str, &[&str]) = match std::env::consts::OS {
        "windows" => ("tasklist", &["/FI", "IMAGENAME eq steam.exe", "/NH"]),
        "macos" => ("pgrep", &["-x", "steam_osx"]),
        _ => ("pgrep", &["-x", "steam"]),
    };
    // pgrep prints PIDs on a match; tasklist always succeeds, so look for the name.
    let mut c = Command::new(cmd);
    hide_window(c.args(args));
    c.output()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).to_lowercase().contains(if cmd == "pgrep" { "" } else { "steam.exe" }) && !o.stdout.is_empty())
}

/// Steam rewrites its config files on exit, so edits made while it runs are lost.
fn edit_steam_file(path: &Path, edit: impl FnOnce(&mut Obj)) -> Result<()> {
    if steam_running() { bail!("Close Steam first, then try again"); }
    let text = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    fs::write(path, vdf_edit(&text, edit)?)?;
    Ok(())
}

fn vdf_edit(text: &str, edit: impl FnOnce(&mut Obj)) -> Result<String> {
    let mut vdf = keyvalues_parser::parse(text)?;
    edit(vdf.value.get_mut_obj().context("vdf root is not an object")?);
    let mut out = String::new();
    write_vdf(&mut out, 0, &vdf.key, &vdf.value);
    Ok(out)
}

/// Steam escapes only `\` and `"` (newlines stay literal), which the crate's renderers don't match.
fn write_vdf(out: &mut String, depth: usize, key: &str, val: &Value) {
    let q = |s: &str| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""));
    let tabs = "\t".repeat(depth);
    match val {
        Value::Str(s) => out.push_str(&format!("{tabs}{}\t\t{}\n", q(key), q(s))),
        Value::Obj(obj) => {
            out.push_str(&format!("{tabs}{}\n{tabs}{{\n", q(key)));
            for (k, vals) in obj.iter() {
                for v in vals { write_vdf(out, depth + 1, k, v); }
            }
            out.push_str(&format!("{tabs}}}\n"));
        }
    }
}

/// Walks (creating as needed) nested objects; Steam's keys are case-insensitive.
fn child<'a, 't>(obj: &'a mut Obj<'t>, key: &str) -> &'a mut Obj<'t> {
    let k = obj.keys().find(|k| k.eq_ignore_ascii_case(key)).cloned()
        .unwrap_or_else(|| Cow::Owned(key.to_owned()));
    let vals = obj.entry(k).or_default();
    if !vals.first().is_some_and(Value::is_obj) { *vals = vec![Value::Obj(Obj::new())]; }
    vals[0].get_mut_obj().unwrap()
}

fn set(obj: &mut Obj, key: &str, val: &str) {
    obj.retain(|k, _| !k.eq_ignore_ascii_case(key));
    obj.insert(Cow::Owned(key.to_owned()), vec![Value::Str(Cow::Owned(val.to_owned()))]);
}

/// Switches GMod to the 64-bit Chromium branch (or back to the default branch).
/// GMod's Steam manifest, two levels up from its folder (`<library>/steamapps/common/GarrysMod`).
fn manifest_path() -> Result<PathBuf> {
    let gmod = gmod_dir()?;
    let steamapps = gmod.parent().and_then(Path::parent).context("GMod folder isn't inside a Steam library")?;
    Ok(steamapps.join(format!("appmanifest_{GMOD_APPID}.acf")))
}

fn set_branch(branch: &str) -> Result<()> {
    if !BRANCHES.iter().any(|b| b.0 == branch) { bail!("unknown branch {branch}"); }
    edit_steam_file(&manifest_path()?, |root| set(child(root, "UserConfig"), "BetaKey", branch))?;
    println!("branch set to {}; Steam will update GMod the next time it starts", if branch.is_empty() { "default" } else { branch });
    Ok(())
}

/// Reads a string at `path` below the root of a VDF file; Steam keys are case-insensitive.
fn vdf_get(text: &str, path: &[&str]) -> Option<String> {
    let vdf = keyvalues_parser::parse(text).ok()?;
    let mut val = &vdf.value;
    for key in path {
        let obj = val.get_obj()?;
        val = obj.iter().find(|(k, _)| k.eq_ignore_ascii_case(key))?.1.first()?;
    }
    val.get_str().map(str::to_owned)
}

/// The branch GMod is on: `UserConfig` is what was picked, `MountedConfig` what Steam actually installed.
pub fn current_branch(installed: bool) -> Result<String> {
    let text = fs::read_to_string(manifest_path()?)?;
    Ok(vdf_get(&text, &[if installed { "MountedConfig" } else { "UserConfig" }, "BetaKey"]).unwrap_or_default())
}

/// The compatibility tool forced for GMod ("" = native), e.g. "proton_experimental" or "Proton-GE Latest".
pub fn compat_tool() -> String {
    (|| {
        let text = fs::read_to_string(steam().ok()?.path().join("config/config.vdf")).ok()?;
        vdf_get(&text, &["Software", "Valve", "Steam", "CompatToolMapping", &GMOD_APPID.to_string(), "name"])
    })().unwrap_or_default()
}

/// Forces a compatibility tool for GMod (Linux only): "native", "proton" (= Experimental) or a tool's internal name.
fn set_compat(tool: &str) -> Result<()> {
    if std::env::consts::OS != "linux" { bail!("Compatibility tools are Linux only"); }
    let tool = if tool == "proton" { "proton_experimental" } else { tool };
    let steam = steam()?;
    edit_steam_file(&steam.path().join("config/config.vdf"), |root| {
        let map = child(child(child(child(root, "Software"), "Valve"), "Steam"), "CompatToolMapping");
        if tool == "native" {
            map.retain(|k, _| k != GMOD_APPID.to_string().as_str());
        } else {
            let gmod = child(map, &GMOD_APPID.to_string());
            set(gmod, "name", tool);
            set(gmod, "config", "");
            set(gmod, "priority", "250");
        }
    })?;
    println!("compatibility tool set to {tool}");
    Ok(())
}

/// Installed compatibility tools as (internal name, display name), Proton Experimental always first
/// (Steam downloads it on demand). Valve's Protons live in library `common/` dirs, custom ones
/// (GE etc.) in `compatibilitytools.d` with a manifest naming them.
pub fn compat_tools() -> Vec<(String, String)> {
    let mut tools = vec![("proton_experimental".to_owned(), "Proton Experimental".to_owned())];
    let Ok(steam) = steam() else { return tools };
    let dirs = |p: PathBuf| fs::read_dir(p).into_iter().flatten().flatten().map(|e| e.path());
    for lib in steam.library_paths().unwrap_or_default() {
        for dir in dirs(lib.join("steamapps/common")) {
            let name = dir.file_name().unwrap().to_string_lossy().into_owned();
            // "Proton - Experimental" is already listed first.
            if let Some(v) = name.strip_prefix("Proton ").filter(|v| !v.contains("Experimental")) {
                tools.push((valve_proton_id(v), name));
            }
        }
    }
    for root in [steam.path().join("compatibilitytools.d"), "/usr/share/steam/compatibilitytools.d".into()] {
        for dir in dirs(root) {
            let Ok(text) = fs::read_to_string(dir.join("compatibilitytool.vdf")) else { continue };
            let Ok(vdf) = keyvalues_parser::parse(&text) else { continue };
            let Some(list) = vdf.value.get_obj().and_then(|o| o.get("compat_tools")).and_then(|v| v[0].get_obj()) else { continue };
            for (id, v) in list.iter() {
                let display = v[0].get_obj().and_then(|o| o.get("display_name")).and_then(|d| d[0].get_str()).unwrap_or(id);
                tools.push((id.to_string(), display.to_owned()));
            }
        }
    }
    let mut seen = std::collections::HashSet::new();
    tools.retain(|t| seen.insert(t.0.clone()));
    tools
}

/// Steam's internal ids for Valve Protons: "9.0" -> proton_9, "5.13" -> proton_513, "Hotfix" -> proton_hotfix.
fn valve_proton_id(version: &str) -> String {
    let v = version.strip_suffix(".0").unwrap_or(version).replace('.', "").to_lowercase();
    format!("proton_{}", v.replace(' ', "_"))
}

/// Whether a game's content / VPhysics-Box3D / VPhysics-Jolt / the Chromium patches are currently installed (for the GUI badges).
pub fn is_installed(item: &str) -> bool {
    let Ok(gmod) = gmod_dir() else { return false };
    match item {
        // Box3D unless Jolt's version is recorded (Box3D installs predate version tracking).
        "vphysics" => vphysics_swapped(&gmod) && installed_version("jolt").is_none(),
        "jolt" => vphysics_swapped(&gmod) && installed_version("jolt").is_some(),
        "chromium" => chromium_patched(&gmod),
        "skategm" => gmod.join("garrysmod/lua/bin/gmcl_skategm_win64.dll").exists(),
        "gwater2" => installed_version("gwater2").is_some()
            && ["win64", "linux64"].iter().any(|b| gmod.join(format!("garrysmod/lua/bin/gmcl_gwater2_{b}.dll")).exists()),
        _ => uses_steam_copy(item) || mount_cfg().and_then(|c| Ok(fs::read_to_string(c)?)).is_ok_and(|c| mount_entry(&c, item).is_some()),
    }
}

/// A physics engine is in place if a backup exists and differs from the live file (Steam verify restores stock).
fn vphysics_swapped(gmod: &Path) -> bool {
    let size = |p: &Path| fs::metadata(p).map(|m| m.len()).ok();
    ["bin/win64/vphysics.dll", "bin/vphysics.dll", "bin/linux64/vphysics.so", "bin/vphysics.so"]
        .iter().map(|p| gmod.join(p)).any(|p| size(&orig(&p)).is_some_and(|o| Some(o) != size(&p)))
}

/// `vphysics.dll` -> `vphysics.dll.orig`
fn orig(p: &Path) -> PathBuf {
    p.with_extension(format!("{}.orig", p.extension().unwrap().to_str().unwrap()))
}

/// CREATE_NO_WINDOW: the app is GUI-only on Windows, so console children would pop a window.
pub fn hide_window(cmd: &mut Command) {
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(cmd, 0x0800_0000);
    let _ = cmd;
}

fn data_dir() -> Result<PathBuf> {
    Ok(dirs::data_dir().context("no data dir")?.join("gmtoolkit"))
}

/// Downloads a release asset, checked against GitHub's sha256 digest.
pub const BOX3D_REPO: &str = "Asphaltian/VPhysics-Box3D";
pub const JOLT_REPO: &str = "misyltoad/VPhysics-Jolt";
/// Jolt's last release (2024) predates GitHub's asset digests, so its GMod zips are pinned here.
// ponytail: only 0.20 is pinned; a newer Jolt release would carry GitHub digests and need none.
pub const JOLT_TAG: &str = "0.20";
const PINNED: &[(&str, &str)] = &[
    ("vphysics_jolt_0.20_gmod_win64.zip", "cdcefbb3ec9763041f5885fabdccb87b1b30a45f2622dfc0d3278b37bd624d0e"),
    ("vphysics_jolt_0.20_gmod_win32.zip", "bf64153fa0134223866cc3816f6b0ff172f6b5bdf343f9580b0edfab2b583e2c"),
    ("vphysics_jolt_0.20_gmod_linux64.zip", "47f2a2e26209d96464eae74182bb490b5d7200ca9ffe9a54f302c2db8d78f1c6"),
    ("vphysics_jolt_0.20_gmod_linux32.zip", "dd9035a659f802966f44008adac45528bb363ef135bad80410a35964b437d2c4"),
    ("gwater2_1.0.zip", "3aefdc5b3b52df4e76ba45a54c0f1c1af3e329ec84e7ad6d9e43670595397464"),
];
pub const PATCHTOOL_REPO: &str = "solsticegamestudios/GModPatchTool";
pub const SKATEGM_REPO: &str = "the-schwilliam/SkateGM";
pub const GWATER2_REPO: &str = "meetric1/gwater2";
/// GWater2's one release (the other is a zip of betas); its Windows zip has no GitHub digest, so it's pinned.
pub const GWATER2_TAG: &str = "v1.0";
/// GWater2's Lua (SWEPs, entities, menu) ships only on the Workshop; the release zips are its engine part.
const GWATER2_WORKSHOP: &str = "3403736992";
/// uv runs SkateGM's Python exporter with its own Python and packages, so users install nothing.
const UV_TAG: &str = "0.13.0";
/// The vgmstream release SkateGM's own installer bundles (its tools/fetch_vgmstream.py).
const VGMSTREAM_TAG: &str = "r2117";

fn github_json(path: &str) -> Result<serde_json::Value> {
    Ok(serde_json::from_reader(ureq::get(&format!("https://api.github.com/repos/{path}"))
        .header("User-Agent", "gmtoolkit").call()?.into_body().into_reader())?)
}

/// A release by tag, or the latest one.
fn github_release(repo: &str, tag: Option<&str>) -> Result<serde_json::Value> {
    github_json(&match tag { Some(t) => format!("{repo}/releases/tags/{t}"), None => format!("{repo}/releases/latest") })
        .with_context(|| format!("{repo}: release {} not found", tag.unwrap_or("latest")))
}

/// Release tags, newest first.
pub fn releases(repo: &str) -> Result<Vec<String>> {
    let list = github_json(&format!("{repo}/releases?per_page=20"))?;
    Ok(list.as_array().into_iter().flatten().filter(|r| r["draft"] != true)
        .filter_map(|r| r["tag_name"].as_str().map(str::to_owned)).collect())
}

/// Version (release tag or Steam build id) recorded at install, kept in `<data>/versions/<id>`.
pub fn installed_version(id: &str) -> Option<String> {
    // First line; versions/chromium also lists the patched files after it.
    fs::read_to_string(data_dir().ok()?.join("versions").join(id)).ok()?.lines().next().map(|s| s.trim().to_owned())
}

fn set_installed_version(id: &str, v: Option<&str>) -> Result<()> {
    let path = data_dir()?.join("versions").join(id);
    match v {
        Some(v) => { fs::create_dir_all(path.parent().unwrap())?; fs::write(path, v)?; }
        None => { let _ = fs::remove_file(path); }
    }
    Ok(())
}

/// Current public build ids of dedicated servers, in one anonymous steamcmd call.
pub fn latest_builds(appids: &[u32]) -> Result<std::collections::HashMap<u32, String>> {
    let steamcmd = ensure_steamcmd(&data_dir()?.join("steamcmd"))?;
    let mut cmd = Command::new(steamcmd);
    cmd.args(["+login", "anonymous", "+app_info_update", "1"]);
    for id in appids { cmd.args(["+app_info_print", &id.to_string()]); }
    hide_window(cmd.arg("+quit"));
    let out = String::from_utf8_lossy(&cmd.output()?.stdout).into_owned();
    Ok(appids.iter().filter_map(|id| Some((*id, parse_build(&out, *id)?))).collect())
}

/// Finds `<appid>` > depots > branches > public > buildid in app_info_print output.
fn parse_build(out: &str, appid: u32) -> Option<String> {
    // Each app's VDF follows its "AppID : <id>," banner (and terminal color codes) up to the next banner.
    let after = out.split(&format!("AppID : {appid},")).nth(1)?.split("AppID :").next()?;
    let block = &after[after.find(&format!("\"{appid}\""))?..];
    let block = &block[..=block.rfind('}')?];
    vdf_get(block, &["depots", "branches", "public", "buildid"])
}

fn github_asset(rel: &serde_json::Value, name: &str) -> Result<Vec<u8>> {
    let asset = rel["assets"].as_array().into_iter().flatten().find(|a| a["name"] == name)
        .with_context(|| format!("release {} has no {name}", rel["tag_name"]))?;
    let url = asset["browser_download_url"].as_str().context("no download url")?;
    let want = asset["digest"].as_str().and_then(|d| d.strip_prefix("sha256:"))
        .or_else(|| PINNED.iter().find(|(n, _)| *n == name).map(|(_, h)| *h))
        .context("release asset has no sha256 digest")?;
    let bytes = ureq::get(url).call()?.into_body().with_config().limit(1 << 30).read_to_vec()?;
    let got: String = sha2::Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
    if got != want { bail!("checksum mismatch for {name}"); }
    Ok(bytes)
}

/// Swaps GMod's physics for VPhysics-Box3D (`vphysics`) or VPhysics-Jolt (`jolt`) in every binary dir that has
/// a vphysics, keeping `.orig` backups. Both engines share the backups, so installing one removes the other.
fn set_vphysics(engine: &str, on: bool, tag: Option<&str>) -> Result<()> {
    if std::env::consts::OS == "macos" { bail!("{engine} has no macOS build"); }
    let gmod = gmod_dir()?;
    let bin = gmod.join("bin");
    // Back to stock first, so an update or a switch of engine never backs up a replaced file.
    let restored = if on && !vphysics_swapped(&gmod) { 0 } else { restore_vphysics(&bin)? };
    if !on {
        if restored == 0 { bail!("no vphysics backup to restore"); }
        for e in ["vphysics", "jolt"] { set_installed_version(e, None)?; }
        println!("restored {restored} vphysics file(s)");
        return Ok(());
    }
    let (repo, tag, name) = match engine {
        "jolt" => (JOLT_REPO, Some(tag.unwrap_or(JOLT_TAG)), "VPhysics-Jolt"),
        _ => (BOX3D_REPO, tag, "VPhysics-Box3D"),
    };
    let release = github_release(repo, tag)?;
    let tag = release["tag_name"].as_str().context("release has no tag")?;
    // (dir inside bin, stock file, Box3D asset, Jolt platform). Proton installs use the Windows dlls on Linux too.
    let targets = [
        ("win64", "vphysics.dll", "gmod-x64.zip", "win64"),
        ("", "vphysics.dll", "gmod-x86.zip", "win32"),
        ("linux64", "vphysics.so", "gmod-linux-x64.zip", "linux64"),
        ("", "vphysics.so", "gmod-linux-x86.zip", "linux32"),
    ];
    let mut n = 0;
    for (dir, file, box3d, jolt) in targets {
        let dir = bin.join(dir);
        if !dir.join(file).exists() { continue; }
        let asset = if engine == "jolt" { format!("vphysics_jolt_{tag}_gmod_{jolt}.zip") } else { box3d.to_owned() };
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(github_asset(&release, &asset)?))?;
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i)?;
            let Some(fname) = entry.enclosed_name().and_then(|p| Some(p.file_name()?.to_str()?.to_owned())) else { continue };
            // Replaces GMod's own files (vphysics, vphysics_client); adds Jolt's per-CPU builds its loader picks from.
            let extra = fname.starts_with("vphysics_jolt_");
            let dst = dir.join(&fname);
            if entry.is_dir() || fname.ends_with(".dbg") || fname.ends_with(".pdb") || !(extra || dst.exists()) { continue; }
            if !extra { fs::copy(&dst, orig(&dst))?; }
            std::io::copy(&mut entry, &mut fs::File::create(&dst)?)?;
        }
        n += 1;
    }
    if n == 0 { bail!("no vphysics files found to replace"); }
    set_installed_version(engine, Some(tag))?;
    set_installed_version(if engine == "jolt" { "vphysics" } else { "jolt" }, None)?;
    println!("installed {name} over {n} vphysics file(s)");
    Ok(())
}

/// Puts back every `vphysics*.orig` backup and deletes Jolt's extra files; returns how many were restored.
fn restore_vphysics(bin: &Path) -> Result<usize> {
    let mut n = 0;
    for dir in [bin.to_owned(), bin.join("win64"), bin.join("linux64")] {
        let Ok(entries) = fs::read_dir(&dir) else { continue };
        for path in entries.filter_map(|e| Some(e.ok()?.path())) {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if let Some(stock) = name.strip_suffix(".orig").filter(|s| s.starts_with("vphysics")) {
                fs::rename(&path, dir.join(stock))?;
                n += 1;
            } else if name.starts_with("vphysics_jolt_") {
                fs::remove_file(&path)?;
            }
        }
    }
    Ok(n)
}

/// Downloads the latest GModPatchTool and runs it with inherited stdio.
fn run_patchtool(tag: Option<&str>) -> Result<()> {
    // The patches target the x86-64 branch's files; MountedConfig confirms Steam finished switching.
    if current_branch(true)? != "x86-64" {
        bail!("GModPatchTool needs the 64-bit branch installed: switch to it and let Steam update GMod first");
    }
    let os = match std::env::consts::OS { "windows" => "Windows", "macos" => "macOS", _ => "Linux" };
    let release = github_release(PATCHTOOL_REPO, tag)?;
    let zip = github_asset(&release, &format!("GModPatchTool-{os}.zip"))?;
    let dir = data_dir()?.join("gmodpatchtool");
    let _ = fs::remove_dir_all(&dir);
    zip::ZipArchive::new(std::io::Cursor::new(zip))?.extract(&dir)?;
    let exe = if os == "macOS" {
        // ponytail: untested, mounts the dmg and runs the app binary inside; verify on a real Mac
        let dmg = fs::read_dir(&dir)?.filter_map(|e| e.ok().map(|e| e.path()))
            .find(|p| p.extension().is_some_and(|x| x == "dmg")).context("no dmg in zip")?;
        let mnt = dir.join("mnt");
        let ok = Command::new("hdiutil").args(["attach", "-nobrowse", "-readonly", "-mountpoint"])
            .arg(&mnt).arg(&dmg).status()?.success();
        if !ok { bail!("could not mount {}", dmg.display()); }
        let app = fs::read_dir(&mnt)?.filter_map(|e| e.ok().map(|e| e.path()))
            .find(|p| p.extension().is_some_and(|x| x == "app")).context("no .app in dmg")?;
        let bin = fs::read_dir(app.join("Contents/MacOS"))?.next().context("empty .app")??.path();
        let patched = run_headless(&bin);
        let _ = Command::new("hdiutil").arg("detach").arg(&mnt).status();
        return record_patch(&release, &patched?);
    } else if os == "Windows" { dir.join("gmodpatchtool.exe") } else { dir.join("gmodpatchtool") };
    #[cfg(unix)]
    fs::set_permissions(&exe, std::os::unix::fs::PermissionsExt::from_mode(0o755))?;
    record_patch(&release, &run_headless(&exe)?)
}

/// Saves the release tag, then the patched files with their mtimes, to `versions/chromium`.
fn record_patch(release: &serde_json::Value, patched: &str) -> Result<()> {
    set_installed_version("chromium", Some(&format!("{}\n{patched}", release["tag_name"].as_str().unwrap_or("unknown"))))
}

/// Runs GModPatchTool without its own window: FORCE_GUI=0 keeps it in CLI mode, and we relay its log.
/// Returns the files it patched, one `<path>\t<mtime>` per line.
fn run_headless(exe: &Path) -> Result<String> {
    use std::io::BufRead;
    use std::process::Stdio;
    let mut cmd = Command::new(exe);
    cmd.arg("-s").env("FORCE_GUI", "0").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::inherit());
    hide_window(&mut cmd);
    let mut child = cmd.spawn()?;
    let gmod = gmod_dir()?;
    let mut patched = String::new();
    for line in std::io::BufReader::new(child.stdout.take().unwrap()).lines() {
        let line = line?;
        println!("[patch] {line}");
        // It lists each patched file as "\t<path relative to GarrysMod>"; remember them with their mtimes.
        let rel = line.trim();
        if line.starts_with('\t') && let Some(t) = mtime(&gmod.join(rel)) { patched.push_str(&format!("{rel}\t{t}\n")); }
    }
    if !child.wait()?.success() { bail!("GModPatchTool failed"); }
    Ok(patched)
}

fn mtime(p: &Path) -> Option<u128> {
    Some(fs::metadata(p).ok()?.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_nanos())
}

/// Patched if every file GModPatchTool last reported is untouched since (Steam updates/verify rewrite them).
fn chromium_patched(gmod: &Path) -> bool {
    let Ok(record) = data_dir().and_then(|d| Ok(fs::read_to_string(d.join("versions/chromium"))?)) else { return false };
    let mut files = record.lines().skip(1).peekable();
    files.peek().is_some() && files.all(|l| l.split_once('\t').is_some_and(|(rel, t)| mtime(&gmod.join(rel)).map(|m| m.to_string()).as_deref() == Some(t)))
}

/// Undoes the patches by having Steam verify GMod's files, which restores the stock ones.
fn unpatch_chromium() -> Result<()> {
    open_url("steam://validate/4000")?;
    set_installed_version("chromium", None)?;
    println!("Steam is verifying Garry's Mod, which restores the unpatched files.");
    println!("Note: this also restores the stock vphysics if VPhysics-Box3D or VPhysics-Jolt was installed.");
    Ok(())
}

/// Opens a URL (here steam:// ones, handled by Steam) with the desktop's default handler.
fn open_url(url: &str) -> Result<()> {
    let mut cmd = match std::env::consts::OS {
        "windows" => Command::new("explorer"),
        "macos" => Command::new("open"),
        _ => Command::new("xdg-open"),
    };
    hide_window(cmd.arg(url));
    cmd.status()?;
    Ok(())
}

/// Deletes a file GMod may hold open; already gone is fine.
fn delete(p: &Path) -> Result<()> {
    match fs::remove_file(p) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => bail!("couldn't remove {} ({e}): close Garry's Mod and try again", p.display()),
        _ => Ok(()),
    }
}

/// SkateGM's engine module is Windows-only, so on Linux GMod has to run under Proton.
pub fn skategm_supported() -> bool {
    cfg!(windows) || gmod_uses_wine()
}

/// Installs SkateGM from its release zip: `skategm.py` converts the user's own Skate 3 disc (.iso or
/// default.xex) with the bundled exporter and copies the module and add-on in. Without a disc, the one
/// used last time is converted again if it's still there, else the converted data is kept.
fn install_skategm(tag: Option<&str>, disc: Option<&str>) -> Result<()> {
    if !skategm_supported() { bail!("SkateGM's module is Windows-only: on Linux, GMod has to run under Proton"); }
    // The module is 64-bit; MountedConfig confirms Steam finished switching.
    if current_branch(true)? != "x86-64" { bail!("SkateGM needs the 64-bit branch installed: switch to it and let Steam update GMod first"); }
    let gmod = gmod_dir()?;
    let dir = data_dir()?.join("skategm");
    fs::create_dir_all(&dir)?;
    let remembered = dir.join("disc.txt");
    if let Some(d) = disc { fs::write(&remembered, d)?; }
    let disc = fs::read_to_string(&remembered).ok().filter(|d| Path::new(d).is_file());
    let release = github_release(SKATEGM_REPO, tag)?;
    let zip = release["assets"].as_array().into_iter().flatten().filter_map(|a| a["name"].as_str())
        .find(|n| n.starts_with("skategm_") && n.ends_with(".zip")).context("release has no skategm zip")?;
    let unpacked = dir.join("release");
    let _ = fs::remove_dir_all(&unpacked);
    zip::ZipArchive::new(std::io::Cursor::new(github_asset(&release, zip)?))?.extract(&unpacked)?;
    let (uv_asset, uv_exe, vgm_asset, vgm_exe) = if cfg!(windows) {
        ("uv-x86_64-pc-windows-msvc.zip", "uv.exe", "vgmstream-win64.zip", "vgmstream-cli.exe")
    } else {
        ("uv-x86_64-unknown-linux-gnu.tar.gz", "uv-x86_64-unknown-linux-gnu/uv", "vgmstream-linux.zip", "vgmstream-cli")
    };
    let uv = github_tool("astral-sh/uv", UV_TAG, uv_asset, &dir.join(format!("uv-{UV_TAG}")), uv_exe)?;
    let vgmstream = github_tool("vgmstream/vgmstream", VGMSTREAM_TAG, vgm_asset, &dir.join(format!("vgmstream-{VGMSTREAM_TAG}")), vgm_exe)?;
    let script = dir.join("install.py");
    fs::write(&script, include_str!("skategm.py"))?;
    let data = dir.join("data");
    let mut cmd = Command::new(uv);
    cmd.env("UV_CACHE_DIR", dir.join("uv-cache")).env("UV_PYTHON_INSTALL_DIR", dir.join("python"))
        .args(["run", "--no-project", "--python", "3.13", "--with", "numpy", "--with", "pillow", "--with", "soundfile", "python", "-I"])
        .arg(&script).arg(&unpacked).arg(&gmod).arg(&data).arg(mount_path(&data.join("assets"))).arg(&vgmstream);
    if let Some(d) = &disc { cmd.arg(d); }
    hide_window(&mut cmd);
    if !cmd.status()?.success() { bail!("SkateGM install failed"); }
    set_installed_version("skategm", release["tag_name"].as_str())
}

/// Removes what SkateGM's own uninstaller does, plus the converted data and downloads.
fn remove_skategm() -> Result<()> {
    let g = gmod_dir()?.join("garrysmod");
    for f in ["lua/bin/gmcl_skategm_win64.dll", "lua/bin/skategm_sdl2.dll", "lua/bin/skategm_gamecontrollerdb.txt",
              "data/skategm/datapath.txt", "data/skategm/skate3_sounds.json", "data/skategm/skate3_music.json"] {
        delete(&g.join(f))?;
    }
    for d in ["addons/skategm", "addons/skategm_maps", "addons/skategm_s3sounds", "addons/skategm_s3music", "data/skategm_hud"] {
        let _ = fs::remove_dir_all(g.join(d));
    }
    let _ = fs::remove_dir_all(data_dir()?.join("skategm"));
    set_installed_version("skategm", None)?;
    println!("SkateGM removed (your settings, boards and replays in data/skategm are kept).");
    Ok(())
}

/// The NVIDIA driver's CUDA library, which GWater2 needs for native Linux GMod (none with AMD/Intel).
fn libcuda() -> Option<PathBuf> {
    // Debian/Ubuntu, Fedora, Arch (where lib64 links to lib), NixOS; 64-bit dirs first, Fedora's /usr/lib is 32-bit.
    ["/usr/lib/x86_64-linux-gnu", "/usr/lib64", "/usr/lib", "/run/opengl-driver/lib"].iter()
        .map(|d| Path::new(d).join("libcuda.so.1")).find(|p| p.exists())
}

/// Unpacks GWater2's release zip (module, NVIDIA FleX dlls, shaders) into the GMod folder like its own
/// installer, and opens its Workshop page if the Lua part isn't subscribed. Natively on Linux the Linux zip
/// is used and libcuda is copied to `bin/linux64`, as its instructions say; Proton uses the Windows zip.
/// The files are listed after the tag in `versions/gwater2`, for updates and uninstall.
fn install_gwater2(tag: Option<&str>) -> Result<()> {
    if cfg!(target_os = "macos") { bail!("GWater2 has no macOS build"); }
    let native_linux = cfg!(target_os = "linux") && !gmod_uses_wine();
    let cuda = if native_linux {
        if current_branch(true)? != "x86-64" { bail!("GWater2 on Linux needs the 64-bit branch installed: switch to it and let Steam update GMod first"); }
        Some(libcuda().context("GWater2 on native Linux needs an NVIDIA GPU and driver (no libcuda.so.1 found); with AMD or Intel, run GMod under Proton")?)
    } else { None };
    let gmod = gmod_dir()?;
    let release = github_release(GWATER2_REPO, Some(tag.unwrap_or(GWATER2_TAG)))?;
    let tag = release["tag_name"].as_str().context("release has no tag")?;
    let ver = tag.trim_start_matches('v');
    let asset = if native_linux { format!("gwater2_{ver}_LINUX.zip") } else { format!("gwater2_{ver}.zip") };
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(github_asset(&release, &asset)?))?;
    remove_gwater2_files(&gmod)?;
    zip.extract(&gmod).context("couldn't unpack GWater2: close Garry's Mod and try again")?;
    let mut files: Vec<String> = zip.file_names().collect::<Result<Vec<_>, _>>()?
        .into_iter().filter(|n| !n.ends_with('/')).map(|n| n.into_owned()).collect();
    if let Some(cuda) = cuda {
        // ponytail: a copy of the current driver's lib; reinstall GWater2 after an NVIDIA driver update
        fs::copy(&cuda, gmod.join("bin/linux64/libcuda.so")).with_context(|| format!("copying {}", cuda.display()))?;
        files.push("bin/linux64/libcuda.so".into());
    }
    set_installed_version("gwater2", Some(&format!("{tag}\n{}", files.join("\n"))))?;
    println!("GWater2 {tag}: installed {} files", files.len());
    if gmod.join("garrysmod/addons/gwater2").exists() {
        println!("Note: garrysmod/addons/gwater2 (an older copy) clashes with the Workshop version; remove it unless you're working on GWater2");
    }
    if !manifest_path()?.with_file_name(format!("workshop/content/{GMOD_APPID}/{GWATER2_WORKSHOP}")).exists() {
        open_url(&format!("steam://url/CommunityFilePage/{GWATER2_WORKSHOP}"))?;
        println!("Subscribe to GWater2 on the Workshop page that just opened: its add-on has the water guns, entities and menu.");
    }
    Ok(())
}

/// Deletes the files the last GWater2 install recorded.
fn remove_gwater2_files(gmod: &Path) -> Result<()> {
    let record = data_dir().and_then(|d| Ok(fs::read_to_string(d.join("versions/gwater2"))?)).unwrap_or_default();
    for rel in record.lines().skip(1) { delete(&gmod.join(rel))?; }
    Ok(())
}

fn remove_gwater2() -> Result<()> {
    remove_gwater2_files(&gmod_dir()?)?;
    set_installed_version("gwater2", None)?;
    println!("GWater2 removed. Unsubscribe from it on the Workshop to remove its add-on too.");
    Ok(())
}

/// A pinned GitHub release asset, downloaded and unpacked into `dir` once; returns `dir/<exe>`.
fn github_tool(repo: &str, tag: &str, asset: &str, dir: &Path, exe: &str) -> Result<PathBuf> {
    let path = dir.join(exe);
    if path.exists() { return Ok(path); }
    let bytes = github_asset(&github_release(repo, Some(tag))?, asset)?;
    fs::create_dir_all(dir)?;
    if asset.ends_with(".zip") {
        zip::ZipArchive::new(std::io::Cursor::new(bytes))?.extract(dir)?;
    } else {
        let archive = dir.join(asset);
        fs::write(&archive, bytes)?;
        let ok = Command::new("tar").arg("xzf").arg(&archive).arg("-C").arg(dir).status()?.success();
        fs::remove_file(&archive)?;
        if !ok { bail!("couldn't unpack {asset}"); }
    }
    #[cfg(unix)]
    fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o755))?;
    Ok(path)
}

fn ensure_steamcmd(dir: &Path) -> Result<PathBuf> {
    let (url, exe) = match std::env::consts::OS {
        "windows" => ("https://steamcdn-a.akamaihd.net/client/installer/steamcmd.zip", "steamcmd.exe"),
        "macos" => ("https://steamcdn-a.akamaihd.net/client/installer/steamcmd_osx.tar.gz", "steamcmd.sh"),
        _ => ("https://steamcdn-a.akamaihd.net/client/installer/steamcmd_linux.tar.gz", "steamcmd.sh"),
    };
    let exe = dir.join(exe);
    if exe.exists() { return Ok(exe); }
    fs::create_dir_all(dir)?;
    let bytes = ureq::get(url).call()?.into_body().with_config().limit(1 << 30).read_to_vec()?;
    if url.ends_with(".zip") {
        zip::ZipArchive::new(std::io::Cursor::new(bytes))?.extract(dir)?;
    } else {
        // Linux and macOS both ship tar.
        let archive = dir.join("steamcmd.tar.gz");
        fs::write(&archive, bytes)?;
        let ok = Command::new("tar").arg("xzf").arg(&archive).arg("-C").arg(dir).status()?.success();
        fs::remove_file(&archive)?;
        if !ok { bail!("couldn't unpack steamcmd"); }
    }
    Ok(exe)
}

#[test]
fn build_ids() {
    let out = "junk\nAppID : 232330, change number : 1\n\"232330\"\n{\n\"depots\"\n{\n\"branches\"\n{\n\"public\"\n{\n\"buildid\"\t\t\"25338297\"\n}\n}\n}\n}\nAppID : 232290, change number : 2\n\"232290\"\n{\n\"depots\"\n{\n\"branches\"\n{\n\"public\"\n{\n\"buildid\"\t\t\"6953255\"\n}\n}\n}\n}\n";
    assert_eq!(parse_build(out, 232330).as_deref(), Some("25338297"));
    assert_eq!(parse_build(out, 232290).as_deref(), Some("6953255"));
    assert_eq!(parse_build(out, 1), None);
}

#[test]
fn wanted_files() {
    let (css, tf) = (&GAMES[0], &GAMES[1]);
    assert!(mount_entries("\"mountcfg\"\n{\n//\t\"a\"\t\"x\"\n\t\"tf\"\t\"Z:/t f\"\n}\n") == vec![("tf".to_owned(), "Z:/t f".to_owned())]);
    assert_eq!(mount_name(&GAMES[4], Path::new("/x/left4dead2_dlc1")), "left4dead2_dlc1");
    assert_eq!(mount_name(&GAMES[4], Path::new("/x/update")), "left4dead2_update");
    assert!(wanted("maps/de_dust2.bsp", css));
    assert!(!wanted("maps/de_dust2.txt", css));
    assert!(!wanted("cstrike_pak_dir.vpk", css));
    assert!(wanted("Materials\\x.vtf", tf));
    assert!(!wanted("scripts/x.txt", tf));
    assert!(wanted("tf2_misc_dir.vpk", tf) && wanted("tf2_misc_017.vpk", tf));
    assert!(!wanted("bin/server.dll", tf) && !wanted("gameinfo.txt", tf));
}

#[test]
fn mount_cfg_edits() {
    let base = "\"mountcfg\"\r\n{\r\n//\t\"cstrike\"\t\"C:\\x\"\r\n\t\"tf\"\t\"/tf\"\r\n}\r\n";
    let out = with_mount(base, "cstrike", Some("/c/cstrike")).unwrap();
    assert_eq!(out, "\"mountcfg\"\r\n{\r\n//\t\"cstrike\"\t\"C:\\x\"\r\n\t\"tf\"\t\"/tf\"\r\n\t\"cstrike\"\t\"/c/cstrike\"\r\n}\r\n");
    assert_eq!(with_mount(&out, "cstrike", Some("/c/cstrike")).unwrap(), out);
    assert_eq!(with_mount(&out, "cstrike", None).unwrap(), base);
    assert_eq!(mount_entry(&out, "cstrike").as_deref(), Some("/c/cstrike"));
    assert_eq!(mount_entry(base, "cstrike"), None);
    if cfg!(target_os = "linux") { assert_eq!(host_path("Z:/home/x/cstrike"), PathBuf::from("/home/x/cstrike")); }
}

#[test]
#[ignore] // run with --ignored: round-trips this machine's real Steam files
fn vdf_roundtrip_real() {
    let steam = steam().unwrap();
    for f in ["config/config.vdf", "steamapps/appmanifest_4000.acf"] {
        let text = fs::read_to_string(steam.path().join(f)).unwrap();
        let out = vdf_edit(&text, |_| {}).unwrap();
        let norm = |s: &str| { let mut v: Vec<_> = s.lines().map(str::trim).filter(|l| !l.is_empty()).map(|l| l.split_whitespace().collect::<Vec<_>>().join(" ")).collect(); v.sort(); v };
        let (a, b) = (norm(&text), norm(&out));
        let diff: Vec<_> = a.iter().filter(|l| !b.contains(l)).chain(b.iter().filter(|l| !a.contains(l))).collect();
        assert!(diff.is_empty(), "{f}: {diff:?}");
    }
}

#[test]
fn proton_ids() {
    assert_eq!(valve_proton_id("9.0"), "proton_9");
    assert_eq!(valve_proton_id("5.13"), "proton_513");
    assert_eq!(valve_proton_id("Hotfix"), "proton_hotfix");
}

#[test]
fn vdf_edits() {
    let acf = "\"AppState\"\n{\n\t\"appid\"\t\"4000\"\n\t\"UserConfig\"\n\t{\n\t\t\"betakey\"\t\"old\"\n\t}\n}\n";
    let out = vdf_edit(acf, |r| set(child(r, "UserConfig"), "BetaKey", "x86-64")).unwrap();
    assert!(out.contains("\"BetaKey\"\t\t\"x86-64\"") && !out.contains("old") && out.contains("4000"), "{out}");
    let cfg = vdf_edit("\"InstallConfigStore\"\n{\n}\n", |r| {
        set(child(child(r, "Software"), "valve"), "x", "y");
    }).unwrap();
    assert!(cfg.contains("\"valve\"") && cfg.contains("\"x\""), "{cfg}");
    assert_eq!(vdf_get(&cfg, &["software", "Valve", "X"]).as_deref(), Some("y"));
}

#[test]
#[ignore] // run with --ignored: lists this machine's compatibility tools
fn compat_tools_real() {
    let tools = compat_tools();
    println!("{tools:?}");
    assert_eq!(tools[0].0, "proton_experimental");
}

#[test]
#[ignore] // run with --ignored: asks Steam (via steamcmd) for the current server builds
fn latest_builds_real() {
    let ids: Vec<u32> = GAMES.iter().map(|g| g.server_appid).collect();
    let builds = latest_builds(&ids).unwrap();
    println!("{builds:?}");
    assert_eq!(builds.len(), ids.len());
    assert!(!releases(BOX3D_REPO).unwrap().is_empty());
}

#[test]
fn restore_vphysics_puts_stock_back() {
    let bin = std::env::temp_dir().join(format!("gmtoolkit-vphysics-{}", std::process::id()));
    fs::create_dir_all(bin.join("win64")).unwrap();
    for (f, c) in [("win64/vphysics.dll", "jolt"), ("win64/vphysics.dll.orig", "stock"), ("win64/vphysics_jolt_avx2.dll", "x"), ("vphysics_stub.dll", "stub")] {
        fs::write(bin.join(f), c).unwrap();
    }
    assert_eq!(restore_vphysics(&bin).unwrap(), 1);
    assert_eq!(fs::read_to_string(bin.join("win64/vphysics.dll")).unwrap(), "stock");
    assert!(!bin.join("win64/vphysics_jolt_avx2.dll").exists() && !bin.join("win64/vphysics.dll.orig").exists());
    assert!(bin.join("vphysics_stub.dll").exists());
    fs::remove_dir_all(bin).unwrap();
}
