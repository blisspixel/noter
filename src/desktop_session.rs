//! Linux desktop session preferences that stay outside the document.
//!
//! Omarchy and other freedesktop sessions publish a text scale and, when
//! Omarchy is installed, a small color file. Noter reads those while the
//! System theme is selected. Explicit themes do not. Nothing here watches the
//! network or loads theme programs.

use std::fs::File;
use std::io::Read as _;
use std::path::{Path, PathBuf};
#[cfg(any(target_os = "linux", test))]
use std::process::{Command, Stdio};
#[cfg(any(target_os = "linux", test))]
use std::thread;
#[cfg(any(target_os = "linux", test))]
use std::time::{Duration, Instant};
use std::time::{SystemTime, UNIX_EPOCH};

use eframe::egui;

/// Largest Omarchy `colors.toml` Noter will parse.
pub const PALETTE_MAX_BYTES: usize = 64 * 1024;
/// How often a focused window rechecks session text scale and palette metadata.
pub const DESKTOP_PROBE_INTERVAL_SECS: f64 = 15.0;
/// Session primary text must clear this ratio against the editor background.
pub const SESSION_TEXT_CONTRAST: f64 = 4.5;
#[cfg(any(target_os = "linux", test))]
const TEXT_SCALE_MIN: f32 = 0.5;
#[cfg(any(target_os = "linux", test))]
const TEXT_SCALE_MAX: f32 = 3.0;
#[cfg(any(target_os = "linux", test))]
const COMMAND_OUTPUT_LIMIT: usize = 4_096;
#[cfg(any(target_os = "linux", test))]
const PORTAL_FILE_LIMIT: usize = 32;

/// Shown when Linux has neither a `FileChooser` portal nor zenity.
#[cfg(any(target_os = "linux", test))]
pub const FILE_CHOOSER_MISSING: &str = "Open and Save need a desktop file chooser. Install xdg-desktop-portal and xdg-desktop-portal-gtk, because the Hyprland portal does not provide one, or install zenity, and try again.";
/// Shown when the session bus does not answer before Noter gives up.
#[cfg(any(target_os = "linux", test))]
pub const FILE_CHOOSER_UNRESPONSIVE: &str = "The desktop file chooser did not answer. Install xdg-desktop-portal and xdg-desktop-portal-gtk, or zenity, and try again. Noter did not wait on it.";

/// Light or dark choice from a session palette.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionMode {
    /// The session asked for a light editor.
    Light,
    /// The session asked for a dark editor.
    Dark,
}

/// Colors accepted from an Omarchy `colors.toml`.
///
/// Only the parser constructs this. A rejected file produces no value, so the
/// caller keeps the built-in System theme.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionColors {
    mode: SessionMode,
    background: egui::Color32,
    foreground: egui::Color32,
    selection: egui::Color32,
    selection_text: egui::Color32,
    accent: Option<egui::Color32>,
}

impl SessionColors {
    /// Whether the session palette is light or dark.
    pub const fn mode(self) -> SessionMode {
        self.mode
    }

    /// Editor, panel, and window background.
    pub const fn background(self) -> egui::Color32 {
        self.background
    }

    /// Primary text color.
    pub const fn foreground(self) -> egui::Color32 {
        self.foreground
    }

    /// Selection background.
    pub const fn selection(self) -> egui::Color32 {
        self.selection
    }

    /// Text drawn on the selection background.
    pub const fn selection_text(self) -> egui::Color32 {
        self.selection_text
    }

    /// Accent color when it still contrasts with the editor background.
    pub const fn accent(self) -> Option<egui::Color32> {
        self.accent
    }
}

/// Identity of the palette file used to skip unchanged reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaletteStamp {
    present: bool,
    modified_nanos: Option<u128>,
    len: u64,
}

/// What a file-chooser probe learned without opening a dialog.
#[cfg(any(target_os = "linux", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BusProbe {
    /// `org.freedesktop.portal.Desktop` owns its bus name.
    Owned,
    /// The bus answered and the portal name is free.
    NotOwned,
    /// A probe ran and did not answer before the deadline.
    Unanswered,
    /// The probe command is not installed.
    CouldNotAsk,
}

