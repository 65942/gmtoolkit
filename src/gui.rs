use eframe::egui::{self, Color32, CornerRadius, Margin, RichText, Stroke};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

const DEFAULT_ACCENT: u32 = 0x588cff;
/// Desktop accent color as 0xRRGGBB, updated by the theme watcher.
static ACCENT: AtomicU32 = AtomicU32::new(DEFAULT_ACCENT);

fn accent() -> Color32 {
    let c = ACCENT.load(Ordering::Relaxed);
    Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

/// Black or white, whichever reads better on the accent.
fn on_accent() -> Color32 {
    let a = accent();
    let lum = 0.299 * a.r() as f32 + 0.587 * a.g() as f32 + 0.114 * a.b() as f32;
    if lum > 150.0 { Color32::from_rgb(15, 17, 22) } else { Color32::WHITE }
}

/// Colors for the current theme.
#[derive(Clone, PartialEq)]
struct Pal {
    dark: bool, bg: Color32, card: Color32, stroke: Color32, button: Color32, button_hover: Color32,
    muted: Color32, title: Color32, good: Color32, danger: Color32, log_bg: Color32, log_text: Color32,
}

/// The KDE color scheme, when running under KDE; replaces the built-in palette.
static KDE: Mutex<Option<Pal>> = Mutex::new(None);

fn builtin(dark: bool) -> Pal {
    let c = Color32::from_rgb;
    if dark {
        Pal { dark, bg: c(16, 18, 23), card: c(25, 28, 35), stroke: c(36, 40, 50), button: c(38, 42, 52), button_hover: c(50, 55, 68),
              muted: c(136, 143, 156), title: c(240, 242, 246), good: c(80, 200, 120), danger: c(240, 110, 110),
              log_bg: c(11, 12, 16), log_text: c(186, 192, 204) }
    } else {
        Pal { dark, bg: c(243, 245, 249), card: Color32::WHITE, stroke: c(224, 228, 236), button: c(234, 237, 243), button_hover: c(222, 226, 235),
              muted: c(98, 106, 120), title: c(20, 22, 28), good: c(28, 145, 78), danger: c(200, 50, 50),
              log_bg: c(234, 237, 243), log_text: c(50, 56, 68) }
    }
}

fn pal(ui: &egui::Ui) -> Pal {
    KDE.lock().unwrap().clone().unwrap_or_else(|| builtin(ui.visuals().dark_mode))
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t) as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

/// Reads `key=R,G,B` from `[Colors:<section>]` of kdeglobals.
fn kde_color(ini: &str, section: &str, key: &str) -> Option<Color32> {
    let header = format!("[Colors:{section}]");
    let body = ini.split(&header).nth(1)?.split("\n[").next()?;
    let val = body.lines().find_map(|l| l.strip_prefix(key)?.strip_prefix('='))?;
    let v: Vec<u8> = val.split(',').filter_map(|n| n.trim().parse().ok()).collect();
    (v.len() >= 3).then(|| Color32::from_rgb(v[0], v[1], v[2]))
}

/// Builds the palette (and accent) from KDE's active color scheme.
fn kde_palette(ini: &str) -> Option<(Pal, Color32)> {
    let get = |s, k| kde_color(ini, s, k);
    let (bg, text) = (get("Window", "BackgroundNormal")?, get("Window", "ForegroundNormal")?);
    let card = get("View", "BackgroundNormal").unwrap_or(bg);
    let button = get("Button", "BackgroundNormal").unwrap_or(card);
    let lum = |c: Color32| 0.299 * c.r() as f32 + 0.587 * c.g() as f32 + 0.114 * c.b() as f32;
    let pal = Pal {
        dark: lum(bg) < 128.0, bg, card, stroke: mix(card, text, 0.10), button, button_hover: mix(button, text, 0.12),
        // KDE's ForegroundInactive is often too faint for small text, so blend instead.
        muted: mix(text, card, 0.38), title: text,
        good: get("View", "ForegroundPositive").unwrap_or(Color32::from_rgb(80, 200, 120)),
        danger: get("View", "ForegroundNegative").unwrap_or(Color32::from_rgb(240, 110, 110)),
        log_bg: get("Window", "BackgroundAlternate").unwrap_or(mix(bg, Color32::BLACK, 0.2)), log_text: mix(text, card, 0.15),
    };
    let accent = get("Selection", "BackgroundNormal").or(get("View", "DecorationFocus")).unwrap_or(accent());
    Some((pal, accent))
}

/// Corner radii that match the desktop's widget style: (controls, cards).
fn radii() -> (u8, u8) {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().to_lowercase();
    match std::env::consts::OS {
        "windows" => (4, 8),       // Windows 11
        "macos" => (6, 10),
        _ if desktop.contains("kde") => (3, 5), // Breeze
        _ if desktop.contains("gnome") => (6, 12), // Adwaita
        _ => (4, 8),
    }
}

/// UI font size in points, from the desktop (set by `load_system_fonts`).
static FONT_PT: AtomicU32 = AtomicU32::new(0);

fn base_px() -> f32 {
    match f32::from_bits(FONT_PT.load(Ordering::Relaxed)) {
        pt if pt > 0.0 => pt * 4.0 / 3.0, // points to pixels at 96 dpi; egui applies display scaling
        _ => 13.5,
    }
}

/// The desktop's UI (or monospace) font as (family, size in points).
fn desktop_font(mono: bool) -> Option<(String, f32)> {
    let parse_size = |s: &str| s.trim().parse::<f32>().ok();
    if cfg!(target_os = "linux") {
        // KDE: kdeglobals `font=Noto Sans,10,...` / `fixed=Hack,10,...`; unset means Plasma's defaults.
        if std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().to_lowercase().contains("kde") {
            let ini = dirs::config_dir().and_then(|d| std::fs::read_to_string(d.join("kdeglobals")).ok()).unwrap_or_default();
            let key = if mono { "fixed=" } else { "font=" };
            let found = ini.lines().find_map(|l| l.strip_prefix(key)).and_then(|v| {
                let mut parts = v.split(',');
                Some((parts.next()?.to_owned(), parse_size(parts.next()?)?))
            });
            return Some(found.unwrap_or_else(|| (if mono { "Hack" } else { "Noto Sans" }.to_owned(), 10.0)));
        }
        // GNOME and others that use its settings: 'Cantarell 11'.
        let key = if mono { "monospace-font-name" } else { "font-name" };
        if let Ok(out) = Command::new("gsettings").args(["get", "org.gnome.desktop.interface", key]).output() {
            let v = String::from_utf8_lossy(&out.stdout).trim().trim_matches('\'').to_owned();
            if let Some((family, size)) = v.rsplit_once(' ').and_then(|(f, s)| Some((f.trim().to_owned(), parse_size(s)?))) {
                return Some((family, size));
            }
        }
        return Some((if mono { "monospace" } else { "sans-serif" }.to_owned(), 10.0));
    }
    // Windows: Segoe UI 9pt / Consolas; macOS: the system font at 13px (~10pt).
    Some(match (std::env::consts::OS, mono) {
        ("windows", false) => ("Segoe UI".into(), 9.0),
        ("windows", true) => ("Consolas".into(), 9.0),
        (_, false) => ("SF".into(), 10.0),
        (_, true) => ("SF Mono".into(), 10.0),
    })
}

/// The font file for a family: fontconfig on Linux, known system paths elsewhere.
fn font_file(family: &str) -> Option<Vec<u8>> {
    let path = match std::env::consts::OS {
        "windows" => {
            let dir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
            let file = if family == "Consolas" { "consola.ttf" } else { "segoeui.ttf" };
            format!("{dir}\\Fonts\\{file}")
        }
        "macos" => if family.contains("Mono") { "/System/Library/Fonts/SFNSMono.ttf" } else { "/System/Library/Fonts/SFNS.ttf" }.to_owned(),
        _ => {
            let out = Command::new("fc-match").args(["-f", "%{file}", family]).output().ok()?;
            String::from_utf8(out.stdout).ok()?
        }
    };
    std::fs::read(path.trim()).ok()
}

/// Uses the desktop's fonts (egui's bundled ones stay as fallbacks for missing glyphs).
fn load_system_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    for (mono, family) in [(false, egui::FontFamily::Proportional), (true, egui::FontFamily::Monospace)] {
        let Some((name, pt)) = desktop_font(mono) else { continue };
        let Some(data) = font_file(&name) else { continue };
        let key = format!("system-{name}");
        fonts.font_data.insert(key.clone(), Arc::new(egui::FontData::from_owned(data)));
        fonts.families.entry(family).or_default().insert(0, key);
        if !mono { FONT_PT.store(pt.to_bits(), Ordering::Relaxed); }
    }
    ctx.set_fonts(fonts);
}

