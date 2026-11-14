//! The application window.

mod dialogs;
mod theme;

use eframe::egui::{self, Align, Color32, Layout, RichText, Sense, Stroke, Vec2};
use projectorhud::config::{address_key, clamp_zoom, AppConfig, SortKey, ZOOM_MAX, ZOOM_MIN};
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
const CARD_H: f32 = 238.0;
const GAP: f32 = 14.0;
const DEMO_GROUP: &str = "Demo";

pub type Item = (ProjectorConfig, ProjectorState);

/// Projectors being dragged to another group.
struct DragIds(Vec<u64>);

/// Everything the UI can ask for. Collected during a frame, applied after it.
pub enum Act {
    Send(u64, Command),
    SendMany(Vec<u64>, Command),
    /// Highlight one projector, or clear the highlight.
    Select(Option<u64>),
    /// Open the details pane for one projector, or close it.
    Detail(Option<u64>),
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
    SetZoom(f32),
    SetCompact(bool),
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
    /// Highlighted projectors: what dragging and the bulk controls act on.
    selection: BTreeSet<u64>,
    /// The projector in the details pane. Opened with a card's Details
    /// button, and independent of what is highlighted.
    detail: Option<u64>,
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

        let mut cfg = AppConfig::load();
        cfg.zoom = clamp_zoom(cfg.zoom);
        cc.egui_ctx.set_zoom_factor(cfg.zoom);
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
            detail: None,
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
                app.detail = by_name(&app, "Rehearsal");
            }
            Ok("multi") => {
                let ids = [by_name(&app, "Left"), by_name(&app, "Right")];
                app.selection.extend(ids.into_iter().flatten());
                app.detail = by_name(&app, "Rehearsal");
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

    fn apply(&mut self, act: Act) {
        match act {
            Act::Send(id, cmd) => self.hub.lock().unwrap().send(id, cmd),
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
            Act::Detail(id) => self.detail = id,
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
            Act::SetCompact(on) => {
                self.cfg.compact = on;
                self.save();
            }
            Act::SetZoom(z) => {
                self.cfg.zoom = clamp_zoom(z);
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
        // Cmd/Ctrl with + - 0 zooms too; keep that and the buttons in step.
        let seen = clamp_zoom(ctx.zoom_factor());
        if (seen - self.cfg.zoom).abs() > 0.01 {
            self.cfg.zoom = seen;
            self.save();
        }
        let zoom_before = self.cfg.zoom;

        let items = self.hub.lock().unwrap().snapshot();
        self.selection
            .retain(|id| items.iter().any(|i| i.0.id == *id));
        self.detail = self.detail.filter(|id| items.iter().any(|i| i.0.id == *id));
        let mut acts: Vec<Act> = Vec::new();

        self.top_bar(ctx, &items, &mut acts);
        self.alert_bar(ctx, &items, &mut acts);
        self.sidebar(ctx, &items, &mut acts);
        if let Some(item) = self
            .detail
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
        if (ctx.zoom_factor() - self.cfg.zoom).abs() > 0.001 || self.cfg.zoom != zoom_before {
            ctx.set_zoom_factor(self.cfg.zoom);
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
                            acts.push(Act::Detail(Some(cfg.id)));
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

                });
                ui.add_space(4.0);

                let footer = 64.0;
                egui::ScrollArea::vertical().max_height(ui.available_height() - footer).auto_shrink([false, true]).show(ui, |ui| {
                    let all: Vec<&Item> = items.iter().collect();
                    let scope = format!("all {} projectors", all.len());
                    let resp = nav_row(ui, "All projectors", &all, self.view == Target::All, false, &scope, acts);
                    if resp.clicked() {
                        acts.push(Act::View(Target::All));
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
                        let row_rect = egui::Rect::from_min_size(next.min, Vec2::new(next.width(), NAV_H));
                        let (over, dropped) = drop_zone(ui, row_rect);
                        let scope = format!("all {} in {g}", members.len());
                        let resp = nav_row(ui, &g, &members, self.view == Target::Group(g.clone()), over, &scope, acts);
                        if let Some(ids) = dropped {
                            acts.push(Act::MoveToGroup(ids, Some(g.clone())));
                        }
                        if resp.double_clicked() {
                            acts.push(Act::StartRename(g.clone()));
                        } else if resp.clicked() {
                            acts.push(Act::View(Target::Group(g.clone())));
                        }
                        resp.on_hover_text("Double-click to rename. Drop projectors here to move them.").context_menu(|ui| group_menu(ui, &g, acts));
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
                });

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
                        card_frame().show(ui, |ui| {
                            let shape = Shape::of(self.cfg.compact);
                            ui.allocate_ui_with_layout(Vec2::new(shape.width() * 2.0 + 8.0, shape.height()), Layout::left_to_right(Align::Center), |ui| {
                                ui.add_enabled_ui(n > 0, |ui| group_controls(ui, &visible, shape, &scope, acts));
                            });
                        });
                    });
                });

                self.tools_row(ui, items, acts);
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
                        self.grid(ui, None, &visible, acts, &mut order);
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
                                let name = group.as_deref().unwrap_or("Ungrouped");
                                let head = Head {
                                    title: name.to_string(),
                                    group: group.clone(),
                                    members: &members,
                                    scope: format!("all {} in {name}", members.len()),
                                };
                                self.grid(ui, Some(&head), &members, acts, &mut order);
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
    fn tools_row(&mut self, ui: &mut egui::Ui, items: &[Item], acts: &mut Vec<Act>) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.spacing_mut().button_padding = Vec2::new(10.0, 3.0);
            ui.label(small("View"));
            for (label, compact, tip) in [
                ("Full", false, "A card for each projector, with every control"),
                ("Compact", true, "One line for each projector, to fit more on screen"),
            ] {
                let active = self.cfg.compact == compact;
                if toggle(ui, Vec2::new(0.0, 24.0), label, active.then_some(ACCENT), true).on_hover_text(tip).clicked() && !active {
                    acts.push(Act::SetCompact(compact));
                }
            }
            ui.add_space(14.0);
            ui.label(small("Sort by"));
            for key in SortKey::ALL {
                let active = self.cfg.sort == key;
                if toggle(ui, Vec2::new(0.0, 24.0), key.label(), active.then_some(ACCENT), true).clicked() && !active {
                    acts.push(Act::SetSort(key));
                }
            }

            ui.add_space(14.0);
            ui.label(small("Zoom"));
            let z = self.cfg.zoom;
            let step = |ui: &mut egui::Ui, text: &str, tip: &str, to: f32, enabled: bool, acts: &mut Vec<Act>| {
                if toggle(ui, Vec2::new(26.0, 24.0), text, None, enabled).on_hover_text(tip).clicked() {
                    acts.push(Act::SetZoom(to));
                }
            };
            step(ui, "−", "Smaller: show more projectors (Cmd/Ctrl −)", z - 0.1, z > ZOOM_MIN + 0.01, acts);
            let pct = format!("{:.0}%", z * 100.0);
            step(ui, &pct, "Back to 100% (Cmd/Ctrl 0)", 1.0, (z - 1.0).abs() > 0.01, acts);
            step(ui, "+", "Larger: show fewer projectors (Cmd/Ctrl +)", z + 0.1, z < ZOOM_MAX - 0.01, acts);

            if self.selection.len() < 2 {
                ui.add_space(14.0);
                ui.add(
                    egui::Label::new(
                        RichText::new("Cmd/Ctrl-click or Shift-click selects several. Drag to move between groups.").color(FAINT).size(12.0),
                    )
                    .truncate(),
                );
            }
        });

        // Several highlighted: their own card of controls, on its own row so
        // it fits beside an open details pane.
        let n = self.selection.len();
        if n < 2 {
            return;
        }
        let ids: Vec<u64> = self.selection.iter().copied().collect();
        let sel: Vec<&Item> = items
            .iter()
            .filter(|i| self.selection.contains(&i.0.id))
            .collect();
        ui.add_space(4.0);
        card_frame()
            .stroke(Stroke::new(1.5_f32, ACCENT))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().button_padding = Vec2::new(10.0, 3.0);
                    ui.label(
                        RichText::new(format!("{n} selected"))
                            .color(ACCENT)
                            .strong(),
                    );
                    ui.add_space(6.0);
                    group_controls(
                        ui,
                        &sel,
                        Shape::of(self.cfg.compact),
                        &format!("the {n} selected projectors"),
                        acts,
                    );
                    ui.add_space(6.0);
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
                    if ui
                        .button(RichText::new("Clear").size(12.0))
                        .on_hover_text("Clear the selection")
                        .clicked()
                    {
                        acts.push(Act::Select(None));
                    }
                });
            });
    }

    /// Cards in rows. A group's own card, when given, leads its projectors.
    fn grid(
        &self,
        ui: &mut egui::Ui,
        head: Option<&Head>,
        items: &[&Item],
        acts: &mut Vec<Act>,
        order: &mut Vec<u64>,
    ) {
        enum Tile<'a> {
            Head(&'a Head<'a>),
            Projector(&'a Item),
        }
        if self.cfg.compact {
            ui.scope(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(6.0, 5.0);
                if let Some(head) = head {
                    group_row(ui, head, acts);
                }
                for (cfg, st) in items.iter().map(|i| (&i.0, &i.1)) {
                    order.push(cfg.id);
                    self.row(ui, cfg, st, acts);
                }
            });
            return;
        }
        let tiles: Vec<Tile> = head
            .map(Tile::Head)
            .into_iter()
            .chain(items.iter().map(|i| Tile::Projector(i)))
            .collect();
        let avail = ui.available_width();
        let cols = (((avail + GAP) / (CARD_MIN_W + GAP)).floor() as usize).max(1);
        let w = ((avail - GAP * (cols as f32 - 1.0)) / cols as f32).floor();
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing = Vec2::splat(GAP);
            for row in tiles.chunks(cols) {
                ui.horizontal(|ui| {
                    for tile in row {
                        match tile {
                            Tile::Head(head) => group_card(ui, w, head, acts),
                            Tile::Projector((cfg, st)) => {
                                order.push(cfg.id);
                                self.card(ui, w, cfg, st, acts);
                            }
                        }
                    }
                });
            }
        });
    }

    /// What a projector's card and its compact line share: the background
    /// (tinted and outlined by fault), highlighting, dragging and the
    /// right-click menu. Returns where to draw the contents.
    fn shell(
        &self,
        ui: &mut egui::Ui,
        size: Vec2,
        radius: f32,
        cfg: &ProjectorConfig,
        st: &ProjectorState,
        acts: &mut Vec<Act>,
    ) -> egui::Rect {
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
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
            .rect(rect, radius, fill, Stroke::new(width, border));
        if selected {
            ui.painter()
                .rect_stroke(rect.expand(3.0), radius + 3.0, Stroke::new(2.0_f32, ACCENT));
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
        rect
    }

    /// Compact mode: a projector on one line. Name, faults, then the same
    /// power and shutter controls as its card, laid out sideways.
    fn row(
        &self,
        ui: &mut egui::Ui,
        cfg: &ProjectorConfig,
        st: &ProjectorState,
        acts: &mut Vec<Act>,
    ) {
        let rect = self.shell(
            ui,
            Vec2::new(ui.available_width(), ROW_H),
            8.0,
            cfg,
            st,
            acts,
        );
        let mut child = row_ui(ui, rect);
        let ui = &mut child;
        details_button(ui, self.detail == Some(cfg.id), cfg.id, acts);
        ui.allocate_ui_with_layout(
            Vec2::new(Shape::COMPACT.width() * 2.0 + 8.0, ROW_H - 10.0),
            Layout::left_to_right(Align::Center),
            |ui| compact_controls(ui, cfg, st, acts),
        );
        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
            dot(ui, power_color(st));
            let (text, color, strong) = fault_line(st);
            row_text(
                ui,
                &cfg.name,
                14.0,
                fault_text(text, color, strong),
                small(subtitle(cfg, st)),
            );
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
        let rect = self.shell(ui, Vec2::new(w, CARD_H), 12.0, cfg, st, acts);
        let sev = st.severity();
        let issues = st.issues();
        let online = st.link == Link::Online;

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
            // Address and model, with the way into the details pane.
            ui.allocate_ui_with_layout(
                Vec2::new(ui.available_width(), 22.0),
                Layout::right_to_left(Align::Center),
                |ui| {
                    details_button(ui, self.detail == Some(cfg.id), cfg.id, acts);
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        ui.add(egui::Label::new(small(subtitle(cfg, st))).truncate());
                    });
                },
            );
            ui.add_space(2.0);

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
                egui::pos2(inner.left(), inner.bottom() - CONTROLS_H),
                inner.right_bottom(),
            );
            let mut row = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(bottom)
                    .layout(Layout::left_to_right(Align::Center)),
            );
            row.spacing_mut().item_spacing.x = 6.0;
            row.spacing_mut().button_padding = Vec2::new(4.0, 4.0);
            control_buttons(&mut row, inner.width(), cfg, st, acts);
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
                        acts.push(Act::Detail(None));
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
                        let w = ui.available_width();
                        control_buttons(ui, w, cfg, st, acts);
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