/// Facts used to decide whether opening a dialog can freeze the window.
#[cfg(any(target_os = "linux", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChooserFacts {
    /// Result of the short session-bus probe.
    pub bus: BusProbe,
    /// A portal config lists the `FileChooser` interface.
    pub file_chooser_backend: bool,
    /// `zenity` is on `PATH`.
    pub zenity_present: bool,
}

/// Whether Noter should open a system file dialog.
#[cfg(any(target_os = "linux", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChooserDecision {
    /// Call the system dialog.
    Open,
    /// Do not call it. Show this message instead.
    Block(&'static str),
}

/// Parses the five Omarchy color fields Noter understands.
///
/// Extra keys are ignored. A missing required field, a non-opaque `#RRGGBB`
/// color, or primary text below 4.5:1 rejects the whole file.
pub fn parse_session_palette(text: &str) -> Option<SessionColors> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut mode = None;
    let mut background = None;
    let mut foreground = None;
    let mut selection = None;
    let mut accent = None;

    for line in text.lines() {
        let Some((key, value)) = split_assignment(line) else {
            continue;
        };
        match key {
            "mode" => mode = Some(parse_mode(value)?),
            "background" => background = Some(parse_rgb(value)?),
            "foreground" => foreground = Some(parse_rgb(value)?),
            "selection" => selection = Some(parse_rgb(value)?),
            "accent" => accent = Some(parse_rgb(value)?),
            _ => {}
        }
    }

    let mode = mode?;
    let background = background?;
    let foreground = foreground?;
    let selection = selection?;
    if contrast_ratio(foreground, background) < SESSION_TEXT_CONTRAST {
        return None;
    }
    let selection_text = if contrast_ratio(foreground, selection) >= SESSION_TEXT_CONTRAST {
        foreground
    } else if contrast_ratio(background, selection) >= SESSION_TEXT_CONTRAST {
        background
    } else {
        return None;
    };
    // A present accent that fails contrast is omitted. The built-in link color
    // remains. A present accent that is not a color already rejected the file.
    let accent = accent.filter(|color| contrast_ratio(*color, background) >= SESSION_TEXT_CONTRAST);
    Some(SessionColors {
        mode,
        background,
        foreground,
        selection,
        selection_text,
        accent,
    })
}

/// Reads a palette file when it is a regular file of at most [`PALETTE_MAX_BYTES`].
pub fn read_session_palette(path: &Path) -> Option<SessionColors> {
    let text = read_bounded_text(path, PALETTE_MAX_BYTES)?;
    parse_session_palette(&text)
}

/// Stats the palette path. A missing file is a stamp, not an error.
pub fn palette_stamp(path: &Path) -> PaletteStamp {
    let Ok(metadata) = std::fs::metadata(path) else {
        return PaletteStamp {
            present: false,
            modified_nanos: None,
            len: 0,
        };
    };
    if !metadata.is_file() {
        return PaletteStamp {
            present: false,
            modified_nanos: None,
            len: 0,
        };
    }
    let modified_nanos = metadata.modified().ok().and_then(system_time_nanos);
    PaletteStamp {
        present: true,
        modified_nanos,
        len: metadata.len(),
    }
}

/// Resolves the live Omarchy palette path from the environment.
pub fn omarchy_colors_path_from(
    xdg_state_home: Option<&std::ffi::OsStr>,
    home: Option<&std::ffi::OsStr>,
) -> Option<PathBuf> {
    let base = xdg_state_home
        .map(PathBuf::from)
        .filter(|path| is_absolute_path(path))
        .or_else(|| home.map(|home| Path::new(home).join(".local").join("state")))?;
    Some(
        base.join("omarchy")
            .join("current")
            .join("theme")
            .join("colors.toml"),
    )
}

fn is_absolute_path(path: &Path) -> bool {
    path.is_absolute()
        || path
            .to_str()
            .is_some_and(|text| text.starts_with('/') || text.starts_with('\\'))
}