/// Width of each row's button column (two buttons side by side).
const BUTTON_WIDTH: f32 = 92.0;
const ACTION_WIDTH: f32 = BUTTON_WIDTH * 2.0 + 10.0;

#[derive(Clone, Copy, PartialEq, Default)]
enum Tab { #[default] Content, Patches, Settings }

fn game_desc(id: &str) -> &'static str {
    if id == "cstrike" { "Maps only. GMod already includes the rest." } else { "Maps, models, materials and sounds" }
}

pub fn run() -> eframe::Result {
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([640.0, 720.0]).with_min_inner_size([520.0, 480.0]),
        ..Default::default()
    };
    let compat = crate::compat_tool();
    let mut app = App {
        branch: crate::current_branch(false).unwrap_or_else(|_| "x86-64".into()),
        tools: crate::compat_tools(),
        compat: compat.clone(),
        ..Default::default()
    };
    // Keep whatever is set now selectable even if we couldn't find it on disk.
    if !compat.is_empty() && !app.tools.iter().any(|t| t.0 == compat) { app.tools.push((compat.clone(), compat)); }
    app.setup = crate::saved_gmod_dir().is_none();
    app.path_input = crate::saved_gmod_dir().map(|p| p.display().to_string()).unwrap_or_default();
    app.refresh_status();
    let remote = app.remote.clone();
    eframe::run_native("GMToolKit", opts, Box::new(|cc| {
        load_system_fonts(&cc.egui_ctx);
        style(&cc.egui_ctx);
        watch_desktop_theme(cc.egui_ctx.clone());
        fetch_remote(remote, cc.egui_ctx.clone());
        Ok(Box::new(app))
    }))
}

/// Looks up the latest Steam builds and GitHub releases without blocking the UI.
fn fetch_remote(remote: Arc<Mutex<Remote>>, ctx: egui::Context) {
    // The VPhysics engines have no macOS build, so their rows (and release lists) only exist elsewhere.
    let mut repos = if cfg!(target_os = "macos") { vec![crate::PATCHTOOL_REPO] } else { vec![crate::BOX3D_REPO, crate::PATCHTOOL_REPO] };
    if crate::skategm_supported() { repos.push(crate::SKATEGM_REPO); }
    // Only Jolt's pinned release can be verified, so it's the only one offered.
    remote.lock().unwrap().releases.insert(crate::JOLT_REPO, vec![crate::JOLT_TAG.to_owned()]);
    remote.lock().unwrap().releases.insert(crate::GWATER2_REPO, vec![crate::GWATER2_TAG.to_owned()]);
    for repo in repos {
        let (remote, ctx) = (remote.clone(), ctx.clone());
        thread::spawn(move || if let Ok(tags) = crate::releases(repo) {
            remote.lock().unwrap().releases.insert(repo, tags);
            ctx.request_repaint();
        });
    }
    thread::spawn(move || {
        let ids: Vec<u32> = crate::GAMES.iter().map(|g| g.server_appid).collect();
        if let Ok(builds) = crate::latest_builds(&ids) {
            remote.lock().unwrap().builds = builds;
            ctx.request_repaint();
        }
    });
}

