//! Colours, type scale and small painted widgets.

use eframe::egui::{
    self, Color32, FontId, Response, RichText, Rounding, Sense, Stroke, TextStyle, Ui, Vec2,
};

pub const BG: Color32 = Color32::from_rgb(0x0D, 0x10, 0x15);
pub const PANEL: Color32 = Color32::from_rgb(0x13, 0x17, 0x1E);
pub const CARD: Color32 = Color32::from_rgb(0x1A, 0x20, 0x2A);
pub const CARD_HOVER: Color32 = Color32::from_rgb(0x1F, 0x27, 0x33);
pub const BUTTON: Color32 = Color32::from_rgb(0x26, 0x2F, 0x3C);
pub const BUTTON_HOVER: Color32 = Color32::from_rgb(0x30, 0x3B, 0x4B);
pub const STROKE: Color32 = Color32::from_rgb(0x2B, 0x35, 0x43);
pub const TEXT: Color32 = Color32::from_rgb(0xE8, 0xED, 0xF4);
pub const DIM: Color32 = Color32::from_rgb(0x8B, 0x97, 0xA9);
pub const FAINT: Color32 = Color32::from_rgb(0x5C, 0x67, 0x78);
pub const ACCENT: Color32 = Color32::from_rgb(0x4D, 0xA3, 0xFF);
pub const GREEN: Color32 = Color32::from_rgb(0x3D, 0xD6, 0x8C);
pub const AMBER: Color32 = Color32::from_rgb(0xF5, 0xB5, 0x44);
pub const RED: Color32 = Color32::from_rgb(0xFF, 0x5D, 0x5D);
pub const COOL: Color32 = Color32::from_rgb(0x5B, 0xC0, 0xEB);

/// Opaque blend of `a` toward `b`.
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t.clamp(0.0, 1.0)).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

pub fn apply(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = PANEL;
    v.window_stroke = Stroke::new(1.0_f32, STROKE);
    v.window_rounding = Rounding::same(14.0);
    v.menu_rounding = Rounding::same(10.0);
    v.extreme_bg_color = Color32::from_rgb(0x0A, 0x0D, 0x11);
    v.faint_bg_color = CARD;
    v.override_text_color = Some(TEXT);
    v.hyperlink_color = ACCENT;
    v.selection.bg_fill = mix(PANEL, ACCENT, 0.45);
    v.selection.stroke = Stroke::new(1.0_f32, TEXT);
    v.popup_shadow = egui::Shadow {
        offset: [0.0, 8.0].into(),
        blur: 24.0,
        spread: 0.0,
        color: Color32::from_black_alpha(120),
    };
    v.window_shadow = egui::Shadow {
        offset: [0.0, 16.0].into(),
        blur: 48.0,
        spread: 0.0,
        color: Color32::from_black_alpha(160),
    };

    let r = Rounding::same(8.0);
    v.widgets.noninteractive.bg_fill = PANEL;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, STROKE);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    v.widgets.noninteractive.rounding = r;
    v.widgets.inactive.bg_fill = BUTTON;
    v.widgets.inactive.weak_bg_fill = BUTTON;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, STROKE);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    v.widgets.inactive.rounding = r;
    v.widgets.hovered.bg_fill = BUTTON_HOVER;
    v.widgets.hovered.weak_bg_fill = BUTTON_HOVER;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, mix(STROKE, TEXT, 0.25));
    v.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, TEXT);
    v.widgets.hovered.rounding = r;
    v.widgets.hovered.expansion = 0.0;
    v.widgets.active.bg_fill = mix(BUTTON_HOVER, ACCENT, 0.3);
    v.widgets.active.weak_bg_fill = mix(BUTTON_HOVER, ACCENT, 0.3);
    v.widgets.active.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    v.widgets.active.fg_stroke = Stroke::new(1.0_f32, TEXT);
    v.widgets.active.rounding = r;
    v.widgets.active.expansion = 0.0;
    v.widgets.open.bg_fill = BUTTON_HOVER;
    v.widgets.open.weak_bg_fill = BUTTON_HOVER;
    v.widgets.open.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    v.widgets.open.rounding = r;
    ctx.set_visuals(v);

    ctx.style_mut(|s| {
        s.text_styles
            .insert(TextStyle::Body, FontId::proportional(14.0));
        s.text_styles
            .insert(TextStyle::Button, FontId::proportional(14.0));
        s.text_styles
            .insert(TextStyle::Heading, FontId::proportional(22.0));
        s.text_styles
            .insert(TextStyle::Small, FontId::proportional(12.0));
        s.text_styles
            .insert(TextStyle::Monospace, FontId::monospace(12.0));
        s.spacing.item_spacing = Vec2::new(8.0, 8.0);
        s.spacing.button_padding = Vec2::new(12.0, 6.0);
        s.spacing.interact_size.y = 30.0;
        s.spacing.window_margin = egui::Margin::same(18.0);
        s.spacing.menu_margin = egui::Margin::same(8.0);
        s.interaction.selectable_labels = false;
    });
}

