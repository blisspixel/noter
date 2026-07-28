use eframe::egui;
use std::sync::Arc;

pub const THEME_STORAGE_KEY: &str = "noter.theme";
const NOTER_PROPORTIONAL_FONT: &str = "Inter Variable";
const NOTER_PROPORTIONAL_FONT_BYTES: &[u8] = include_bytes!("../assets/fonts/InterVariable.ttf");

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub enum AppTheme {
    #[default]
    System,
    Light,
    Dark,
    GreenScreen,
    AmberScreen,
}

impl AppTheme {
    pub const ALL: [Self; 5] = [
        Self::System,
        Self::Light,
        Self::Dark,
        Self::GreenScreen,
        Self::AmberScreen,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Light => "Light",
            Self::Dark => "Dark",
            Self::GreenScreen => "Green Screen",
            Self::AmberScreen => "Amber Screen",
        }
    }

    pub const fn compact_label(self) -> &'static str {
        match self {
            Self::GreenScreen => "Green",
            Self::AmberScreen => "Amber",
            _ => self.label(),
        }
    }

    pub const fn storage_value(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
            Self::GreenScreen => "green",
            Self::AmberScreen => "amber",
        }
    }

    pub fn from_storage_value(value: &str) -> Option<Self> {
        match value {
            "system" => Some(Self::System),
            "light" => Some(Self::Light),
            "dark" => Some(Self::Dark),
            "green" => Some(Self::GreenScreen),
            "amber" => Some(Self::AmberScreen),
            _ => None,
        }
    }

    pub fn from_storage(storage: Option<&dyn eframe::Storage>) -> Self {
        storage
            .and_then(|storage| storage.get_string(THEME_STORAGE_KEY))
            .as_deref()
            .and_then(Self::from_storage_value)
            .unwrap_or_default()
    }

    pub fn apply(self, context: &egui::Context) {
        match self {
            Self::System => {
                apply_palette(context, egui::Theme::Light, VisualPalette::Light);
                apply_palette(context, egui::Theme::Dark, VisualPalette::Dark);
                context.set_theme(egui::ThemePreference::System);
            }
            Self::Light => {
                apply_palette(context, egui::Theme::Light, VisualPalette::Light);
                context.set_theme(egui::ThemePreference::Light);
            }
            Self::Dark => {
                apply_palette(context, egui::Theme::Dark, VisualPalette::Dark);
                context.set_theme(egui::ThemePreference::Dark);
            }
            Self::GreenScreen => {
                apply_palette(context, egui::Theme::Dark, VisualPalette::GreenScreen);
                context.set_theme(egui::ThemePreference::Dark);
            }
            Self::AmberScreen => {
                apply_palette(context, egui::Theme::Dark, VisualPalette::AmberScreen);
                context.set_theme(egui::ThemePreference::Dark);
            }
        }
    }
}

#[derive(Clone, Copy)]
enum VisualPalette {
    Light,
    Dark,
    GreenScreen,
    AmberScreen,
}

pub fn configure_styles(context: &egui::Context) {
    configure_fonts(context);
    context.all_styles_mut(|style| {
        for (text_style, font_id) in &mut style.text_styles {
            if *text_style == egui::TextStyle::Body {
                font_id.size = 15.0;
            } else if *text_style == egui::TextStyle::Button {
                font_id.size = 14.0;
            } else if *text_style == egui::TextStyle::Small {
                font_id.size = 12.0;
            } else if *text_style == egui::TextStyle::Monospace {
                font_id.size = 15.0;
            } else if *text_style == egui::TextStyle::Heading {
                font_id.size = 24.0;
            }
        }
        style.spacing.item_spacing = egui::vec2(6.0, 6.0);
        style.spacing.button_padding = egui::vec2(10.0, 5.0);
        style.spacing.interact_size = egui::vec2(44.0, 28.0);
        configure_visual_details(&mut style.visuals);
    });

    apply_palette(context, egui::Theme::Light, VisualPalette::Light);
    apply_palette(context, egui::Theme::Dark, VisualPalette::Dark);
}