/// Follows the Linux desktop's dark/light setting and accent color through the XDG settings portal
/// (KDE, GNOME and others). Elsewhere, or if the portal isn't there, egui's own OS detection is used.
fn watch_desktop_theme(ctx: egui::Context) {
    if !cfg!(target_os = "linux") { return; }
    let kdeglobals = dirs::config_dir().map(|d| d.join("kdeglobals"));
    thread::spawn(move || loop {
        let kde = kdeglobals.as_ref().and_then(|p| std::fs::read_to_string(p).ok()).as_deref().and_then(kde_palette);
        let (theme, accent) = match &kde {
            Some((pal, acc)) => (
                if pal.dark { egui::ThemePreference::Dark } else { egui::ThemePreference::Light },
                (acc.r() as u32) << 16 | (acc.g() as u32) << 8 | acc.b() as u32,
            ),
            None => portal_theme(),
        };
        let kde = kde.map(|(pal, _)| pal);
        let changed = (ACCENT.swap(accent, Ordering::Relaxed) != accent)
            | (ctx.options(|o| o.theme_preference) != theme)
            | (*KDE.lock().unwrap() != kde);
        if changed {
            *KDE.lock().unwrap() = kde;
            ctx.set_theme(theme);
            style(&ctx);
            ctx.request_repaint();
        }
        thread::sleep(Duration::from_secs(3));
    });
}

/// Dark/light preference and accent from the XDG settings portal (GNOME and other non-KDE desktops).
fn portal_theme() -> (egui::ThemePreference, u32) {
    let read = |key: &str| -> Option<Vec<f64>> {
        // ponytail: shells out to busctl (systemd), add a D-Bus crate if non-systemd distros matter
        let out = Command::new("busctl").args(["--user", "call", "org.freedesktop.portal.Desktop",
            "/org/freedesktop/portal/desktop", "org.freedesktop.portal.Settings", "ReadOne", "ss", "org.freedesktop.appearance", key])
            .output().ok()?;
        // Output looks like `v u 1` or `v (ddd) 0.56 0.73 0.73`.
        let nums: Vec<f64> = String::from_utf8_lossy(&out.stdout).split_whitespace().filter_map(|t| t.parse().ok()).collect();
        (out.status.success() && !nums.is_empty()).then_some(nums)
    };
    let theme = match read("color-scheme").as_deref() {
        Some([1.0]) => egui::ThemePreference::Dark,
        Some([2.0]) => egui::ThemePreference::Light,
        _ => egui::ThemePreference::System,
    };
    // Components outside 0..1 mean "no accent set".
    let accent = match read("accent-color").as_deref() {
        Some(&[r, g, b]) if [r, g, b].iter().all(|c| (0.0..=1.0).contains(c)) =>
            ((r * 255.0) as u32) << 16 | ((g * 255.0) as u32) << 8 | (b * 255.0) as u32,
        _ => DEFAULT_ACCENT,
    };
    (theme, accent)
}

fn style(ctx: &egui::Context) {
    ctx.all_styles_mut(|s| {
        use egui::{FontId, TextStyle::*};
        let dark = s.visuals.dark_mode;
        let px = base_px();
        s.text_styles.insert(Heading, FontId::proportional(px * 1.6));
        s.text_styles.insert(Body, FontId::proportional(px));
        s.text_styles.insert(Button, FontId::proportional(px));
        s.text_styles.insert(Small, FontId::proportional(px * 0.88));
        s.text_styles.insert(Monospace, FontId::monospace(px * 0.92));
        s.spacing.button_padding = egui::vec2(px * 0.9, px * 0.42);
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.spacing.interact_size.y = px * 1.9;
        let (control, card) = radii();
        let p = KDE.lock().unwrap().clone().unwrap_or_else(|| builtin(dark));
        let v = &mut s.visuals;
        v.panel_fill = p.bg;
        v.window_fill = p.card;
        v.override_text_color = Some(p.title);
        v.extreme_bg_color = p.log_bg;
        v.selection.bg_fill = accent().gamma_multiply(0.85);
        v.selection.stroke = Stroke::new(1.0, on_accent());
        v.hyperlink_color = accent();
        v.window_stroke = Stroke::new(1.0, p.stroke);
        for w in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
            w.corner_radius = CornerRadius::same(control);
        }
        v.widgets.inactive.weak_bg_fill = p.button;
        v.widgets.inactive.bg_stroke = Stroke::NONE;
        v.widgets.hovered.weak_bg_fill = p.button_hover;
        v.widgets.hovered.bg_stroke = Stroke::NONE;
        v.widgets.active.weak_bg_fill = mix(p.button_hover, p.title, 0.1);
        v.widgets.open.weak_bg_fill = p.button_hover;
        v.window_corner_radius = CornerRadius::same(card);
        v.menu_corner_radius = CornerRadius::same(card);
    });
}

