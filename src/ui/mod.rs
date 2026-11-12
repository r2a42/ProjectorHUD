//! The application window.

mod dialogs;
mod theme;

use eframe::egui::{self, Align, Color32, Layout, RichText, Sense, Stroke, Vec2};
use projectorhud::config::{address_key, AppConfig, SortKey};
use projectorhud::discovery::Scan;
use projectorhud::model::*;
use projectorhud::schedule::{self, LastRun, Schedule};
use projectorhud::sim::{self, SimHandle};
use projectorhud::worker::{Hub, Repaint, Target};
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use theme::*;

const CARD_MIN_W: f32 = 340.0;
const CARD_H: f32 = 172.0;
const GAP: f32 = 14.0;
const DEMO_GROUP: &str = "Demo";

pub type Item = (ProjectorConfig, ProjectorState);

/// Projectors being dragged to another group.
struct DragIds(Vec<u64>);

/// Everything the UI can ask for. Collected during a frame, applied after it.
pub enum Act {
    Send(u64, Command),
    SendTarget(Target, Command),
    SendMany(Vec<u64>, Command),
    /// Show one projector (from the alert bar) or clear the selection.
    Select(Option<u64>),
    /// A click on a card: `toggle` with Cmd/Ctrl, `range` with Shift.
    Click {
        id: u64,
        toggle: bool,
        range: bool,
    },
    View(Target),
    Confirm(Confirm),
    EditNew,
    Edit(u64),
    SaveProjector(ProjectorConfig),
    Remove(u64),
    MoveToGroup(Vec<u64>, Option<String>),
    AddGroup(String),
    RenameGroup(String, String),
    StartRename(String),
    DeleteGroup(String),
    SetSort(SortKey),
    OpenScan,
    OpenSchedules,
    SetDemo(bool),
}

pub struct Confirm {
    pub title: String,
    pub body: String,
    pub button: String,
    pub danger: bool,
    pub act: Box<Act>,
}

struct Rename {
    old: String,
    text: String,
    focus: bool,
}

pub struct App {
    cfg: AppConfig,
    hub: Arc<Mutex<Hub>>,
    schedules: Arc<Mutex<Vec<Schedule>>>,
    last_run: LastRun,
    repaint: Repaint,
    sims: Vec<SimHandle>,
    view: Target,
    /// Highlighted projectors. With exactly one, its details are shown.
    selection: BTreeSet<u64>,
    /// Where a Shift-click range starts.
    anchor: Option<u64>,
    /// Cards in the order they were last drawn, for Shift-click ranges.
    order: Vec<u64>,
    confirm: Option<Confirm>,
    editor: Option<dialogs::Editor>,
    show_scan: bool,
    scan: Option<Scan>,
    scan_subnet: String,
    show_schedules: bool,
    new_group: Option<String>,
    rename: Option<Rename>,
    shot: Option<Shot>,
}

/// Developer aid: `PROJECTORHUD_SCREENSHOT=out.png` saves a frame and exits.
struct Shot {
    path: String,
    at: Instant,
    requested: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::apply(&cc.egui_ctx);
        let ctx = cc.egui_ctx.clone();
        let repaint: Repaint = Arc::new(move || ctx.request_repaint());

        let cfg = AppConfig::load();
        let hub = Arc::new(Mutex::new(Hub::new(repaint.clone())));
        let schedules = Arc::new(Mutex::new(cfg.schedules.clone()));
        let last_run: LastRun = Arc::default();
        schedule::start(schedules.clone(), hub.clone(), last_run.clone());