/// Height of the control block on a card: button, status, button.
const CONTROLS_H: f32 = 84.0;

/// The controls for one projector. Power and shutter each get a stack: the
/// "on" action above, the "off" action below, and the present state between
/// them, so both actions are always in the same place. Whichever would do
/// nothing is greyed out. Mute and freeze sit in a third, narrower stack.
fn control_buttons(
    ui: &mut egui::Ui,
    width: f32,
    cfg: &ProjectorConfig,
    st: &ProjectorState,
    acts: &mut Vec<Act>,
) {
    const BTN_H: f32 = 28.0;
    const GAP: f32 = 8.0;
    let online = st.link == Link::Online;
    let on = online && st.power == Power::On;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, CONTROLS_H), Sense::hover());
    let side = 66.0_f32.min(width * 0.22);
    let col = ((width - side - GAP * 2.0) / 2.0).floor();
    let column = |ui: &mut egui::Ui, x: f32, w: f32| {
        let r = egui::Rect::from_min_size(
            egui::pos2(rect.left() + x, rect.top()),
            Vec2::new(w, CONTROLS_H),
        );
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(r)
                .layout(Layout::top_down(Align::Min)),
        );
        child.spacing_mut().item_spacing.y = 0.0;
        child
    };
    let status_h = CONTROLS_H - BTN_H * 2.0;

    // Power
    {
        let mut ui = column(ui, 0.0, col);
        let size = Vec2::new(col, BTN_H);
        let r = state_button(
            &mut ui,
            size,
            "Power on",
            13.0,
            Look::Neutral,
            online && st.power == Power::Standby,
        );
        if r.on_disabled_hover_text("Available when the projector is in standby")
            .clicked()
        {
            acts.push(Act::Send(cfg.id, Command::PowerOn));
        }
        let (text, look) = match st.power {
            _ if !online => ("—", Look::Neutral),
            Power::On => ("ON", POWER_ON),
            Power::Standby => ("OFF", POWER_OFF),
            Power::Warming => (
                "WARMING",
                Look::Solid {
                    fill: AMBER,
                    text: Color32::from_rgb(0x08, 0x0B, 0x10),
                },
            ),
            Power::Cooling => (
                "COOLING",
                Look::Solid {
                    fill: COOL,
                    text: Color32::from_rgb(0x08, 0x0B, 0x10),
                },
            ),
            Power::Unknown => ("—", Look::Neutral),
        };
        status_bar(&mut ui, Vec2::new(col, status_h), text, look);
        let r = state_button(&mut ui, size, "Power off", 13.0, Look::Neutral, on);
        if r.on_disabled_hover_text("Available when the projector is on")
            .clicked()
        {
            acts.push(power_off_confirm(cfg));
        }
    }

    // Shutter
    {
        let mut ui = column(ui, col + GAP, col);
        let size = Vec2::new(col, BTN_H);
        let state = if on { st.video_mute } else { None };
        let r = state_button(
            &mut ui,
            size,
            "Open shutter",
            13.0,
            Look::Neutral,
            state == Some(true),
        );
        if r.on_disabled_hover_text("Available when the shutter is closed")
            .clicked()
        {
            acts.push(Act::Send(cfg.id, Command::VideoMute(false)));
        }
        let (text, look) = match state {
            Some(false) => ("OPEN", SHUTTER_OPEN),
            Some(true) => ("CLOSED", SHUTTER_CLOSED),
            None => ("—", Look::Neutral),
        };
        status_bar(&mut ui, Vec2::new(col, status_h), text, look);
        let r = state_button(
            &mut ui,
            size,
            "Close shutter",
            13.0,
            Look::Neutral,
            state == Some(false),
        );
        if r.on_disabled_hover_text("Available when the projector is on and the shutter is open")
            .clicked()
        {
            acts.push(Act::Send(cfg.id, Command::VideoMute(true)));
        }
    }

    // Mute and freeze
    {
        let mut ui = column(ui, (col + GAP) * 2.0, side);
        let size = Vec2::new(side, BTN_H);
        let ink = Color32::from_rgb(0x08, 0x0B, 0x10);
        let muted = st.audio_mute == Some(true);
        let look = if muted {
            Look::Solid {
                fill: AMBER,
                text: ink,
            }
        } else {
            Look::Neutral
        };
        let r = state_button(
            &mut ui,
            size,
            "Mute",
            13.0,
            look,
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
        ui.add_space(status_h);
        let frozen = st.freeze == Some(true);
        let look = if frozen {
            Look::Solid {
                fill: COOL,
                text: ink,
            }
        } else {
            Look::Neutral
        };
        let r = state_button(
            &mut ui,
            size,
            "Freeze",
            13.0,
            look,
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
}

/// The state of a whole group, for its buttons.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tri {
    AllOn,
    AllOff,
    Mixed,
    Unknown,
}

fn tri(states: impl Iterator<Item = bool>) -> Tri {
    let (mut on, mut off) = (false, false);
    for s in states {
        if s {
            on = true;
        } else {
            off = true;
        }
    }
    match (on, off) {
        (true, true) => Tri::Mixed,
        (true, false) => Tri::AllOn,
        (false, true) => Tri::AllOff,
        (false, false) => Tri::Unknown,
    }
}

/// "On" means closed. Only projectors that are on and report their shutter count.
fn shutter_state(items: &[&Item]) -> Tri {
    tri(items
        .iter()
        .filter(|i| i.1.link == Link::Online && i.1.power == Power::On)
        .filter_map(|i| i.1.video_mute))
}

fn power_state(items: &[&Item]) -> Tri {
    tri(items
        .iter()
        .filter(|i| i.1.link == Link::Online)
        .filter_map(|i| match i.1.power {
            Power::On | Power::Warming => Some(true),
            Power::Standby | Power::Cooling => Some(false),
            Power::Unknown => None,
        }))
}

#[derive(Clone, Copy)]
enum Kind {
    Power,
    Shutter,
}

const STACK_W: f32 = 112.0;
const STACK_BTN_H: f32 = 22.0;
const STACK_H: f32 = STACK_BTN_H * 3.0;

/// A group's control for power or for shutters: the "on" action above, the
/// "off" action below, and the group's state between them (striped when its
/// members differ). A button that would change nothing is greyed out.
/// Returns the state to put the group in: `Some(true)` = power on / close.
fn group_stack(ui: &mut egui::Ui, kind: Kind, state: Tri, shape: Shape) -> Option<bool> {
    let known = state != Tri::Unknown && ui.is_enabled();
    // `top_sets`: the state the first button puts the group in.
    let (top, bottom, top_sets, on_look, off_look, on_text, off_text) = match kind {
        Kind::Power => (
            if shape.horizontal { "On" } else { "Power on" },
            if shape.horizontal { "Off" } else { "Power off" },
            true,
            POWER_ON,
            POWER_OFF,
            "ON",
            "OFF",
        ),
        Kind::Shutter => (
            if shape.horizontal {
                "Open"
            } else {
                "Open shutters"
            },
            if shape.horizontal {
                "Close"
            } else {
                "Close shutters"
            },
            false,
            SHUTTER_CLOSED,
            SHUTTER_OPEN,
            "CLOSED",
            "OPEN",
        ),
    };
    let already = |v: bool| state == if v { Tri::AllOn } else { Tri::AllOff };
    let (text, look) = match state {
        Tri::AllOn => (on_text, on_look),
        Tri::AllOff => (off_text, off_look),
        Tri::Mixed => ("MIXED", Look::striped(on_look, off_look)),
        Tri::Unknown => ("—", Look::Neutral),
    };
    let tips = match kind {
        Kind::Power => ("Power them on", "Power them off"),
        Kind::Shutter => ("Open their shutters", "Close their shutters"),
    };
    let (first, second) = three(
        ui,
        shape,
        (top, known && !already(top_sets), tips.0),
        (text, look),
        (bottom, known && !already(!top_sets), tips.1),
    );
    if first {
        Some(top_sets)
    } else if second {
        Some(!top_sets)
    } else {
        None
    }
}

/// Power and shutter stacks, side by side, for a set of projectors.
fn group_controls(
    ui: &mut egui::Ui,
    items: &[&Item],
    shape: Shape,
    scope: &str,
    acts: &mut Vec<Act>,
) {
    let ids: Vec<u64> = items.iter().map(|i| i.0.id).collect();
    ui.spacing_mut().item_spacing.x = 8.0;
    if let Some(on) = group_stack(ui, Kind::Power, power_state(items), shape) {
        acts.push(power_many(ids.clone(), on, scope));
    }
    if let Some(close) = group_stack(ui, Kind::Shutter, shutter_state(items), shape) {
        acts.push(Act::SendMany(ids, Command::VideoMute(close)));
    }
}

fn power_many(ids: Vec<u64>, on: bool, scope: &str) -> Act {
    Act::Confirm(Confirm {
        title: format!("Power {} {scope}?", if on { "on" } else { "off" }),
        body: if on {
            "Projectors already on are not affected."
        } else {
            "They will cool down and go to standby."
        }
        .into(),
        button: if on { "Power on" } else { "Power off" }.into(),
        danger: !on,
        act: Box::new(Act::SendMany(
            ids,
            if on {
                Command::PowerOn
            } else {
                Command::PowerOff
            },
        )),
    })
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

/// Height of one row of the left column: its name, then its controls.
const NAV_H: f32 = 34.0 + STACK_H + 10.0;

/// One row of the left column: a group's name and count, with its power and
/// shutter controls underneath. Returns the response of the name strip.
fn nav_row(
    ui: &mut egui::Ui,
    label: &str,
    members: &[&Item],
    selected: bool,
    drop_hover: bool,
    scope: &str,
    acts: &mut Vec<Act>,
) -> egui::Response {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), NAV_H), Sense::hover());
    let main = egui::Rect::from_min_size(rect.min, Vec2::new(rect.width(), 34.0));
    let resp = ui.interact(main, ui.id().with(("nav", label)), Sense::click());

    let fill = if selected {
        mix(PANEL, ACCENT, 0.2)
    } else if resp.hovered() {
        CARD
    } else {
        mix(PANEL, CARD, 0.45)
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
    let mut right = main.right() - 12.0;
    let count_galley = p.layout_no_wrap(
        members.len().to_string(),
        egui::FontId::proportional(12.0),
        DIM,
    );
    right -= count_galley.size().x;
    p.galley(
        egui::pos2(right, main.center().y - count_galley.size().y / 2.0),
        count_galley,
        DIM,
    );
    let sev = worst(members);
    if sev != Severity::Ok {
        right -= 12.0;
        p.circle_filled(egui::pos2(right, main.center().y), 4.0, sev_color(sev));
        right -= 6.0;
    }
    let text_color = if selected { TEXT } else { mix(DIM, TEXT, 0.6) };
    let galley = p.layout_no_wrap(
        label.to_string(),
        egui::FontId::proportional(14.0),
        text_color,
    );
    let clip = egui::Rect::from_min_max(main.min, egui::pos2(right - 8.0, main.max.y));
    let text_pos = egui::pos2(main.left() + 12.0, main.center().y - galley.size().y / 2.0);
    p.with_clip_rect(clip).galley(text_pos, galley, text_color);

    let area = egui::Rect::from_min_max(
        egui::pos2(rect.left() + 10.0, main.bottom()),
        egui::pos2(rect.right() - 10.0, rect.bottom() - 10.0),
    );
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(area)
            .layout(Layout::left_to_right(Align::Min)),
    );
    let w = ((area.width() - 8.0) / 2.0).floor();
    child.add_enabled_ui(!members.is_empty(), |ui| {
        group_controls(ui, members, Shape::stack(w, STACK_BTN_H), scope, acts)
    });
    resp
}