#[derive(Default)]
struct App {
    log: Arc<Mutex<String>>,
    busy: Arc<AtomicBool>,
    /// Row id of the running task, to show its spinner in place of its button; cleared (and install state
    /// re-read) once `busy` drops.
    running: Option<String>,
    tab: Tab,
    branch: String,
    tools: Vec<(String, String)>,
    compat: String,
    installed: HashMap<&'static str, bool>,
    /// Process id of the running task's child (its own process group on Unix), for Cancel.
    child: Arc<Mutex<Option<u32>>>,
    cancelled: Arc<AtomicBool>,
    /// Game downloads that were interrupted and can be resumed.
    partial: HashMap<&'static str, bool>,
    /// Games mounted from the user's own Steam install.
    steam_copy: HashMap<&'static str, bool>,
    /// Version recorded at install (release tag or Steam build id).
    versions: HashMap<&'static str, Option<String>>,
    /// Latest versions, fetched in the background at startup.
    remote: Arc<Mutex<Remote>>,
    /// Release picked in a row's version dropdown ("" = latest).
    selected: HashMap<String, String>,
    mounted_branch: String,
    show_log: bool,
    /// Action waiting for confirmation: (question, button, CLI args).
    confirm: Option<(String, &'static str, Vec<String>)>,
    /// No GMod folder chosen yet: show the first-launch screen.
    setup: bool,
    /// The GMod folder field (setup screen and Settings).
    path_input: String,
    /// Result of the last check/save of `path_input`: (ok, message).
    path_msg: Option<(bool, String)>,
    /// Folder picked in the Browse dialog, which runs on its own thread.
    picked_folder: Arc<Mutex<Option<std::path::PathBuf>>>,
    /// SkateGM install args, completed with the disc picked in its file dialog.
    picked_disc: Arc<Mutex<Option<Vec<String>>>>,
}

#[derive(Default)]
struct Remote {
    builds: HashMap<u32, String>,
    releases: HashMap<&'static str, Vec<String>>,
}

/// Where a row's versions come from.
#[derive(Clone, Copy)]
enum Source { Steam(u32), GitHub(&'static str) }

fn source(id: &str) -> Source {
    match id {
        "vphysics" => Source::GitHub(crate::BOX3D_REPO),
        "jolt" => Source::GitHub(crate::JOLT_REPO),
        "chromium" => Source::GitHub(crate::PATCHTOOL_REPO),
        "skategm" => Source::GitHub(crate::SKATEGM_REPO),
        "gwater2" => Source::GitHub(crate::GWATER2_REPO),
        _ => Source::Steam(crate::GAMES.iter().find(|g| g.id == id).map_or(0, |g| g.server_appid)),
    }
}

impl App {
    fn refresh_status(&mut self) {
        for line in crate::sync_steam_copies() { self.log.lock().unwrap().push_str(&format!("{line}\n")); }
        for g in crate::GAMES {
            self.steam_copy.insert(g.id, crate::uses_steam_copy(g.id));
            self.partial.insert(g.id, crate::is_partial(g.id));
        }
        for item in crate::GAMES.iter().map(|g| g.id).chain(["vphysics", "jolt", "chromium", "skategm", "gwater2"]) {
            self.installed.insert(item, crate::is_installed(item));
            self.versions.insert(item, crate::installed_version(item));
        }
        self.mounted_branch = crate::current_branch(true).unwrap_or_default();
    }

    /// Runs this same binary with CLI args in the background and streams its output into the log.
    fn start(&mut self, ctx: &egui::Context, args: &[&str]) {
        self.running = args.first().map(|s| s.to_string());
        self.busy.store(true, Ordering::SeqCst);
        self.cancelled.store(false, Ordering::SeqCst);
        let (log, busy, ctx) = (self.log.clone(), self.busy.clone(), ctx.clone());
        let (child_pid, cancelled) = (self.child.clone(), self.cancelled.clone());
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        log.lock().unwrap().push_str(&format!("> {}\n", args.join(" ")));
        thread::spawn(move || {
            let push = |s: &str| { log.lock().unwrap().push_str(s); ctx.request_repaint(); };
            let result = (|| -> std::io::Result<bool> {
                let mut cmd = Command::new(std::env::current_exe()?);
                cmd.args(&args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
                crate::hide_window(&mut cmd);
                // Own process group, so Cancel can stop steamcmd and anything else the task started.
                #[cfg(unix)]
                std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
                let mut child = cmd.spawn()?;
                *child_pid.lock().unwrap() = Some(child.id());
                let err = child.stderr.take().unwrap();
                let (log2, ctx2) = (log.clone(), ctx.clone());
                let errs = thread::spawn(move || pipe(err, &log2, &ctx2));
                pipe(child.stdout.take().unwrap(), &log, &ctx);
                let _ = errs.join();
                Ok(child.wait()?.success())
            })();
            *child_pid.lock().unwrap() = None;
            match result {
                _ if cancelled.load(Ordering::SeqCst) => push("cancelled\n\n"),
                Ok(true) => push("done\n\n"),
                Ok(false) => push("failed\n\n"),
                Err(e) => push(&format!("failed: {e}\n\n")),
            }
            busy.store(false, Ordering::SeqCst);
            ctx.request_repaint();
        });
    }

    /// Stops the running task and everything it started (steamcmd, GModPatchTool).
    fn cancel(&self) {
        let Some(pid) = *self.child.lock().unwrap() else { return };
        self.cancelled.store(true, Ordering::SeqCst);
        let mut kill = if cfg!(windows) {
            let mut c = Command::new("taskkill");
            c.args(["/T", "/F", "/PID", &pid.to_string()]);
            c
        } else {
            let mut c = Command::new("kill");
            c.args(["-TERM", "--", &format!("-{pid}")]); // negative pid = the whole process group
            c
        };
        crate::hide_window(&mut kill);
        let _ = kill.status();
    }

    /// Last meaningful log line, for the status bar.
    fn status_line(&self) -> String {
        let log = self.log.lock().unwrap();
        let line = log.lines().rev().find(|l| !l.trim().is_empty() && !l.contains("Update state")).unwrap_or("Ready");
        line.trim_start_matches("[patch]").trim().chars().take(90).collect()
    }

    /// One item: name, description, version, and Install, or Update/Uninstall when installed (spinner while it runs).
    /// GitHub-released items get a version dropdown.
    fn item_row(&mut self, ui: &mut egui::Ui, id: &'static str, name: &str, desc: &str, can_install: Result<(), &str>, busy: bool) {
        let p = pal(ui);
        let installed = self.installed.get(id).copied().unwrap_or(false);
        let have = self.versions.get(id).cloned().flatten();
        let (latest, tags) = {
            let r = self.remote.lock().unwrap();
            match source(id) {
                Source::Steam(app) => (r.builds.get(&app).cloned(), Vec::new()),
                Source::GitHub(repo) => {
                    let tags = r.releases.get(repo).cloned().unwrap_or_default();
                    (tags.first().cloned(), tags)
                }
            }
        };
        let steam = self.steam_copy.get(id).copied().unwrap_or(false);
        let update = installed && !steam && matches!((&have, &latest), (Some(h), Some(l)) if h != l);
        let picked = self.selected.get(id).cloned().filter(|t| !t.is_empty());
        // A different release picked in the dropdown counts as a pending change too.
        let change = picked.as_ref().is_some_and(|t| Some(t) != have.as_ref());
        ui.horizontal(|ui| {
            ui.set_min_height(46.0);
            ui.vertical(|ui| {
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(name).strong().color(p.title));
                    if installed {
                        // Painted dot: the default font has no bullet glyph.
                        let (rect, resp) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                        ui.painter().circle_filled(rect.center(), 4.0, if update { accent() } else { p.good });
                        resp.on_hover_text(if update { "Update available" } else { "Installed" });
                    }
                    if update { ui.label(RichText::new("Update available").small().strong().color(accent())); }
                });
                let mut sub = desc.to_owned();
                if self.partial.get(id).copied().unwrap_or(false) && self.running.as_deref() != Some(id) {
                    sub = format!("{sub} · download interrupted");
                } else if steam {
                    sub = format!("{sub} · using your Steam copy");
                } else if installed {
                    match (&have, source(id)) {
                        (Some(v), Source::Steam(_)) => sub = format!("{sub} · build {v}"),
                        (Some(v), Source::GitHub(_)) => sub = format!("{sub} · {v}"),
                        // Installed before versions were tracked.
                        (None, _) => sub = format!("{sub} · version unknown"),
                    }
                }
                ui.label(RichText::new(sub).small().color(p.muted));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Fixed-width button column, so the version dropdowns line up across rows.
                let size = egui::vec2(ACTION_WIDTH, ui.available_height());
                ui.allocate_ui_with_layout(size, egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if busy && self.running.as_deref() == Some(id) {
                        if ui.button("Cancel").clicked() { self.cancel(); }
                        ui.spinner();
                        ui.label(RichText::new(if installed { "Working…" } else { "Installing…" }).small().color(p.muted));
                        return;
                    }
                    if self.partial.get(id).copied().unwrap_or(false) {
                        if ui.add_enabled(!busy, primary("Resume").min_size(egui::vec2(BUTTON_WIDTH, 0.0))).clicked() { self.start(ui.ctx(), &[id]); }
                        let b = egui::Button::new(RichText::new("Discard").color(p.danger))
                            .fill(p.danger.gamma_multiply(0.12)).min_size(egui::vec2(BUTTON_WIDTH, 0.0));
                        if ui.add_enabled(!busy, b).on_hover_text("Delete the partly downloaded files").clicked() { self.start(ui.ctx(), &[id, "off"]); }
                        return;
                    }
                    if steam {
                        let b = egui::Button::new("Steam").min_size(egui::vec2(BUTTON_WIDTH, 0.0));
                        ui.add_enabled(false, b).on_disabled_hover_text("Installed through Steam. GMod mounts it itself; tick it in GMod's Games menu.");
                        return;
                    }
                    let mut args = vec![id.to_owned()];
                    if let Some(t) = &picked { args.push(t.clone()); }
                    let label = if !installed { "Install" } else if change { "Switch" } else if update { "Update" } else { "" };
                    if !label.is_empty() {
                        let b = ui.add_enabled(!busy && can_install.is_ok(), primary(label).min_size(egui::vec2(BUTTON_WIDTH, 0.0)));
                        let b = if let Err(why) = can_install { b.on_disabled_hover_text(why) } else { b };
                        // Box3D and Jolt replace the same files, so installing one removes the other.
                        let other = match id { "vphysics" => Some(("jolt", "VPhysics-Jolt")), "jolt" => Some(("vphysics", "VPhysics-Box3D")), _ => None };
                        if b.clicked() {
                            match other.filter(|(o, _)| self.installed.get(o).copied().unwrap_or(false)) {
                                Some((_, other)) => self.confirm = Some((format!("Installing {name} will uninstall {other}."), "Install", args.clone())),
                                // A first install converts the user's own Skate 3 disc; updates reuse it.
                                None if id == "skategm" && !installed => {
                                    let (slot, ctx) = (self.picked_disc.clone(), ui.ctx().clone());
                                    thread::spawn(move || {
                                        if let Some(f) = rfd::FileDialog::new().set_title("Your Skate 3 disc image (.iso) or default.xex")
                                            .add_filter("Skate 3 disc image or default.xex", &["iso", "xex"]).pick_file() {
                                            *slot.lock().unwrap() = Some(args.into_iter().chain([f.display().to_string()]).collect());
                                            ctx.request_repaint();
                                        }
                                    });
                                }
                                None => self.start(ui.ctx(), &args.iter().map(String::as_str).collect::<Vec<_>>()),
                            }
                        }
                    }
                    if installed {
                        let b = egui::Button::new(RichText::new("Uninstall").color(p.danger))
                            .fill(p.danger.gamma_multiply(0.12)).min_size(egui::vec2(BUTTON_WIDTH, 0.0));
                        if ui.add_enabled(!busy, b).clicked() { self.confirm = Some((format!("Uninstall {name}?"), "Uninstall", vec![id.into(), "off".into()])); }
                    }
                });
                if let Source::GitHub(_) = source(id) {
                    let sel = self.selected.entry(id.to_owned()).or_default();
                    let latest_label = tags.first().map_or_else(|| "Loading…".to_owned(), |t| format!("{t} (latest)"));
                    let shown = if sel.is_empty() { latest_label.clone() } else { sel.clone() };
                    ui.add_enabled_ui(!tags.is_empty(), |ui| {
                        egui::ComboBox::from_id_salt(("ver", id)).width(140.0).selected_text(shown).show_ui(ui, |ui| {
                            ui.selectable_value(sel, String::new(), latest_label);
                            for t in tags.iter().skip(1) { ui.selectable_value(sel, t.clone(), t); }
                        });
                    });
                }
            });
        });
    }
}