        let mut app = Self {
            cfg,
            hub,
            schedules,
            last_run,
            repaint,
            sims: Vec::new(),
            view: Target::All,
            selection: BTreeSet::new(),
            anchor: None,
            order: Vec::new(),
            confirm: None,
            editor: None,
            show_scan: false,
            scan: None,
            scan_subnet: String::new(),
            show_schedules: false,
            new_group: None,
            rename: None,
            shot: std::env::var("PROJECTORHUD_SCREENSHOT")
                .ok()
                .map(|path| Shot {
                    path,
                    at: Instant::now() + Duration::from_secs(5),
                    requested: false,
                }),
        };
        if app.cfg.demo {
            app.start_sims();
        } else if app.shot.is_some() && app.cfg.projectors.is_empty() {
            app.set_demo(true);
        }
        {
            let mut hub = app.hub.lock().unwrap();
            for p in &app.cfg.projectors {
                hub.upsert(p.clone());
            }
        }
        let by_name = |app: &App, part: &str| {
            app.cfg
                .projectors
                .iter()
                .find(|p| p.name.contains(part))
                .map(|p| p.id)
        };
        match std::env::var("PROJECTORHUD_SHOT_VIEW").as_deref() {
            Ok("detail") => {
                let id = by_name(&app, "Rehearsal");
                app.selection.extend(id);
            }
            Ok("multi") => {
                let ids = [by_name(&app, "Left"), by_name(&app, "Right")];
                app.selection.extend(ids.into_iter().flatten());
            }
            Ok("schedules") => app.show_schedules = true,
            Ok("scan") => app.apply(Act::OpenScan),
            _ => {}
        }
        app
    }

    fn save(&mut self) {
        *self.schedules.lock().unwrap() = self.cfg.schedules.clone();
        if let Err(e) = self.cfg.save() {
            eprintln!("could not save config: {e}");
        }
    }

    fn start_sims(&mut self) {
        self.sims = sim::demo_fleet()
            .into_iter()
            .filter_map(|s| sim::spawn(s).ok())
            .collect();
    }

    fn set_demo(&mut self, on: bool) {
        if on == self.cfg.demo {
            return;
        }
        self.cfg.demo = on;
        if on {
            self.start_sims();
            if !self.cfg.groups.iter().any(|g| g == DEMO_GROUP) {
                self.cfg.groups.push(DEMO_GROUP.into());
            }
            for spec in sim::demo_fleet() {
                let id = self.cfg.alloc_id();
                let p = ProjectorConfig {
                    id,
                    name: spec.name,
                    host: "127.0.0.1".into(),
                    port: spec.port,
                    protocol: Protocol::PjLink,
                    username: String::new(),
                    password: spec.password.unwrap_or_default(),
                    group: Some(DEMO_GROUP.into()),
                    demo: true,
                };
                self.hub.lock().unwrap().upsert(p.clone());
                self.cfg.projectors.push(p);
            }
        } else {
            self.sims.clear();
            let mut hub = self.hub.lock().unwrap();
            for p in self.cfg.projectors.iter().filter(|p| p.demo) {
                hub.remove(p.id);
            }
            drop(hub);
            self.cfg.projectors.retain(|p| !p.demo);
            if !self
                .cfg
                .projectors
                .iter()
                .any(|p| p.group.as_deref() == Some(DEMO_GROUP))
            {
                self.cfg.groups.retain(|g| g != DEMO_GROUP);
                if self.view == Target::Group(DEMO_GROUP.into()) {
                    self.view = Target::All;
                }
            }
            self.selection.clear();
        }
        self.save();
    }

    /// The one projector whose details are shown, if exactly one is highlighted.
    fn detail_id(&self) -> Option<u64> {
        if self.selection.len() == 1 {
            self.selection.iter().next().copied()
        } else {
            None
        }
    }

    fn apply(&mut self, act: Act) {
        match act {
            Act::Send(id, cmd) => self.hub.lock().unwrap().send(id, cmd),
            Act::SendTarget(target, cmd) => {
                self.hub.lock().unwrap().send_target(&target, cmd);
            }
            Act::SendMany(ids, cmd) => {
                let hub = self.hub.lock().unwrap();
                for id in ids {
                    hub.send(id, cmd.clone());
                }
            }
            Act::Select(id) => {
                self.selection.clear();
                self.selection.extend(id);
                self.anchor = id;
            }
            Act::Click { id, toggle, range } => {
                if range {
                    let a = self
                        .anchor
                        .and_then(|a| self.order.iter().position(|x| *x == a));
                    let b = self.order.iter().position(|x| *x == id);
                    if let (Some(a), Some(b)) = (a, b) {
                        let (lo, hi) = (a.min(b), a.max(b));
                        if !toggle {
                            self.selection.clear();
                        }
                        self.selection.extend(self.order[lo..=hi].iter().copied());
                        return;
                    }
                }
                if toggle {
                    if !self.selection.remove(&id) {
                        self.selection.insert(id);
                    }
                } else if self.selection.len() == 1 && self.selection.contains(&id) {
                    self.selection.clear();
                } else {
                    self.selection.clear();
                    self.selection.insert(id);
                }
                self.anchor = Some(id);
            }
            Act::View(t) => self.view = t,
            Act::Confirm(c) => self.confirm = Some(c),
            Act::EditNew => {
                let group = match &self.view {
                    Target::Group(g) => Some(g.clone()),
                    _ => None,
                };
                self.editor = Some(dialogs::Editor::new(group));
            }
            Act::Edit(id) => {
                if let Some(p) = self.cfg.projectors.iter().find(|p| p.id == id) {
                    self.editor = Some(dialogs::Editor::edit(p));
                }
            }
            Act::SaveProjector(mut p) => {
                if p.id == 0 {
                    p.id = self.cfg.alloc_id();
                    self.cfg.projectors.push(p.clone());
                } else if let Some(slot) = self.cfg.projectors.iter_mut().find(|x| x.id == p.id) {
                    *slot = p.clone();
                }
                self.hub.lock().unwrap().upsert(p);
                self.save();
            }
            Act::Remove(id) => {
                self.hub.lock().unwrap().remove(id);
                self.cfg.projectors.retain(|p| p.id != id);
                self.cfg
                    .schedules
                    .retain(|s| s.target != Target::Projector(id));
                self.selection.remove(&id);
                self.save();
            }
            Act::MoveToGroup(ids, group) => {
                let mut hub = self.hub.lock().unwrap();
                for p in self
                    .cfg
                    .projectors
                    .iter_mut()
                    .filter(|p| ids.contains(&p.id))
                {
                    p.group = group.clone();
                    hub.upsert(p.clone());
                }
                drop(hub);
                self.save();
            }
            Act::AddGroup(name) => {
                let name = name.trim().to_string();
                if !name.is_empty() && !self.cfg.groups.contains(&name) {
                    self.cfg.groups.push(name);
                    self.save();
                }
            }
            Act::StartRename(old) => {
                self.rename = Some(Rename {
                    text: old.clone(),
                    old,
                    focus: true,
                });
            }
            Act::RenameGroup(old, new) => {
                let new = new.trim().to_string();
                if new.is_empty() || new == old || self.cfg.groups.contains(&new) {
                    return;
                }
                for g in self.cfg.groups.iter_mut().filter(|g| **g == old) {
                    *g = new.clone();
                }
                let mut hub = self.hub.lock().unwrap();
                for p in self
                    .cfg
                    .projectors
                    .iter_mut()
                    .filter(|p| p.group.as_deref() == Some(old.as_str()))
                {
                    p.group = Some(new.clone());
                    hub.upsert(p.clone());
                }
                drop(hub);
                for s in self.cfg.schedules.iter_mut() {
                    if s.target == Target::Group(old.clone()) {
                        s.target = Target::Group(new.clone());
                    }
                }
                if self.view == Target::Group(old) {
                    self.view = Target::Group(new);
                }
                self.save();
            }
            Act::DeleteGroup(name) => {
                self.cfg.groups.retain(|g| g != &name);
                let mut hub = self.hub.lock().unwrap();
                for p in self
                    .cfg
                    .projectors
                    .iter_mut()
                    .filter(|p| p.group.as_deref() == Some(name.as_str()))
                {
                    p.group = None;
                    hub.upsert(p.clone());
                }
                drop(hub);
                self.cfg
                    .schedules
                    .retain(|s| s.target != Target::Group(name.clone()));
                if self.view == Target::Group(name) {
                    self.view = Target::All;
                }
                self.save();
            }
            Act::SetSort(key) => {
                self.cfg.sort = key;
                self.save();
            }
            Act::OpenScan => {
                self.show_scan = true;
                if self.scan.is_none() {
                    self.scan = Some(projectorhud::discovery::start(
                        Vec::new(),
                        self.repaint.clone(),
                    ));
                }
            }
            Act::OpenSchedules => self.show_schedules = true,
            Act::SetDemo(on) => self.set_demo(on),
        }
    }

    fn view_title(&self) -> String {
        match &self.view {
            Target::Group(g) => g.clone(),
            _ => "All projectors".into(),
        }
    }

    fn sorted<'a>(&self, items: impl Iterator<Item = &'a Item>) -> Vec<&'a Item> {
        let mut v: Vec<&Item> = items.collect();
        match self.cfg.sort {
            SortKey::Name => v.sort_by_key(|i| i.0.name.to_lowercase()),
            SortKey::Address => v.sort_by_key(|i| address_key(&i.0.host, i.0.port)),
            SortKey::Type => v.sort_by_key(|i| {
                (
                    format!("{} {}", i.1.manufacturer.trim(), i.1.model.trim()).to_lowercase(),
                    i.0.name.to_lowercase(),
                )
            }),
        }
        v
    }

    /// Does this projector belong under the "Ungrouped" subheading?
    fn ungrouped(&self, cfg: &ProjectorConfig) -> bool {
        match &cfg.group {
            Some(g) => !self.cfg.groups.contains(g),
            None => true,
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let items = self.hub.lock().unwrap().snapshot();
        self.selection
            .retain(|id| items.iter().any(|i| i.0.id == *id));
        let mut acts: Vec<Act> = Vec::new();

        self.top_bar(ctx, &items, &mut acts);
        self.alert_bar(ctx, &items, &mut acts);
        self.sidebar(ctx, &items, &mut acts);
        if let Some(item) = self
            .detail_id()
            .and_then(|id| items.iter().find(|i| i.0.id == id))
        {
            self.detail_panel(ctx, item, &mut acts);
        }
        self.central(ctx, &items, &mut acts);
        self.dialogs(ctx, &items, &mut acts);
        drag_label(ctx);

        for act in acts {
            self.apply(act);
        }

        // Keep the error pulse animating; otherwise tick slowly for timed messages.
        let attention = items.iter().any(|i| i.1.severity() == Severity::Error);
        ctx.request_repaint_after(Duration::from_millis(if attention { 60 } else { 1000 }));
        self.screenshot(ctx);
    }
}