/// The frame of a card, for controls that sit outside the grid.
fn card_frame() -> egui::Frame {
    egui::Frame::none()
        .fill(CARD)
        .stroke(Stroke::new(1.0_f32, STROKE))
        .rounding(12.0)
        .inner_margin(egui::Margin::same(10.0))
}

/// What a group's card shows and acts on.
struct Head<'a> {
    title: String,
    /// The group it stands for; `None` for the ungrouped projectors.
    group: Option<String>,
    members: &'a [&'a Item],
    /// How a confirmation names these projectors, e.g. "all 3 in Stage".
    scope: String,
}

fn group_menu(ui: &mut egui::Ui, g: &str, acts: &mut Vec<Act>) {
    if ui.button("Rename").clicked() {
        acts.push(Act::StartRename(g.to_string()));
        ui.close_menu();
    }
    if ui
        .button(RichText::new("Delete group…").color(RED))
        .clicked()
    {
        acts.push(Act::Confirm(Confirm {
            title: format!("Delete group “{g}”?"),
            body: "Its projectors are kept and become ungrouped.".into(),
            button: "Delete group".into(),
            danger: true,
            act: Box::new(Act::DeleteGroup(g.to_string())),
        }));
        ui.close_menu();
    }
}

/// A group's own card. It has the shape of a projector's card, with the power
/// and shutter stacks in the same place, so a row reads the same way across.
fn group_card(ui: &mut egui::Ui, w: f32, head: &Head, acts: &mut Vec<Act>) {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, CARD_H), Sense::click());
    let members = head.members;
    let fill = mix(CARD, ACCENT, if resp.hovered() { 0.13 } else { 0.09 });
    ui.painter().rect(
        rect,
        12.0,
        fill,
        Stroke::new(1.0_f32, mix(STROKE, ACCENT, 0.55)),
    );
    if let Some(g) = &head.group {
        if resp.double_clicked() {
            acts.push(Act::StartRename(g.clone()));
        }
        resp.on_hover_text("Double-click to rename. Drop projectors here to move them.")
            .context_menu(|ui| group_menu(ui, g, acts));
    }

    let inner = rect.shrink2(Vec2::new(16.0, 14.0));
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(Layout::top_down(Align::Min)),
    );
    {
        let ui = &mut child;
        ui.spacing_mut().item_spacing = Vec2::new(8.0, 4.0);
        ui.allocate_ui_with_layout(
            Vec2::new(ui.available_width(), 22.0),
            Layout::right_to_left(Align::Center),
            |ui| {
                pill(ui, "GROUP", ACCENT);
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.add(
                        egui::Label::new(RichText::new(&head.title).size(16.0).strong()).truncate(),
                    );
                });
            },
        );

        let n = members.len();
        let count =
            |f: &dyn Fn(&ProjectorState) -> bool| members.iter().filter(|i| f(&i.1)).count();
        let on = count(&|s| s.link == Link::Online && s.power == Power::On);
        let checking = count(&|s| s.link == Link::Connecting);
        let troubled = count(&|s| s.severity() != Severity::Ok);
        let noun = if n == 1 { "projector" } else { "projectors" };
        ui.allocate_ui_with_layout(
            Vec2::new(ui.available_width(), 22.0),
            Layout::left_to_right(Align::Center),
            |ui| {
                let text = if n == 0 {
                    "No projectors yet".to_string()
                } else {
                    format!("{n} {noun}  ·  {on} on")
                };
                ui.add(egui::Label::new(small(text)).truncate());
            },
        );
        ui.add_space(2.0);
        ui.allocate_ui_with_layout(
            Vec2::new(ui.available_width(), 28.0),
            Layout::left_to_right(Align::Center),
            |ui| {
                ui.add(
                    egui::Label::new(dim(if head.group.is_some() {
                        "Controls every projector in this group"
                    } else {
                        "Controls every ungrouped projector"
                    }))
                    .truncate(),
                );
            },
        );
        ui.allocate_ui_with_layout(
            Vec2::new(ui.available_width(), 18.0),
            Layout::left_to_right(Align::Center),
            |ui| {
                let sev = worst(members);
                let (text, color) = if troubled > 0 {
                    let verb = if troubled == 1 { "needs" } else { "need" };
                    (format!("⚠  {troubled} {verb} attention"), sev_color(sev))
                } else if n == 0 {
                    ("Drag projectors here".to_string(), FAINT)
                } else if checking > 0 {
                    ("Checking…".to_string(), FAINT)
                } else {
                    ("✔  No errors".to_string(), GREEN)
                };
                let mut rich = RichText::new(text).color(color).size(13.0);
                if troubled > 0 {
                    rich = rich.strong();
                }
                ui.add(egui::Label::new(rich).truncate());
            },
        );

        // The same columns as a projector's power and shutter stacks.
        let bottom = egui::Rect::from_min_max(
            egui::pos2(inner.left(), inner.bottom() - CONTROLS_H),
            inner.right_bottom(),
        );
        let mut row = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(bottom)
                .layout(Layout::left_to_right(Align::Min)),
        );
        row.spacing_mut().button_padding = Vec2::new(4.0, 4.0);
        let side = 66.0_f32.min(inner.width() * 0.22);
        let col = ((inner.width() - side - 16.0) / 2.0).floor();
        row.add_enabled_ui(n > 0, |ui| {
            group_controls(
                ui,
                members,
                Shape::stack(col, CONTROLS_H / 3.0),
                &head.scope,
                acts,
            )
        });
    }
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

