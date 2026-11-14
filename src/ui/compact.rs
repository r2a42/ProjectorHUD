//! The two small windows. Medium is a short list: one projector per row with
//! labelled switches. Small is a narrow strip with one picture button each for
//! power and shutter. Either one is the whole app, not a pane of the large
//! window, so it can sit in a corner of the screen during a show.

use super::*;
use egui::{Id, Rect};

const BTN: f32 = 22.0;
const PAD: f32 = 10.0;
const ICON: Vec2 = Vec2::new(26.0, BTN);
/// The smallest and the starting size of the large window.
pub(super) const LARGE_MIN: Vec2 = Vec2::new(920.0, 560.0);
pub(super) const LARGE_DEFAULT: Vec2 = Vec2::new(1280.0, 820.0);
/// Room a confirmation needs, however few rows are showing.
pub(super) const CONFIRM_H: f32 = 170.0;

pub(super) fn width(size: WindowSize) -> f32 {
    match size {
        WindowSize::Large => LARGE_DEFAULT.x,
        WindowSize::Medium => 440.0,
        WindowSize::Small => 280.0,
    }
}

/// Heights of the top row, a group's row and a projector's row.
fn heights(size: WindowSize) -> (f32, f32, f32) {
    match size {
        WindowSize::Small => (36.0, 30.0, 30.0),
        _ => (38.0, 32.0, 42.0),
    }
}

#[derive(Clone, Copy)]
enum Face<'a> {
    Text(&'a str),
    Icon(Icon),
}

struct Seg<'a> {
    face: Face<'a>,
    lit: Option<Tint>,
    tip: &'a str,
}

/// Buttons joined in one outline. Returns the one that was clicked.
fn segmented(ui: &mut egui::Ui, id: Id, enabled: bool, segs: &[Seg]) -> Option<usize> {
    let w_of = |s: &Seg| match s.face {
        Face::Text(t) => (12.0 + 7.0 * t.chars().count() as f32).max(22.0),
        Face::Icon(_) => 26.0,
    };
    let total: f32 = segs.iter().map(w_of).sum();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(total, BTN), Sense::hover());
    let p = ui.painter().clone();
    p.rect_filled(
        rect,
        6.0,
        if enabled {
            BUTTON
        } else {
            mix(BUTTON, BG, 0.5)
        },
    );
    let mut clicked = None;
    let mut x = rect.left();
    for (i, s) in segs.iter().enumerate() {
        let w = w_of(s);
        let r = Rect::from_min_size(egui::pos2(x, rect.top()), Vec2::new(w, BTN));
        x += w;
        let sense = if enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        let mut resp = ui.interact(r, id.with(i), sense);
        if !s.tip.is_empty() {
            resp = resp.on_hover_text(s.tip);
        }
        let (l, rr) = (
            if i == 0 { 6.0 } else { 0.0 },
            if i + 1 == segs.len() { 6.0 } else { 0.0 },
        );
        let rounding = egui::Rounding {
            nw: l,
            sw: l,
            ne: rr,
            se: rr,
        };
        let ink = match s.lit {
            Some(t) => {
                p.rect_filled(r, rounding, t.fill);
                t.ink
            }
            None if enabled && resp.hovered() => {
                p.rect_filled(r, rounding, BUTTON_HOVER);
                TEXT
            }
            None if enabled => DIM,
            None => mix(FAINT, BG, 0.35),
        };
        match s.face {
            Face::Text(t) => {
                p.text(
                    r.center(),
                    egui::Align2::CENTER_CENTER,
                    t,
                    egui::FontId::proportional(12.0),
                    ink,
                );
            }
            Face::Icon(icon) => paint_icon(&p, r, icon, ink),
        }
        if resp.clicked() {
            clicked = Some(i);
        }
    }
    p.rect_stroke(rect, 6.0, Stroke::new(1.0_f32, STROKE));
    clicked
}