/// While projectors are being dragged, say so next to the pointer.
fn drag_label(ctx: &egui::Context) {
    let Some(payload) = egui::DragAndDrop::payload::<DragIds>(ctx) else {
        return;
    };
    let Some(pos) = ctx.pointer_interact_pos() else {
        return;
    };
    let n = payload.0.len();
    let text = format!("Move {n} projector{}", if n == 1 { "" } else { "s" });
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Tooltip,
        egui::Id::new("drag_label"),
    ));
    let galley = painter.layout_no_wrap(text, egui::FontId::proportional(13.0), TEXT);
    let rect = egui::Rect::from_min_size(
        pos + Vec2::new(14.0, 14.0),
        galley.size() + Vec2::new(20.0, 12.0),
    );
    painter.rect(
        rect,
        8.0,
        mix(PANEL, ACCENT, 0.35),
        Stroke::new(1.0_f32, ACCENT),
    );
    painter.galley(rect.min + Vec2::new(10.0, 6.0), galley, TEXT);
}

/// Is a projector drag hovering over `rect`, and was it dropped there?
fn drop_zone(ui: &egui::Ui, rect: egui::Rect) -> (bool, Option<Vec<u64>>) {
    if !egui::DragAndDrop::has_payload_of_type::<DragIds>(ui.ctx()) {
        return (false, None);
    }
    let over = ui
        .ctx()
        .pointer_interact_pos()
        .is_some_and(|p| rect.contains(p));
    if over && ui.input(|i| i.pointer.any_released()) {
        let ids = egui::DragAndDrop::take_payload::<DragIds>(ui.ctx()).map(|p| p.0.clone());
        return (true, ids);
    }
    (over, None)
}

fn power_color(st: &ProjectorState) -> Color32 {
    match (st.link, st.power) {
        (Link::Offline | Link::AuthFailed, _) => RED,
        (Link::Connecting, _) => FAINT,
        (_, Power::On) => GREEN,
        (_, Power::Warming) => AMBER,
        (_, Power::Cooling) => COOL,
        _ => DIM,
    }
}

fn power_label(st: &ProjectorState) -> &'static str {
    match st.link {
        Link::Offline => "OFFLINE",
        Link::AuthFailed => "LOCKED",
        Link::Connecting => "CONNECTING",
        Link::Online => st.power.label(),
    }
}

fn subtitle(cfg: &ProjectorConfig, st: &ProjectorState) -> String {
    let model = format!("{} {}", st.manufacturer.trim(), st.model.trim());
    let host = if cfg.port == cfg.protocol.default_port() {
        cfg.host.clone()
    } else {
        format!("{}:{}", cfg.host, cfg.port)
    };
    if model.trim().is_empty() {
        host
    } else {
        format!("{}  ·  {}", model.trim(), host)
    }
}

fn sev_color(sev: Severity) -> Color32 {
    match sev {
        Severity::Error => RED,
        Severity::Warning => AMBER,
        Severity::Ok => GREEN,
    }
}

fn worst(items: &[&Item]) -> Severity {
    items
        .iter()
        .map(|i| i.1.severity())
        .max()
        .unwrap_or_default()
}

fn power_off_confirm(cfg: &ProjectorConfig) -> Act {
    Act::Confirm(Confirm {
        title: format!("Power off {}?", cfg.name),
        body: "The projector will cool down and go to standby.".into(),
        button: "Power off".into(),
        danger: true,
        act: Box::new(Act::Send(cfg.id, Command::PowerOff)),
    })
}

/// A compact button for rows and headings.
fn mini(ui: &mut egui::Ui, text: &str, width: f32) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(text).size(12.0).color(mix(DIM, TEXT, 0.6)))
            .fill(BUTTON)
            .stroke(Stroke::new(1.0_f32, STROKE))
            .rounding(6.0)
            .min_size(Vec2::new(width, 24.0)),
    )
}