fn pipe(r: impl Read, log: &Mutex<String>, ctx: &egui::Context) {
    for line in BufReader::new(r).lines().map_while(Result::ok) {
        let mut l = log.lock().unwrap();
        l.push_str(&line);
        l.push('\n');
        ctx.request_repaint();
    }
}

fn card<R>(ui: &mut egui::Ui, body: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let p = pal(ui);
    egui::Frame::new().fill(p.card).corner_radius(radii().1).inner_margin(Margin::symmetric(16, 12))
        .stroke(Stroke::new(1.0, p.stroke))
        .show(ui, |ui| { ui.set_width(ui.available_width()); body(ui) }).inner
}

fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(4.0);
    ui.label(RichText::new(title.to_uppercase()).small().strong().color(pal(ui).muted));
}

fn primary(text: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(text).color(on_accent()).strong()).fill(accent())
}

/// Pill-style tab switcher.
fn tabs(ui: &mut egui::Ui, tab: &mut Tab) {
    let p = pal(ui);
    let (control, card) = radii();
    egui::Frame::new().fill(p.card).stroke(Stroke::new(1.0, p.stroke)).corner_radius(card).inner_margin(Margin::same(3)).show(ui, |ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.horizontal(|ui| {
            for (t, label) in [(Tab::Content, "Game content"), (Tab::Patches, "Patches"), (Tab::Settings, "Settings")] {
                let on = *tab == t;
                let text = RichText::new(label).strong().color(if on { on_accent() } else { p.muted });
                let b = egui::Button::new(text).fill(if on { accent() } else { Color32::TRANSPARENT }).corner_radius(control);
                if ui.add(b).clicked() { *tab = t; }
            }
        });
    });
}

