//! The application window.

mod dialogs;
mod theme;

use eframe::egui::{self, Align, Color32, Layout, RichText, Sense, Stroke, Vec2};
use projectorhud::config::AppConfig;
use projectorhud::discovery::Scan;
use projectorhud::model::*;
use projectorhud::schedule::{self, LastRun, Schedule};
use projectorhud::sim::{self, SimHandle};
use projectorhud::worker::{Hub, Repaint, Target};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use theme::*;

const CARD_MIN_W: f32 = 340.0;
const CARD_H: f32 = 172.0;
const GAP: f32 = 14.0;
const DEMO_GROUP: &str = "Demo";

pub type Item = (ProjectorConfig, ProjectorState);

/// Everything the UI can ask for. Collected during a frame, applied after it.
pub enum Act {
    Send(u64, Command),
    SendTarget(Target, Command),
    Select(Option<u64>),
    View(Target),
    Confirm(Confirm),
    EditNew,
    Edit(u64),
    SaveProjector(ProjectorConfig),
    Remove(u64),
    SetGroup(u64, Option<String>),
    AddGroup(String),
    DeleteGroup(String),
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

pub struct App {
    cfg: AppConfig,
    hub: Arc<Mutex<Hub>>,
    schedules: Arc<Mutex<Vec<Schedule>>>,
    last_run: LastRun,
    repaint: Repaint,
    sims: Vec<SimHandle>,
    view: Target,
    selected: Option<u64>,
    confirm: Option<Confirm>,
    editor: Option<dialogs::Editor>,
    show_scan: bool,
    scan: Option<Scan>,
    scan_subnet: String,
    show_schedules: bool,
    new_group: Option<String>,
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
            selected: None,
            confirm: None,
            editor: None,
            show_scan: false,
            scan: None,
            scan_subnet: String::new(),
            show_schedules: false,
            new_group: None,
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
        match std::env::var("PROJECTORHUD_SHOT_VIEW").as_deref() {
            Ok("detail") => {
                app.selected = app
                    .cfg
                    .projectors
                    .iter()
                    .find(|p| p.name.contains("Rehearsal"))
                    .map(|p| p.id)
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
            self.selected = None;
        }
        self.save();
    }