/// The palette path for this process, or nothing when no home is known.
pub fn default_omarchy_colors_path() -> Option<PathBuf> {
    omarchy_colors_path_from(
        std::env::var_os("XDG_STATE_HOME").as_deref(),
        std::env::var_os("HOME").as_deref(),
    )
}

/// Accepts a gsettings double. Non-positive and non-finite values are rejected.
/// Finite values outside 0.5 to 3 are clamped into that range.
#[cfg(any(target_os = "linux", test))]
pub fn parse_text_scaling_factor(text: &str) -> Option<f32> {
    let token = text.split_whitespace().next_back()?;
    let value = token.parse::<f32>().ok()?;
    sanitize_text_scale(value)
}

/// Clamps a finite positive scale into the desktop range Omarchy uses.
#[cfg(any(target_os = "linux", test))]
pub fn sanitize_text_scale(value: f32) -> Option<f32> {
    if !value.is_finite() || value <= 0.0 {
        return None;
    }
    Some(value.clamp(TEXT_SCALE_MIN, TEXT_SCALE_MAX))
}

/// Decides whether a file dialog is safe to open on the UI thread.
#[cfg(any(target_os = "linux", test))]
pub const fn classify_file_chooser(facts: ChooserFacts) -> ChooserDecision {
    match facts.bus {
        BusProbe::Unanswered => ChooserDecision::Block(FILE_CHOOSER_UNRESPONSIVE),
        // The portal daemon is down. A config file cannot answer, but zenity can.
        BusProbe::NotOwned if facts.zenity_present => ChooserDecision::Open,
        BusProbe::Owned | BusProbe::CouldNotAsk
            if facts.file_chooser_backend || facts.zenity_present =>
        {
            ChooserDecision::Open
        }
        BusProbe::NotOwned | BusProbe::Owned | BusProbe::CouldNotAsk => {
            ChooserDecision::Block(FILE_CHOOSER_MISSING)
        }
    }
}

/// True when any portal config under `roots` lists `FileChooser`.
#[cfg(any(target_os = "linux", test))]
pub fn portals_offer_file_chooser(roots: &[PathBuf]) -> bool {
    let mut seen = 0_usize;
    for root in roots {
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };
        for entry in entries.flatten() {
            if seen >= PORTAL_FILE_LIMIT {
                return false;
            }
            seen += 1;
            let path = entry.path();
            if path
                .extension()
                .is_some_and(|extension| extension == "portal")
                && read_bounded_text(&path, PALETTE_MAX_BYTES)
                    .is_some_and(|text| portal_config_offers_file_chooser(&text))
            {
                return true;
            }
        }
    }
    false
}

/// True when a portal desktop file implements `FileChooser`.
#[cfg(any(target_os = "linux", test))]
pub fn portal_config_offers_file_chooser(text: &str) -> bool {
    text.lines().any(|line| {
        let line = strip_comment(line).trim();
        let Some(interfaces) = line
            .strip_prefix("Interfaces")
            .or_else(|| line.strip_prefix("interfaces"))
        else {
            return false;
        };
        let Some(value) = interfaces.trim_start().strip_prefix('=') else {
            return false;
        };
        value
            .split([';', ','])
            .any(|interface| interface.trim() == "org.freedesktop.impl.portal.FileChooser")
    })
}

/// Runs a command and returns its stdout, or nothing if it misses the deadline.
#[cfg(any(target_os = "linux", test))]
pub fn command_output_with_deadline(
    program: &str,
    args: &[&str],
    deadline: Duration,
) -> Option<String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => thread::sleep(Duration::from_millis(10)),
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    if !status.success() {
        return None;
    }
    let mut stdout = child.stdout?;
    let mut buffer = vec![0_u8; COMMAND_OUTPUT_LIMIT];
    let mut filled = 0_usize;
    while filled < buffer.len() {
        match stdout.read(&mut buffer[filled..]) {
            Ok(0) => break,
            Ok(count) => filled += count,
            Err(_) => return None,
        }
    }
    String::from_utf8(buffer[..filled].to_vec()).ok()
}

/// Reads the GNOME text scale on Linux. Other platforms return nothing.
#[cfg(target_os = "linux")]
pub fn read_desktop_text_scale() -> Option<f32> {
    let output = command_output_with_deadline(
        "gsettings",
        &["get", "org.gnome.desktop.interface", "text-scaling-factor"],
        Duration::from_millis(300),
    )?;
    parse_text_scaling_factor(&output)
}

