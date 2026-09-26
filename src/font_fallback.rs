//! Local system fonts for characters the bundled fonts cannot draw.
//!
//! The window ships Inter and egui's default fonts, which cover Latin, Greek,
//! Cyrillic, and emoji. Text in any other script would otherwise draw as
//! replacement boxes. When text arrives that contains such characters, a
//! background thread looks through the operating system's own font
//! directories, loads the first files that cover them, and gives those to
//! egui as fallbacks. Nothing is downloaded, and no font file is read until a
//! document needs one.

use eframe::egui;
use skrifa::MetadataProvider;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

/// Font files larger than this are skipped; the largest common system fonts,
/// such as Noto CJK collections, stay well below it.
const MAX_FONT_FILE_BYTES: u64 = 96 * 1024 * 1024;
/// Loaded fallback fonts stay resident, so their total size is bounded.
const MAX_LOADED_BYTES: u64 = 192 * 1024 * 1024;
/// Examining a font means reading it once. Characters no font covers must not
/// make the search read every font on the system again and again.
const MAX_EXAMINED_BYTES: u64 = 768 * 1024 * 1024;
const MAX_DISCOVERED_FILES: usize = 4_096;
const MAX_DIRECTORY_DEPTH: usize = 5;

/// File stems, lowercased, that cover the most scripts per byte on each
/// platform. They are examined first; every other font follows.
const PREFERRED_STEMS: &[&str] = &[
    // Windows
    "segoeui",
    "msyh",
    "yugothr",
    "meiryo",
    "malgun",
    "msjh",
    "nirmala",
    "leelawui",
    "ebrima",
    "seguisym",
    "gadugi",
    "mmrtext",
    "himalaya",
    "simsun",
    // macOS
    "arial unicode",
    "pingfang",
    "hiragino sans gb",
    "applesdgothicneo",
    "geezapro",
    "sfarabic",
    "kohinoor",
    "thonburi",
    "apple symbols",
    // Linux
    "notosanscjk-regular",
    "notosanscjksc-regular",
    "notosanscjkjp-regular",
    "dejavusans",
    "droidsansfallbackfull",
    "droidsansfallback",
    "wqy-microhei",
    "wqy-zenhei",
    "freesans",
];

const STYLE_WORDS: &[&str] = &[
    "bold",
    "italic",
    "oblique",
    "light",
    "thin",
    "black",
    "medium",
    "semibold",
    "heavy",
    "condensed",
    "extra",
    "mono",
    "serif",
    "display",
];

/// Queues text for glyph coverage and installs fonts that complete it.
pub struct FontFallback {
    context: egui::Context,
    sender: Option<mpsc::Sender<String>>,
}

impl FontFallback {
    /// Returns a fallback that starts its worker on the first text needing it.
    pub const fn new(context: egui::Context) -> Self {
        Self {
            context,
            sender: None,
        }
    }

    /// Makes sure every character in `text` can be drawn, if a local font
    /// can draw it. Returns at once; the fonts arrive on a later frame.
    pub fn observe(&mut self, text: &str) {
        if text.is_ascii() {
            return;
        }
        if self.sender.is_none() {
            self.sender = spawn_worker(self.context.clone());
        }
        if let Some(sender) = &self.sender
            && sender.send(text.to_owned()).is_err()
        {
            self.sender = None;
        }
    }
}

fn spawn_worker(context: egui::Context) -> Option<mpsc::Sender<String>> {
    let (sender, receiver) = mpsc::channel::<String>();
    std::thread::Builder::new()
        .name("noter-fonts".to_owned())
        .spawn(move || {
            let mut resolver = Resolver::new(bundled_coverage(), discover_candidates);
            while let Ok(first) = receiver.recv() {
                let mut missing = BTreeSet::new();
                resolver.collect_missing(&first, &mut missing);
                while let Ok(next) = receiver.try_recv() {
                    resolver.collect_missing(&next, &mut missing);
                }
                let fonts = resolver.resolve(missing);
                if fonts.is_empty() {
                    continue;
                }
                for font in fonts {
                    context.add_font(egui::epaint::text::FontInsert::new(
                        &font.name,
                        font.data,
                        vec![
                            fallback_family(egui::FontFamily::Proportional),
                            fallback_family(egui::FontFamily::Monospace),
                        ],
                    ));
                }
                context.request_repaint();
            }
        })
        .ok()
        .map(|_| sender)
}