/// Always-on-top and the size switch, for the top row of every window.
/// Meant for a right-to-left row: the size switch ends up outermost.
pub(super) fn window_toggles(ui: &mut egui::Ui, cfg: &AppConfig, acts: &mut Vec<Act>) {
    let tips = [
        "Small window: a strip of buttons",
        "Medium window: a short list",
        "Large window: everything",
    ];
    let names = ["S", "M", "L"];
    let segs: Vec<Seg> = WindowSize::ALL
        .iter()
        .enumerate()
        .map(|(i, s)| Seg {
            face: Face::Text(names[i]),
            lit: (cfg.size == *s).then(|| tint(ACCENT)),
            tip: tips[i],
        })
        .collect();
    if let Some(i) = segmented(ui, Id::new("window-size"), true, &segs) {
        acts.push(Act::SetSize(WindowSize::ALL[i]));
    }
    if icon_button(ui, ICON, Icon::Pin, true, cfg.on_top.then(|| tint(ACCENT)))
        .on_hover_text(if cfg.on_top {
            "Staying on top of other windows. Click to stop."
        } else {
            "Keep this window on top of other windows"
        })
        .clicked()
    {
        acts.push(Act::SetOnTop(!cfg.on_top));
    }
}

/// A full-width row. `controls` are laid out from its right edge; what is
/// left over, for the name, is returned.
fn strip(ui: &mut egui::Ui, h: f32, fill: Color32, controls: impl FnOnce(&mut egui::Ui)) -> Rect {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), h), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, fill);
    ui.painter().hline(
        rect.x_range(),
        rect.bottom() - 0.5,
        Stroke::new(1.0_f32, STROKE),
    );
    let inner = rect.shrink2(Vec2::new(PAD, 0.0));
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(Layout::right_to_left(Align::Center)),
    );
    child.spacing_mut().item_spacing.x = 6.0;
    controls(&mut child);
    let used = child.min_rect().left().min(inner.right());
    Rect::from_min_max(
        inner.min,
        egui::pos2((used - 8.0).max(inner.left()), inner.bottom()),
    )
}

/// One or two lines of text, centred vertically and cut short with "…".
fn lines(ui: &mut egui::Ui, rect: Rect, top: RichText, sub: Option<RichText>) {
    if rect.width() < 8.0 {
        return;
    }
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(Layout::top_down(Align::Min)),
    );
    child.set_clip_rect(rect.intersect(ui.clip_rect()));
    child.spacing_mut().item_spacing.y = 0.0;
    let text_h = if sub.is_some() { 32.0 } else { 17.0 };
    child.add_space(((rect.height() - text_h) / 2.0).max(0.0));
    child.add(egui::Label::new(top).truncate().selectable(false));
    if let Some(sub) = sub {
        child.add(egui::Label::new(sub).truncate().selectable(false));
    }
}

/// Which side of the power switch is lit, and in what colour.
fn power_side(st: &ProjectorState) -> Option<(bool, Tint)> {
    if st.link != Link::Online {
        return None;
    }
    match st.power {
        Power::On => Some((true, tint(GREEN))),
        Power::Warming => Some((true, tint(AMBER))),
        Power::Cooling => Some((false, tint(COOL))),
        Power::Standby => Some((false, TINT_PLAIN)),
        Power::Unknown => None,
    }
}

/// What to say about a projector under its name, and how urgently.
fn about(cfg: &ProjectorConfig, st: &ProjectorState) -> (String, Color32) {
    match st.link {
        Link::Offline => ("Offline".into(), RED),
        Link::AuthFailed => ("Wrong password".into(), RED),
        Link::Connecting => ("Connecting…".into(), FAINT),
        Link::Online => {
            let issues: Vec<String> = st.issues().into_iter().map(|i| i.0).collect();
            if !issues.is_empty() {
                (issues.join(", "), sev_color(st.severity()))
            } else {
                match st.power {
                    Power::Warming => ("Warming up".into(), AMBER),
                    Power::Cooling => ("Cooling down".into(), COOL),
                    _ => (subtitle(cfg, st), FAINT),
                }
            }
        }
    }
}