    fn apply(&mut self, act: Act) {
        match act {
            Act::Send(id, cmd) => self.hub.lock().unwrap().send(id, cmd),
            Act::SendTarget(target, cmd) => {
                self.hub.lock().unwrap().send_target(&target, cmd);
            }
            Act::Select(id) => self.selected = if self.selected == id { None } else { id },
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
                if self.selected == Some(id) {
                    self.selected = None;
                }
                self.save();
            }
            Act::SetGroup(id, group) => {
                if let Some(p) = self.cfg.projectors.iter_mut().find(|p| p.id == id) {
                    p.group = group;
                    self.hub.lock().unwrap().upsert(p.clone());
                }
                self.save();
            }
            Act::AddGroup(name) => {
                let name = name.trim().to_string();
                if !name.is_empty() && !self.cfg.groups.contains(&name) {
                    self.cfg.groups.push(name.clone());
                    self.view = Target::Group(name);
                    self.save();
                }
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
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let items = self.hub.lock().unwrap().snapshot();
        let mut acts: Vec<Act> = Vec::new();

        self.top_bar(ctx, &items, &mut acts);
        self.alert_bar(ctx, &items, &mut acts);
        self.sidebar(ctx, &items, &mut acts);
        if let Some(item) = self
            .selected
            .and_then(|id| items.iter().find(|i| i.0.id == id))
        {
            self.detail_panel(ctx, item, &mut acts);
        }
        self.central(ctx, &items, &mut acts);
        self.dialogs(ctx, &items, &mut acts);

        for act in acts {
            self.apply(act);
        }

        // Keep the error pulse animating; otherwise tick slowly for timed messages.
        let attention = items.iter().any(|i| i.1.severity() == Severity::Error);
        ctx.request_repaint_after(Duration::from_millis(if attention { 60 } else { 1000 }));
        self.screenshot(ctx);
    }
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
                            pill(ui, &format!("{faults} with faults"), AMBER);
                        }
                        if offline == 0 && faults == 0 {
                            pill(ui, "All healthy", GREEN);
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
        let worst = troubled
            .iter()
            .map(|i| i.1.severity())
            .max()
            .unwrap_or_default();
        let color = sev_color(worst);
        egui::TopBottomPanel::top("alerts")
            .frame(
                egui::Frame::none()
                    .fill(mix(BG, color, 0.16))
                    .inner_margin(egui::Margin::symmetric(18.0, 8.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    let n = troubled.len();
                    ui.label(
                        RichText::new(format!(
                            "⚠  {n} projector{} need{} attention",
                            if n == 1 { "" } else { "s" },
                            if n == 1 { "s" } else { "" }
                        ))
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
                            if self.selected != Some(cfg.id) {
                                acts.push(Act::Select(Some(cfg.id)));
                            }
                        }
                    }
                });
            });
    }

    fn sidebar(&mut self, ctx: &egui::Context, items: &[Item], acts: &mut Vec<Act>) {
        egui::SidePanel::left("groups")
            .exact_width(210.0)
            .resizable(false)
            .frame(egui::Frame::none().fill(PANEL).inner_margin(egui::Margin::symmetric(10.0, 12.0)))
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                ui.horizontal(|ui| {
                    ui.add_space(8.0);
                    section(ui, "Groups");
                });
                ui.add_space(4.0);

                let worst = |f: &dyn Fn(&ProjectorConfig) -> bool| {
                    items.iter().filter(|i| f(&i.0)).map(|i| i.1.severity()).max().unwrap_or_default()
                };
                if nav_row(ui, "All projectors", items.len(), worst(&|_| true), self.view == Target::All).clicked() {
                    acts.push(Act::View(Target::All));
                }
                for g in self.cfg.groups.clone() {
                    let in_group = |c: &ProjectorConfig| c.group.as_deref() == Some(g.as_str());
                    let count = items.iter().filter(|i| in_group(&i.0)).count();
                    let resp = nav_row(ui, &g, count, worst(&in_group), self.view == Target::Group(g.clone()));
                    if resp.clicked() {
                        acts.push(Act::View(Target::Group(g.clone())));
                    }
                    resp.context_menu(|ui| {
                        if ui.button("Delete group").clicked() {
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
            .frame(egui::Frame::none().fill(BG).inner_margin(egui::Margin {
                left: 20.0,
                right: 8.0,
                top: 16.0,
                bottom: 0.0,
            }))
            .show(ctx, |ui| {
                if items.is_empty() {
                    welcome(ui, acts);
                    return;
                }
                let visible: Vec<&Item> =
                    items.iter().filter(|i| self.view.matches(&i.0)).collect();

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
                                acts.push(Act::SendTarget(
                                    self.view.clone(),
                                    Command::VideoMute(false),
                                ));
                            }
                            if ui.button("Close shutters").clicked() {
                                acts.push(Act::SendTarget(
                                    self.view.clone(),
                                    Command::VideoMute(true),
                                ));
                            }
                            if ui.button("Power off all").clicked() {
                                acts.push(Act::Confirm(Confirm {
                                    title: format!("Power off {scope}?"),
                                    body: "They will cool down and go to standby.".into(),
                                    button: "Power off all".into(),
                                    danger: true,
                                    act: Box::new(Act::SendTarget(
                                        self.view.clone(),
                                        Command::PowerOff,
                                    )),
                                }));
                            }
                            if ui.button("Power on all").clicked() {
                                acts.push(Act::Confirm(Confirm {
                                    title: format!("Power on {scope}?"),
                                    body: "Projectors already on are not affected.".into(),
                                    button: "Power on all".into(),
                                    danger: false,
                                    act: Box::new(Act::SendTarget(
                                        self.view.clone(),
                                        Command::PowerOn,
                                    )),
                                }));
                            }
                        });
                    });
                });
                ui.add_space(8.0);

                if visible.is_empty() {
                    ui.add_space(40.0);
                    ui.vertical_centered(|ui| {
                        ui.label(dim("No projectors in this group yet."));
                        ui.label(small(
                            "Right-click a projector and choose “Move to group”, or add a new one.",
                        ));
                    });
                    return;
                }

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let avail = ui.available_width() - 12.0;
                        let cols = (((avail + GAP) / (CARD_MIN_W + GAP)).floor() as usize).max(1);
                        let w = ((avail - GAP * (cols as f32 - 1.0)) / cols as f32).floor();
                        ui.spacing_mut().item_spacing = Vec2::splat(GAP);
                        for row in visible.chunks(cols) {
                            ui.horizontal(|ui| {
                                for (cfg, st) in row.iter().map(|i| (&i.0, &i.1)) {
                                    self.card(ui, w, cfg, st, acts);
                                }
                            });
                        }
                        ui.add_space(8.0);
                    });
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
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, CARD_H), Sense::click());
        if resp.clicked() {
            acts.push(Act::Select(Some(cfg.id)));
        }
        let selected = self.selected == Some(cfg.id);
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
        ui.painter()
            .rect(rect, 12.0, fill, Stroke::new(width, border));
        if selected {
            ui.painter()
                .rect_stroke(rect.expand(3.0), 15.0, Stroke::new(2.0_f32, ACCENT));
        }

        resp.context_menu(|ui| {
            if ui.button("Edit…").clicked() {
                acts.push(Act::Edit(cfg.id));
                ui.close_menu();
            }
            ui.menu_button("Move to group", |ui| {
                if ui
                    .selectable_label(cfg.group.is_none(), "No group")
                    .clicked()
                {
                    acts.push(Act::SetGroup(cfg.id, None));
                    ui.close_menu();
                }
                for g in &self.cfg.groups {
                    if ui
                        .selectable_label(cfg.group.as_deref() == Some(g.as_str()), g)
                        .clicked()
                    {
                        acts.push(Act::SetGroup(cfg.id, Some(g.clone())));
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

            // Faults (loud), failed command, or engaged functions (quiet)
            let recent_error = st
                .command_error
                .as_ref()
                .filter(|(_, at)| at.elapsed() < Duration::from_secs(8));
            if !issues.is_empty() && online {
                let text = issues
                    .iter()
                    .map(|i| i.0.as_str())
                    .collect::<Vec<_>>()
                    .join("  ·  ");
                ui.add(
                    egui::Label::new(
                        RichText::new(format!("⚠  {text}"))
                            .color(sev_color(sev))
                            .strong(),
                    )
                    .truncate(),
                );
            } else if let Some((msg, _)) = recent_error {
                ui.add(egui::Label::new(RichText::new(msg).color(AMBER).size(12.0)).truncate());
            } else if st.link == Link::AuthFailed {
                ui.label(
                    RichText::new("Right-click → Edit to set the password")
                        .color(RED)
                        .size(12.0),
                );
            } else if st.link == Link::Offline {
                ui.label(
                    RichText::new("⚠  Not responding — retrying automatically")
                        .color(RED)
                        .size(12.0),
                );
            } else {
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
                ui.label(RichText::new(flags.join("  ·  ")).color(AMBER).size(12.0));
            }

            // Controls pinned to the bottom edge
            let bottom = egui::Rect::from_min_max(
                egui::pos2(inner.left(), inner.bottom() - 30.0),
                inner.right_bottom(),
            );
            ui.allocate_new_ui(
                egui::UiBuilder::new()
                    .max_rect(bottom)
                    .layout(Layout::top_down(Align::Min)),
                |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        ui.spacing_mut().button_padding = Vec2::new(4.0, 4.0);
                        let bw = ((inner.width() - 18.0) / 4.0).floor();
                        control_buttons(ui, Vec2::new(bw, 30.0), cfg, st, acts);
                    });
                },
            );
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
                    if ui.add(egui::Button::new(RichText::new("×").size(20.0).color(DIM)).frame(false)).on_hover_text("Close").clicked() {
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

                    section(ui, "Health");
                    match &st.errors {
                        Some(errors) if st.link == Link::Online => {
                            egui::Grid::new("health").num_columns(2).spacing([24.0, 6.0]).show(ui, |ui| {
                                for (i, (name, sev)) in errors.items().into_iter().enumerate() {
                                    ui.horizontal(|ui| {
                                        dot(ui, sev_color(sev));
                                        ui.label(name);
                                        ui.label(RichText::new(match sev {
                                            Severity::Ok => "OK",
                                            Severity::Warning => "Warning",
                                            Severity::Error => "Error",
                                        })
                                        .color(sev_color(sev))
                                        .strong());
                                    });
                                    if i % 2 == 1 {
                                        ui.end_row();
                                    }
                                }
                            });
                        }
                        _ if st.link == Link::Online => {
                            ui.label(dim("This protocol does not report fault status. Use PJLink for full monitoring."));
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
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(format!("{} {mark} {}", e.time.format("%H:%M:%S"), e.text))
                                                .monospace()
                                                .color(color),
                                        )
                                        .truncate(),
                                    );
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
    if r.on_hover_text(if shut {
        "Shutter is closed. Click to open."
    } else {
        "Blank the picture"
    })
    .clicked()
    {
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

fn nav_row(
    ui: &mut egui::Ui,
    label: &str,
    count: usize,
    sev: Severity,
    selected: bool,
) -> egui::Response {
    let (rect, resp) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 34.0), Sense::click());
    let fill = if selected {
        mix(PANEL, ACCENT, 0.2)
    } else if resp.hovered() {
        CARD
    } else {
        Color32::TRANSPARENT
    };
    let p = ui.painter();
    p.rect_filled(rect, 8.0, fill);
    let font = egui::FontId::proportional(14.0);
    let mut right = rect.right() - 12.0;
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
    let galley = p.layout_no_wrap(label.to_string(), font, text_color);
    let clip = egui::Rect::from_min_max(rect.min, egui::pos2(right - 8.0, rect.max.y));
    p.with_clip_rect(clip).galley(
        egui::pos2(rect.left() + 12.0, rect.center().y - galley.size().y / 2.0),
        galley,
        text_color,
    );
    resp
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
