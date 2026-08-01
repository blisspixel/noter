use eframe::egui;

pub const WORD_WRAP_STORAGE_KEY: &str = "noter.word-wrap";
pub const ZOOM_STORAGE_KEY: &str = "noter.editor-zoom-percent";

const DEFAULT_ZOOM_PERCENT: u16 = 100;
const MINIMUM_ZOOM_PERCENT: u16 = 50;
const MAXIMUM_ZOOM_PERCENT: u16 = 300;
const ZOOM_STEP_PERCENT: u16 = 10;
const POINTER_ZOOM_STEP_FACTOR: f32 = 1.1;
const MINIMUM_ACCUMULATED_POINTER_FACTOR: f32 = 0.25;
const MAXIMUM_ACCUMULATED_POINTER_FACTOR: f32 = 4.0;
const MAXIMUM_POINTER_ZOOM_STEPS_PER_EVENT: usize = 32;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct EditorZoom(u16);

impl Default for EditorZoom {
    fn default() -> Self {
        Self(DEFAULT_ZOOM_PERCENT)
    }
}

impl EditorZoom {
    pub fn from_storage(storage: Option<&dyn eframe::Storage>) -> Self {
        storage
            .and_then(|storage| storage.get_string(ZOOM_STORAGE_KEY))
            .as_deref()
            .and_then(Self::from_storage_value)
            .unwrap_or_default()
    }

    fn from_storage_value(value: &str) -> Option<Self> {
        let percent = value.parse::<u16>().ok()?;
        ((MINIMUM_ZOOM_PERCENT..=MAXIMUM_ZOOM_PERCENT).contains(&percent)
            && percent.is_multiple_of(ZOOM_STEP_PERCENT))
        .then_some(Self(percent))
    }

    pub const fn percent(self) -> u16 {
        self.0
    }

    pub const fn can_zoom_in(self) -> bool {
        self.0 < MAXIMUM_ZOOM_PERCENT
    }

    pub const fn can_zoom_out(self) -> bool {
        self.0 > MINIMUM_ZOOM_PERCENT
    }

    pub fn scale(self) -> f32 {
        f32::from(self.0) / 100.0
    }

    pub fn zoom_in(&mut self) -> bool {
        self.set(self.0.saturating_add(ZOOM_STEP_PERCENT))
    }

    pub fn zoom_out(&mut self) -> bool {
        self.set(self.0.saturating_sub(ZOOM_STEP_PERCENT))
    }

    pub fn reset(&mut self) -> bool {
        self.set(DEFAULT_ZOOM_PERCENT)
    }

    fn set(&mut self, requested: u16) -> bool {
        let bounded = requested.clamp(MINIMUM_ZOOM_PERCENT, MAXIMUM_ZOOM_PERCENT);
        let changed = self.0 != bounded;
        self.0 = bounded;
        changed
    }

    pub fn storage_value(self) -> String {
        self.0.to_string()
    }
}

/// Accumulates smooth pointer magnification into the same discrete, bounded
/// steps used by menu and keyboard zoom.
#[derive(Clone, Copy, Debug)]
pub struct PointerZoomAccumulator(f32);

impl Default for PointerZoomAccumulator {
    fn default() -> Self {
        Self(1.0)
    }
}

impl PointerZoomAccumulator {
    pub fn apply(&mut self, delta: f32, zoom: &mut EditorZoom) -> bool {
        if !delta.is_finite() || delta <= 0.0 {
            self.reset();
            return false;
        }

        self.0 = (self.0 * delta).clamp(
            MINIMUM_ACCUMULATED_POINTER_FACTOR,
            MAXIMUM_ACCUMULATED_POINTER_FACTOR,
        );
        let mut changed = false;
        for _ in 0..MAXIMUM_POINTER_ZOOM_STEPS_PER_EVENT {
            if self.0 < POINTER_ZOOM_STEP_FACTOR {
                break;
            }
            if !zoom.zoom_in() {
                self.reset();
                return changed;
            }
            self.0 /= POINTER_ZOOM_STEP_FACTOR;
            changed = true;
        }

        let zoom_out_threshold = POINTER_ZOOM_STEP_FACTOR.recip();
        for _ in 0..MAXIMUM_POINTER_ZOOM_STEPS_PER_EVENT {
            if self.0 > zoom_out_threshold {
                break;
            }
            if !zoom.zoom_out() {
                self.reset();
                return changed;
            }
            self.0 *= POINTER_ZOOM_STEP_FACTOR;
            changed = true;
        }
        changed
    }