/// Power and shutter for one projector, laid out right to left.
fn projector_controls(
    ui: &mut egui::Ui,
    small: bool,
    cfg: &ProjectorConfig,
    st: &ProjectorState,
    detail_open: bool,
    acts: &mut Vec<Act>,
) {
    let id = Id::new(("compact", cfg.id));
    let online = st.link == Link::Online;
    let side = power_side(st);
    let can_shutter = online && st.power == Power::On;
    let closed = can_shutter && st.video_mute == Some(true);
    let open = can_shutter && st.video_mute == Some(false);
    let mut shutter = None;
    let mut power = None;

    if small {
        let (icon, tip) = if closed {
            (Icon::Close, "Shutter closed. Click to open.")
        } else if open {
            (Icon::Open, "Shutter open. Click to close.")
        } else {
            (Icon::Open, "Shutter")
        };
        if icon_button(ui, ICON, icon, can_shutter, closed.then_some(TINT_CLOSED))
            .on_hover_text(tip)
            .clicked()
        {
            shutter = Some(!closed);
        }
        let (lit, tip) = match side {
            Some((true, t)) => (
                Some(t),
                if st.power == Power::Warming {
                    "Warming up. Click to power off."
                } else {
                    "On. Click to power off."
                },
            ),
            Some((false, t)) if st.power == Power::Cooling => {
                (Some(t), "Cooling down. Click to power on.")
            }
            Some((false, _)) => (None, "Off. Click to power on."),
            None => (None, "Power"),
        };
        if icon_button(ui, ICON, Icon::Power, online, lit)
            .on_hover_text(tip)
            .clicked()
        {
            power = Some(!matches!(side, Some((true, _))));
        }
    } else {
        if icon_button(
            ui,
            ICON,
            Icon::Info,
            true,
            detail_open.then(|| tint(ACCENT)),
        )
        .on_hover_text("Details, in a window of their own")
        .clicked()
        {
            acts.push(Act::Detail((!detail_open).then_some(cfg.id)));
        }
        let segs = [
            Seg {
                face: Face::Icon(Icon::Open),
                lit: open.then_some(TINT_OPEN),
                tip: "Open the shutter",
            },
            Seg {
                face: Face::Icon(Icon::Close),
                lit: closed.then_some(TINT_CLOSED),
                tip: "Close the shutter",
            },
        ];
        if let Some(i) = segmented(ui, id.with("shutter"), can_shutter, &segs) {
            shutter = Some(i == 1);
        }
        let lit = |on: bool| side.filter(|s| s.0 == on).map(|s| s.1);
        let segs = [
            Seg {
                face: Face::Text("On"),
                lit: lit(true),
                tip: "Power on",
            },
            Seg {
                face: Face::Text("Off"),
                lit: lit(false),
                tip: "Power off",
            },
        ];
        if let Some(i) = segmented(ui, id.with("power"), online, &segs) {
            power = Some(i == 0);
        }
    }

    if let Some(close) = shutter {
        acts.push(Act::Send(cfg.id, Command::VideoMute(close)));
    }
    match power {
        Some(true) => acts.push(Act::Send(cfg.id, Command::PowerOn)),
        Some(false) => acts.push(power_off_confirm(cfg)),
        None => {}
    }
}

/// A picture button for a set of projectors. When they all agree it flips
/// them to the other state (`flip`); when they differ it asks which.
#[allow(clippy::too_many_arguments)]
fn choice_button(
    ui: &mut egui::Ui,
    id: Id,
    icon: Icon,
    enabled: bool,
    lit: Option<Tint>,
    tip: &str,
    flip: Option<usize>,
    choices: [&str; 2],
) -> Option<usize> {
    let resp = icon_button(ui, ICON, icon, enabled, lit).on_hover_text(tip);
    if resp.clicked() {
        if flip.is_some() {
            return flip;
        }
        ui.memory_mut(|m| m.toggle_popup(id));
    }
    let mut out = None;
    egui::popup_below_widget(
        ui,
        id,
        &resp,
        egui::PopupCloseBehavior::CloseOnClick,
        |ui| {
            ui.set_min_width(130.0);
            for (i, c) in choices.iter().enumerate() {
                if ui.button(*c).clicked() {
                    out = Some(i);
                }
            }
        },
    );
    out
}