impl App {
    fn top_bar(&mut self, ctx: &egui::Context, items: &[Item], acts: &mut Vec<Act>) {
        egui::TopBottomPanel::top("top")
            .frame(
                egui::Frame::none()
                    .fill(PANEL)
                    .inner_margin(egui::Margin::symmetric(18.0, 12.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Projector").size(19.0).strong());
                    ui.add_space(-8.0);
                    ui.label(RichText::new("HUD").size(19.0).strong().color(ACCENT));
                    ui.add_space(14.0);

                    let total = items.len();
                    let on = items
                        .iter()
                        .filter(|i| i.1.link == Link::Online && i.1.power == Power::On)
                        .count();
                    let offline = items
                        .iter()
                        .filter(|i| matches!(i.1.link, Link::Offline | Link::AuthFailed))
                        .count();
                    let faults = items
                        .iter()
                        .filter(|i| i.1.link == Link::Online && i.1.severity() != Severity::Ok)
                        .count();
                    if total > 0 {
                        pill(
                            ui,
                            &format!("{on} of {total} on"),
                            if on > 0 { GREEN } else { DIM },
                        );
                        if offline > 0 {
                            pill(ui, &format!("{offline} offline"), RED);
                        }
                        if faults > 0 {
                            pill(ui, &format!("{faults} with errors"), AMBER);
                        }
                        if offline == 0 && faults == 0 {
                            pill(ui, "No errors", GREEN);
                        }
                    }

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button("Schedules").clicked() {
                            acts.push(Act::OpenSchedules);
                        }
                        if ui.button("➕ Add projector").clicked() {
                            acts.push(Act::EditNew);
                        }
                        if primary(ui, "🔍 Discover", ACCENT)
                            .on_hover_text("Find projectors on your network")
                            .clicked()
                        {
                            acts.push(Act::OpenScan);
                        }
                    });
                });
            });
    }

    /// Unmissable strip listing every projector that needs attention.
    fn alert_bar(&mut self, ctx: &egui::Context, items: &[Item], acts: &mut Vec<Act>) {
        let troubled: Vec<&Item> = items
            .iter()
            .filter(|i| i.1.severity() != Severity::Ok)
            .collect();
        if troubled.is_empty() {
            return;
        }
        let color = sev_color(worst(&troubled));
        egui::TopBottomPanel::top("alerts")
            .frame(
                egui::Frame::none()
                    .fill(mix(BG, color, 0.16))
                    .inner_margin(egui::Margin::symmetric(18.0, 8.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    let n = troubled.len();
                    let noun = if n == 1 {
                        "projector needs"
                    } else {
                        "projectors need"
                    };
                    ui.label(
                        RichText::new(format!("⚠  {n} {noun} attention"))
                            .color(color)
                            .strong(),
                    );
                    ui.add_space(6.0);
                    for (cfg, st) in troubled {
                        let c = sev_color(st.severity());
                        let what = st
                            .issues()
                            .into_iter()
                            .map(|i| i.0)
                            .collect::<Vec<_>>()
                            .join(", ");
                        let resp = ui.add(
                            egui::Button::new(
                                RichText::new(format!("{}  ·  {}", cfg.name, what))
                                    .color(c)
                                    .size(13.0),
                            )
                            .fill(mix(BG, c, 0.10))
                            .stroke(Stroke::new(1.0_f32, mix(BG, c, 0.5)))
                            .rounding(14.0),
                        );
                        if resp.on_hover_text("Show details").clicked() {
                            acts.push(Act::View(Target::All));
                            acts.push(Act::Select(Some(cfg.id)));
                        }
                    }
                });
            });
    }

    fn sidebar(&mut self, ctx: &egui::Context, items: &[Item], acts: &mut Vec<Act>) {
        egui::SidePanel::left("groups")
            .exact_width(264.0)
            .resizable(false)
            .frame(egui::Frame::none().fill(PANEL).inner_margin(egui::Margin::symmetric(10.0, 12.0)))
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                ui.horizontal(|ui| {
                    ui.add_space(8.0);
                    section(ui, "Groups");
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add_space(14.0);
                        ui.label(RichText::new("SHUTTERS").color(FAINT).size(11.0).strong());
                    });
                });
                ui.add_space(4.0);

                let all: Vec<&Item> = items.iter().collect();
                let out = nav_row(ui, "All projectors", all.len(), worst(&all), self.view == Target::All, false);
                if out.resp.clicked() {
                    acts.push(Act::View(Target::All));
                }
                if let Some(close) = out.shutter {
                    acts.push(Act::SendTarget(Target::All, Command::VideoMute(close)));
                }

                for g in self.cfg.groups.clone() {
                    // Renaming replaces the row with a text field.
                    if let Some(r) = self.rename.as_mut().filter(|r| r.old == g) {
                        let resp = ui.add(egui::TextEdit::singleline(&mut r.text).desired_width(f32::INFINITY));
                        if r.focus {
                            resp.request_focus();
                            r.focus = false;
                        }
                        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                            self.rename = None;
                        } else if resp.lost_focus() {
                            acts.push(Act::RenameGroup(g.clone(), r.text.clone()));
                            self.rename = None;
                        }
                        continue;
                    }

                    let members: Vec<&Item> = items.iter().filter(|i| i.0.group.as_deref() == Some(g.as_str())).collect();
                    let next = ui.available_rect_before_wrap();
                    let row_rect = egui::Rect::from_min_size(next.min, Vec2::new(next.width(), 34.0));
                    let (over, dropped) = drop_zone(ui, row_rect);
                    let out = nav_row(ui, &g, members.len(), worst(&members), self.view == Target::Group(g.clone()), over);
                    if let Some(ids) = dropped {
                        acts.push(Act::MoveToGroup(ids, Some(g.clone())));
                    }
                    if out.resp.double_clicked() {
                        acts.push(Act::StartRename(g.clone()));
                    } else if out.resp.clicked() {
                        acts.push(Act::View(Target::Group(g.clone())));
                    }
                    if let Some(close) = out.shutter {
                        acts.push(Act::SendTarget(Target::Group(g.clone()), Command::VideoMute(close)));
                    }
                    out.resp.on_hover_text("Double-click to rename. Drop projectors here to move them.").context_menu(|ui| {
                        if ui.button("Rename").clicked() {
                            acts.push(Act::StartRename(g.clone()));
                            ui.close_menu();
                        }
                        if ui.button(RichText::new("Delete group…").color(RED)).clicked() {
                            acts.push(Act::Confirm(Confirm {
                                title: format!("Delete group “{g}”?"),
                                body: "Its projectors are kept and become ungrouped.".into(),
                                button: "Delete group".into(),
                                danger: true,
                                act: Box::new(Act::DeleteGroup(g.clone())),
                            }));
                            ui.close_menu();
                        }
                    });
                }

                ui.add_space(6.0);
                match &mut self.new_group {
                    Some(name) => {
                        let resp = ui.add(egui::TextEdit::singleline(name).hint_text("Group name").desired_width(f32::INFINITY));
                        resp.request_focus();
                        let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                        let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
                        if enter {
                            acts.push(Act::AddGroup(name.clone()));
                        }
                        if enter || escape {
                            self.new_group = None;
                        }
                        ui.label(small("Enter to create, Esc to cancel"));
                    }
                    None => {
                        if ui.add(egui::Button::new(dim("➕  New group")).frame(false)).clicked() {
                            self.new_group = Some(String::new());
                        }
                    }
                }

                ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
                    let mut demo = self.cfg.demo;
                    if ui
                        .checkbox(&mut demo, dim("Demo projectors"))
                        .on_hover_text("Adds six simulated projectors so you can try everything without hardware")
                        .changed()
                    {
                        acts.push(Act::SetDemo(demo));
                    }
                    if let Some(last) = self.last_run.lock().unwrap().clone() {
                        ui.label(small(last));
                        ui.label(RichText::new("LAST SCHEDULE").color(FAINT).size(10.0).strong());
                    }
                });
            });
    }

    fn central(&mut self, ctx: &egui::Context, items: &[Item], acts: &mut Vec<Act>) {
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(BG).inner_margin(egui::Margin { left: 20.0, right: 8.0, top: 16.0, bottom: 0.0 }))
            .show(ctx, |ui| {
                if items.is_empty() {
                    welcome(ui, acts);
                    return;
                }
                let visible = self.sorted(items.iter().filter(|i| self.view.matches(&i.0)));

                ui.horizontal(|ui| {
                    ui.label(RichText::new(self.view_title()).size(22.0).strong());
                    ui.label(dim(format!("{}", visible.len())));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add_space(12.0);
                        let n = visible.len();
                        let scope = match &self.view {
                            Target::Group(g) => format!("all {n} in {g}"),
                            _ => format!("all {n} projectors"),
                        };
                        ui.add_enabled_ui(n > 0, |ui| {
                            if ui.button("Open shutters").clicked() {
                                acts.push(Act::SendTarget(self.view.clone(), Command::VideoMute(false)));
                            }
                            if ui.button("Close shutters").clicked() {
                                acts.push(Act::SendTarget(self.view.clone(), Command::VideoMute(true)));
                            }
                            if ui.button("Power off all").clicked() {
                                acts.push(Act::Confirm(Confirm {
                                    title: format!("Power off {scope}?"),
                                    body: "They will cool down and go to standby.".into(),
                                    button: "Power off all".into(),
                                    danger: true,
                                    act: Box::new(Act::SendTarget(self.view.clone(), Command::PowerOff)),
                                }));
                            }
                            if ui.button("Power on all").clicked() {
                                acts.push(Act::Confirm(Confirm {
                                    title: format!("Power on {scope}?"),
                                    body: "Projectors already on are not affected.".into(),
                                    button: "Power on all".into(),
                                    danger: false,
                                    act: Box::new(Act::SendTarget(self.view.clone(), Command::PowerOn)),
                                }));
                            }
                        });
                    });
                });

                self.tools_row(ui, acts);
                ui.add_space(4.0);

                let mut order: Vec<u64> = Vec::new();
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    ui.set_width(ui.available_width() - 12.0);
                    let grouped = self.view == Target::All && !self.cfg.groups.is_empty();
                    if !grouped {
                        if visible.is_empty() {
                            ui.add_space(40.0);
                            ui.vertical_centered(|ui| {
                                ui.label(dim("No projectors in this group yet."));
                                ui.label(small("Drag projectors onto the group in the left column, or add a new one."));
                            });
                        }
                        self.grid(ui, &visible, acts, &mut order);
                    } else {
                        let dragging = egui::DragAndDrop::has_payload_of_type::<DragIds>(ui.ctx());
                        let mut sections: Vec<Option<String>> = self.cfg.groups.iter().cloned().map(Some).collect();
                        if dragging || visible.iter().any(|i| self.ungrouped(&i.0)) {
                            sections.push(None);
                        }
                        for group in sections {
                            let members: Vec<&Item> = visible
                                .iter()
                                .copied()
                                .filter(|i| match &group {
                                    Some(g) => i.0.group.as_ref() == Some(g),
                                    None => self.ungrouped(&i.0),
                                })
                                .collect();
                            let inner = ui.scope(|ui| {
                                ui.spacing_mut().item_spacing.y = 8.0;
                                self.section_header(ui, &group, &members, acts);
                                if members.is_empty() {
                                    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 44.0), Sense::hover());
                                    ui.painter().rect(rect, 10.0, Color32::TRANSPARENT, Stroke::new(1.0_f32, STROKE));
                                    ui.painter().text(
                                        rect.center(),
                                        egui::Align2::CENTER_CENTER,
                                        "Drag projectors here",
                                        egui::FontId::proportional(13.0),
                                        FAINT,
                                    );
                                } else {
                                    self.grid(ui, &members, acts, &mut order);
                                }
                            });
                            let zone = inner.response.rect.expand2(Vec2::new(8.0, 6.0));
                            let (over, dropped) = drop_zone(ui, zone);
                            if over {
                                ui.painter().rect(zone, 14.0, ACCENT.gamma_multiply(0.06), Stroke::new(1.5_f32, ACCENT));
                            }
                            if let Some(ids) = dropped {
                                acts.push(Act::MoveToGroup(ids, group.clone()));
                            }
                            ui.add_space(14.0);
                        }
                    }
                    ui.add_space(8.0);
                });
                self.order = order;
            });
    }

    /// Sorting, and what to do with the highlighted projectors.
    fn tools_row(&mut self, ui: &mut egui::Ui, acts: &mut Vec<Act>) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.spacing_mut().button_padding = Vec2::new(10.0, 3.0);
            ui.label(small("Sort by"));
            for key in SortKey::ALL {
                let active = self.cfg.sort == key;
                if toggle(ui, Vec2::new(0.0, 24.0), key.label(), active.then_some(ACCENT), true).clicked() && !active {
                    acts.push(Act::SetSort(key));
                }
            }

            let n = self.selection.len();
            ui.add_space(14.0);
            if n > 1 {
                let ids: Vec<u64> = self.selection.iter().copied().collect();
                ui.label(RichText::new(format!("{n} selected")).color(ACCENT).strong());
                if mini(ui, "Power on", 0.0).clicked() {
                    acts.push(Act::SendMany(ids.clone(), Command::PowerOn));
                }
                if mini(ui, "Power off", 0.0).clicked() {
                    acts.push(Act::Confirm(Confirm {
                        title: format!("Power off the {n} selected projectors?"),
                        body: "They will cool down and go to standby.".into(),
                        button: "Power off".into(),
                        danger: true,
                        act: Box::new(Act::SendMany(ids.clone(), Command::PowerOff)),
                    }));
                }
                if mini(ui, "Close shutters", 0.0).clicked() {
                    acts.push(Act::SendMany(ids.clone(), Command::VideoMute(true)));
                }
                if mini(ui, "Open shutters", 0.0).clicked() {
                    acts.push(Act::SendMany(ids.clone(), Command::VideoMute(false)));
                }
                ui.menu_button(RichText::new("Move to…").size(12.0), |ui| {
                    if ui.button("No group").clicked() {
                        acts.push(Act::MoveToGroup(ids.clone(), None));
                        ui.close_menu();
                    }
                    for g in &self.cfg.groups {
                        if ui.button(g).clicked() {
                            acts.push(Act::MoveToGroup(ids.clone(), Some(g.clone())));
                            ui.close_menu();
                        }
                    }
                });
                if ui.add(egui::Button::new(small("Clear")).frame(false)).clicked() {
                    acts.push(Act::Select(None));
                }
            } else {
                ui.add(
                    egui::Label::new(
                        RichText::new("Cmd/Ctrl-click or Shift-click selects several. Drag to move between groups.").color(FAINT).size(12.0),
                    )
                    .truncate(),
                );
            }
        });
    }

    /// Subheading for one group inside "All projectors".
    fn section_header(
        &self,
        ui: &mut egui::Ui,
        group: &Option<String>,
        members: &[&Item],
        acts: &mut Vec<Act>,
    ) {
        ui.horizontal(|ui| {
            ui.set_height(28.0);
            let title = group.clone().unwrap_or_else(|| "Ungrouped".into());
            let text = RichText::new(title)
                .size(16.0)
                .strong()
                .color(mix(DIM, TEXT, 0.75));
            let label = ui.add(egui::Label::new(text).sense(Sense::click()));
            if let Some(g) = group {
                if label
                    .on_hover_text("Double-click to rename")
                    .double_clicked()
                {
                    acts.push(Act::StartRename(g.clone()));
                }
            }
            ui.label(dim(format!("{}", members.len())));
            let sev = worst(members);
            if sev != Severity::Ok {
                dot(ui, sev_color(sev));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.spacing_mut().button_padding = Vec2::new(10.0, 3.0);
                ui.add_enabled_ui(!members.is_empty(), |ui| {
                    let ids: Vec<u64> = members.iter().map(|i| i.0.id).collect();
                    if mini(ui, "Open shutters", 0.0).clicked() {
                        acts.push(Act::SendMany(ids.clone(), Command::VideoMute(false)));
                    }
                    if mini(ui, "Close shutters", 0.0).clicked() {
                        acts.push(Act::SendMany(ids, Command::VideoMute(true)));
                    }
                });
                // A rule from the title to the buttons.
                let rest = ui.available_rect_before_wrap();
                if rest.width() > 16.0 {
                    ui.painter().hline(
                        (rest.left() + 6.0)..=(rest.right() - 10.0),
                        rest.center().y,
                        Stroke::new(1.0_f32, STROKE),
                    );
                }
            });
        });
    }

    fn grid(&self, ui: &mut egui::Ui, items: &[&Item], acts: &mut Vec<Act>, order: &mut Vec<u64>) {
        let avail = ui.available_width();
        let cols = (((avail + GAP) / (CARD_MIN_W + GAP)).floor() as usize).max(1);
        let w = ((avail - GAP * (cols as f32 - 1.0)) / cols as f32).floor();
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing = Vec2::splat(GAP);
            for row in items.chunks(cols) {
                ui.horizontal(|ui| {
                    for (cfg, st) in row.iter().map(|i| (&i.0, &i.1)) {
                        order.push(cfg.id);
                        self.card(ui, w, cfg, st, acts);
                    }
                });
            }
        });
    }

    fn card(
        &self,
        ui: &mut egui::Ui,
        w: f32,
        cfg: &ProjectorConfig,
        st: &ProjectorState,
        acts: &mut Vec<Act>,
    ) {
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, CARD_H), Sense::click_and_drag());
        let selected = self.selection.contains(&cfg.id);
        if resp.clicked() {
            let m = ui.input(|i| i.modifiers);
            acts.push(Act::Click {
                id: cfg.id,
                toggle: m.command,
                range: m.shift,
            });
        }
        // Dragging a highlighted card takes every highlighted one with it.
        if resp.drag_started() {
            let ids = if selected {
                self.selection.iter().copied().collect()
            } else {
                vec![cfg.id]
            };
            egui::DragAndDrop::set_payload(ui.ctx(), DragIds(ids));
        }
        let dragged =
            egui::DragAndDrop::payload::<DragIds>(ui.ctx()).is_some_and(|p| p.0.contains(&cfg.id));

        let sev = st.severity();
        let issues = st.issues();
        let online = st.link == Link::Online;

        // Background: faults tint the whole card and pulse the border.
        let mut fill = if resp.hovered() { CARD_HOVER } else { CARD };
        let (border, width): (Color32, f32) = match sev {
            Severity::Error => {
                let t = ui.input(|i| i.time);
                let pulse = ((t * 3.2).sin() * 0.5 + 0.5) as f32;
                fill = mix(fill, RED, 0.07 + 0.05 * pulse);
                (mix(mix(CARD, RED, 0.55), RED, pulse), 2.0)
            }
            Severity::Warning => {
                fill = mix(fill, AMBER, 0.05);
                (mix(CARD, AMBER, 0.75), 1.5)
            }
            Severity::Ok => (STROKE, 1.0),
        };
        if dragged {
            fill = mix(fill, BG, 0.5);
        }
        ui.painter()
            .rect(rect, 12.0, fill, Stroke::new(width, border));
        if selected {
            ui.painter()
                .rect_stroke(rect.expand(3.0), 15.0, Stroke::new(2.0_f32, ACCENT));
        }

        resp.context_menu(|ui| {
            // Acts on every highlighted projector when this is one of them.
            let ids: Vec<u64> = if selected {
                self.selection.iter().copied().collect()
            } else {
                vec![cfg.id]
            };
            if ui.button("Edit…").clicked() {
                acts.push(Act::Edit(cfg.id));
                ui.close_menu();
            }
            let move_label = if ids.len() > 1 {
                format!("Move {} to group", ids.len())
            } else {
                "Move to group".to_string()
            };
            ui.menu_button(move_label, |ui| {
                if ui
                    .selectable_label(ids.len() == 1 && cfg.group.is_none(), "No group")
                    .clicked()
                {
                    acts.push(Act::MoveToGroup(ids.clone(), None));
                    ui.close_menu();
                }
                for g in &self.cfg.groups {
                    if ui
                        .selectable_label(
                            ids.len() == 1 && cfg.group.as_deref() == Some(g.as_str()),
                            g,
                        )
                        .clicked()
                    {
                        acts.push(Act::MoveToGroup(ids.clone(), Some(g.clone())));
                        ui.close_menu();
                    }
                }
            });
            if ui.button("Refresh now").clicked() {
                acts.push(Act::Send(cfg.id, Command::Refresh));
                ui.close_menu();
            }
            ui.separator();
            if ui.button(RichText::new("Remove…").color(RED)).clicked() {
                acts.push(remove_confirm(cfg));
                ui.close_menu();
            }
        });

        let inner = rect.shrink2(Vec2::new(16.0, 14.0));
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(inner)
                .layout(Layout::top_down(Align::Min)),
        );
        {
            let ui = &mut child;
            ui.spacing_mut().item_spacing = Vec2::new(8.0, 4.0);

            // Name + state
            ui.allocate_ui_with_layout(
                Vec2::new(ui.available_width(), 22.0),
                Layout::right_to_left(Align::Center),
                |ui| {
                    pill(ui, power_label(st), power_color(st));
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        dot(ui, power_color(st));
                        ui.add(
                            egui::Label::new(RichText::new(&cfg.name).size(16.0).strong())
                                .truncate(),
                        );
                    });
                },
            );
            ui.add(egui::Label::new(small(subtitle(cfg, st))).truncate());
            ui.add_space(6.0);

            // Input + lamp
            ui.horizontal(|ui| {
                ui.set_height(28.0);
                if online && st.power == Power::On && !st.inputs.is_empty() {
                    input_combo(ui, cfg.id, st, 150.0, acts);
                } else if online {
                    ui.label(dim(match st.power {
                        Power::Warming => "Warming up…",
                        Power::Cooling => "Cooling down…",
                        Power::Standby => "In standby",
                        _ => "",
                    }));
                } else {
                    let why = st
                        .link_error
                        .clone()
                        .unwrap_or_else(|| "Connecting…".into());
                    ui.add(egui::Label::new(dim(why)).truncate());
                }
                if online {
                    if let Some(lamp) = st.lamps.first() {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(small(format!("Lamp {} h", thousands(lamp.hours))));
                        });
                    }
                }
            });

            // Errors: the line always says something, so an empty space never
            // has to be read as "fine".
            ui.allocate_ui_with_layout(
                Vec2::new(ui.available_width(), 18.0),
                Layout::right_to_left(Align::Center),
                |ui| {
                    // Right: a failed command (briefly), else which functions are engaged.
                    let recent_error = st
                        .command_error
                        .as_ref()
                        .filter(|(_, at)| at.elapsed() < Duration::from_secs(8));
                    if let Some((msg, _)) = recent_error {
                        ui.add(
                            egui::Label::new(RichText::new(msg).color(AMBER).size(12.0)).truncate(),
                        )
                        .on_hover_text(msg);
                        return; // the message needs the whole line
                    }
                    if online {
                        let mut flags = Vec::new();
                        if st.video_mute == Some(true) {
                            flags.push("Shutter closed");
                        }
                        if st.audio_mute == Some(true) {
                            flags.push("Audio muted");
                        }
                        if st.freeze == Some(true) {
                            flags.push("Frozen");
                        }
                        if !flags.is_empty() {
                            ui.label(RichText::new(flags.join("  ·  ")).color(AMBER).size(12.0));
                        }
                    }
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        let (text, color) = match st.link {
                            Link::Connecting => ("Checking…".to_string(), FAINT),
                            Link::AuthFailed => {
                                ("⚠  Password rejected: right-click → Edit".to_string(), RED)
                            }
                            Link::Offline => ("⚠  Not responding, retrying".to_string(), RED),
                            Link::Online if !issues.is_empty() => (
                                format!(
                                    "⚠  {}",
                                    issues
                                        .iter()
                                        .map(|i| i.0.as_str())
                                        .collect::<Vec<_>>()
                                        .join("  ·  ")
                                ),
                                sev_color(sev),
                            ),
                            Link::Online if st.errors.is_some() => {
                                ("✔  No errors".to_string(), GREEN)
                            }
                            Link::Online => {
                                ("Errors not reported by this protocol".to_string(), FAINT)
                            }
                        };
                        let mut rich = RichText::new(text).color(color).size(13.0);
                        if sev != Severity::Ok {
                            rich = rich.strong();
                        }
                        ui.add(egui::Label::new(rich).truncate());
                    });
                },
            );

            // Controls pinned to the bottom edge
            let bottom = egui::Rect::from_min_max(
                egui::pos2(inner.left(), inner.bottom() - 30.0),
                inner.right_bottom(),
            );
            let mut row = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(bottom)
                    .layout(Layout::left_to_right(Align::Center)),
            );
            row.spacing_mut().item_spacing.x = 6.0;
            row.spacing_mut().button_padding = Vec2::new(4.0, 4.0);
            let bw = ((inner.width() - 18.0) / 4.0).floor();
            control_buttons(&mut row, Vec2::new(bw, 30.0), cfg, st, acts);
        }
    }

    fn detail_panel(&mut self, ctx: &egui::Context, item: &Item, acts: &mut Vec<Act>) {
        let (cfg, st) = item;
        let log = self.hub.lock().unwrap().get(cfg.id).map(|h| h.log.clone());
        egui::SidePanel::right("detail")
            .exact_width(340.0)
            .resizable(false)
            .frame(egui::Frame::none().fill(PANEL).inner_margin(egui::Margin::same(18.0)))
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                ui.allocate_ui_with_layout(Vec2::new(ui.available_width(), 26.0), Layout::right_to_left(Align::Center), |ui| {
                    let close = egui::Button::new(RichText::new("×").size(20.0).color(DIM)).frame(false);
                    if ui.add(close).on_hover_text("Close").clicked() {
                        acts.push(Act::Select(None));
                    }
                    pill(ui, power_label(st), power_color(st));
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        ui.add(egui::Label::new(RichText::new(&cfg.name).size(20.0).strong()).truncate());
                    });
                });
                ui.label(small(subtitle(cfg, st)));

                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    ui.set_width(ui.available_width());

                    if let Some(err) = &st.link_error {
                        ui.add_space(6.0);
                        banner(ui, RED, err);
                    }
                    if let Some((msg, at)) = &st.command_error {
                        if at.elapsed() < Duration::from_secs(12) {
                            ui.add_space(6.0);
                            banner(ui, AMBER, msg);
                        }
                    }

                    section(ui, "Control");
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        ui.spacing_mut().button_padding = Vec2::new(4.0, 4.0);
                        let bw = ((ui.available_width() - 18.0) / 4.0).floor();
                        control_buttons(ui, Vec2::new(bw, 34.0), cfg, st, acts);
                    });
                    let on = st.link == Link::Online && st.power == Power::On;
                    if on && st.class >= 2 {
                        ui.horizontal(|ui| {
                            ui.label(dim("Volume"));
                            if ui.button("  −  ").clicked() {
                                acts.push(Act::Send(cfg.id, Command::VolumeDown));
                            }
                            if ui.button("  +  ").clicked() {
                                acts.push(Act::Send(cfg.id, Command::VolumeUp));
                            }
                        });
                    }

                    if on && !st.inputs.is_empty() {
                        section(ui, "Input");
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = Vec2::splat(6.0);
                            for input in &st.inputs {
                                let active = st.input.as_deref() == Some(input.code.as_str());
                                if toggle(ui, Vec2::new(0.0, 30.0), &input.name, active.then_some(ACCENT), true).clicked() && !active {
                                    acts.push(Act::Send(cfg.id, Command::Input(input.code.clone())));
                                }
                            }
                        });
                    }

                    section(ui, "Errors");
                    match &st.errors {
                        Some(errors) if st.link == Link::Online => {
                            if errors.worst() == Severity::Ok {
                                ui.label(RichText::new("✔  No errors reported").color(GREEN).strong());
                            }
                            egui::Grid::new("health").num_columns(2).spacing([24.0, 6.0]).show(ui, |ui| {
                                for (i, (name, sev)) in errors.items().into_iter().enumerate() {
                                    ui.horizontal(|ui| {
                                        dot(ui, sev_color(sev));
                                        ui.label(name);
                                        let word = match sev {
                                            Severity::Ok => "OK",
                                            Severity::Warning => "Warning",
                                            Severity::Error => "Error",
                                        };
                                        ui.label(RichText::new(word).color(sev_color(sev)).strong());
                                    });
                                    if i % 2 == 1 {
                                        ui.end_row();
                                    }
                                }
                            });
                        }
                        _ if st.link == Link::Online => {
                            ui.label(dim("This protocol does not report errors. Use PJLink for full monitoring."));
                        }
                        _ => {
                            ui.label(dim("Unavailable while the projector is unreachable."));
                        }
                    }

                    if !st.lamps.is_empty() {
                        section(ui, "Light source");
                        for (i, lamp) in st.lamps.iter().enumerate() {
                            ui.horizontal(|ui| {
                                dot(ui, if lamp.on { GREEN } else { FAINT });
                                ui.label(format!("Lamp {}", i + 1));
                                ui.label(RichText::new(format!("{} hours", thousands(lamp.hours))).strong());
                                ui.label(dim(if lamp.on { "lit" } else { "off" }));
                            });
                        }
                    }

                    section(ui, "Device");
                    egui::Grid::new("device").num_columns(2).spacing([16.0, 5.0]).show(ui, |ui| {
                        let mut row = |k: &str, v: &str| {
                            if !v.trim().is_empty() {
                                ui.label(dim(k));
                                ui.label(v.trim());
                                ui.end_row();
                            }
                        };
                        row("Manufacturer", &st.manufacturer);
                        row("Model", &st.model);
                        row("Reported name", &st.device_name);
                        row("Serial", &st.serial);
                        row("Firmware", &st.firmware);
                        row("Address", &format!("{}:{}", cfg.host, cfg.port));
                        let proto = match (cfg.protocol, st.class) {
                            (Protocol::PjLink, c) if c > 0 => format!("PJLink Class {c}"),
                            (p, _) => p.label().to_string(),
                        };
                        row("Protocol", &proto);
                        row("Group", cfg.group.as_deref().unwrap_or("—"));
                    });
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        if ui.button("Edit…").clicked() {
                            acts.push(Act::Edit(cfg.id));
                        }
                        if ui.button("Refresh").clicked() {
                            acts.push(Act::Send(cfg.id, Command::Refresh));
                        }
                        if ui.button(RichText::new("Remove…").color(RED)).clicked() {
                            acts.push(remove_confirm(cfg));
                        }
                    });

                    section(ui, "Command log");
                    if let Some(log) = log {
                        let entries = log.snapshot();
                        egui::Frame::none()
                            .fill(Color32::from_rgb(0x0A, 0x0D, 0x11))
                            .rounding(8.0)
                            .inner_margin(egui::Margin::same(10.0))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.spacing_mut().item_spacing.y = 2.0;
                                if entries.is_empty() {
                                    ui.label(small("Nothing yet."));
                                }
                                for e in entries.iter().rev().take(60) {
                                    let (mark, color) = match e.kind {
                                        LogKind::Tx => ("→", DIM),
                                        LogKind::Rx => ("←", TEXT),
                                        LogKind::Info => ("•", ACCENT),
                                        LogKind::Error => ("!", RED),
                                    };
                                    let line = format!("{} {mark} {}", e.time.format("%H:%M:%S"), e.text);
                                    ui.add(egui::Label::new(RichText::new(line).monospace().color(color)).truncate());
                                }
                            });
                    }
                    ui.add_space(8.0);
                });
            });
    }

    fn screenshot(&mut self, ctx: &egui::Context) {
        let Some(shot) = &mut self.shot else { return };
        if !shot.requested && Instant::now() >= shot.at {
            shot.requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot);
        }
        let image = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = image {
            let [w, h] = image.size;
            let _ = image::save_buffer(
                &shot.path,
                image.as_raw(),
                w as u32,
                h as u32,
                image::ColorType::Rgba8,
            );
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ctx.request_repaint_after(Duration::from_millis(100));
    }
}