/// Height of a projector's line in compact mode.
const ROW_H: f32 = 40.0;

/// How a button / state / button control is laid out: stacked on the cards,
/// sideways with short labels in compact mode.
#[derive(Clone, Copy)]
struct Shape {
    horizontal: bool,
    button: Vec2,
    status: Vec2,
    font: f32,
}

impl Shape {
    const COMPACT: Shape = Shape {
        horizontal: true,
        button: Vec2::new(42.0, 26.0),
        status: Vec2::new(62.0, 26.0),
        font: 12.0,
    };
    fn stack(width: f32, btn_h: f32) -> Shape {
        Shape {
            horizontal: false,
            button: Vec2::new(width, btn_h),
            status: Vec2::new(width, btn_h),
            font: if btn_h >= 28.0 { 13.0 } else { 12.0 },
        }
    }
    /// The stacks for a view or a selection, in the current mode.
    fn of(compact: bool) -> Shape {
        if compact {
            Shape::COMPACT
        } else {
            Shape::stack(STACK_W, STACK_BTN_H)
        }
    }
    fn width(&self) -> f32 {
        if self.horizontal {
            self.button.x * 2.0 + self.status.x + 6.0
        } else {
            self.button.x
        }
    }
    fn height(&self) -> f32 {
        if self.horizontal {
            self.button.y
        } else {
            self.button.y * 2.0 + self.status.y
        }
    }
}