impl eframe::App for App {
    /// Closing the window mid-task stops it instead of leaving steamcmd running unseen; downloads resume next time.
    fn on_exit(&mut self) {
        self.cancel();
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let busy = self.busy.load(Ordering::SeqCst);
        if !busy && self.running.take().is_some() { self.refresh_status(); }
        let on_x64 = self.mounted_branch == "x86-64";
        let p = pal(ui);

        // Status bar with the latest log line; click to expand the full log.
        egui::Panel::bottom("status").frame(egui::Frame::new().fill(p.card).inner_margin(Margin::symmetric(20, 10))
            .stroke(Stroke::new(1.0, p.stroke))).show(ui, |ui| {
            ui.horizontal(|ui| {
                if busy { ui.spinner(); }
                ui.label(RichText::new(self.status_line()).small().color(p.muted));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let toggle = egui::Button::new(RichText::new(if self.show_log { "Hide log" } else { "Show log" }).small()).frame(false);
                    if ui.add(toggle).clicked() { self.show_log = !self.show_log; }
                });
            });
            if self.show_log {
                egui::Frame::new().fill(p.log_bg).corner_radius(radii().1).inner_margin(Margin::same(10)).show(ui, |ui| {
                    egui::ScrollArea::vertical().max_height(220.0).stick_to_bottom(true).auto_shrink([false, true]).show(ui, |ui| {
                        ui.add(egui::Label::new(RichText::new(self.log.lock().unwrap().as_str()).monospace().color(p.log_text)).wrap());
                    });
                });
            }
        });

        if let Some((question, action, args)) = self.confirm.clone() {
            let modal = egui::Modal::new(egui::Id::new("confirm")).show(ui.ctx(), |ui| {
                ui.label(RichText::new(question).strong().color(p.title));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let b = if action == "Uninstall" {
                        egui::Button::new(RichText::new(action).color(p.danger)).fill(p.danger.gamma_multiply(0.12))
                    } else { primary(action) };
                    if ui.add_enabled(!busy, b).clicked() {
                        self.start(ui.ctx(), &args.iter().map(String::as_str).collect::<Vec<_>>());
                        self.confirm = None;
                    }
                    if ui.button("Cancel").clicked() { self.confirm = None; }
                });
            });
            if modal.should_close() { self.confirm = None; }
        }

        let disc = self.picked_disc.lock().unwrap().take();
        if let Some(args) = disc { self.start(ui.ctx(), &args.iter().map(String::as_str).collect::<Vec<_>>()); }
        if let Some(p) = self.picked_folder.lock().unwrap().take() {
            self.path_input = p.display().to_string();
            self.path_msg = None;
        }

        egui::CentralPanel::default().frame(egui::Frame::new().fill(p.bg).inner_margin(Margin::symmetric(24, 20))).show(ui, |ui| {
            if self.setup {
                ui.add_space(30.0);
                ui.heading(RichText::new("Welcome to GMToolKit").strong().color(p.title));
                ui.label(RichText::new("First, choose your Garry's Mod folder. You can change it later in Settings.").color(p.muted));
                ui.add_space(12.0);
                card(ui, |ui| self.folder_picker(ui, "Continue"));
                return;
            }
            ui.heading(RichText::new("GMToolKit").strong().color(p.title));
            ui.add_space(4.0);
            // Shrink-wrap the tab pill instead of letting it fill the row.
            ui.horizontal(|ui| tabs(ui, &mut self.tab));
            ui.add_space(8.0);

            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 10.0;
                match self.tab {
                    Tab::Content => {
                        for (title, supported) in [("Supported by GMod", true), ("Other Source games", false)] {
                            section(ui, title);
                            card(ui, |ui| {
                                for (i, g) in crate::GAMES.iter().filter(|g| g.gmod_supported == supported).enumerate() {
                                    if i > 0 { ui.separator(); }
                                    self.item_row(ui, g.id, g.name, game_desc(g.id), Ok(()), busy);
                                }
                            });
                        }
                        ui.label(RichText::new("Games you own on Steam are used directly. Others are downloaded anonymously with SteamCMD, trimmed to what GMod needs, and added to mount.cfg. Content from \"Other Source games\" mostly works, but some materials may look wrong.").small().color(p.muted));
                    }
                    Tab::Patches => {
                        section(ui, "Game version");
                        card(ui, |ui| self.version_card(ui, busy));
                        section(ui, "Patches");
                        card(ui, |ui| {
                            let needs_x64 = if on_x64 { Ok(()) } else { Err("Switch to the 64-bit branch and let Steam update GMod first") };
                            self.item_row(ui, "chromium", "Chromium patches", "A newer Chromium and launch fixes, via GModPatchTool", needs_x64, busy);
                            if !cfg!(target_os = "macos") {
                                ui.separator();
                                self.item_row(ui, "vphysics", "VPhysics-Box3D", "A faster physics engine. GMod's original is backed up.", Ok(()), busy);
                                ui.separator();
                                self.item_row(ui, "jolt", "VPhysics-Jolt", "Physics engine built on Jolt. Last released in 2024. GMod's original is backed up.", Ok(()), busy);
                            }
                            if !cfg!(target_os = "macos") {
                                // Native Linux runs it on CUDA, which only the 64-bit branch's module supports.
                                let native = cfg!(target_os = "linux") && self.compat.is_empty();
                                let (desc, can) = if native {
                                    ("Real-time water and fluids. Natively on Linux it needs an NVIDIA GPU.", needs_x64)
                                } else { ("Real-time water and fluids. Needs a DirectX 11 GPU.", Ok(())) };
                                ui.separator();
                                self.item_row(ui, "gwater2", "GWater2", desc, can, busy);
                            }
                            if crate::skategm_supported() {
                                ui.separator();
                                self.item_row(ui, "skategm", "SkateGM", "Skate 3's skating, converted from your own Skate 3 disc (.iso or default.xex)", needs_x64, busy);
                            }
                        });
                        if self.installed.get("chromium").copied().unwrap_or(false) {
                            let note = if cfg!(target_os = "macos") {
                                "Uninstalling the Chromium patches makes Steam verify GMod's files."
                            } else {
                                "Uninstalling the Chromium patches makes Steam verify GMod's files, which also removes VPhysics-Box3D or VPhysics-Jolt."
                            };
                            ui.label(RichText::new(note).small().color(p.muted));
                        }
                    }
                    Tab::Settings => {
                        section(ui, "Garry's Mod folder");
                        card(ui, |ui| self.folder_picker(ui, "Save"));
                    }
                }
            });
        });
    }
}