pub fn dim(text: impl Into<String>) -> RichText {
    RichText::new(text).color(DIM)
}

pub fn small(text: impl Into<String>) -> RichText {
    RichText::new(text).color(DIM).size(12.0)
}

/// Section heading in small caps style.
pub fn section(ui: &mut Ui, text: &str) {
    ui.add_space(6.0);
    ui.label(
        RichText::new(text.to_uppercase())
            .color(FAINT)
            .size(11.0)
            .strong(),
    );
}

/// Rounded status tag, e.g. `ON` or `OFFLINE`.
pub fn pill(ui: &mut Ui, text: &str, color: Color32) -> Response {
    let font = FontId::proportional(11.0);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, color);
    let size = galley.size() + Vec2::new(16.0, 8.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect(
        rect,
        size.y / 2.0,
        mix(CARD, color, 0.16),
        Stroke::new(1.0_f32, mix(CARD, color, 0.45)),
    );
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, color);
    resp
}

pub fn dot(ui: &mut Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
    ui.painter()
        .circle_filled(rect.center(), 6.5, mix(CARD, color, 0.25));
    ui.painter().circle_filled(rect.center(), 4.0, color);
}

/// A button that lights up in `active` colour when its function is engaged.
pub fn toggle(
    ui: &mut Ui,
    size: Vec2,
    text: &str,
    active: Option<Color32>,
    enabled: bool,
) -> Response {
    let (fill, fg, stroke) = match active {
        Some(c) if enabled => (mix(CARD, c, 0.22), c, mix(CARD, c, 0.6)),
        _ => (BUTTON, TEXT, STROKE),
    };
    ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(text).color(fg).size(13.0))
            .fill(fill)
            .stroke(Stroke::new(1.0_f32, stroke))
            .rounding(8.0)
            .min_size(size),
    )
}

/// Filled call-to-action button.
pub fn primary(ui: &mut Ui, text: &str, color: Color32) -> Response {
    ui.add(
        egui::Button::new(
            RichText::new(text)
                .color(Color32::from_rgb(0x08, 0x0B, 0x10))
                .strong(),
        )
        .fill(color)
        .stroke(Stroke::NONE)
        .rounding(8.0),
    )
}

pub fn thousands(n: u32) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// How a state button is drawn: the colour shows the present state, the
/// label says what a click will do.
#[derive(Clone, Copy, PartialEq)]
pub enum Look {
    Solid {
        fill: Color32,
        text: Color32,
    },
    /// A group whose members are in different states.
    Striped(Color32, Color32),
    Neutral,
}

const INK: Color32 = Color32::from_rgb(0x08, 0x0B, 0x10);
/// Shutter closed: black. Shutter open: white.
pub const SHUTTER_CLOSED: Look = Look::Solid {
    fill: Color32::from_rgb(0x04, 0x05, 0x07),
    text: TEXT,
};
pub const SHUTTER_OPEN: Look = Look::Solid {
    fill: Color32::from_rgb(0xF2, 0xF4, 0xF7),
    text: INK,
};
/// Power on: green. Power off: red.
pub const POWER_ON: Look = Look::Solid {
    fill: GREEN,
    text: INK,
};
pub const POWER_OFF: Look = Look::Solid {
    fill: Color32::from_rgb(0xD9, 0x48, 0x48),
    text: Color32::WHITE,
};