/// A button, the present state, and the opposite button. Each button is
/// (label, enabled, tooltip). Returns which of the two was clicked.
fn three(
    ui: &mut egui::Ui,
    shape: Shape,
    first: (&str, bool, &str),
    status: (&str, Look),
    second: (&str, bool, &str),
) -> (bool, bool) {
    let mut out = (false, false);
    let layout = if shape.horizontal {
        Layout::left_to_right(Align::Center)
    } else {
        Layout::top_down(Align::Min)
    };
    ui.allocate_ui_with_layout(Vec2::new(shape.width(), shape.height()), layout, |ui| {
        ui.spacing_mut().item_spacing = if shape.horizontal {
            Vec2::new(3.0, 0.0)
        } else {
            Vec2::ZERO
        };
        ui.spacing_mut().button_padding = Vec2::new(4.0, 3.0);
        let button = |ui: &mut egui::Ui, (label, enabled, tip): (&str, bool, &str)| {
            let r = state_button(ui, shape.button, label, shape.font, Look::Neutral, enabled);
            if shape.horizontal {
                r.on_hover_text(tip).clicked()
            } else {
                r.clicked()
            }
        };
        out.0 = button(ui, first);
        status_bar(ui, shape.status, status.0, status.1);
        out.1 = button(ui, second);
    });
    out
}