/// Power and shutter for a set of projectors, laid out right to left.
fn set_controls(
    ui: &mut egui::Ui,
    id: Id,
    small: bool,
    members: &[&Item],
    scope: &str,
    acts: &mut Vec<Act>,
) {
    let ids: Vec<u64> = members.iter().map(|i| i.0.id).collect();
    let power = power_state(members);
    let shutter = shutter_state(members);
    let any = !members.is_empty();
    let mut set_shutter = None;
    let mut set_power = None;

    if small {
        let (icon, lit, tip, flip) = match shutter {
            Tri::AllOn => (
                Icon::Close,
                Some(TINT_CLOSED),
                "Shutters closed. Click to open them.",
                Some(0),
            ),
            Tri::AllOff => (
                Icon::Open,
                None,
                "Shutters open. Click to close them.",
                Some(1),
            ),
            Tri::Mixed => (
                Icon::Close,
                None,
                "Some shutters closed. Click to choose.",
                None,
            ),
            Tri::Unknown => (Icon::Open, None, "Shutters", None),
        };
        let choices = ["Open shutters", "Close shutters"];
        let can = any && shutter != Tri::Unknown;
        if let Some(i) = choice_button(ui, id.with("shutter"), icon, can, lit, tip, flip, choices) {
            set_shutter = Some(i == 1);
        }
        let (lit, tip, flip) = match power {
            Tri::AllOn => (Some(tint(GREEN)), "All on. Click to power off.", Some(1)),
            Tri::AllOff => (None, "All off. Click to power on.", Some(0)),
            Tri::Mixed => (
                Some(tint(GREEN)),
                "Some on, some off. Click to choose.",
                None,
            ),
            Tri::Unknown => (None, "Power", None),
        };
        let choices = ["Power on", "Power off"];
        if let Some(i) = choice_button(
            ui,
            id.with("power"),
            Icon::Power,
            any,
            lit,
            tip,
            flip,
            choices,
        ) {
            set_power = Some(i == 0);
        }
    } else {
        let segs = [
            Seg {
                face: Face::Icon(Icon::Open),
                lit: matches!(shutter, Tri::AllOff | Tri::Mixed).then_some(TINT_OPEN),
                tip: "Open shutters",
            },
            Seg {
                face: Face::Icon(Icon::Close),
                lit: matches!(shutter, Tri::AllOn | Tri::Mixed).then_some(TINT_CLOSED),
                tip: "Close shutters",
            },
        ];
        if let Some(i) = segmented(ui, id.with("shutter"), any, &segs) {
            set_shutter = Some(i == 1);
        }
        let segs = [
            Seg {
                face: Face::Text("On"),
                lit: matches!(power, Tri::AllOn | Tri::Mixed).then(|| tint(GREEN)),
                tip: "Power on",
            },
            Seg {
                face: Face::Text("Off"),
                lit: matches!(power, Tri::AllOff | Tri::Mixed).then_some(TINT_PLAIN),
                tip: "Power off",
            },
        ];
        if let Some(i) = segmented(ui, id.with("power"), any, &segs) {
            set_power = Some(i == 0);
        }
    }

    if let Some(close) = set_shutter {
        acts.push(Act::SendMany(ids.clone(), Command::VideoMute(close)));
    }
    if let Some(on) = set_power {
        acts.push(power_many(ids, on, scope));
    }
}

fn chevron(ui: &egui::Ui, at: egui::Pos2, open: bool) {
    let s = 4.0;
    let points = if open {
        vec![
            at + Vec2::new(-s, -s * 0.6),
            at + Vec2::new(s, -s * 0.6),
            at + Vec2::new(0.0, s * 0.8),
        ]
    } else {
        vec![
            at + Vec2::new(-s * 0.6, -s),
            at + Vec2::new(-s * 0.6, s),
            at + Vec2::new(s * 0.8, 0.0),
        ]
    };
    ui.painter()
        .add(egui::Shape::convex_polygon(points, DIM, Stroke::NONE));
}