    pub const fn reset(&mut self) {
        self.0 = 1.0;
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub enum TextWrap {
    #[default]
    Wrapped,
    Unwrapped,
}

impl TextWrap {
    pub fn from_storage(storage: Option<&dyn eframe::Storage>) -> Self {
        storage
            .and_then(|storage| storage.get_string(WORD_WRAP_STORAGE_KEY))
            .as_deref()
            .and_then(Self::from_storage_value)
            .unwrap_or_default()
    }

    fn from_storage_value(value: &str) -> Option<Self> {
        match value {
            "true" => Some(Self::Wrapped),
            "false" => Some(Self::Unwrapped),
            _ => None,
        }
    }

    pub const fn is_wrapped(self) -> bool {
        matches!(self, Self::Wrapped)
    }

    pub const fn toggle(&mut self) {
        *self = match self {
            Self::Wrapped => Self::Unwrapped,
            Self::Unwrapped => Self::Wrapped,
        };
    }

    pub const fn storage_value(self) -> &'static str {
        match self {
            Self::Wrapped => "true",
            Self::Unwrapped => "false",
        }
    }
}

pub fn apply_editor_zoom(style: &mut egui::Style, zoom: EditorZoom) {
    let scale = zoom.scale();
    for text_style in [
        egui::TextStyle::Body,
        egui::TextStyle::Monospace,
        egui::TextStyle::Heading,
    ] {
        if let Some(font) = style.text_styles.get_mut(&text_style) {
            font.size *= scale;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_is_bounded_and_resettable() {
        let mut zoom = EditorZoom::default();
        assert!(zoom.can_zoom_in());
        assert!(zoom.can_zoom_out());
        assert!(!zoom.reset());
        for _ in 0..100 {
            let _ = zoom.zoom_in();
        }
        assert_eq!(zoom.percent(), MAXIMUM_ZOOM_PERCENT);
        assert!(!zoom.can_zoom_in());
        assert!(zoom.can_zoom_out());
        assert!(!zoom.zoom_in());

        for _ in 0..100 {
            let _ = zoom.zoom_out();
        }
        assert_eq!(zoom.percent(), MINIMUM_ZOOM_PERCENT);
        assert!(zoom.can_zoom_in());
        assert!(!zoom.can_zoom_out());
        assert!(!zoom.zoom_out());
        assert!(zoom.reset());
        assert_eq!(zoom.percent(), DEFAULT_ZOOM_PERCENT);
    }

    #[test]
    fn stored_zoom_rejects_malformed_and_out_of_range_values() {
        assert_eq!(
            EditorZoom::from_storage_value("50").map(EditorZoom::percent),
            Some(50)
        );
        assert_eq!(
            EditorZoom::from_storage_value("300").map(EditorZoom::percent),
            Some(300)
        );
        for invalid in ["", "49", "55", "301", "100.0", "lots"] {
            assert_eq!(EditorZoom::from_storage_value(invalid), None);
        }
    }

    #[test]
    fn stored_word_wrap_accepts_only_canonical_boolean_values() {
        assert_eq!(
            TextWrap::from_storage_value("true"),
            Some(TextWrap::Wrapped)
        );
        assert_eq!(
            TextWrap::from_storage_value("false"),
            Some(TextWrap::Unwrapped)
        );
        for invalid in ["", "TRUE", "0", "yes"] {
            assert_eq!(TextWrap::from_storage_value(invalid), None);
        }
    }

    #[test]
    fn editor_zoom_scales_document_styles_without_touching_controls() {
        let mut style = egui::Style::default();
        let body = style.text_styles[&egui::TextStyle::Body].size;
        let button = style.text_styles[&egui::TextStyle::Button].size;

        apply_editor_zoom(&mut style, EditorZoom(150));

        assert_eq!(
            style.text_styles[&egui::TextStyle::Body].size.to_bits(),
            (body * 1.5).to_bits()
        );
        assert_eq!(
            style.text_styles[&egui::TextStyle::Button].size.to_bits(),
            button.to_bits()
        );
    }

    #[test]
    fn smooth_pointer_zoom_accumulates_into_bounded_discrete_steps() {
        let mut accumulator = PointerZoomAccumulator::default();
        let mut zoom = EditorZoom::default();

        assert!(!accumulator.apply(1.05, &mut zoom));
        assert!(accumulator.apply(1.05, &mut zoom));
        assert_eq!(zoom.percent(), 110);
        accumulator.reset();
        assert!(accumulator.apply(POINTER_ZOOM_STEP_FACTOR.recip(), &mut zoom));
        assert_eq!(zoom.percent(), 100);

        assert!(!accumulator.apply(f32::NAN, &mut zoom));
        for _ in 0..100 {
            let _ = accumulator.apply(2.0, &mut zoom);
        }
        assert_eq!(zoom.percent(), MAXIMUM_ZOOM_PERCENT);
        assert!(!accumulator.apply(2.0, &mut zoom));
    }
}