fn power_status(st: &ProjectorState) -> (&'static str, Look) {
    let ink = Color32::from_rgb(0x08, 0x0B, 0x10);
    match st.power {
        _ if st.link != Link::Online => ("—", Look::Neutral),
        Power::On => ("ON", POWER_ON),
        Power::Standby => ("OFF", POWER_OFF),
        Power::Warming => (
            "WARMING",
            Look::Solid {
                fill: AMBER,
                text: ink,
            },
        ),
        Power::Cooling => (
            "COOLING",
            Look::Solid {
                fill: COOL,
                text: ink,
            },
        ),
        Power::Unknown => ("—", Look::Neutral),
    }
}

/// One projector's power and shutter controls on a line.
fn compact_controls(
    ui: &mut egui::Ui,
    cfg: &ProjectorConfig,
    st: &ProjectorState,
    acts: &mut Vec<Act>,
) {
    let online = st.link == Link::Online;
    let on = online && st.power == Power::On;
    ui.spacing_mut().item_spacing.x = 8.0;
    let (power_on, power_off) = three(
        ui,
        Shape::COMPACT,
        ("On", online && st.power == Power::Standby, "Power on"),
        power_status(st),
        ("Off", on, "Power off"),
    );
    if power_on {
        acts.push(Act::Send(cfg.id, Command::PowerOn));
    }
    if power_off {
        acts.push(power_off_confirm(cfg));
    }
    let shutter = if on { st.video_mute } else { None };
    let (open, close) = three(
        ui,
        Shape::COMPACT,
        ("Open", shutter == Some(true), "Open shutter"),
        match shutter {
            Some(false) => ("OPEN", SHUTTER_OPEN),
            Some(true) => ("CLOSED", SHUTTER_CLOSED),
            None => ("—", Look::Neutral),
        },
        ("Close", shutter == Some(false), "Close shutter"),
    );
    if open {
        acts.push(Act::Send(cfg.id, Command::VideoMute(false)));
    }
    if close {
        acts.push(Act::Send(cfg.id, Command::VideoMute(true)));
    }
}