impl App {
    /// GMod folder field with Auto-detect and Browse; `action` saves it (and leaves the setup screen).
    fn folder_picker(&mut self, ui: &mut egui::Ui, action: &str) {
        let p = pal(ui);
        ui.label(RichText::new("The folder that contains gmod.exe / hl2.sh and the garrysmod folder, usually …/steamapps/common/GarrysMod").small().color(p.muted));
        let edit = ui.add(egui::TextEdit::singleline(&mut self.path_input).hint_text("Path to your GarrysMod folder").desired_width(f32::INFINITY));
        if edit.changed() { self.path_msg = None; }
        ui.horizontal(|ui| {
            if ui.button("Auto-detect").clicked() {
                match crate::detect_gmod_dir() {
                    Ok(d) => { self.path_input = d.display().to_string(); self.path_msg = Some((true, "Found Garry's Mod through Steam.".into())); }
                    Err(e) => self.path_msg = Some((false, format!("Couldn't find it automatically: {e}. Use Browse instead."))),
                }
            }
            if ui.button("Browse…").clicked() {
                // The dialog blocks, so keep it off the UI thread.
                let (slot, ctx) = (self.picked_folder.clone(), ui.ctx().clone());
                thread::spawn(move || {
                    if let Some(d) = rfd::FileDialog::new().set_title("Choose your GarrysMod folder").pick_folder() {
                        *slot.lock().unwrap() = Some(d);
                        ctx.request_repaint();
                    }
                });
            }
            let path = std::path::PathBuf::from(self.path_input.trim());
            let valid = crate::is_gmod_dir(&path);
            let unchanged = crate::saved_gmod_dir().is_some_and(|s| s == path);
            let b = ui.add_enabled(valid && !unchanged, primary(action));
            let b = if valid { b } else { b.on_disabled_hover_text("Pick a folder that contains a garrysmod folder") };
            if b.clicked() {
                match crate::save_gmod_dir(&path) {
                    Ok(()) => {
                        self.setup = false;
                        self.path_msg = Some((true, "Saved.".into()));
                        self.branch = crate::current_branch(false).unwrap_or_else(|_| "x86-64".into());
                        self.refresh_status();
                    }
                    Err(e) => self.path_msg = Some((false, e.to_string())),
                }
            }
        });
        let path = std::path::PathBuf::from(self.path_input.trim());
        let msg = match &self.path_msg {
            Some(m) => Some(m.clone()),
            None if !self.path_input.trim().is_empty() && !crate::is_gmod_dir(&path) =>
                Some((false, "This folder doesn't contain a garrysmod folder.".into())),
            None => None,
        };
        if let Some((ok, m)) = msg { ui.label(RichText::new(m).small().color(if ok { p.good } else { p.danger })); }
    }

