//! Modal-style windows: confirmation, add/edit, discovery, schedules.

use super::theme::*;
use super::{Act, App, Item};
use eframe::egui::{self, Align, Align2, Layout, RichText, Vec2};
use projectorhud::discovery::{self, Found};
use projectorhud::model::*;
use projectorhud::schedule::{Action, Schedule};
use projectorhud::worker::Target;

pub struct Editor {
    id: u64,
    name: String,
    host: String,
    port: String,
    protocol: Protocol,
    username: String,
    password: String,
    group: Option<String>,
    demo: bool,
    error: Option<String>,
    focus: bool,
}

impl Editor {
    pub fn new(group: Option<String>) -> Self {
        Self {
            id: 0,
            name: String::new(),
            host: String::new(),
            port: "4352".into(),
            protocol: Protocol::PjLink,
            username: String::new(),
            password: String::new(),
            group,
            demo: false,
            error: None,
            focus: true,
        }
    }

    pub fn edit(p: &ProjectorConfig) -> Self {
        Self {
            id: p.id,
            name: p.name.clone(),
            host: p.host.clone(),
            port: p.port.to_string(),
            protocol: p.protocol,
            username: p.username.clone(),
            password: p.password.clone(),
            group: p.group.clone(),
            demo: p.demo,
            error: None,
            focus: false,
        }
    }

    fn from_found(f: &Found, group: Option<String>) -> Self {
        Self {
            name: f.title(),
            host: f.ip.to_string(),
            port: f.port.to_string(),
            protocol: f.protocol,
            username: if f.protocol == Protocol::Panasonic {
                "admin1".into()
            } else {
                String::new()
            },
            error: Some("This projector needs a password.".into()),
            focus: false,
            ..Self::new(group)
        }
    }

    fn build(&self) -> Result<ProjectorConfig, String> {
        let host = self.host.trim();
        if host.is_empty() {
            return Err("Enter the projector's IP address or host name.".into());
        }
        let port: u16 = self
            .port
            .trim()
            .parse()
            .map_err(|_| "Port must be a number from 1 to 65535.".to_string())?;
        if port == 0 {
            return Err("Port must be a number from 1 to 65535.".into());
        }
        let name = if self.name.trim().is_empty() {
            host.to_string()
        } else {
            self.name.trim().to_string()
        };
        Ok(ProjectorConfig {
            id: self.id,
            name,
            host: host.to_string(),
            port,
            protocol: self.protocol,
            username: self.username.trim().to_string(),
            password: self.password.clone(),
            group: self.group.clone(),
            demo: self.demo,
        })
    }
}

fn found_config(f: &Found, group: Option<String>) -> ProjectorConfig {
    ProjectorConfig {
        id: 0,
        name: f.title(),
        host: f.ip.to_string(),
        port: f.port,
        protocol: f.protocol,
        username: String::new(),
        password: String::new(),
        group,
        demo: false,
    }
}

fn dialog(title: &str) -> egui::Window<'_> {
    egui::Window::new(RichText::new(title).size(17.0).strong())
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, [0.0, -20.0])
}

impl App {
    pub(super) fn dialogs(&mut self, ctx: &egui::Context, items: &[Item], acts: &mut Vec<Act>) {
        self.scan_dialog(ctx, items, acts);
        self.schedules_dialog(ctx, items);
        self.editor_dialog(ctx, acts);
        self.confirm_dialog(ctx, acts);
    }

