use std::time::Duration;

use eframe::egui;

use crate::theme::ThemeIdleEffect;

pub const GREEN_SCREEN_IDLE_DELAY: Duration = Duration::from_mins(33);
const ACTIVE_FRAME_INTERVAL: Duration = Duration::from_millis(33);
const COLUMN_WIDTH: f32 = 20.0;
const GLYPH_SIZE: f32 = 15.0;
const MAX_COLUMNS: usize = 96;
const MIN_TRAIL_LENGTH: usize = 8;
const TRAIL_LENGTH_VARIATION: usize = 8;
const RAIN_SYMBOLS: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ{}[]<>/\\|+-=*";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct IdleDecision {
    active: bool,
    repaint_after: Option<Duration>,
}

#[derive(Default)]
pub struct IdleScreen {
    last_activity_at: Option<f64>,
    active: bool,
    #[cfg(feature = "screenshot-qa")]
    force_capture: bool,
}

impl IdleScreen {
    pub fn show(&mut self, context: &egui::Context, effect: Option<ThemeIdleEffect>) -> bool {
        let (now, focused, input_activity, close_requested, viewport) = context.input(|input| {
            (
                input.time,
                input.focused,
                input.events.iter().any(is_user_activity),
                input.viewport().close_requested(),
                input.viewport_rect(),
            )
        });
        let was_active = self.active;
        #[cfg(feature = "screenshot-qa")]
        let decision = if self.force_capture && effect == Some(ThemeIdleEffect::DigitalRain) {
            IdleDecision {
                active: true,
                repaint_after: Some(ACTIVE_FRAME_INTERVAL),
            }
        } else {
            self.advance(now, focused, input_activity || close_requested, effect)
        };
        #[cfg(not(feature = "screenshot-qa"))]
        let decision = self.advance(now, focused, input_activity || close_requested, effect);
        let hold_dismissal_frame = was_active && input_activity && !close_requested;
        let blocks_application_ui = decision.active || hold_dismissal_frame;
        if blocks_application_ui {
            paint_digital_rain(context, viewport, now);
        }
        if hold_dismissal_frame {
            // Do not let the key or pointer action used to dismiss the idle
            // layer edit the document underneath it. The next frame restores
            // the application after egui has consumed that input event.
            context.request_repaint();
        }
        if let Some(delay) = decision.repaint_after {
            context.request_repaint_after(delay);
        }
        blocks_application_ui
    }

    #[cfg(feature = "screenshot-qa")]
    pub const fn force_active_for_capture(&mut self) {
        self.force_capture = true;
    }

    fn advance(
        &mut self,
        now: f64,
        focused: bool,
        activity: bool,
        effect: Option<ThemeIdleEffect>,
    ) -> IdleDecision {
        let now = if now.is_finite() && now >= 0.0 {
            now
        } else {
            self.last_activity_at.unwrap_or(0.0)
        };
        let clock_moved_back = self
            .last_activity_at
            .is_some_and(|last_activity| now < last_activity);
        if effect.is_none() || !focused || activity || clock_moved_back {
            self.last_activity_at = Some(now);
            self.active = false;
            return IdleDecision {
                active: false,
                repaint_after: (effect.is_some() && focused).then_some(GREEN_SCREEN_IDLE_DELAY),
            };
        }

        let last_activity = *self.last_activity_at.get_or_insert(now);
        let idle_seconds = (now - last_activity).max(0.0);
        if idle_seconds >= GREEN_SCREEN_IDLE_DELAY.as_secs_f64() {
            self.active = true;
            IdleDecision {
                active: true,
                repaint_after: Some(ACTIVE_FRAME_INTERVAL),
            }
        } else {
            self.active = false;
            IdleDecision {
                active: false,
                repaint_after: Some(Duration::from_secs_f64(
                    GREEN_SCREEN_IDLE_DELAY.as_secs_f64() - idle_seconds,
                )),
            }
        }
    }
}