fn apply_palette(context: &egui::Context, theme: egui::Theme, palette: VisualPalette) {
    context.style_mut_of(theme, |style| {
        let text_options = style.visuals.text_options;
        style.visuals = match palette {
            VisualPalette::Light => egui::Visuals::light(),
            VisualPalette::Dark | VisualPalette::GreenScreen | VisualPalette::AmberScreen => {
                egui::Visuals::dark()
            }
        };
        style.visuals.text_options = text_options;
        configure_visual_details(&mut style.visuals);

        match palette {
            VisualPalette::Light => apply_light_palette(&mut style.visuals),
            VisualPalette::Dark => apply_dark_palette(&mut style.visuals),
            VisualPalette::GreenScreen => apply_green_screen_palette(&mut style.visuals),
            VisualPalette::AmberScreen => apply_amber_screen_palette(&mut style.visuals),
        }
    });
}

fn configure_visual_details(visuals: &mut egui::Visuals) {
    visuals.text_options.font_hinting = true;
    visuals.text_options.subpixel_binning = true;
    visuals.interact_cursor = Some(egui::CursorIcon::PointingHand);
    visuals.widgets.inactive.corner_radius = 4.into();
    visuals.widgets.hovered.corner_radius = 4.into();
    visuals.widgets.active.corner_radius = 4.into();
    visuals.widgets.open.corner_radius = 4.into();
}

fn apply_light_palette(visuals: &mut egui::Visuals) {
    visuals.override_text_color = Some(egui::Color32::from_rgb(31, 33, 36));
    visuals.weak_text_color = Some(egui::Color32::from_rgb(95, 99, 104));
    visuals.panel_fill = egui::Color32::from_rgb(246, 247, 248);
    visuals.window_fill = egui::Color32::from_rgb(252, 252, 251);
    visuals.extreme_bg_color = egui::Color32::from_rgb(255, 255, 254);
    visuals.text_edit_bg_color = Some(egui::Color32::from_rgb(255, 255, 254));
    visuals.faint_bg_color = egui::Color32::from_rgb(238, 240, 243);
    visuals.code_bg_color = egui::Color32::from_rgb(238, 240, 243);
    visuals.hyperlink_color = egui::Color32::from_rgb(36, 91, 161);
    visuals.selection.bg_fill = egui::Color32::from_rgb(190, 215, 245);
    visuals.selection.stroke = egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(22, 64, 112));
}

fn apply_dark_palette(visuals: &mut egui::Visuals) {
    visuals.override_text_color = Some(egui::Color32::from_rgb(230, 232, 236));
    visuals.weak_text_color = Some(egui::Color32::from_rgb(160, 165, 175));
    visuals.panel_fill = egui::Color32::from_rgb(30, 33, 40);
    visuals.window_fill = egui::Color32::from_rgb(27, 29, 35);
    visuals.extreme_bg_color = egui::Color32::from_rgb(23, 25, 30);
    visuals.text_edit_bg_color = Some(egui::Color32::from_rgb(23, 25, 30));
    visuals.faint_bg_color = egui::Color32::from_rgb(38, 42, 50);
    visuals.code_bg_color = egui::Color32::from_rgb(38, 42, 50);
    visuals.hyperlink_color = egui::Color32::from_rgb(126, 172, 238);
    visuals.selection.bg_fill = egui::Color32::from_rgb(55, 92, 139);
    visuals.selection.stroke = egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(235, 241, 250));
}

fn apply_green_screen_palette(visuals: &mut egui::Visuals) {
    let text = egui::Color32::from_rgb(174, 255, 181);
    let weak_text = egui::Color32::from_rgb(113, 191, 121);
    let panel = egui::Color32::from_rgb(7, 20, 10);
    let editor = egui::Color32::from_rgb(3, 10, 5);
    let outline = egui::Color32::from_rgb(77, 190, 91);

    apply_terminal_palette(
        visuals,
        TerminalPalette {
            text,
            weak_text,
            panel,
            window: egui::Color32::from_rgb(5, 16, 8),
            editor,
            raised: egui::Color32::from_rgb(12, 38, 18),
            active: egui::Color32::from_rgb(25, 75, 34),
            outline,
            hyperlink: egui::Color32::from_rgb(111, 235, 158),
            selection: egui::Color32::from_rgb(30, 88, 40),
            selected_text: egui::Color32::from_rgb(226, 255, 229),
            warning: egui::Color32::from_rgb(242, 218, 109),
            error: egui::Color32::from_rgb(255, 135, 123),
        },
    );
}