impl App {
    /// Draws a small window and returns the height its rows need.
    pub(super) fn compact(&self, ctx: &egui::Context, items: &[Item], acts: &mut Vec<Act>) -> f32 {
        let small = self.cfg.size == WindowSize::Small;
        let (head_h, group_h, row_h) = heights(self.cfg.size);
        let all: Vec<&Item> = items.iter().collect();

        // (key for remembering the fold, title, members)
        let mut sections: Vec<(String, String, Vec<&Item>)> = self
            .cfg
            .groups
            .iter()
            .map(|g| {
                let members = self.sorted(items.iter().filter(|i| i.0.group.as_deref() == Some(g)));
                (g.clone(), g.clone(), members)
            })
            .collect();
        let loose = self.sorted(items.iter().filter(|i| self.ungrouped(&i.0)));
        let grouped = !sections.is_empty();
        if grouped && !loose.is_empty() {
            sections.push((String::new(), "Ungrouped".into(), loose.clone()));
        }

        let mut height = head_h;
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(BG))
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                let left = strip(ui, head_h, PANEL, |ui| {
                    window_toggles(ui, &self.cfg, acts);
                    if icon_button(ui, ICON, Icon::Plus, true, None)
                        .on_hover_text("Add a projector")
                        .clicked()
                    {
                        acts.push(Act::EditNew);
                    }
                    ui.add_space(4.0);
                    let scope = format!("all {} projectors", all.len());
                    set_controls(ui, Id::new("compact-all"), small, &all, &scope, acts);
                });
                let title = if small { "All" } else { "All projectors" };
                lines(ui, left, RichText::new(title).size(13.5).strong(), None);