/// Reads the GNOME text scale on Linux. Other platforms return nothing.
#[cfg(not(target_os = "linux"))]
pub const fn read_desktop_text_scale() -> Option<f32> {
    None
}

/// Returns a message when opening a Linux file dialog could hang the window.
#[cfg(target_os = "linux")]
pub fn file_dialog_block_reason() -> Option<&'static str> {
    let facts = ChooserFacts {
        bus: probe_portal_bus(),
        file_chooser_backend: portals_offer_file_chooser(&portal_config_directories()),
        zenity_present: command_on_path("zenity"),
    };
    match classify_file_chooser(facts) {
        ChooserDecision::Open => None,
        ChooserDecision::Block(message) => Some(message),
    }
}

/// Returns a message when opening a Linux file dialog could hang the window.
#[cfg(not(target_os = "linux"))]
pub const fn file_dialog_block_reason() -> Option<&'static str> {
    None
}

#[cfg(target_os = "linux")]
fn probe_portal_bus() -> BusProbe {
    if !command_on_path("dbus-send") {
        return BusProbe::CouldNotAsk;
    }
    let Some(output) = command_output_with_deadline(
        "dbus-send",
        &[
            "--session",
            "--reply-timeout=250",
            "--dest=org.freedesktop.DBus",
            "--type=method_call",
            "--print-reply",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus.NameHasOwner",
            "string:org.freedesktop.portal.Desktop",
        ],
        Duration::from_millis(400),
    ) else {
        return BusProbe::Unanswered;
    };
    if output.contains("boolean true") {
        BusProbe::Owned
    } else if output.contains("boolean false") {
        BusProbe::NotOwned
    } else {
        BusProbe::Unanswered
    }
}

#[cfg(target_os = "linux")]
fn portal_config_directories() -> Vec<PathBuf> {
    let mut directories = Vec::new();
    if let Some(value) = std::env::var_os("XDG_DATA_DIRS") {
        for directory in std::env::split_paths(&value) {
            if directory.is_absolute() {
                directories.push(directory.join("xdg-desktop-portal").join("portals"));
            }
        }
    }
    if directories.is_empty() {
        directories.push(PathBuf::from("/usr/local/share/xdg-desktop-portal/portals"));
        directories.push(PathBuf::from("/usr/share/xdg-desktop-portal/portals"));
    }
    directories
}

#[cfg(target_os = "linux")]
fn command_on_path(name: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|directory| directory.join(name).is_file())
}

fn split_assignment(line: &str) -> Option<(&str, &str)> {
    let line = strip_comment(line).trim();
    if line.is_empty() || line.starts_with('[') {
        return None;
    }
    let (key, value) = line.split_once('=')?;
    let key = key.trim();
    if key.is_empty() || key.contains(char::is_whitespace) {
        return None;
    }
    Some((key, unquote(value.trim())?))
}

fn strip_comment(line: &str) -> &str {
    let mut in_single = false;
    let mut in_double = false;
    let bytes = line.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\'' if !in_double => in_single = !in_single,
            b'"' if !in_single => in_double = !in_double,
            b'#' if !in_single && !in_double => {
                if index + 6 < bytes.len()
                    && bytes[index + 1..index + 7]
                        .iter()
                        .all(u8::is_ascii_hexdigit)
                    && (index + 7 == bytes.len()
                        || bytes[index + 7].is_ascii_whitespace()
                        || bytes[index + 7] == b'#')
                    && !line[..index].trim().is_empty()
                {
                    index += 7;
                    continue;
                }
                return &line[..index];
            }
            _ => {}
        }
        index += 1;
    }
    line
}

fn unquote(value: &str) -> Option<&str> {
    let bytes = value.as_bytes();
    if bytes.len() >= 2
        && (bytes[0] == b'"' || bytes[0] == b'\'')
        && bytes[bytes.len() - 1] == bytes[0]
    {
        return Some(&value[1..value.len() - 1]);
    }
    if value.contains(char::is_whitespace) {
        return None;
    }
    Some(value)
}