fn apply_amber_screen_palette(visuals: &mut egui::Visuals) {
    let text = egui::Color32::from_rgb(255, 215, 137);
    let weak_text = egui::Color32::from_rgb(202, 158, 82);
    let panel = egui::Color32::from_rgb(25, 16, 5);
    let editor = egui::Color32::from_rgb(14, 9, 2);
    let outline = egui::Color32::from_rgb(215, 147, 42);

    apply_terminal_palette(
        visuals,
        TerminalPalette {
            text,
            weak_text,
            panel,
            window: egui::Color32::from_rgb(21, 13, 4),
            editor,
            raised: egui::Color32::from_rgb(49, 30, 7),
            active: egui::Color32::from_rgb(92, 57, 11),
            outline,
            hyperlink: egui::Color32::from_rgb(255, 225, 132),
            selection: egui::Color32::from_rgb(105, 65, 10),
            selected_text: egui::Color32::from_rgb(255, 242, 199),
            warning: egui::Color32::from_rgb(255, 231, 135),
            error: egui::Color32::from_rgb(255, 137, 119),
        },
    );
}

#[derive(Clone, Copy)]
struct TerminalPalette {
    text: egui::Color32,
    weak_text: egui::Color32,
    panel: egui::Color32,
    window: egui::Color32,
    editor: egui::Color32,
    raised: egui::Color32,
    active: egui::Color32,
    outline: egui::Color32,
    hyperlink: egui::Color32,
    selection: egui::Color32,
    selected_text: egui::Color32,
    warning: egui::Color32,
    error: egui::Color32,
}

fn apply_terminal_palette(visuals: &mut egui::Visuals, palette: TerminalPalette) {
    visuals.override_text_color = Some(palette.text);
    visuals.weak_text_color = Some(palette.weak_text);
    visuals.panel_fill = palette.panel;
    visuals.window_fill = palette.window;
    visuals.extreme_bg_color = palette.editor;
    visuals.text_edit_bg_color = Some(palette.editor);
    visuals.faint_bg_color = palette.raised;
    visuals.code_bg_color = palette.raised;
    visuals.hyperlink_color = palette.hyperlink;
    visuals.selection.bg_fill = palette.selection;
    visuals.selection.stroke = egui::Stroke::new(1.0, palette.selected_text);
    visuals.warn_fg_color = palette.warning;
    visuals.error_fg_color = palette.error;
    visuals.window_stroke = egui::Stroke::new(1.0, palette.outline);

    configure_widget(
        &mut visuals.widgets.noninteractive,
        palette.window,
        palette.panel,
        palette.outline,
        palette.text,
    );
    configure_widget(
        &mut visuals.widgets.inactive,
        palette.raised,
        palette.raised,
        palette.outline,
        palette.text,
    );
    configure_widget(
        &mut visuals.widgets.hovered,
        palette.active,
        palette.active,
        palette.outline,
        palette.selected_text,
    );
    configure_widget(
        &mut visuals.widgets.active,
        palette.active,
        palette.active,
        palette.selected_text,
        palette.selected_text,
    );
    configure_widget(
        &mut visuals.widgets.open,
        palette.active,
        palette.active,
        palette.outline,
        palette.selected_text,
    );
}

fn configure_widget(
    widget: &mut egui::style::WidgetVisuals,
    background: egui::Color32,
    weak_background: egui::Color32,
    outline: egui::Color32,
    foreground: egui::Color32,
) {
    widget.bg_fill = background;
    widget.weak_bg_fill = weak_background;
    widget.bg_stroke = egui::Stroke::new(1.0, outline);
    widget.fg_stroke = egui::Stroke::new(1.0, foreground);
}