    fn version_card(&mut self, ui: &mut egui::Ui, busy: bool) {
        let p = pal(ui);
        let on_x64 = self.mounted_branch == "x86-64";
        ui.horizontal(|ui| {
            ui.label(RichText::new("Installed:").small().color(p.muted));
            let installed = crate::BRANCHES.iter().find(|b| b.0 == self.mounted_branch).map_or(self.mounted_branch.as_str(), |b| b.1);
            ui.label(RichText::new(installed).small().color(if on_x64 { p.good } else { p.title }));
        });
        ui.add_space(2.0);
        egui::Grid::new("version").num_columns(2).spacing([14.0, 8.0]).show(ui, |ui| {
            ui.label(RichText::new("Branch").color(p.muted));
            let label = |b: &str| crate::BRANCHES.iter().find(|x| x.0 == b).map_or(b, |x| x.1).to_owned();
            egui::ComboBox::from_id_salt("branch").width(290.0).selected_text(label(&self.branch)).show_ui(ui, |ui| {
                for (key, name) in crate::BRANCHES {
                    ui.selectable_value(&mut self.branch, key.to_string(), *name);
                }
            });
            ui.end_row();
            if cfg!(target_os = "linux") {
                ui.label(RichText::new("Runs with").color(p.muted));
                let name = |id: &str| match id {
                    "" => "Native (Linux)".to_owned(),
                    "proton_experimental" => "Proton Experimental (recommended)".to_owned(),
                    _ => self.tools.iter().find(|t| t.0 == id).map_or(id.to_owned(), |t| t.1.clone()),
                };
                egui::ComboBox::from_id_salt("compat").width(290.0).selected_text(name(&self.compat)).show_ui(ui, |ui| {
                    for id in self.tools.iter().map(|t| t.0.clone()).chain([String::new()]).collect::<Vec<_>>() {
                        let n = name(&id);
                        ui.selectable_value(&mut self.compat, id, n);
                    }
                });
                ui.end_row();
            }
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if busy && self.running.as_deref() == Some("switch") {
                ui.spinner();
            } else if ui.add_enabled(!busy, primary("Switch")).clicked() {
                let branch = if self.branch.is_empty() { "default".to_owned() } else { self.branch.clone() };
                let mut args = vec!["switch".to_owned(), branch];
                if cfg!(target_os = "linux") {
                    args.push(if self.compat.is_empty() { "native".into() } else { self.compat.clone() });
                }
                self.start(ui.ctx(), &args.iter().map(String::as_str).collect::<Vec<_>>());
            }
            ui.label(RichText::new("Close Steam first. GMod updates the next time Steam starts.").small().color(p.muted));
        });
    }
}

#[test]
fn start_streams_child_output() {
    // In tests current_exe is the test harness, so `--list` makes the child print test names.
    let mut app = App::default();
    app.start(&egui::Context::default(), &["--list"]);
    while app.busy.load(Ordering::SeqCst) { thread::sleep(Duration::from_millis(20)); }
    let log = app.log.lock().unwrap();
    assert!(log.contains("vdf_edits: test") && log.ends_with("done\n\n"), "{log}");
}

#[test]
fn kde_scheme() {
    let ini = "[Colors:Selection]\nBackgroundNormal=143,188,187\n\n[Colors:Window]\nBackgroundNormal=59,66,82\nForegroundNormal=216,222,233\n[General]\nX=1\n";
    let (p, acc) = kde_palette(ini).unwrap();
    assert!(p.dark && p.bg == Color32::from_rgb(59, 66, 82) && acc == Color32::from_rgb(143, 188, 187));
    assert!(kde_palette("[General]\n").is_none());
}

#[test]
#[ignore] // run with --ignored: reads this desktop's font settings
fn system_fonts_real() {
    for mono in [false, true] {
        let (family, pt) = desktop_font(mono).unwrap();
        println!("{family} {pt}pt");
        assert!(font_file(&family).is_some_and(|f| f.len() > 1000), "no file for {family}");
    }
}

#[test]
#[ignore] // only run as the child process of `cancel_stops_task`
fn slow_child_for_cancel_test() {
    thread::sleep(Duration::from_secs(60));
}

#[test]
fn cancel_stops_task() {
    // current_exe is the test harness; this runs the sleeping test above as the task.
    let mut app = App::default();
    app.start(&egui::Context::default(), &["gui::slow_child_for_cancel_test", "--exact", "--ignored"]);
    let wait = std::time::Instant::now();
    while app.child.lock().unwrap().is_none() {
        assert!(wait.elapsed() < Duration::from_secs(10), "child never started");
        thread::sleep(Duration::from_millis(20));
    }
    thread::sleep(Duration::from_millis(300)); // let it reach the sleeping test
    let started = std::time::Instant::now();
    app.cancel();
    while app.busy.load(Ordering::SeqCst) { thread::sleep(Duration::from_millis(20)); }
    assert!(started.elapsed() < Duration::from_secs(10), "task didn't stop");
    assert!(app.log.lock().unwrap().ends_with("cancelled\n\n"));
}