const fn parse_mode(value: &str) -> Option<SessionMode> {
    if value.eq_ignore_ascii_case("dark") {
        Some(SessionMode::Dark)
    } else if value.eq_ignore_ascii_case("light") {
        Some(SessionMode::Light)
    } else {
        None
    }
}

fn parse_rgb(value: &str) -> Option<egui::Color32> {
    let hex = value.strip_prefix('#')?;
    if hex.len() != 6 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let channel = |start: usize| u8::from_str_radix(&hex[start..start + 2], 16).ok();
    Some(egui::Color32::from_rgb(
        channel(0)?,
        channel(2)?,
        channel(4)?,
    ))
}

fn contrast_ratio(first: egui::Color32, second: egui::Color32) -> f64 {
    let brighter = relative_luminance(first).max(relative_luminance(second));
    let darker = relative_luminance(first).min(relative_luminance(second));
    (brighter + 0.05) / (darker + 0.05)
}

fn relative_luminance(color: egui::Color32) -> f64 {
    let [red, green, blue, _] = color.to_array();
    [red, green, blue]
        .into_iter()
        .zip([0.2126_f64, 0.7152, 0.0722])
        .map(|(channel, weight)| {
            let channel = f64::from(channel) / 255.0;
            let linear = if channel <= 0.04045 {
                channel / 12.92
            } else {
                ((channel + 0.055) / 1.055).powf(2.4)
            };
            linear * weight
        })
        .sum()
}

fn system_time_nanos(time: SystemTime) -> Option<u128> {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_nanos())
}