impl Look {
    fn fill(self) -> Color32 {
        match self {
            Look::Solid { fill, .. } => fill,
            _ => BUTTON,
        }
    }
    pub fn striped(a: Look, b: Look) -> Look {
        Look::Striped(a.fill(), b.fill())
    }
}

pub fn state_button(
    ui: &mut Ui,
    min_size: Vec2,
    text: &str,
    font: f32,
    look: Look,
    enabled: bool,
) -> Response {
    let look = if enabled { look } else { Look::Neutral };
    let text_color = match look {
        Look::Solid { text, .. } => text,
        Look::Striped(..) => TEXT,
        Look::Neutral if enabled => TEXT,
        Look::Neutral => FAINT,
    };
    let galley =
        ui.painter()
            .layout_no_wrap(text.to_string(), FontId::proportional(font), text_color);
    let size = Vec2::new(
        min_size.x.max(galley.size().x + 12.0),
        min_size.y.max(galley.size().y + 6.0),
    );
    let (rect, resp) = ui.allocate_exact_size(
        size,
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    if !ui.is_rect_visible(rect) {
        return resp;
    }
    let lift = if resp.is_pointer_button_down_on() {
        0.22
    } else if resp.hovered() {
        0.12
    } else {
        0.0
    };
    let p = ui.painter();
    let text_pos = rect.center() - galley.size() / 2.0;
    match look {
        Look::Solid { fill, text } => {
            // Lighten dark buttons and darken light ones on hover.
            p.rect(
                rect,
                8.0,
                mix(fill, text, lift),
                Stroke::new(1.0_f32, mix(fill, text, 0.35)),
            );
        }
        Look::Striped(a, b) => {
            p.rect_filled(rect, 8.0, b);
            let clip = p.with_clip_rect(rect.shrink(1.0));
            let h = rect.height();
            let mut x = rect.left() - h;
            while x < rect.right() {
                clip.line_segment(
                    [egui::pos2(x, rect.bottom()), egui::pos2(x + h, rect.top())],
                    Stroke::new(6.0_f32, a),
                );
                x += 16.0;
            }
            p.rect_stroke(
                rect,
                8.0,
                Stroke::new(1.5_f32, mix(STROKE, TEXT, 0.3 + lift)),
            );
            // Text needs a plain backing to stay readable over the stripes.
            let back =
                egui::Rect::from_min_size(text_pos, galley.size()).expand2(Vec2::new(5.0, 1.0));
            p.rect_filled(
                back,
                5.0,
                Color32::from_rgba_unmultiplied(0x10, 0x14, 0x1B, 235),
            );
        }
        Look::Neutral => {
            let fill = if enabled {
                mix(BUTTON, TEXT, lift * 0.5)
            } else {
                mix(BUTTON, BG, 0.45)
            };
            p.rect(rect, 8.0, fill, Stroke::new(1.0_f32, STROKE));
        }
    }
    p.galley(text_pos, galley, text_color);
    resp
}

/// The present state, shown between the two buttons that change it.
pub fn status_bar(ui: &mut Ui, size: Vec2, text: &str, look: Look) {
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let bar = rect.shrink2(Vec2::new(0.0, 4.0));
    let (fill, color, stroke) = match look {
        Look::Solid { fill, text } => (fill, text, mix(fill, text, 0.35)),
        _ => (mix(BUTTON, BG, 0.5), FAINT, STROKE),
    };
    ui.painter()
        .rect(bar, 6.0, fill, Stroke::new(1.0_f32, stroke));
    ui.painter().text(
        bar.center(),
        egui::Align2::CENTER_CENTER,
        text,
        FontId::proportional(11.5),
        color,
    );
}