fn remove_confirm(cfg: &ProjectorConfig) -> Act {
    Act::Confirm(Confirm {
        title: format!("Remove {}?", cfg.name),
        body: "This only removes it from ProjectorHUD. The projector itself is not changed.".into(),
        button: "Remove".into(),
        danger: true,
        act: Box::new(Act::Remove(cfg.id)),
    })
}

/// Power / Shutter / Mute / Freeze, shared by cards and the detail panel.
fn control_buttons(
    ui: &mut egui::Ui,
    size: Vec2,
    cfg: &ProjectorConfig,
    st: &ProjectorState,
    acts: &mut Vec<Act>,
) {
    let online = st.link == Link::Online;
    let on = online && st.power == Power::On;

    let (label, enabled, accent) = match st.power {
        _ if !online => ("Power", false, None),
        Power::Standby => ("Power on", true, Some(GREEN)),
        Power::On => ("Power off", true, None),
        Power::Warming => ("Warming…", false, None),
        Power::Cooling => ("Cooling…", false, None),
        Power::Unknown => ("Power", false, None),
    };
    if toggle(ui, size, label, accent, enabled).clicked() {
        acts.push(if st.power == Power::On {
            power_off_confirm(cfg)
        } else {
            Act::Send(cfg.id, Command::PowerOn)
        });
    }

    let shut = st.video_mute == Some(true);
    let r = toggle(
        ui,
        size,
        "Shutter",
        shut.then_some(AMBER),
        on && st.video_mute.is_some(),
    );
    let tip = if shut {
        "Shutter is closed. Click to open."
    } else {
        "Close the shutter (blank the picture)"
    };
    if r.on_hover_text(tip).clicked() {
        acts.push(Act::Send(cfg.id, Command::VideoMute(!shut)));
    }

    let muted = st.audio_mute == Some(true);
    let r = toggle(
        ui,
        size,
        "Mute",
        muted.then_some(AMBER),
        on && st.audio_mute.is_some(),
    );
    if r.on_hover_text(if muted {
        "Audio is muted. Click to unmute."
    } else {
        "Mute audio"
    })
    .on_disabled_hover_text("This projector has no separate audio mute, or is not powered on")
    .clicked()
    {
        acts.push(Act::Send(cfg.id, Command::AudioMute(!muted)));
    }

    let frozen = st.freeze == Some(true);
    let r = toggle(
        ui,
        size,
        "Freeze",
        frozen.then_some(COOL),
        on && st.freeze.is_some(),
    );
    if r.on_hover_text(if frozen {
        "Picture is frozen. Click to resume."
    } else {
        "Freeze the picture"
    })
    .on_disabled_hover_text("Needs PJLink Class 2 and the projector powered on")
    .clicked()
    {
        acts.push(Act::Send(cfg.id, Command::Freeze(!frozen)));
    }
}