const fn fallback_family(family: egui::FontFamily) -> egui::epaint::text::InsertFontFamily {
    egui::epaint::text::InsertFontFamily {
        family,
        priority: egui::epaint::text::FontPriority::Lowest,
    }
}

fn bundled_coverage() -> Vec<Coverage> {
    let mut fonts = vec![Coverage::of(crate::theme::NOTER_PROPORTIONAL_FONT_BYTES, 0)];
    fonts.extend(
        egui::FontDefinitions::default()
            .font_data
            .values()
            .map(|data| Coverage::of(&data.font, data.index)),
    );
    fonts
}

/// Characters that draw as nothing, or that no general font is expected to
/// have, never start a search.
const fn needs_glyph(character: char) -> bool {
    !(character.is_ascii()
        || character.is_control()
        || character.is_whitespace()
        || matches!(
            character,
            '\u{200B}'..='\u{200F}'
                | '\u{2028}'..='\u{202E}'
                | '\u{2060}'..='\u{206F}'
                | '\u{FE00}'..='\u{FE0F}'
                | '\u{FEFF}'
                | '\u{E000}'..='\u{F8FF}'
                | '\u{E0000}'..='\u{E007F}'
                | '\u{E0100}'..='\u{E01EF}'
                | '\u{F0000}'..
        ))
}

/// The characters one font face maps, as sorted inclusive ranges.
#[derive(Debug, Default, PartialEq, Eq)]
struct Coverage {
    ranges: Vec<(u32, u32)>,
}

impl Coverage {
    fn of(bytes: &[u8], index: u32) -> Self {
        let Ok(font) = skrifa::FontRef::from_index(bytes, index) else {
            return Self::default();
        };
        let mut points: Vec<u32> = font.charmap().mappings().map(|(point, _)| point).collect();
        points.sort_unstable();
        points.dedup();
        let mut ranges: Vec<(u32, u32)> = Vec::new();
        for point in points {
            match ranges.last_mut() {
                Some((_, end)) if end.checked_add(1) == Some(point) => *end = point,
                _ => ranges.push((point, point)),
            }
        }
        Self { ranges }
    }

    fn contains(&self, character: char) -> bool {
        let point = u32::from(character);
        self.ranges
            .binary_search_by(|&(start, end)| {
                if end < point {
                    std::cmp::Ordering::Less
                } else if start > point {
                    std::cmp::Ordering::Greater
                } else {
                    std::cmp::Ordering::Equal
                }
            })
            .is_ok()
    }
}

/// A font ready to hand to egui.
struct LoadedFont {
    name: String,
    data: egui::FontData,
}

struct Candidate {
    path: PathBuf,
    /// Filled once the file has been read.
    coverage: Option<Coverage>,
    loaded: bool,
}

/// Decides which font files to load for the characters it is shown.
struct Resolver<D> {
    covered: Vec<Coverage>,
    unresolvable: BTreeSet<char>,
    discover: Option<D>,
    candidates: Vec<Candidate>,
    loaded_bytes: u64,
    examined_bytes: u64,
}

impl<D: FnOnce() -> Vec<PathBuf>> Resolver<D> {
    const fn new(covered: Vec<Coverage>, discover: D) -> Self {
        Self {
            covered,
            unresolvable: BTreeSet::new(),
            discover: Some(discover),
            candidates: Vec::new(),
            loaded_bytes: 0,
            examined_bytes: 0,
        }
    }

    fn collect_missing(&self, text: &str, missing: &mut BTreeSet<char>) {
        for character in text.chars() {
            if needs_glyph(character)
                && !self.unresolvable.contains(&character)
                && !self.covered.iter().any(|font| font.contains(character))
            {
                missing.insert(character);
            }
        }
    }