/// What a projector has to say about faults: text, colour, and whether it
/// needs attention.
fn fault_line(st: &ProjectorState) -> (String, Color32, bool) {
    let issues = st.issues();
    let sev = st.severity();
    match st.link {
        Link::Connecting => ("Checking…".into(), FAINT, false),
        Link::AuthFailed => ("⚠  Password rejected".into(), RED, true),
        Link::Offline => ("⚠  Not responding".into(), RED, true),
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
            true,
        ),
        Link::Online if st.errors.is_some() => ("✔  No errors".into(), GREEN, false),
        Link::Online => ("Errors not reported".into(), FAINT, false),
    }
}

/// The same for a set of projectors.
fn group_fault_line(members: &[&Item]) -> (String, Color32, bool) {
    let troubled = members
        .iter()
        .filter(|i| i.1.severity() != Severity::Ok)
        .count();
    let checking = members.iter().any(|i| i.1.link == Link::Connecting);
    if troubled > 0 {
        let verb = if troubled == 1 { "needs" } else { "need" };
        (
            format!("⚠  {troubled} {verb} attention"),
            sev_color(worst(members)),
            true,
        )
    } else if members.is_empty() {
        ("Drag projectors here".into(), FAINT, false)
    } else if checking {
        ("Checking…".into(), FAINT, false)
    } else {
        ("✔  No errors".into(), GREEN, false)
    }
}