fn input_combo(ui: &mut egui::Ui, id: u64, st: &ProjectorState, width: f32, acts: &mut Vec<Act>) {
    egui::ComboBox::from_id_salt(("input", id))
        .width(width)
        .selected_text(st.input_name().unwrap_or_else(|| "Input".into()))
        .show_ui(ui, |ui| {
            for input in &st.inputs {
                let active = st.input.as_deref() == Some(input.code.as_str());
                if ui.selectable_label(active, &input.name).clicked() && !active {
                    acts.push(Act::Send(id, Command::Input(input.code.clone())));
                }
            }
        });
}

fn banner(ui: &mut egui::Ui, color: Color32, text: &str) {
    egui::Frame::none()
        .fill(mix(PANEL, color, 0.14))
        .stroke(Stroke::new(1.0_f32, mix(PANEL, color, 0.5)))
        .rounding(8.0)
        .inner_margin(egui::Margin::symmetric(12.0, 8.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(text).color(color));
        });
}

struct NavOut {
    resp: egui::Response,
    /// `Some(true)` = close shutters, `Some(false)` = open.
    shutter: Option<bool>,
}

/// One row of the left column: a group name, its count, and shutter buttons.
fn nav_row(
    ui: &mut egui::Ui,
    label: &str,
    count: usize,
    sev: Severity,
    selected: bool,
    drop_hover: bool,
) -> NavOut {
    const BTN_W: f32 = 42.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 34.0), Sense::hover());
    let buttons = egui::Rect::from_min_max(
        egui::pos2(rect.right() - (BTN_W * 2.0 + 4.0) - 6.0, rect.top()),
        egui::pos2(rect.right() - 6.0, rect.bottom()),
    );
    let main = egui::Rect::from_min_max(rect.min, egui::pos2(buttons.left() - 4.0, rect.bottom()));
    let resp = ui.interact(main, ui.id().with(("nav", label)), Sense::click());

    let fill = if selected {
        mix(PANEL, ACCENT, 0.2)
    } else if resp.hovered() {
        CARD
    } else {
        Color32::TRANSPARENT
    };
    let p = ui.painter();
    p.rect_filled(rect, 8.0, fill);
    if drop_hover {
        p.rect(
            rect,
            8.0,
            ACCENT.gamma_multiply(0.12),
            Stroke::new(1.5_f32, ACCENT),
        );
    }
    let mut right = main.right() - 4.0;
    let count_galley = p.layout_no_wrap(count.to_string(), egui::FontId::proportional(12.0), DIM);
    right -= count_galley.size().x;
    p.galley(
        egui::pos2(right, rect.center().y - count_galley.size().y / 2.0),
        count_galley,
        DIM,
    );
    if sev != Severity::Ok {
        right -= 12.0;
        p.circle_filled(egui::pos2(right, rect.center().y), 4.0, sev_color(sev));
        right -= 6.0;
    }
    let text_color = if selected { TEXT } else { mix(DIM, TEXT, 0.5) };
    let galley = p.layout_no_wrap(
        label.to_string(),
        egui::FontId::proportional(14.0),
        text_color,
    );
    let clip = egui::Rect::from_min_max(rect.min, egui::pos2(right - 8.0, rect.max.y));
    let text_pos = egui::pos2(rect.left() + 12.0, rect.center().y - galley.size().y / 2.0);
    p.with_clip_rect(clip).galley(text_pos, galley, text_color);

    let mut shutter = None;
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(buttons)
            .layout(Layout::left_to_right(Align::Center)),
    );
    child.spacing_mut().item_spacing.x = 4.0;
    child.spacing_mut().button_padding = Vec2::new(2.0, 2.0);
    child.add_enabled_ui(count > 0, |ui| {
        if mini(ui, "Close", BTN_W)
            .on_hover_text(format!("Close shutters: {label}"))
            .clicked()
        {
            shutter = Some(true);
        }
        if mini(ui, "Open", BTN_W)
            .on_hover_text(format!("Open shutters: {label}"))
            .clicked()
        {
            shutter = Some(false);
        }
    });
    NavOut { resp, shutter }
}

fn welcome(ui: &mut egui::Ui, acts: &mut Vec<Act>) {
    ui.vertical_centered(|ui| {
        ui.add_space((ui.available_height() * 0.22).max(40.0));
        ui.label(RichText::new("No projectors yet").size(28.0).strong());
        ui.add_space(4.0);
        ui.label(dim(
            "Find the projectors on your network, or add one by address.",
        ));
        ui.add_space(22.0);
        ui.spacing_mut().button_padding = Vec2::new(22.0, 11.0);
        if primary(ui, "🔍  Discover projectors", ACCENT).clicked() {
            acts.push(Act::OpenScan);
        }
        ui.add_space(4.0);
        if ui.button("➕  Add manually").clicked() {
            acts.push(Act::EditNew);
        }
        ui.add_space(4.0);
        if ui
            .add(egui::Button::new(dim("Try it with demo projectors")).frame(false))
            .clicked()
        {
            acts.push(Act::SetDemo(true));
        }
    });
}
