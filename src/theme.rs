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
}

impl AppTheme {
    pub const ALL: [Self; 3] = [Self::System, Self::Light, Self::Dark];

    pub const fn label(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }

    pub const fn storage_value(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    pub fn from_storage_value(value: &str) -> Option<Self> {
        match value {
            "system" => Some(Self::System),
            "light" => Some(Self::Light),
            "dark" => Some(Self::Dark),
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
        let preference = match self {
            Self::System => egui::ThemePreference::System,
            Self::Light => egui::ThemePreference::Light,
            Self::Dark => egui::ThemePreference::Dark,
        };
        context.set_theme(preference);
    }
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
        style.visuals.text_options.font_hinting = true;
        style.visuals.text_options.subpixel_binning = true;
        style.visuals.interact_cursor = Some(egui::CursorIcon::PointingHand);
        style.visuals.widgets.inactive.corner_radius = 4.into();
        style.visuals.widgets.hovered.corner_radius = 4.into();
        style.visuals.widgets.active.corner_radius = 4.into();
        style.visuals.widgets.open.corner_radius = 4.into();
    });

    context.style_mut_of(egui::Theme::Light, |style| {
        let visuals = &mut style.visuals;
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
    });

    context.style_mut_of(egui::Theme::Dark, |style| {
        let visuals = &mut style.visuals;
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
        visuals.selection.stroke =
            egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(235, 241, 250));
    });
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

        AppTheme::Light.apply(&context);
        assert_eq!(context.theme(), egui::Theme::Light);
        AppTheme::Dark.apply(&context);
        assert_eq!(context.theme(), egui::Theme::Dark);
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
}