fn configure_fonts(context: &egui::Context) {
    let mut definitions = egui::FontDefinitions::default();
    definitions.font_data.insert(
        NOTER_PROPORTIONAL_FONT.to_owned(),
        Arc::new(egui::FontData::from_static(NOTER_PROPORTIONAL_FONT_BYTES)),
    );
    definitions
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .insert(0, NOTER_PROPORTIONAL_FONT.to_owned());
    context.set_fonts(definitions);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persisted_values_round_trip() {
        for theme in AppTheme::ALL {
            assert_eq!(
                AppTheme::from_storage_value(theme.storage_value()),
                Some(theme)
            );
        }
    }

    #[test]
    fn unknown_persisted_value_is_rejected() {
        assert_eq!(AppTheme::from_storage_value("sepia"), None);
    }

    #[test]
    fn applying_each_theme_updates_the_context_preference() {
        let context = egui::Context::default();
        configure_styles(&context);

        AppTheme::Light.apply(&context);
        assert_eq!(context.theme(), egui::Theme::Light);
        AppTheme::Dark.apply(&context);
        assert_eq!(context.theme(), egui::Theme::Dark);
        AppTheme::GreenScreen.apply(&context);
        assert_eq!(context.theme(), egui::Theme::Dark);
        AppTheme::AmberScreen.apply(&context);
        assert_eq!(context.theme(), egui::Theme::Dark);
    }

    #[test]
    fn specialty_palettes_exceed_text_and_control_contrast_thresholds() {
        let context = egui::Context::default();
        configure_styles(&context);

        for theme in [AppTheme::GreenScreen, AppTheme::AmberScreen] {
            theme.apply(&context);
            let visuals = &context.style_of(egui::Theme::Dark).visuals;
            let text = visuals
                .override_text_color
                .expect("specialty themes define an exact text color");
            let weak_text = visuals
                .weak_text_color
                .expect("specialty themes define an exact weak-text color");

            assert!(contrast_ratio(text, visuals.text_edit_bg_color()) >= 7.0);
            assert!(contrast_ratio(weak_text, visuals.text_edit_bg_color()) >= 4.5);
            assert!(
                contrast_ratio(visuals.selection.stroke.color, visuals.selection.bg_fill) >= 4.5
            );
            assert!(
                contrast_ratio(visuals.widgets.inactive.bg_stroke.color, visuals.panel_fill) >= 3.0
            );
        }
    }

    #[test]
    fn selecting_a_standard_or_system_theme_removes_specialty_palette_state() {
        let context = egui::Context::default();
        configure_styles(&context);
        let standard_dark = context.style_of(egui::Theme::Dark).visuals.clone();

        AppTheme::GreenScreen.apply(&context);
        assert_ne!(context.style_of(egui::Theme::Dark).visuals, standard_dark);

        AppTheme::Dark.apply(&context);
        assert_eq!(context.style_of(egui::Theme::Dark).visuals, standard_dark);

        AppTheme::AmberScreen.apply(&context);
        AppTheme::System.apply(&context);
        assert_eq!(context.style_of(egui::Theme::Dark).visuals, standard_dark);
    }

    #[test]
    fn configured_styles_use_readable_type_and_modern_rasterization() {
        let context = egui::Context::default();

        configure_styles(&context);

        for theme in [egui::Theme::Light, egui::Theme::Dark] {
            let style = context.style_of(theme);
            assert_eq!(
                style.text_styles[&egui::TextStyle::Body].size.to_bits(),
                15.0_f32.to_bits()
            );
            assert_eq!(
                style.text_styles[&egui::TextStyle::Button].size.to_bits(),
                14.0_f32.to_bits()
            );
            assert_eq!(
                style.text_styles[&egui::TextStyle::Monospace]
                    .size
                    .to_bits(),
                15.0_f32.to_bits()
            );
            assert!(style.visuals.text_options.font_hinting);
            assert!(style.visuals.text_options.subpixel_binning);
            assert_eq!(style.spacing.interact_size.y.to_bits(), 28.0_f32.to_bits());
        }
        assert_eq!(
            context
                .style_of(egui::Theme::Light)
                .visuals
                .text_options
                .color_transfer_function,
            egui::epaint::FontColorTransferFunction::LIGHT_MODE_DEFAULT
        );
        assert_eq!(
            context
                .style_of(egui::Theme::Dark)
                .visuals
                .text_options
                .color_transfer_function,
            egui::epaint::FontColorTransferFunction::DARK_MODE_DEFAULT
        );
    }

    #[test]
    fn bundled_proportional_font_supports_real_weight_variation() {
        let data = egui::FontData::from_static(NOTER_PROPORTIONAL_FONT_BYTES);
        let weight = data
            .variation_axes()
            .into_iter()
            .find(|axis| axis.tag.to_be_bytes() == *b"wght")
            .expect("the bundled font must expose a weight axis");

        assert!(weight.range.min <= 400.0);
        assert!(weight.range.max >= 700.0);
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
            .zip([0.2126, 0.7152, 0.0722])
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
}