    fn confirm_dialog(&mut self, ctx: &egui::Context, acts: &mut Vec<Act>) {
        let Some(c) = &self.confirm else { return };
        let mut decision: Option<bool> = None;
        dialog(&c.title).show(ctx, |ui| {
            ui.set_width(360.0);
            ui.label(dim(&c.body));
            ui.add_space(12.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if primary(ui, &c.button, if c.danger { RED } else { ACCENT }).clicked() {
                    decision = Some(true);
                }
                if ui.button("Cancel").clicked() {
                    decision = Some(false);
                }
            });
        });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            decision = Some(false);
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
            decision = Some(true);
        }
        if let Some(yes) = decision {
            let c = self.confirm.take().expect("checked above");
            if yes {
                acts.push(*c.act);
            }
        }
    }

    fn editor_dialog(&mut self, ctx: &egui::Context, acts: &mut Vec<Act>) {
        let groups = self.cfg.groups.clone();
        let Some(ed) = &mut self.editor else { return };
        let mut close = false;
        let mut submit = false;
        let title = if ed.id == 0 {
            "Add projector"
        } else {
            "Edit projector"
        };
        dialog(title).show(ctx, |ui| {
            ui.set_width(400.0);
            egui::Grid::new("editor").num_columns(2).spacing([14.0, 10.0]).show(ui, |ui| {
                let field = 270.0;
                ui.label(dim("Name"));
                ui.add(egui::TextEdit::singleline(&mut ed.name).hint_text("e.g. Main Stage Left").desired_width(field));
                ui.end_row();

                ui.label(dim("Address"));
                ui.horizontal(|ui| {
                    let r = ui.add(egui::TextEdit::singleline(&mut ed.host).hint_text("192.168.1.50").desired_width(field - 78.0));
                    if ed.focus {
                        r.request_focus();
                        ed.focus = false;
                    }
                    ui.add(egui::TextEdit::singleline(&mut ed.port).desired_width(52.0)).on_hover_text("Port");
                });
                ui.end_row();

                ui.label(dim("Protocol"));
                let before = ed.protocol;
                egui::ComboBox::from_id_salt("protocol").width(field).selected_text(ed.protocol.label()).show_ui(ui, |ui| {
                    ui.selectable_value(&mut ed.protocol, Protocol::PjLink, "PJLink (all brands, recommended)");
                    ui.selectable_value(&mut ed.protocol, Protocol::Panasonic, "Panasonic native");
                });
                if ed.protocol != before && ed.port.trim() == before.default_port().to_string() {
                    ed.port = ed.protocol.default_port().to_string();
                }
                ui.end_row();

                if ed.protocol == Protocol::Panasonic {
                    ui.label(dim("User name"));
                    ui.add(egui::TextEdit::singleline(&mut ed.username).hint_text("admin1").desired_width(field));
                    ui.end_row();
                }

                ui.label(dim("Password"));
                ui.add(
                    egui::TextEdit::singleline(&mut ed.password)
                        .password(true)
                        .hint_text("Leave empty if none is set")
                        .desired_width(field),
                );
                ui.end_row();

                ui.label(dim("Group"));
                egui::ComboBox::from_id_salt("group")
                    .width(field)
                    .selected_text(ed.group.clone().unwrap_or_else(|| "No group".into()))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut ed.group, None, "No group");
                        for g in &groups {
                            ui.selectable_value(&mut ed.group, Some(g.clone()), g);
                        }
                    });
                ui.end_row();
            });

            if ed.protocol == Protocol::Panasonic {
                ui.add_space(4.0);
                ui.label(small("Panasonic native control does not report fan, lamp or temperature faults. PJLink does."));
            }
            if let Some(err) = &ed.error {
                ui.add_space(4.0);
                ui.label(RichText::new(err).color(AMBER));
            }
            ui.add_space(12.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if primary(ui, if ed.id == 0 { "Add projector" } else { "Save" }, ACCENT).clicked() {
                    submit = true;
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
            });
        });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            close = true;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
            submit = true;
        }
        if submit {
            match ed.build() {
                Ok(cfg) => {
                    acts.push(Act::SaveProjector(cfg));
                    close = true;
                }
                Err(e) => ed.error = Some(e),
            }
        }
        if close {
            self.editor = None;
        }
    }

    fn scan_dialog(&mut self, ctx: &egui::Context, items: &[Item], acts: &mut Vec<Act>) {
        if !self.show_scan {
            return;
        }
        let mut open = true;
        let mut rescan = false;
        let group = match &self.view {
            Target::Group(g) => Some(g.clone()),
            _ => None,
        };
        let known = |f: &Found| {
            items
                .iter()
                .any(|i| i.0.host == f.ip.to_string() && i.0.protocol == f.protocol)
        };

        dialog("Discover projectors").open(&mut open).show(ctx, |ui| {
            ui.set_width(520.0);
            let Some(scan) = &self.scan else { return };
            let results = scan.results.lock().unwrap().clone();
            let running = scan.is_running();

            ui.horizontal(|ui| {
                if running {
                    ui.spinner();
                    ui.label(dim(format!("Searching {}…", scan.subnets.lock().unwrap().join(", "))));
                } else {
                    ui.label(dim(format!(
                        "Found {} projector{} on {}",
                        results.len(),
                        if results.len() == 1 { "" } else { "s" },
                        scan.subnets.lock().unwrap().join(", ")
                    )));
                }
            });
            if running {
                ui.add(egui::ProgressBar::new(scan.progress()).desired_height(4.0).desired_width(ui.available_width()));
            }
            ui.add_space(6.0);

            egui::ScrollArea::vertical().max_height(320.0).auto_shrink([false, true]).show(ui, |ui| {
                if results.is_empty() && !running {
                    ui.add_space(10.0);
                    ui.label("Nothing answered.");
                    ui.label(small(
                        "Check that network control (PJLink) is enabled in each projector's menu and that this \
                         computer is on the same network. Projectors on another subnet can be scanned below.",
                    ));
                    ui.add_space(10.0);
                }
                for f in &results {
                    egui::Frame::none().fill(CARD).rounding(10.0).inner_margin(egui::Margin::symmetric(14.0, 10.0)).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = 2.0;
                                ui.label(RichText::new(f.title()).strong());
                                let mut line = format!("{}  ·  {}", f.ip, f.protocol.label());
                                let model = format!("{} {}", f.manufacturer.trim(), f.model.trim());
                                if !model.trim().is_empty() && model.trim() != f.title() {
                                    line.push_str(&format!("  ·  {}", model.trim()));
                                }
                                if f.needs_password {
                                    line.push_str("  ·  password required");
                                }
                                ui.label(small(line));
                            });
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if known(f) {
                                    ui.label(RichText::new("✔ Added").color(GREEN));
                                } else if ui.button("Add").clicked() {
                                    if f.needs_password {
                                        self.editor = Some(Editor::from_found(f, group.clone()));
                                    } else {
                                        acts.push(Act::SaveProjector(found_config(f, group.clone())));
                                    }
                                }
                            });
                        });
                    });
                }
            });

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(dim("Another subnet"));
                ui.add(egui::TextEdit::singleline(&mut self.scan_subnet).hint_text("e.g. 10.0.20").desired_width(110.0));
                if ui.add_enabled(!running, egui::Button::new("Scan again")).clicked() {
                    rescan = true;
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let new: Vec<&Found> = results.iter().filter(|f| !known(f)).collect();
                    if !new.is_empty() && primary(ui, &format!("Add all {}", new.len()), ACCENT).clicked() {
                        for f in new {
                            acts.push(Act::SaveProjector(found_config(f, group.clone())));
                        }
                    }
                });
            });
        });

        if rescan {
            let extra = discovery::parse_prefix(&self.scan_subnet)
                .into_iter()
                .collect();
            self.scan = Some(discovery::start(extra, self.repaint.clone()));
        }
        if !open
            || (self.editor.is_none()
                && self.confirm.is_none()
                && ctx.input(|i| i.key_pressed(egui::Key::Escape)))
        {
            self.show_scan = false;
            self.scan = None;
        }
    }

    fn schedules_dialog(&mut self, ctx: &egui::Context, items: &[Item]) {
        if !self.show_schedules {
            return;
        }
        let mut open = true;
        let before = self.cfg.schedules.clone();
        let mut add = false;
        let mut delete: Option<u64> = None;

        let mut targets: Vec<(Target, String)> = vec![(Target::All, "All projectors".into())];
        targets.extend(
            self.cfg
                .groups
                .iter()
                .map(|g| (Target::Group(g.clone()), format!("Group: {g}"))),
        );
        targets.extend(
            items
                .iter()
                .map(|i| (Target::Projector(i.0.id), i.0.name.clone())),
        );
        let target_label = |t: &Target| {
            targets
                .iter()
                .find(|x| &x.0 == t)
                .map(|x| x.1.clone())
                .unwrap_or_else(|| "(removed)".into())
        };

        dialog("Schedules").open(&mut open).show(ctx, |ui| {
            ui.set_width(700.0);
            ui.label(dim("Send commands automatically at set times. Schedules run while ProjectorHUD is open."));
            ui.add_space(6.0);
            if self.cfg.schedules.is_empty() {
                ui.add_space(8.0);
                ui.label("No schedules yet.");
                ui.add_space(8.0);
            }
            egui::ScrollArea::vertical().max_height(360.0).auto_shrink([false, true]).show(ui, |ui| {
                for s in self.cfg.schedules.iter_mut() {
                    egui::Frame::none().fill(CARD).rounding(10.0).inner_margin(egui::Margin::symmetric(14.0, 10.0)).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            ui.checkbox(&mut s.enabled, "").on_hover_text("Enabled");
                            ui.add(egui::DragValue::new(&mut s.hour).range(0..=23).custom_formatter(|n, _| format!("{:02}", n as u32)));
                            ui.label(":");
                            ui.add(egui::DragValue::new(&mut s.minute).range(0..=59).custom_formatter(|n, _| format!("{:02}", n as u32)));
                            ui.add_space(6.0);
                            egui::ComboBox::from_id_salt(("action", s.id)).width(118.0).selected_text(s.action.label()).show_ui(ui, |ui| {
                                for a in Action::ALL {
                                    ui.selectable_value(&mut s.action, a, a.label());
                                }
                            });
                            egui::ComboBox::from_id_salt(("target", s.id)).width(150.0).selected_text(target_label(&s.target)).show_ui(ui, |ui| {
                                for (t, label) in &targets {
                                    ui.selectable_value(&mut s.target, t.clone(), label);
                                }
                            });
                            ui.add_space(6.0);
                            ui.spacing_mut().item_spacing.x = 3.0;
                            ui.spacing_mut().button_padding = Vec2::new(2.0, 4.0);
                            for (i, d) in ["M", "T", "W", "T", "F", "S", "S"].iter().enumerate() {
                                if toggle(ui, Vec2::new(24.0, 26.0), d, s.days[i].then_some(ACCENT), true)
                                    .on_hover_text(["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"][i])
                                    .clicked()
                                {
                                    s.days[i] = !s.days[i];
                                }
                            }
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if ui.add(egui::Button::new(RichText::new("×").size(20.0).color(DIM)).frame(false)).on_hover_text("Delete schedule").clicked() {
                                    delete = Some(s.id);
                                }
                            });
                        });
                    });
                }
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("➕ Add schedule").clicked() {
                    add = true;
                }
                if let Some(last) = self.last_run.lock().unwrap().clone() {
                    ui.label(small(format!("Last run: {last}")));
                }
            });
        });

        if add {
            let id = self.cfg.alloc_id();
            self.cfg.schedules.push(Schedule::new(id));
        }
        if let Some(id) = delete {
            self.cfg.schedules.retain(|s| s.id != id);
        }
        if self.cfg.schedules != before {
            self.save();
        }
        if !open || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.show_schedules = false;
        }
    }
}