const fn is_user_activity(event: &egui::Event) -> bool {
    matches!(
        event,
        egui::Event::Copy
            | egui::Event::Cut
            | egui::Event::Paste(_)
            | egui::Event::Text(_)
            | egui::Event::Key { .. }
            | egui::Event::PointerMoved(_)
            | egui::Event::MouseMoved(_)
            | egui::Event::PointerButton { .. }
            | egui::Event::PointerGone
            | egui::Event::Zoom(_)
            | egui::Event::Rotate(_)
            | egui::Event::Ime(_)
            | egui::Event::Touch { .. }
            | egui::Event::MouseWheel { .. }
            | egui::Event::WindowFocused(true)
            | egui::Event::AccessKitActionRequest(_)
    )
}

fn paint_digital_rain(context: &egui::Context, viewport: egui::Rect, now: f64) {
    if !viewport.is_positive() {
        return;
    }
    let painter = context.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("noter-green-screen-idle"),
    ));
    painter.rect_filled(viewport, 0.0, egui::Color32::from_rgb(1, 7, 3));

    let columns = rain_column_count(viewport.width());
    let font = egui::FontId::monospace(GLYPH_SIZE);
    let dim = egui::Color32::from_rgb(38, 128, 51);
    let head = egui::Color32::from_rgb(174, 255, 181);
    let animation_time = if now.is_finite() && now >= 0.0 {
        now
    } else {
        0.0
    };
    let frame = Duration::try_from_secs_f64(animation_time).map_or(u64::MAX, |duration| {
        u64::try_from(duration.as_millis() / 100).unwrap_or(u64::MAX)
    });
    for column in 0..columns {
        let column_hash = mix64(column as u64 ^ 0xA076_1D64_78BD_642F);
        let trail_length = MIN_TRAIL_LENGTH + column_hash as usize % TRAIL_LENGTH_VARIATION;
        let trail_height = trail_length as f32 * GLYPH_SIZE;
        let fall_rate = 34.0 + (column_hash >> 16) as f32 % 48.0;
        let cycle = GLYPH_SIZE.mul_add(4.0, viewport.height() + trail_height);
        let phase = animation_time.mul_add(f64::from(fall_rate), (column_hash >> 32) as f64)
            % f64::from(cycle);
        let head_y = viewport.top() + phase as f32 - trail_height;
        let x = (column as f32 + 0.5).mul_add(COLUMN_WIDTH, viewport.left());
        let mut trail = String::with_capacity(trail_length * 2);
        for row in 0..trail_length {
            if row > 0 {
                trail.push('\n');
            }
            trail.push(rain_symbol(column, row, frame));
        }
        painter.text(
            egui::pos2(x, head_y),
            egui::Align2::CENTER_BOTTOM,
            trail,
            font.clone(),
            dim,
        );
        painter.text(
            egui::pos2(x, head_y),
            egui::Align2::CENTER_TOP,
            rain_symbol(column, trail_length, frame),
            font.clone(),
            head,
        );
    }

    painter.text(
        viewport.right_bottom() - egui::vec2(14.0, 12.0),
        egui::Align2::RIGHT_BOTTOM,
        "MOVE POINTER OR PRESS A KEY",
        egui::FontId::monospace(11.0),
        egui::Color32::from_rgb(74, 145, 82),
    );
}

fn rain_column_count(width: f32) -> usize {
    let mut columns = 1_usize;
    while columns < MAX_COLUMNS && (columns + 1) as f32 * COLUMN_WIDTH <= width.max(0.0) {
        columns += 1;
    }
    columns
}

fn rain_symbol(column: usize, row: usize, frame: u64) -> char {
    let mixed = mix64(
        column as u64
            ^ (row as u64).rotate_left(21)
            ^ frame.rotate_left(42)
            ^ 0xE703_7ED1_A0B4_28DB,
    );
    char::from(RAIN_SYMBOLS[mixed as usize % RAIN_SYMBOLS.len()])
}