                egui::ScrollArea::vertical()
                    .auto_shrink(false)
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing = Vec2::ZERO;
                        for (key, title, members) in &sections {
                            let folded = self.cfg.collapsed.contains(key);
                            self.group_strip(ui, group_h, key, title, members, folded, acts);
                            height += group_h;
                            if !folded {
                                for item in members {
                                    self.projector_strip(ui, row_h, item, acts);
                                    height += row_h;
                                }
                            }
                        }
                        if !grouped {
                            for item in &loose {
                                self.projector_strip(ui, row_h, item, acts);
                                height += row_h;
                            }
                        }
                        if items.is_empty() {
                            ui.add_space(14.0);
                            ui.horizontal_wrapped(|ui| {
                                ui.add_space(PAD);
                                ui.label(dim("No projectors yet. Add one with the + button."));
                            });
                            height += 64.0;
                        }
                    });
            });
        height
    }

    #[allow(clippy::too_many_arguments)]
    fn group_strip(
        &self,
        ui: &mut egui::Ui,
        h: f32,
        key: &str,
        title: &str,
        members: &[&Item],
        folded: bool,
        acts: &mut Vec<Act>,
    ) {
        let small = self.cfg.size == WindowSize::Small;
        let id = Id::new(("compact-group", key));
        let left = strip(ui, h, mix(PANEL, ACCENT, 0.16), |ui| {
            if !small {
                // Line the switches up with the projectors' below.
                ui.add_space(ICON.x + 6.0);
            }
            let scope = if key.is_empty() {
                format!("all {} ungrouped", members.len())
            } else {
                format!("all {} in {title}", members.len())
            };
            set_controls(ui, id, small, members, &scope, acts);
        });
        let on = members
            .iter()
            .filter(|i| i.1.link == Link::Online && i.1.power == Power::On)
            .count();
        let count = format!("{on} of {} on", members.len());
        let sev = worst(members);
        let ink = match sev {
            Severity::Ok => mix(TEXT, ACCENT, 0.45),
            other => sev_color(other),
        };
        chevron(ui, egui::pos2(left.left() + 5.0, left.center().y), !folded);
        let text = if small {
            title.to_string()
        } else {
            format!("{title}  ·  {count}")
        };
        let text_rect = Rect::from_min_max(egui::pos2(left.left() + 16.0, left.top()), left.max);
        lines(
            ui,
            text_rect,
            RichText::new(text).size(13.0).strong().color(ink),
            None,
        );
        let troubled: Vec<String> = members
            .iter()
            .filter(|i| i.1.severity() != Severity::Ok)
            .map(|i| format!("{}: {}", i.0.name, about(&i.0, &i.1).0))
            .collect();
        let mut tip = format!("{title}: {count}");
        for t in &troubled {
            tip.push('\n');
            tip.push_str(t);
        }
        tip.push_str(if folded {
            "\nClick to show its projectors."
        } else {
            "\nClick to hide its projectors."
        });
        if ui
            .interact(left, id.with("fold"), Sense::click())
            .on_hover_text(tip)
            .clicked()
        {
            acts.push(Act::ToggleFold(key.to_string()));
        }
    }

    fn projector_strip(&self, ui: &mut egui::Ui, h: f32, item: &Item, acts: &mut Vec<Act>) {
        let (cfg, st) = item;
        let small = self.cfg.size == WindowSize::Small;
        let left = strip(ui, h, BG, |ui| {
            projector_controls(ui, small, cfg, st, self.detail == Some(cfg.id), acts);
        });
        let sev = st.severity();
        let dot = if sev != Severity::Ok {
            sev_color(sev)
        } else {
            power_color(st)
        };
        let at = egui::pos2(left.left() + 5.0, left.center().y);
        ui.painter().circle_filled(at, 4.5, dot);
        let (note, note_color) = about(cfg, st);
        let name_color = if small && sev != Severity::Ok {
            sev_color(sev)
        } else {
            TEXT
        };
        let text_rect = Rect::from_min_max(egui::pos2(left.left() + 16.0, left.top()), left.max);
        lines(
            ui,
            text_rect,
            RichText::new(&cfg.name).size(13.5).color(name_color),
            (!small).then(|| RichText::new(&note).size(11.0).color(note_color)),
        );
        let tip = format!(
            "{}\n{}\n{}",
            cfg.name,
            subtitle(cfg, st),
            if note == subtitle(cfg, st) {
                power_label(st).to_string()
            } else {
                note
            }
        );
        ui.interact(left, Id::new(("compact-name", cfg.id)), Sense::click())
            .on_hover_text(format!("{tip}\nRight-click to edit or remove."))
            .context_menu(|ui| {
                if ui.button("Details").clicked() {
                    acts.push(Act::Detail(Some(cfg.id)));
                    ui.close_menu();
                }
                if ui.button("Edit…").clicked() {
                    acts.push(Act::Edit(cfg.id));
                    ui.close_menu();
                }
                ui.menu_button("Move to group", |ui| {
                    if ui
                        .selectable_label(cfg.group.is_none(), "No group")
                        .clicked()
                    {
                        acts.push(Act::MoveToGroup(vec![cfg.id], None));
                        ui.close_menu();
                    }
                    for g in &self.cfg.groups {
                        if ui
                            .selectable_label(cfg.group.as_deref() == Some(g.as_str()), g)
                            .clicked()
                        {
                            acts.push(Act::MoveToGroup(vec![cfg.id], Some(g.clone())));
                            ui.close_menu();
                        }
                    }
                });
                ui.separator();
                if ui.button(RichText::new("Remove…").color(RED)).clicked() {
                    acts.push(remove_confirm(cfg));
                    ui.close_menu();
                }
            });
    }
}

impl App {
    /// A separate little window beside a small one, for what will not fit
    /// inside it. Returns true when its close button was pressed.
    pub(super) fn pop_out(
        &mut self,
        ctx: &egui::Context,
        key: &str,
        title: &str,
        size: [f32; 2],
        acts: &mut Vec<Act>,
        mut contents: impl FnMut(&mut Self, &egui::Context, &mut Vec<Act>),
    ) -> bool {
        let mut builder = egui::ViewportBuilder::default()
            .with_title(title)
            .with_inner_size(size)
            .with_resizable(false);
        if self.cfg.on_top {
            builder = builder.with_always_on_top();
        }
        ctx.show_viewport_immediate(egui::ViewportId::from_hash_of(key), builder, |ctx, _| {
            contents(self, ctx, acts);
            egui::CentralPanel::default()
                .frame(egui::Frame::none().fill(PANEL))
                .show(ctx, |_| {});
            ctx.input(|i| i.viewport().close_requested())
        })
    }
}