    fn resolve(&mut self, mut missing: BTreeSet<char>) -> Vec<LoadedFont> {
        if missing.is_empty() {
            return Vec::new();
        }
        if let Some(discover) = self.discover.take() {
            self.candidates = discover()
                .into_iter()
                .map(|path| Candidate {
                    path,
                    coverage: None,
                    loaded: false,
                })
                .collect();
        }
        let mut fonts = Vec::new();
        for index in 0..self.candidates.len() {
            if missing.is_empty() {
                break;
            }
            if let Some(font) = self.try_candidate(index, &mut missing) {
                fonts.push(font);
            }
        }
        self.unresolvable.extend(missing);
        fonts
    }

    fn try_candidate(&mut self, index: usize, missing: &mut BTreeSet<char>) -> Option<LoadedFont> {
        let candidate = &self.candidates[index];
        if candidate.loaded {
            return None;
        }
        let known_useless = candidate
            .coverage
            .as_ref()
            .is_some_and(|coverage| !missing.iter().any(|&c| coverage.contains(c)));
        if known_useless {
            return None;
        }
        let size = std::fs::metadata(&candidate.path).ok()?.len();
        if size > MAX_FONT_FILE_BYTES {
            return None;
        }
        let first_read = candidate.coverage.is_none();
        if first_read && self.examined_bytes.saturating_add(size) > MAX_EXAMINED_BYTES {
            return None;
        }
        let bytes = std::fs::read(&candidate.path).ok()?;
        let size = bytes.len() as u64;
        if first_read {
            self.examined_bytes = self.examined_bytes.saturating_add(size);
        }
        let coverage = Coverage::of(&bytes, 0);
        let helps = missing.iter().any(|&c| coverage.contains(c));
        let within_budget = self.loaded_bytes.saturating_add(size) <= MAX_LOADED_BYTES;
        let candidate = &mut self.candidates[index];
        if !(helps && within_budget) {
            candidate.coverage = Some(coverage);
            return None;
        }
        missing.retain(|&c| !coverage.contains(c));
        candidate.loaded = true;
        self.loaded_bytes = self.loaded_bytes.saturating_add(size);
        self.covered.push(coverage);
        Some(LoadedFont {
            name: format!("system:{}", candidate.path.display()),
            data: egui::FontData::from_owned(bytes),
        })
    }
}

/// Lists local font files, most useful first.
fn discover_candidates() -> Vec<PathBuf> {
    let mut files = Vec::new();
    for directory in font_directories() {
        collect_font_files(&directory, 0, &mut files);
    }
    rank_candidates(files)
}

fn font_directories() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut directories = Vec::new();
    if cfg!(windows) {
        let windows =
            std::env::var_os("WINDIR").map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
        directories.push(windows.join("Fonts"));
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            directories.push(Path::new(&local).join(r"Microsoft\Windows\Fonts"));
        }
    } else if cfg!(target_os = "macos") {
        directories.push(PathBuf::from("/System/Library/Fonts"));
        directories.push(PathBuf::from("/Library/Fonts"));
        if let Some(home) = &home {
            directories.push(home.join("Library/Fonts"));
        }
    } else {
        let data_home = std::env::var_os("XDG_DATA_HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .or_else(|| home.as_ref().map(|home| home.join(".local/share")));
        if let Some(data_home) = data_home {
            directories.push(data_home.join("fonts"));
        }
        if let Some(home) = &home {
            directories.push(home.join(".fonts"));
        }
        directories.push(PathBuf::from("/usr/local/share/fonts"));
        directories.push(PathBuf::from("/usr/share/fonts"));
    }
    directories
}

fn collect_font_files(directory: &Path, depth: usize, files: &mut Vec<PathBuf>) {
    if depth > MAX_DIRECTORY_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        if files.len() >= MAX_DISCOVERED_FILES {
            return;
        }
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            collect_font_files(&path, depth + 1, files);
        } else if is_font_file(&path) {
            files.push(path);
        }
    }
}

fn is_font_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            ["ttf", "otf", "ttc", "otc"]
                .iter()
                .any(|known| extension.eq_ignore_ascii_case(known))
        })
}