const fn mix64(mut value: u64) -> u64 {
    value ^= value >> 30;
    value = value.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    value ^= value >> 27;
    value = value.wrapping_mul(0x94D0_49BB_1331_11EB);
    value ^ (value >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EFFECT: Option<ThemeIdleEffect> = Some(ThemeIdleEffect::DigitalRain);

    #[test]
    fn effect_starts_at_exactly_thirty_three_idle_minutes() {
        let mut screen = IdleScreen::default();

        let initial = screen.advance(10.0, true, false, EFFECT);
        let before = screen.advance(
            10.0 + GREEN_SCREEN_IDLE_DELAY.as_secs_f64() - 0.25,
            true,
            false,
            EFFECT,
        );
        let active = screen.advance(
            10.0 + GREEN_SCREEN_IDLE_DELAY.as_secs_f64(),
            true,
            false,
            EFFECT,
        );

        assert_eq!(initial.repaint_after, Some(GREEN_SCREEN_IDLE_DELAY));
        assert!(!before.active);
        assert_eq!(before.repaint_after, Some(Duration::from_millis(250)));
        assert_eq!(
            active,
            IdleDecision {
                active: true,
                repaint_after: Some(ACTIVE_FRAME_INTERVAL),
            }
        );
    }

    #[test]
    fn any_activity_dismisses_effect_and_restarts_the_full_delay() {
        let mut screen = IdleScreen::default();
        let _ = screen.advance(0.0, true, false, EFFECT);
        assert!(
            screen
                .advance(GREEN_SCREEN_IDLE_DELAY.as_secs_f64(), true, false, EFFECT)
                .active
        );

        let dismissed = screen.advance(
            GREEN_SCREEN_IDLE_DELAY.as_secs_f64() + 1.0,
            true,
            true,
            EFFECT,
        );

        assert_eq!(
            dismissed,
            IdleDecision {
                active: false,
                repaint_after: Some(GREEN_SCREEN_IDLE_DELAY),
            }
        );
    }

    #[test]
    fn disabled_theme_focus_loss_and_clock_reset_cannot_leave_effect_active() {
        let mut screen = IdleScreen::default();
        let _ = screen.advance(0.0, true, false, EFFECT);
        let _ = screen.advance(GREEN_SCREEN_IDLE_DELAY.as_secs_f64(), true, false, EFFECT);

        assert!(!screen.advance(2_000.0, true, false, None).active);
        assert!(!screen.advance(4_000.0, false, false, EFFECT).active);
        assert!(!screen.advance(3_000.0, true, false, EFFECT).active);
    }

    #[test]
    fn screenshot_and_focus_loss_are_not_misclassified_as_user_activity() {
        let screenshot = egui::Event::Screenshot {
            viewport_id: egui::ViewportId::ROOT,
            user_data: egui::UserData::default(),
            image: std::sync::Arc::new(egui::ColorImage::new([1, 1], vec![egui::Color32::BLACK])),
        };

        assert!(!is_user_activity(&screenshot));
        assert!(!is_user_activity(&egui::Event::WindowFocused(false)));
        assert!(is_user_activity(&egui::Event::Text("x".to_owned())));
        assert!(is_user_activity(&egui::Event::PointerMoved(
            egui::Pos2::ZERO
        )));
    }

    #[test]
    fn rain_generation_is_deterministic_and_strictly_bounded() {
        assert_eq!(rain_column_count(1_200.0), 60);
        assert_eq!(rain_column_count(100_000.0), MAX_COLUMNS);
        assert_eq!(rain_column_count(-1.0), 1);
        assert_eq!(rain_symbol(4, 7, 11), rain_symbol(4, 7, 11));
        assert!(rain_symbol(4, 7, 11).is_ascii());
    }

    #[test]
    fn active_frame_paints_a_bounded_full_window_layer() {
        fn collect_text(shape: &egui::Shape, rendered: &mut Vec<String>) {
            match shape {
                egui::Shape::Text(text) => rendered.push(text.galley.job.text.clone()),
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        collect_text(shape, rendered);
                    }
                }
                _ => {}
            }
        }

        let context = egui::Context::default();
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(640.0, 360.0));
        let mut screen = IdleScreen::default();
        let initial_input = egui::RawInput {
            screen_rect: Some(viewport),
            time: Some(0.0),
            focused: true,
            ..Default::default()
        };
        let _ = context.run_ui(initial_input, |ui| {
            assert!(!screen.show(ui.ctx(), EFFECT));
        });

        let active_input = egui::RawInput {
            screen_rect: Some(viewport),
            time: Some(GREEN_SCREEN_IDLE_DELAY.as_secs_f64()),
            focused: true,
            ..Default::default()
        };
        let output = context.run_ui(active_input, |ui| {
            assert!(screen.show(ui.ctx(), EFFECT));
        });
        let mut rendered = Vec::new();
        for shape in &output.shapes {
            collect_text(&shape.shape, &mut rendered);
        }

        assert!(
            rendered
                .iter()
                .any(|text| text == "MOVE POINTER OR PRESS A KEY")
        );
        assert!(rendered.len() <= MAX_COLUMNS * 2 + 1);
        assert!(rendered.iter().all(|text| text.is_ascii()));
    }

    #[test]
    fn dismissal_input_is_consumed_before_the_application_ui_returns() {
        let context = egui::Context::default();
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(640.0, 360.0));
        let mut screen = IdleScreen::default();
        for time in [0.0, GREEN_SCREEN_IDLE_DELAY.as_secs_f64()] {
            let input = egui::RawInput {
                screen_rect: Some(viewport),
                time: Some(time),
                focused: true,
                ..Default::default()
            };
            let _ = context.run_ui(input, |ui| {
                let visible = screen.show(ui.ctx(), EFFECT);
                assert_eq!(visible, time > 0.0);
            });
        }

        let mut dismissal = egui::RawInput {
            screen_rect: Some(viewport),
            time: Some(GREEN_SCREEN_IDLE_DELAY.as_secs_f64() + 1.0),
            focused: true,
            ..Default::default()
        };
        dismissal.events.push(egui::Event::Text("x".to_owned()));
        let _ = context.run_ui(dismissal, |ui| {
            assert!(screen.show(ui.ctx(), EFFECT));
            assert!(!screen.active);
        });

        let restored = egui::RawInput {
            screen_rect: Some(viewport),
            time: Some(GREEN_SCREEN_IDLE_DELAY.as_secs_f64() + 1.1),
            focused: true,
            ..Default::default()
        };
        let _ = context.run_ui(restored, |ui| {
            assert!(!screen.show(ui.ctx(), EFFECT));
        });
    }

    #[test]
    fn native_close_request_is_never_held_behind_the_idle_layer() {
        let context = egui::Context::default();
        let mut screen = IdleScreen::default();
        let initial = egui::RawInput {
            time: Some(0.0),
            focused: true,
            ..Default::default()
        };
        let _ = context.run_ui(initial, |ui| {
            assert!(!screen.show(ui.ctx(), EFFECT));
        });
        let active = egui::RawInput {
            time: Some(GREEN_SCREEN_IDLE_DELAY.as_secs_f64()),
            focused: true,
            ..Default::default()
        };
        let _ = context.run_ui(active, |ui| {
            assert!(screen.show(ui.ctx(), EFFECT));
        });

        let mut close = egui::RawInput {
            time: Some(GREEN_SCREEN_IDLE_DELAY.as_secs_f64() + 1.0),
            focused: true,
            ..Default::default()
        };
        close
            .viewports
            .entry(egui::ViewportId::ROOT)
            .or_default()
            .events
            .push(egui::ViewportEvent::Close);
        let _ = context.run_ui(close, |ui| {
            assert!(!screen.show(ui.ctx(), EFFECT));
            assert!(!screen.active);
        });
    }
}