fn read_bounded_text(path: &Path, max_bytes: usize) -> Option<String> {
    let file = File::open(path).ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() || metadata.len() > u64::try_from(max_bytes).ok()? {
        return None;
    }
    let mut buffer = Vec::new();
    file.take(u64::try_from(max_bytes).ok()? + 1)
        .read_to_end(&mut buffer)
        .ok()?;
    if buffer.len() > max_bytes {
        return None;
    }
    String::from_utf8(buffer).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const MATTE_BLACK: &str = r##"
mode = "dark"
accent = "#e68e0d"
selection = "#2a2a2a"
background = "#121212"
foreground = "#bebebe"
red = "#D35F5F"
"##;

    #[test]
    fn matte_black_palette_is_accepted() {
        let colors = parse_session_palette(MATTE_BLACK).expect("matte black should pass");
        assert_eq!(colors.mode(), SessionMode::Dark);
        assert_eq!(
            colors.background(),
            egui::Color32::from_rgb(0x12, 0x12, 0x12)
        );
        assert_eq!(
            colors.foreground(),
            egui::Color32::from_rgb(0xbe, 0xbe, 0xbe)
        );
        assert_eq!(
            colors.selection(),
            egui::Color32::from_rgb(0x2a, 0x2a, 0x2a)
        );
        assert_eq!(colors.selection_text(), colors.foreground());
        assert_eq!(
            colors.accent(),
            Some(egui::Color32::from_rgb(0xe6, 0x8e, 0x0d))
        );
    }

    #[test]
    fn low_primary_contrast_rejects_the_whole_palette() {
        let text = "mode = \"dark\"\nbackground = \"#000000\"\nforeground = \"#010101\"\nselection = \"#222222\"\n";
        assert!(parse_session_palette(text).is_none());
    }

    #[test]
    fn unreadable_selection_rejects_the_whole_palette() {
        let text = "mode = \"light\"\nbackground = \"#ffffff\"\nforeground = \"#555555\"\nselection = \"#808080\"\n";
        assert!(parse_session_palette(text).is_none());
    }

    #[test]
    fn selection_text_falls_back_to_the_background() {
        let text = "mode = \"light\"\nbackground = \"#ffffff\"\nforeground = \"#000000\"\nselection = \"#000000\"\n";
        let colors = parse_session_palette(text).expect("background contrasts with the selection");
        assert_eq!(colors.selection_text(), colors.background());
    }

    #[test]
    fn unreadable_accent_is_dropped_and_the_palette_kept() {
        let text = "mode = \"light\"\nbackground = \"#ffffff\"\nforeground = \"#000000\"\nselection = \"#cce0ff\"\naccent = \"#eeeeee\"\n";
        let colors = parse_session_palette(text).expect("primary contrast is high");
        assert_eq!(colors.accent(), None);
    }

    #[test]
    fn missing_accent_is_allowed() {
        let text =
            "mode = 'light'\nbackground = #ffffff\nforeground = #111111\nselection = #cce0ff\n";
        let colors = parse_session_palette(text).expect("bare colors are valid");
        assert_eq!(colors.mode(), SessionMode::Light);
        assert_eq!(colors.accent(), None);
    }

    #[test]
    fn bad_mode_short_hex_and_alpha_are_rejected() {
        assert!(parse_session_palette("mode = \"auto\"\nbackground = \"#ffffff\"\nforeground = \"#000000\"\nselection = \"#cccccc\"\n").is_none());
        assert!(parse_session_palette("mode = \"dark\"\nbackground = \"#fff\"\nforeground = \"#000000\"\nselection = \"#222222\"\n").is_none());
        assert!(parse_session_palette("mode = \"dark\"\nbackground = \"#12121280\"\nforeground = \"#ffffff\"\nselection = \"#333333\"\n").is_none());
    }

    #[test]
    fn later_duplicate_key_wins_and_comments_are_ignored() {
        let text = "mode = \"light\" # noon\nmode = \"dark\"\nbackground = \"#121212\" # floor\nforeground = \"#eeeeee\"\nselection = \"#333333\"\n";
        let colors = parse_session_palette(text).expect("the second mode is valid");
        assert_eq!(colors.mode(), SessionMode::Dark);
    }

    #[test]
    fn text_scale_parses_gsettings_and_clamps() {
        assert_eq!(
            parse_text_scaling_factor("1.3636\n").unwrap().to_bits(),
            1.3636_f32.to_bits()
        );
        assert_eq!(
            parse_text_scaling_factor("  1.0").unwrap().to_bits(),
            1.0_f32.to_bits()
        );
        assert_eq!(
            sanitize_text_scale(4.0).unwrap().to_bits(),
            3.0_f32.to_bits()
        );
        assert_eq!(
            sanitize_text_scale(0.2).unwrap().to_bits(),
            0.5_f32.to_bits()
        );
        assert!(parse_text_scaling_factor("").is_none());
        assert!(parse_text_scaling_factor("nan").is_none());
        assert!(parse_text_scaling_factor("inf").is_none());
        assert!(sanitize_text_scale(0.0).is_none());
        assert!(sanitize_text_scale(-1.0).is_none());
        assert!(sanitize_text_scale(f32::NAN).is_none());
    }

    #[test]
    fn file_chooser_decision_blocks_a_silent_or_hung_portal() {
        assert_eq!(
            classify_file_chooser(ChooserFacts {
                bus: BusProbe::Unanswered,
                file_chooser_backend: true,
                zenity_present: true,
            }),
            ChooserDecision::Block(FILE_CHOOSER_UNRESPONSIVE)
        );
        assert_eq!(
            classify_file_chooser(ChooserFacts {
                bus: BusProbe::Owned,
                file_chooser_backend: false,
                zenity_present: false,
            }),
            ChooserDecision::Block(FILE_CHOOSER_MISSING)
        );
        assert_eq!(
            classify_file_chooser(ChooserFacts {
                bus: BusProbe::NotOwned,
                file_chooser_backend: false,
                zenity_present: true,
            }),
            ChooserDecision::Open
        );
        assert_eq!(
            classify_file_chooser(ChooserFacts {
                bus: BusProbe::CouldNotAsk,
                file_chooser_backend: true,
                zenity_present: false,
            }),
            ChooserDecision::Open
        );
        assert_eq!(
            classify_file_chooser(ChooserFacts {
                bus: BusProbe::CouldNotAsk,
                file_chooser_backend: false,
                zenity_present: false,
            }),
            ChooserDecision::Block(FILE_CHOOSER_MISSING)
        );
    }

    #[test]
    fn portal_config_detects_the_file_chooser_interface() {
        let text = "[portal]\nDBusName=org.freedesktop.impl.portal.desktop.gtk\nInterfaces=org.freedesktop.impl.portal.Settings;org.freedesktop.impl.portal.FileChooser;\n";
        assert!(portal_config_offers_file_chooser(text));
        assert!(!portal_config_offers_file_chooser(
            "Interfaces=org.freedesktop.impl.portal.Settings;\n"
        ));
    }

    #[test]
    fn portal_directory_scan_stops_at_the_chooser() {
        let root = tempfile::tempdir().expect("temp dir");
        let portals = root.path().join("portals");
        std::fs::create_dir(&portals).expect("portals dir");
        std::fs::write(
            portals.join("hyprland.portal"),
            "Interfaces=org.freedesktop.impl.portal.Settings;\n",
        )
        .expect("hyprland portal");
        std::fs::write(
            portals.join("gtk.portal"),
            "Interfaces=org.freedesktop.impl.portal.FileChooser;\n",
        )
        .expect("gtk portal");
        assert!(portals_offer_file_chooser(&[portals]));
        assert!(!portals_offer_file_chooser(&[root.path().join("missing")]));
    }

    #[test]
    fn palette_read_rejects_oversize_and_non_files() {
        let root = tempfile::tempdir().expect("temp dir");
        let file = root.path().join("colors.toml");
        std::fs::write(&file, MATTE_BLACK).expect("palette");
        assert!(read_session_palette(&file).is_some());
        assert!(palette_stamp(&file).present);

        let huge = root.path().join("huge.toml");
        std::fs::write(&huge, vec![b'a'; PALETTE_MAX_BYTES + 1]).expect("huge");
        assert!(read_session_palette(&huge).is_none());

        let directory = root.path().join("dir.toml");
        std::fs::create_dir(&directory).expect("dir");
        assert!(read_session_palette(&directory).is_none());
        assert!(!palette_stamp(&directory).present);
        assert!(!palette_stamp(&root.path().join("absent.toml")).present);
    }

    #[test]
    fn omarchy_path_honors_an_absolute_state_home() {
        let path = omarchy_colors_path_from(
            Some(std::ffi::OsStr::new("/var/state")),
            Some(std::ffi::OsStr::new("/home/editor")),
        )
        .expect("absolute state home");
        assert_eq!(
            path,
            Path::new("/var/state")
                .join("omarchy")
                .join("current")
                .join("theme")
                .join("colors.toml")
        );
        let fallback = omarchy_colors_path_from(
            Some(std::ffi::OsStr::new("relative")),
            Some(std::ffi::OsStr::new("/home/editor")),
        )
        .expect("home fallback");
        assert_eq!(
            fallback,
            Path::new("/home/editor")
                .join(".local")
                .join("state")
                .join("omarchy")
                .join("current")
                .join("theme")
                .join("colors.toml")
        );
        assert!(omarchy_colors_path_from(None, None).is_none());
    }

    #[test]
    fn command_deadline_returns_output_and_gives_up() {
        let started = Instant::now();
        #[cfg(unix)]
        let output =
            command_output_with_deadline("sh", &["-c", "printf ready"], Duration::from_secs(2));
        #[cfg(windows)]
        let output =
            command_output_with_deadline("cmd", &["/c", "echo ready"], Duration::from_secs(2));
        assert!(output.as_deref().is_some_and(|text| text.contains("ready")));

        #[cfg(unix)]
        let stalled = command_output_with_deadline("sleep", &["30"], Duration::from_millis(200));
        #[cfg(windows)]
        let stalled = command_output_with_deadline(
            "ping",
            &["-n", "8", "127.0.0.1"],
            Duration::from_millis(200),
        );
        assert!(stalled.is_none());
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(
            command_output_with_deadline("noter-missing-command", &[], Duration::from_secs(1))
                .is_none()
        );
    }

    #[test]
    fn live_session_probes_return_without_hanging() {
        let started = Instant::now();
        let _ = read_desktop_text_scale();
        let _ = file_dialog_block_reason();
        assert!(started.elapsed() < Duration::from_secs(3));
    }
}