fn rank_candidates(mut files: Vec<PathBuf>) -> Vec<PathBuf> {
    files.sort_by_cached_key(|path| {
        let stem = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let preferred = PREFERRED_STEMS
            .iter()
            .position(|name| stem == *name)
            .unwrap_or(PREFERRED_STEMS.len());
        let styled = STYLE_WORDS.iter().any(|word| stem.contains(word));
        (preferred, styled, stem, path.clone())
    });
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inter_coverage() -> Coverage {
        Coverage::of(crate::theme::NOTER_PROPORTIONAL_FONT_BYTES, 0)
    }

    #[test]
    fn coverage_reads_the_character_map_as_ranges() {
        let coverage = inter_coverage();
        assert!(coverage.contains('a'));
        assert!(coverage.contains('é'));
        assert!(coverage.contains('Ж'));
        assert!(!coverage.contains('世'));
        assert!(
            coverage
                .ranges
                .windows(2)
                .all(|pair| pair[0].1 + 1 < pair[1].0)
        );
        assert_eq!(Coverage::of(b"not a font", 0), Coverage::default());
        assert!(!Coverage::default().contains('a'));
    }

    #[test]
    fn bundled_fonts_cover_european_scripts_and_emoji_but_not_cjk() {
        let bundled = bundled_coverage();
        let covered = |c: char| bundled.iter().any(|font| font.contains(c));
        for character in ['a', 'ß', 'Ω', 'Я', '😀'] {
            assert!(covered(character), "{character} should be bundled");
        }
        for character in ['世', 'あ', '한', 'ع', 'ש', 'क', 'ก'] {
            assert!(!covered(character), "{character} should need a system font");
        }
    }

    #[test]
    fn invisible_and_private_characters_never_start_a_search() {
        for character in [
            'a',
            '\n',
            '\u{00A0}',
            '\u{200D}',
            '\u{FE0F}',
            '\u{E000}',
            '\u{F0000}',
        ] {
            assert!(!needs_glyph(character), "{character:?}");
        }
        for character in ['é', '世', 'ع', '\u{0301}'] {
            assert!(needs_glyph(character), "{character:?}");
        }
    }

    #[test]
    fn resolver_loads_only_fonts_that_cover_missing_characters() {
        let directory = tempfile::tempdir().unwrap();
        let useless = directory.path().join("a-useless.ttf");
        let inter = directory.path().join("b-inter.ttf");
        std::fs::write(&useless, b"not a font").unwrap();
        std::fs::write(&inter, crate::theme::NOTER_PROPORTIONAL_FONT_BYTES).unwrap();
        let paths = vec![useless, inter.clone()];
        let mut resolver = Resolver::new(Vec::new(), move || paths);

        let mut missing = BTreeSet::new();
        resolver.collect_missing("plain ascii", &mut missing);
        assert!(missing.is_empty());
        assert!(resolver.resolve(missing).is_empty());
        assert!(resolver.discover.is_some(), "no search without a need");

        let mut missing = BTreeSet::new();
        resolver.collect_missing("café 世界", &mut missing);
        assert_eq!(missing, BTreeSet::from(['é', '世', '界']));
        let fonts = resolver.resolve(missing);
        assert_eq!(fonts.len(), 1);
        assert_eq!(fonts[0].name, format!("system:{}", inter.display()));
        assert_eq!(
            resolver.loaded_bytes,
            crate::theme::NOTER_PROPORTIONAL_FONT_BYTES.len() as u64
        );
        assert!(resolver.candidates[1].loaded);
        assert_eq!(resolver.candidates[0].coverage, Some(Coverage::default()));

        let mut missing = BTreeSet::new();
        resolver.collect_missing("résumé 世界 Ω", &mut missing);
        assert!(
            missing.is_empty(),
            "covered and unresolvable characters are remembered"
        );
        let examined = resolver.examined_bytes;
        assert!(resolver.resolve(BTreeSet::from(['世'])).is_empty());
        assert_eq!(
            resolver.examined_bytes, examined,
            "known files are not read twice"
        );
    }

    #[test]
    fn resolver_rereads_an_examined_font_when_it_becomes_useful() {
        let directory = tempfile::tempdir().unwrap();
        let inter = directory.path().join("inter.ttf");
        std::fs::write(&inter, crate::theme::NOTER_PROPORTIONAL_FONT_BYTES).unwrap();
        let paths = vec![inter];
        let mut resolver = Resolver::new(Vec::new(), move || paths);

        assert!(resolver.resolve(BTreeSet::from(['世'])).is_empty());
        assert!(!resolver.candidates[0].loaded);
        assert_eq!(resolver.resolve(BTreeSet::from(['é'])).len(), 1);
        assert!(resolver.candidates[0].loaded);
        assert_eq!(resolver.loaded_bytes, resolver.examined_bytes);
    }

    #[test]
    fn resolver_respects_the_examination_budget() {
        let directory = tempfile::tempdir().unwrap();
        let inter = directory.path().join("inter.ttf");
        std::fs::write(&inter, crate::theme::NOTER_PROPORTIONAL_FONT_BYTES).unwrap();
        let paths = vec![inter, directory.path().join("missing.ttf")];
        let mut resolver = Resolver::new(Vec::new(), move || paths);
        resolver.examined_bytes = MAX_EXAMINED_BYTES;

        assert!(resolver.resolve(BTreeSet::from(['é'])).is_empty());
        assert!(resolver.candidates[0].coverage.is_none());

        resolver.examined_bytes = 0;
        resolver.loaded_bytes = MAX_LOADED_BYTES;
        resolver.unresolvable.clear();
        assert!(resolver.resolve(BTreeSet::from(['é'])).is_empty());
        assert!(resolver.candidates[0].coverage.is_some());
        assert!(!resolver.candidates[0].loaded);
    }

    #[test]
    fn discovery_finds_font_files_in_nested_directories() {
        let directory = tempfile::tempdir().unwrap();
        let nested = directory.path().join("truetype/noto");
        std::fs::create_dir_all(&nested).unwrap();
        for name in [
            "NotoSansArabic-Regular.ttf",
            "Upper.OTF",
            "notes.txt",
            "c.ttc",
        ] {
            std::fs::write(nested.join(name), b"").unwrap();
        }
        let mut deep = directory.path().to_path_buf();
        for level in 0..=MAX_DIRECTORY_DEPTH {
            deep.push(format!("d{level}"));
        }
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(deep.join("too-deep.ttf"), b"").unwrap();

        let mut files = Vec::new();
        collect_font_files(directory.path(), 0, &mut files);
        let names: Vec<_> = files
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["NotoSansArabic-Regular.ttf", "Upper.OTF", "c.ttc"]);

        let mut none = Vec::new();
        collect_font_files(&directory.path().join("absent"), 0, &mut none);
        assert!(none.is_empty());
    }

    #[test]
    fn ranking_prefers_broad_regular_fonts() {
        let ranked = rank_candidates(
            [
                "/f/zeta-Bold.ttf",
                "/f/NotoSansThai-Regular.ttf",
                "/f/alpha.ttf",
                "/f/DejaVuSans.ttf",
                "/f/segoeui.ttf",
                "/f/NotoSerif-Regular.ttf",
            ]
            .into_iter()
            .map(PathBuf::from)
            .collect(),
        );
        let names: Vec<_> = ranked
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            [
                "segoeui.ttf",
                "DejaVuSans.ttf",
                "alpha.ttf",
                "NotoSansThai-Regular.ttf",
                "NotoSerif-Regular.ttf",
                "zeta-Bold.ttf",
            ]
        );
    }

    #[test]
    fn font_directories_include_the_platform_system_directory() {
        let directories = font_directories();
        let expected = if cfg!(windows) {
            "Fonts"
        } else if cfg!(target_os = "macos") {
            "/System/Library/Fonts"
        } else {
            "/usr/share/fonts"
        };
        assert!(directories.iter().any(|path| path.ends_with(expected)));
    }

    #[test]
    fn observe_ignores_ascii_and_installs_a_worker_for_other_text() {
        let context = egui::Context::default();
        let mut fallback = FontFallback::new(context);
        fallback.observe("ascii only");
        assert!(fallback.sender.is_none());
        fallback.observe("é");
        assert!(fallback.sender.is_some());
    }
}