/// The contents of a compact line, filled from the right.
fn row_ui(ui: &mut egui::Ui, rect: egui::Rect) -> egui::Ui {
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(Vec2::new(12.0, 5.0)))
            .layout(Layout::right_to_left(Align::Center)),
    );
    child.spacing_mut().item_spacing.x = 8.0;
    child
}

/// The left of a compact line: a name, the fault text, and whatever else
/// fits. With room, the columns line up down the list; without it, the name
/// takes only what it needs so the fault text stays readable.
fn row_text(ui: &mut egui::Ui, name: &str, name_size: f32, fault: RichText, extra: RichText) {
    let h = ui.available_height();
    let left = ui.available_width();
    let natural = ui
        .painter()
        .layout_no_wrap(
            name.to_string(),
            egui::FontId::proportional(name_size),
            TEXT,
        )
        .size()
        .x
        + 4.0;
    let name_w = if left >= 420.0 {
        (left * 0.38).min(240.0)
    } else {
        natural.min(left * 0.55)
    };
    let column = |ui: &mut egui::Ui, w: f32, text: RichText, tip: &str| {
        ui.allocate_ui_with_layout(
            Vec2::new(w, h),
            Layout::left_to_right(Align::Center),
            |ui| {
                ui.set_min_width(w);
                ui.add(egui::Label::new(text).truncate()).on_hover_text(tip);
            },
        );
    };
    column(
        ui,
        name_w,
        RichText::new(name).size(name_size).strong(),
        name,
    );
    let rest = ui.available_width();
    let fault_w = if rest > 300.0 { rest * 0.45 } else { rest };
    let tip = fault.text().to_string();
    column(ui, fault_w.max(0.0), fault, &tip);
    if ui.available_width() > 90.0 {
        ui.add(egui::Label::new(extra).truncate());
    }
}

fn fault_text(text: String, color: Color32, strong: bool) -> RichText {
    let rich = RichText::new(text).color(color).size(13.0);
    if strong {
        rich.strong()
    } else {
        rich
    }
}

fn details_button(ui: &mut egui::Ui, open: bool, id: u64, acts: &mut Vec<Act>) {
    ui.scope(|ui| {
        ui.spacing_mut().button_padding = Vec2::new(8.0, 1.0);
        let (fill, color) = if open {
            (mix(CARD, ACCENT, 0.35), TEXT)
        } else {
            (mix(CARD, TEXT, 0.08), mix(DIM, TEXT, 0.6))
        };
        let button = egui::Button::new(RichText::new("Details").size(12.0).color(color))
            .fill(fill)
            .stroke(Stroke::new(1.0_f32, STROKE))
            .rounding(6.0)
            .min_size(Vec2::new(58.0, 20.0));
        let tip = if open {
            "Close the details pane"
        } else {
            "Status, errors, lamp hours and the message log"
        };
        if ui.add(button).on_hover_text(tip).clicked() {
            acts.push(Act::Detail((!open).then_some(id)));
        }
    });
}

/// Compact mode: a group's line, leading its projectors, with its controls
/// in the same columns as theirs.
fn group_row(ui: &mut egui::Ui, head: &Head, acts: &mut Vec<Act>) {
    let (rect, resp) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::click());
    let members = head.members;
    let fill = mix(CARD, ACCENT, if resp.hovered() { 0.16 } else { 0.12 });
    ui.painter().rect(
        rect,
        8.0,
        fill,
        Stroke::new(1.0_f32, mix(STROKE, ACCENT, 0.55)),
    );
    if let Some(g) = &head.group {
        if resp.double_clicked() {
            acts.push(Act::StartRename(g.clone()));
        }
        resp.on_hover_text("Double-click to rename. Drop projectors here to move them.")
            .context_menu(|ui| group_menu(ui, g, acts));
    }
    let mut child = row_ui(ui, rect);
    let ui = &mut child;
    // The width of a projector's Details button, so the controls line up.
    ui.allocate_ui_with_layout(
        Vec2::new(58.0, ROW_H - 10.0),
        Layout::centered_and_justified(egui::Direction::LeftToRight),
        |ui| ui.label(RichText::new("GROUP").color(ACCENT).size(11.0).strong()),
    );
    ui.allocate_ui_with_layout(
        Vec2::new(Shape::COMPACT.width() * 2.0 + 8.0, ROW_H - 10.0),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.add_enabled_ui(!members.is_empty(), |ui| {
                group_controls(ui, members, Shape::COMPACT, &head.scope, acts)
            });
        },
    );
    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
        let n = members.len();
        let on = members
            .iter()
            .filter(|i| i.1.link == Link::Online && i.1.power == Power::On)
            .count();
        let (text, color, strong) = group_fault_line(members);
        row_text(
            ui,
            &head.title,
            15.0,
            fault_text(text, color, strong),
            small(format!(
                "{n} {}  ·  {on} on",
                if n == 1 { "projector" } else { "projectors" }
            )),
        );
    });
}
